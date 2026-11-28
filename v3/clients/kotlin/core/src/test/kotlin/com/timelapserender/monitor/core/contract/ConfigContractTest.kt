// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.state.CONFIG_CONFLICT_MESSAGE
import com.timelapserender.monitor.core.state.ConfigEditorModel
import com.timelapserender.monitor.core.state.ConfigEditorUi
import com.timelapserender.monitor.core.state.DirectoryBrowserModel
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigPathSource
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.RenderVariant
import io.grpc.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files

class ConfigContractTest {
    private fun Config.withFirstCodec(codec: String): Config =
        toBuilder().setRendering(0, getRendering(0).toBuilder().setCodec(codec).build()).build()

    private suspend fun ConfigEditorModel.loaded(): ConfigEditorUi {
        load()
        return state.reach("the config loads") { !it.loading && it.config != null }
    }

    private suspend fun ConfigEditorModel.validated(): ConfigEditorUi =
        state.reach("the validation answers") { !it.validationPending }

    private suspend fun ConfigEditorModel.saved(): ConfigEditorUi {
        save()
        return state.reach("the save ends") { !it.saving }
    }

    private fun TrdProcess.renderingCodecs(): List<Any?> =
        (config()["rendering"] as List<*>).map { (it as Map<*, *>)["codec"] }

    @Test
    fun configEditorLoadsValidatesSavesAndDetectsAConflict() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val client = trd.connected()
            val model = ConfigEditorModel(client, trd.scope)
            val loaded = model.loaded()
            assertNull(loaded.error)
            assertEquals(listOf("ProResMQ", "h264", "ProResUHQ"), loaded.config!!.renderingList.map { it.codec })
            assertEquals(listOf("ProResMQ", "ProResUHQ", "h264"), loaded.codecs.map { it.name })
            assertFalse(loaded.dirty)

            // An error is reported on its field and blocks the save.
            model.edit { it.withFirstCodec("no-such-codec") }
            val invalid = model.validated()
            val issues = invalid.issuesByField["/rendering/0/codec"].orEmpty()
            assertTrue(invalid.issues.toString(), issues.any { it.severity == IssueSeverity.ISSUE_ERROR })
            assertFalse(invalid.canSave)
            val checked = client.validateConfig(invalid.config!!)
            assertTrue(checked.issuesList.any { it.field == "/rendering/0/codec" })

            model.edit { it.withFirstCodec("ProResMQ").toBuilder().setRecentRenderHistoryDays(7).build() }
            val valid = model.validated()
            assertTrue(valid.issues.toString(), valid.canSave)
            assertEquals(listOf(PRORES_MQ, H264, PRORES_UHQ), valid.rendering.map { it.variantKey })
            val saved = model.saved()
            assertNull(saved.error)
            assertFalse(saved.dirty)
            assertEquals(7L, trd.config()["recent_render_history_days"])
            assertEquals(7, client.getConfig().config.recentRenderHistoryDays)

            // The file changes underneath the editor.
            trd.editConfigFile { it["recent_render_history_days"] = 9 }
            model.edit { it.toBuilder().setRecentRenderHistoryDays(11).build() }
            assertTrue(model.validated().canSave)
            val conflicted = model.saved()
            assertTrue(conflicted.toString(), conflicted.conflict)
            assertEquals(CONFIG_CONFLICT_MESSAGE, conflicted.error)
            assertEquals("a refused save writes nothing", 9L, trd.config()["recent_render_history_days"])

            model.reload()
            val reloaded = model.state.reach("the reload") { !it.loading && it.config?.recentRenderHistoryDays == 9 }
            assertFalse(reloaded.conflict)
            assertFalse(reloaded.dirty)
            model.edit { it.toBuilder().setRecentRenderHistoryDays(11).build() }
            assertTrue(model.validated().canSave)
            assertNull(model.saved().error)
            assertEquals(11L, trd.config()["recent_render_history_days"])

            // UpdateConfig itself, with a stale revision.
            val state = client.getConfig()
            val stale = runCatching { client.updateConfig(state.config, "0".repeat(64)) }
            assertEquals(Status.Code.FAILED_PRECONDITION, Status.fromThrowable(stale.exceptionOrNull()!!).code)
        }
    }

    @Test
    fun updateCodecsWritesTheCodecTable() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val client = trd.connected()
            val before = client.getConfig()
            val model = ConfigEditorModel(client, trd.scope)
            model.loaded()

            model.editCodecs { codecs ->
                val h264 = codecs.single { it.name == "h264" }
                codecs + h264.toBuilder()
                    .setName("h264Draft")
                    .setFilename(h264.filename.toBuilder().setQuality("DRAFT"))
                    .build()
            }
            model.edit {
                it.toBuilder()
                    .addRendering(RenderVariant.newBuilder().setCodec("h264Draft").setFrameRate(24).addResolution(640))
                    .build()
            }
            val valid = model.validated()
            assertTrue(valid.issues.toString(), valid.canSave)
            val saved = model.saved()
            assertNull(saved.error)
            assertFalse(saved.dirty)

            val after = client.getConfig()
            assertTrue(after.codecsList.any { it.name == "h264Draft" })
            assertNotEquals(before.codecsRevision, after.codecsRevision)
            assertEquals("h264Draft", trd.renderingCodecs().last())
            assertTrue("h264Draft" in trd.readJson(trd.tree.resolve("video_codecs.json")).keys)

            val stale = runCatching { client.updateCodecs(before.codecsList, before.codecsRevision) }
            assertEquals(Status.Code.FAILED_PRECONDITION, Status.fromThrowable(stale.exceptionOrNull()!!).code)

            // The table without a codec that a rendering entry uses is refused.
            val removed = runCatching { client.updateCodecs(before.codecsList, after.codecsRevision) }
            assertEquals(Status.Code.INVALID_ARGUMENT, Status.fromThrowable(removed.exceptionOrNull()!!).code)
        }
    }

    @Test
    fun setConfigPathCreatesAndSelectsAnotherFile() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val client = trd.connected()
            assertEquals(ConfigPathSource.CONFIG_PATH_FLAG, client.getConfig().pathSource)
            val other = trd.root.resolve("other").resolve("config.json")

            val state = client.setConfigPath(other.toString(), true)
            assertEquals(other.toString(), state.path)
            assertEquals(ConfigPathSource.CONFIG_PATH_SETTING, state.pathSource)
            assertTrue(state.exists)
            assertTrue(Files.isRegularFile(other))
            assertEquals(other.toString(), client.getConfig().path)

            val relative = runCatching { client.setConfigPath("config.json", false) }
            assertEquals(Status.Code.INVALID_ARGUMENT, Status.fromThrowable(relative.exceptionOrNull()!!).code)
        }
    }

    @Test
    fun directoryBrowserListsTheSandbox() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val client = trd.connected()
            val browser = DirectoryBrowserModel(client, trd.scope)
            browser.openRoots()
            val roots = browser.state.reach("the roots") { !it.loading && it.roots.isNotEmpty() }
            assertTrue(roots.atRoots)
            assertTrue(roots.roots.toString(), trd.home.toString() in roots.roots)

            browser.navigate(trd.tree.toString())
            val listing = browser.state.reach("the sandbox") { !it.loading && it.path == trd.tree.toString() }
            assertNull(listing.error)
            assertEquals(trd.root.toString(), listing.parent)
            val names = listing.entries.map { it.name }
            assertTrue(names.toString(), names.containsAll(listOf("backup", "export", "logs", "master", "raw")))
            assertTrue("directories only", listing.entries.all { it.isDir })

            browser.up()
            val above = browser.state.reach("the parent") { !it.loading && it.path == trd.root.toString() }
            assertTrue(above.entries.any { it.name == "tree" })

            browser.navigate(trd.root.resolve("missing").toString())
            val missing = browser.state.reach("the failure") { !it.loading && it.error != null }
            assertEquals("the previous listing stays", trd.root.toString(), missing.path)

            val all = client.listDirectory(trd.tree.toString(), includeFiles = true, showHidden = true)
            assertTrue(all.entriesList.any { it.name == "config.json" && !it.isDir })
            assertTrue(all.entriesList.any { it.name == ".trd-sandbox" })
            val visible = client.listDirectory(trd.tree.toString(), includeFiles = true)
            assertTrue("hidden entries are left out", visible.entriesList.none { it.name == ".trd-sandbox" })
        }
    }
}
