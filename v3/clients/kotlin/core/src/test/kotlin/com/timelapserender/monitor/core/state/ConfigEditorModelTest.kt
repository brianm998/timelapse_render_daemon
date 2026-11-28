// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigIssue
import com.timelapserender.trd.v1.ConfigState
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.RenderVariant
import com.timelapserender.trd.v1.UpdateCodecsRequest
import com.timelapserender.trd.v1.UpdateConfigRequest
import com.timelapserender.trd.v1.ValidateConfigRequest
import com.timelapserender.trd.v1.ValidateConfigResponse
import com.timelapserender.trd.v1.VideoCodec
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

@OptIn(ExperimentalCoroutinesApi::class)
class ConfigEditorModelTest {
    private val codecA = VideoCodec.newBuilder().setName("a").build()
    private val codecB = VideoCodec.newBuilder().setName("b").build()
    private val baseConfig = Config.newBuilder().addExportPaths("/exp").setRecentRenderHistoryDays(30).build()
    private val base = ConfigState.newBuilder()
        .setPath("/cfg.json").setExists(true).setConfig(baseConfig).setRevision("rev1")
        .addCodecs(codecA).setCodecsRevision("crev1")
        .addIssues(ConfigIssue.newBuilder().setSeverity(IssueSeverity.ISSUE_WARNING).setField("/x").setMessage("loaded warning"))
        .build()

    private fun issue(severity: IssueSeverity, field: String, message: String) =
        ConfigIssue.newBuilder().setSeverity(severity).setField(field).setMessage(message).build()

    private fun addPath(path: String): (Config) -> Config = { it.toBuilder().addExportPaths(path).build() }

    private suspend fun loaded(env: Env, scope: CoroutineScope): ConfigEditorModel {
        env.fake.configState = base
        val m = ConfigEditorModel(env.client, scope)
        m.load()
        m.state.first { !it.loading && it.config != null }
        return m
    }

    /** Lets the 400 ms of quiet pass in virtual time and waits for the validation to answer. */
    private suspend fun TestScope.settle(m: ConfigEditorModel) {
        testScheduler.advanceTimeBy(400)
        testScheduler.runCurrent()
        m.state.first { !it.validationPending }
    }

    @Test
    fun loadExposesTheConfigAndItsIssues() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            val ui = m.state.value
            assertEquals(baseConfig, ui.config)
            assertEquals(listOf(codecA), ui.codecs)
            assertEquals(listOf("loaded warning"), ui.issuesByField["/x"]!!.map { it.message })
            assertFalse(ui.dirty)
            assertTrue(ui.canSave)
            assertFalse(ui.conflict)
        }
    }

    @Test
    fun aLoadNeverPublishesALoadedStateWithoutAConfig() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.configState = base
            val m = ConfigEditorModel(env.client, backgroundScope)
            val seen = mutableListOf<ConfigEditorUi>()
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { m.state.collect { seen += it } }
            m.load()
            m.state.first { !it.loading && it.config != null }
            assertEquals(emptyList<ConfigEditorUi>(), seen.filter { !it.loading && it.config == null })
        }
    }

    @Test
    fun theFilePathAndRawTextFollowTheLoadAndTheSave() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.configState = base.toBuilder().setRawJson("{}").build()
            val m = ConfigEditorModel(env.client, backgroundScope)
            m.load()
            val ui = m.state.first { !it.loading && it.config != null }
            assertEquals("/cfg.json", ui.path)
            assertEquals("{}", ui.rawJson)
            env.fake.updateConfigResult = env.fake.configState.toBuilder()
                .setConfig(addPath("/x")(baseConfig)).setRawJson("{ \"saved\": true }").build()
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            assertEquals("{ \"saved\": true }", m.state.first { !it.saving }.rawJson)
        }
    }

    @Test
    fun validatesOnlyAfter400msOfQuiet() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.validateResponse = ValidateConfigResponse.newBuilder()
                .addIssues(issue(IssueSeverity.ISSUE_ERROR, "/rendering/2/codec", "unknown codec"))
                .addRendering(RenderVariant.newBuilder().setVariantKey("k1").setExampleFilename("LRT_example_k1.mov"))
                .build()
            val m = loaded(env, backgroundScope)
            m.edit(addPath("/one"))
            testScheduler.advanceTimeBy(300)
            testScheduler.runCurrent()
            m.edit(addPath("/two")) // restarts the quiet period
            testScheduler.advanceTimeBy(399)
            testScheduler.runCurrent()
            assertEquals(0, env.fake.callCount("ValidateConfig"))
            assertTrue(m.state.value.validationPending)
            assertFalse(m.state.value.canSave)
            testScheduler.advanceTimeBy(1)
            testScheduler.runCurrent()
            val ui = m.state.first { !it.validationPending }
            assertEquals(1, env.fake.callCount("ValidateConfig"))
            val request = env.fake.requestsOf<ValidateConfigRequest>().single()
            assertEquals(listOf("/exp", "/one", "/two"), request.config.exportPathsList)
            assertEquals(0, request.codecsCount) // codecs unchanged: the server's current table
            assertEquals(listOf("unknown codec"), ui.issuesByField["/rendering/2/codec"]!!.map { it.message })
            assertEquals("LRT_example_k1.mov", ui.rendering.single().exampleFilename)
        }
    }

    @Test
    fun saveIsDisabledWhileAnErrorIssueExists() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.validateResponse = ValidateConfigResponse.newBuilder()
                .addIssues(issue(IssueSeverity.ISSUE_ERROR, "/export_paths/1", "does not exist")).build()
            val m = loaded(env, backgroundScope)
            m.edit(addPath("/bad"))
            settle(m)
            assertTrue(m.state.value.hasErrors)
            assertFalse(m.state.value.canSave)
            m.save()
            assertEquals(0, env.fake.callCount("UpdateConfig"))

            env.fake.validateResponse = ValidateConfigResponse.newBuilder()
                .addIssues(issue(IssueSeverity.ISSUE_WARNING, "/export_paths/1", "slow disk")).build()
            m.edit { it.toBuilder().clearExportPaths().addExportPaths("/exp").addExportPaths("/ok").build() }
            settle(m)
            assertFalse(m.state.value.hasErrors)
            assertTrue(m.state.value.canSave)
        }
    }

    @Test
    fun saveSendsTheLoadedRevisionsAndUpdatesCodecsOnlyWhenTheyChanged() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            val edited = addPath("/x")(baseConfig)
            env.fake.updateConfigResult = base.toBuilder().setConfig(edited).setRevision("rev2").build()
            env.fake.updateCodecsResult = base.toBuilder().setConfig(edited).setRevision("rev2")
                .clearCodecs().addCodecs(codecA).addCodecs(codecB).setCodecsRevision("crev2").build()
            m.edit(addPath("/x"))
            m.editCodecs { it + codecB }
            settle(m)
            assertEquals(2, env.fake.requestsOf<ValidateConfigRequest>().single().codecsCount)
            assertTrue(m.state.value.dirty)
            m.save()
            val saved = m.state.first { !it.saving }
            assertEquals("rev1", env.fake.requestsOf<UpdateConfigRequest>().single().expectedRevision)
            val codecsRequest = env.fake.requestsOf<UpdateCodecsRequest>().single()
            assertEquals("crev1", codecsRequest.expectedRevision)
            assertEquals(listOf(codecA, codecB), codecsRequest.codecsList)
            assertFalse(saved.dirty)
            assertNull(saved.error)
            assertEquals(edited, saved.config)

            // the next save carries the new revision, and unchanged codecs are not sent again
            env.fake.updateConfigResult = env.fake.updateCodecsResult!!.toBuilder().setConfig(addPath("/y")(edited)).setRevision("rev3").build()
            m.edit(addPath("/y"))
            settle(m)
            m.save()
            m.state.first { !it.saving }
            assertEquals(listOf("rev1", "rev2"), env.fake.requestsOf<UpdateConfigRequest>().map { it.expectedRevision })
            assertEquals(1, env.fake.callCount("UpdateCodecs"))
        }
    }

    @Test
    fun aConfigOnlyEditDoesNotTouchTheCodecs() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            env.fake.updateConfigResult = base.toBuilder().setConfig(addPath("/x")(baseConfig)).setRevision("rev2").build()
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            m.state.first { !it.saving }
            assertEquals(1, env.fake.callCount("UpdateConfig"))
            assertEquals(0, env.fake.callCount("UpdateCodecs"))
        }
    }

    @Test
    fun failedPreconditionSetsConflictAndReloadRefreshes() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            env.fake.failWith["UpdateConfig"] = Status.FAILED_PRECONDITION.withDescription("revision mismatch")
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            val conflict = m.state.first { it.conflict }
            assertEquals("The config changed on the server", conflict.error)
            assertFalse(conflict.saving)
            assertTrue(conflict.dirty) // the user's edits are still there

            env.fake.failWith.clear()
            val other = Config.newBuilder().addExportPaths("/other").build()
            env.fake.configState = base.toBuilder().setConfig(other).setRevision("rev9").build()
            m.reload()
            val reloaded = m.state.first { !it.conflict && it.config?.exportPathsList == listOf("/other") }
            assertFalse(reloaded.dirty)
            assertNull(reloaded.error)
        }
    }

    @Test
    fun otherSaveFailuresAreReportedWithoutAConflict() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            env.fake.failWith["UpdateConfig"] = Status.INVALID_ARGUMENT.withDescription("bad value")
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            val ui = m.state.first { it.error != null }
            assertEquals("bad value", ui.error)
            assertFalse(ui.conflict)
        }
    }

    @Test
    fun dirtyIsTrueOnlyWhileTheCopyDiffersFromTheLoadedConfig() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            m.edit(addPath("/x"))
            assertTrue(m.state.value.dirty)
            m.edit { it.toBuilder().clearExportPaths().addExportPaths("/exp").build() }
            assertFalse(m.state.value.dirty)
            m.editCodecs { it + codecB }
            assertTrue(m.state.value.dirty)
            m.editCodecs { it.dropLast(1) }
            assertFalse(m.state.value.dirty)
        }
    }

    @Test
    fun aFailedLoadIsReported() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["GetConfig"] = Status.UNAVAILABLE.withDescription("not ready")
            val m = ConfigEditorModel(env.client, backgroundScope)
            m.load()
            val ui = m.state.first { it.error != null }
            assertEquals("not ready", ui.error)
            assertFalse(ui.loading)
            assertFalse(ui.canSave)
        }
    }

    @Test
    fun anExternalCodecChangeIsNotOverwrittenByAConfigOnlyEdit() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            // video_codecs.json changed on the server since the load
            env.fake.updateConfigResult = base.toBuilder().setConfig(addPath("/x")(baseConfig)).setRevision("rev2")
                .addCodecs(codecB).setCodecsRevision("external").build()
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            val saved = m.state.first { !it.saving }
            assertEquals(0, env.fake.callCount("UpdateCodecs"))
            assertEquals(listOf(codecA, codecB), saved.codecs)
            assertFalse(saved.dirty)
        }
    }

    @Test
    fun editedCodecsKeepTheLoadedRevisionSoAnExternalChangeConflictsOnEveryTry() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            env.fake.updateConfigResult = base.toBuilder().setConfig(addPath("/x")(baseConfig)).setRevision("rev2")
                .setCodecsRevision("external").build()
            env.fake.failWith["UpdateCodecs"] = Status.FAILED_PRECONDITION
            m.edit(addPath("/x"))
            m.editCodecs { it + codecB }
            settle(m)
            m.save()
            val first = m.state.first { it.conflict }
            assertEquals(listOf(codecA, codecB), first.codecs)
            assertTrue(first.dirty)

            env.fake.failWith.clear()
            env.fake.updateCodecsResult = env.fake.updateConfigResult
            m.save()
            m.state.first { !it.saving }
            assertEquals("crev1", env.fake.requestsOf<UpdateCodecsRequest>().single().expectedRevision)
            assertEquals(1, env.fake.callCount("UpdateConfig")) // the saved config is not sent again
        }
    }

    @Test
    fun aCodecThatTheConfigStartsUsingIsSavedFirst() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            val use: (Config) -> Config = { it.toBuilder().addRendering(RenderVariant.newBuilder().setCodec("b")).build() }
            env.fake.updateCodecsResult = base.toBuilder().addCodecs(codecB).setCodecsRevision("crev2").build()
            env.fake.updateConfigResult = env.fake.updateCodecsResult!!.toBuilder().setConfig(use(baseConfig)).setRevision("rev2").build()
            m.editCodecs { it + codecB }
            m.edit(use)
            settle(m)
            m.save()
            val saved = m.state.first { !it.saving }
            assertEquals(
                listOf("UpdateCodecs", "UpdateConfig"),
                env.fake.calls.map { it.method }.filter { it.startsWith("Update") },
            )
            assertEquals("rev1", env.fake.requestsOf<UpdateConfigRequest>().single().expectedRevision)
            assertFalse(saved.dirty)
        }
    }

    @Test
    fun anEditDuringASaveIsKeptAndNoSecondSaveStarts() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val m = loaded(env, backgroundScope)
            env.fake.updateConfigResult = base.toBuilder().setConfig(addPath("/x")(baseConfig)).setRevision("rev2").build()
            m.edit(addPath("/x"))
            settle(m)
            m.save()
            // the test dispatcher has not run the save yet
            m.edit(addPath("/y"))
            assertTrue(m.state.value.saving)
            assertFalse(m.state.value.canSave)
            m.save()
            val saved = m.state.first { !it.saving && !it.validationPending }
            assertEquals(1, env.fake.callCount("UpdateConfig"))
            assertEquals(listOf("/exp", "/x", "/y"), saved.config!!.exportPathsList)
            assertTrue(saved.dirty)
        }
    }
}
