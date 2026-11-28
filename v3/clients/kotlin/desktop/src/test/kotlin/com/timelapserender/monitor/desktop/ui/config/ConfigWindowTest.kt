// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.timelapserender.monitor.core.state.DirectoryUi
import com.timelapserender.trd.v1.ConfigIssue
import com.timelapserender.trd.v1.DirEntry
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.ListDirectoryRequest
import com.timelapserender.trd.v1.ListDirectoryResponse
import com.timelapserender.trd.v1.RenderVariant
import com.timelapserender.trd.v1.SetConfigPathRequest
import com.timelapserender.trd.v1.ValidateConfigRequest
import com.timelapserender.trd.v1.ValidateConfigResponse
import io.grpc.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigWindowTest : ConfigTestBase() {
    private fun dir(path: String, name: String = path.substringAfterLast('/'), isDir: Boolean = true) =
        DirEntry.newBuilder().setName(name).setPath(path).setIsDir(isDir).build()

    private fun listing(path: String, parent: String, vararg entries: DirEntry) =
        ListDirectoryResponse.newBuilder().setPath(path).setParent(parent).addAllEntries(entries.toList()).build()

    private fun folders() {
        fake.directories = mapOf(
            "" to ListDirectoryResponse.newBuilder().addRoots("/").addRoots("/home/me").build(),
            "/" to listing("/", "", dir("/media"), dir("/src")),
            "/media" to listing("/media", "/", dir("/media/shots")),
            "/src" to listing("/src", "/"),
            "/etc/trd" to listing(
                "/etc/trd", "/etc", dir("/etc/trd/old"), dir("/etc/trd/other.json", isDir = false),
            ),
        )
    }

    private fun chooseMode(mode: ResolutionMode) =
        node(RenderingTags.choice(RenderingTags.mode(0), mode.ordinal)).performClick()

    private fun assertIssueUnder(field: String, text: String) {
        compose.onNodeWithText(text).assert(hasAnyAncestor(hasTestTag(ConfigTags.issues(field))))
        val input = node(ConfigTags.field(field)).fetchSemanticsNode().boundsInRoot
        val issue = compose.onNodeWithText(text).fetchSemanticsNode().boundsInRoot
        assertTrue("the issue is below the field", issue.top >= input.bottom - 1)
    }

    @Test
    fun anInvalidRegexShowsTheServersIssueUnderTheFieldAndDisablesSave() {
        show()
        open(ConfigGroup.MATCHING)
        val field = "/ignore_sequence_regexes/0"
        answer(error(field, "\"([\" does not compile as a regular expression"))
        type(ConfigTags.field(field), "([")
        settle()
        assertIssueUnder(field, "Error: \"([\" does not compile as a regular expression")
        node(ConfigTags.SAVE).assertIsNotEnabled()
        compose.onNodeWithTag(ConfigTags.issueCount(ConfigGroup.MATCHING), useUnmergedTree = true)
            .assertTextEquals("1")
        compose.onNodeWithTag(ConfigTags.issueCount(ConfigGroup.LOCATIONS), useUnmergedTree = true)
            .assertDoesNotExist()
        val validated = fake.requestsOf<ValidateConfigRequest>().last()
        assertEquals(listOf("(["), validated.config.ignoreSequenceRegexesList)
        controller.save()
        compose.waitForIdle()
        assertTrue(configUpdates().isEmpty())

        answer()
        type(ConfigTags.field(field), "^old")
        settle()
        compose.onNodeWithTag(ConfigTags.issues(field)).assertDoesNotExist()
        node(ConfigTags.SAVE).assertIsEnabled()
    }

    @Test
    fun aPathAddedThroughTheFolderBrowserIsSavedWithTheLoadedRevision() {
        folders()
        show()
        node(ConfigTags.SAVE).assertIsNotEnabled()
        click(ListTags.add("export_paths"))
        awaitTag(BrowserTags.open("/"))
        node(BrowserTags.UP).assertIsNotEnabled()
        node(BrowserTags.CHOOSE).assertIsNotEnabled()
        node(BrowserTags.open("/")).performClick()
        awaitTag(BrowserTags.entry("/media"))
        node(BrowserTags.entry("/media")).performClick()
        val saved = baseConfig.toBuilder().addExportPaths("/media").build()
        fake.updateConfigResult = state(saved, revision = "rev2")
        node(BrowserTags.CHOOSE).performClick()
        compose.onNodeWithTag(BrowserTags.CHOOSE).assertDoesNotExist()
        settle()
        node(ConfigTags.SAVE).assertIsEnabled().performClick()
        await { !controller.editor.state.value.dirty }
        val request = configUpdates().single()
        assertEquals("rev1", request.expectedRevision)
        assertEquals(listOf("/exp", "/media"), request.config.exportPathsList)
        assertTrue(codecUpdates().isEmpty())
    }

    @Test
    fun theFolderBrowserStartsAtTheCurrentValueAndGoesUp() {
        folders()
        show()
        click(ListTags.browse("/source_paths/0"))
        await { fake.requestsOf<ListDirectoryRequest>().isNotEmpty() }
        assertEquals("/src", fake.requestsOf<ListDirectoryRequest>().first().path)
        awaitTag(BrowserTags.UP)
        await {
            compose.onAllNodes(hasTestTag(BrowserTags.UP) and isEnabled()).fetchSemanticsNodes().isNotEmpty()
        }
        node(BrowserTags.UP).performClick()
        awaitTag(BrowserTags.open("/media"))
        node(BrowserTags.open("/media")).performClick()
        awaitTag(BrowserTags.entry("/media/shots"))
        // With nothing picked, the folder being shown is chosen; it replaces the browsed entry.
        node(BrowserTags.CHOOSE).performClick()
        settle()
        assertEquals(listOf("/media"), config.sourcePathsList)
    }

    @Test
    fun aValueThatIsNoFolderStartsAtTheRootsAndCancelChangesNothing() {
        folders()
        show()
        click(ListTags.browse("/export_paths/0"))
        awaitTag(BrowserTags.open("/home/me"))
        node(BrowserTags.CANCEL).performClick()
        compose.onNodeWithTag(BrowserTags.CANCEL).assertDoesNotExist()
        assertEquals(baseConfig, config)
        assertFalse(controller.dirty)
    }

    @Test
    fun pathsCanBeTypedReorderedAndRemoved() {
        show()
        type(ConfigTags.field("/export_paths/0"), "/exp/\$YEAR")
        assertEquals(listOf("/exp/\$YEAR"), config.exportPathsList)
        type(ConfigTags.field("/source_paths/0"), "/src2")
        click(ListTags.remove("/source_paths/0"))
        assertTrue(config.sourcePathsList.isEmpty())
        open(ConfigGroup.MATCHING)
        click(ListTags.add("raw_types"))
        type(ConfigTags.field("/raw_types/1"), "DNG")
        click(ListTags.up("/raw_types/1"))
        assertEquals(listOf("DNG", "ARW"), config.rawTypesList)
        click(ListTags.down("/raw_types/0"))
        assertEquals(listOf("ARW", "DNG"), config.rawTypesList)
        node(ListTags.up("/raw_types/0")).assertIsNotEnabled()
        node(ListTags.down("/raw_types/1")).assertIsNotEnabled()
        click(ListTags.remove("/raw_types/0"))
        assertEquals(listOf("DNG"), config.rawTypesList)
        type(ConfigTags.field("/image_extensions/0"), "tiff")
        type(ConfigTags.field("/image_name_prefix"), "DSC_")
        type(ConfigTags.field("/image_sequence_dirname_prefix"), "seq_")
        assertEquals(listOf("tiff"), config.imageExtensionsList)
        assertEquals("DSC_", config.imageNamePrefix)
        assertEquals("seq_", config.imageSequenceDirnamePrefix)
    }

    @Test
    fun everyLocationListIsEdited() {
        show()
        for (key in listOf("source_paths", "backup_paths", "export_paths", "master_videos")) {
            node(ListTags.add(key)).assertExists()
        }
    }

    @Test
    fun theRenderingTableShowsTheComputedRowsAndEditsVariantsInline() {
        fake.validateResponse = ValidateConfigResponse.newBuilder()
            .addRendering(
                RenderVariant.newBuilder().setCodec("h264").setFrameRate(30)
                    .setVariantKey("h264_ori_30").setExampleFilename("LRT_example_h264.mov"),
            )
            .build()
        show()
        open(ConfigGroup.RENDERING)
        awaitText("LRT_example_h264.mov")
        compose.onNodeWithText("h264_ori_30").assertExists()
        assertFalse(controller.dirty)

        fake.validateResponse = ValidateConfigResponse.newBuilder()
            .addRendering(
                RenderVariant.newBuilder().setCodec("prores").setFrameRate(24).addResolution(1920).addResolution(1080)
                    .setVariantKey("prores_1920x1080_24").setExampleFilename("LRT_example_prores.mov"),
            )
            .build()
        click(RenderingTags.codec(0))
        node(RenderingTags.choice(RenderingTags.codec(0), 1)).performClick()
        type(ConfigTags.field("/rendering/0/frame_rate"), "24")
        click(RenderingTags.mode(0))
        chooseMode(ResolutionMode.EXACT)
        compose.waitForIdle()
        type(ConfigTags.field("/rendering/0/resolution/0"), "1920")
        type(ConfigTags.field("/rendering/0/resolution/1"), "1080")
        settle()
        awaitText("LRT_example_prores.mov")
        compose.onNodeWithText("prores_1920x1080_24").assertExists()
        assertEquals("prores", config.getRendering(0).codec)
        assertEquals(24, config.getRendering(0).frameRate)
        assertEquals(listOf(1920, 1080), config.getRendering(0).resolutionList)

        click(RenderingTags.mode(0))
        chooseMode(ResolutionMode.WIDTH)
        compose.waitForIdle()
        assertEquals(listOf(1920), config.getRendering(0).resolutionList)
        click(RenderingTags.mode(0))
        chooseMode(ResolutionMode.ORIGINAL)
        compose.waitForIdle()
        assertEquals(0, config.getRendering(0).resolutionCount)

        click(RenderingTags.ADD)
        assertEquals(2, config.renderingCount)
        assertEquals("h264", config.getRendering(1).codec)
        click(ListTags.up("/rendering/1"))
        assertEquals(listOf("h264", "prores"), config.renderingList.map { it.codec })
        click(ListTags.down("/rendering/0"))
        assertEquals(listOf("prores", "h264"), config.renderingList.map { it.codec })
        click(ListTags.remove("/rendering/0"))
        assertEquals(listOf("h264"), config.renderingList.map { it.codec })
    }

    @Test
    fun renderingIssuesShowUnderTheirFields() {
        show()
        open(ConfigGroup.RENDERING)
        answer(
            error("/rendering/0/frame_rate", "frame_rate must be 1 to 240"),
            error("/rendering/0", "duplicate variant_key x"),
        )
        type(ConfigTags.field("/rendering/0/frame_rate"), "")
        settle()
        assertEquals(0, config.getRendering(0).frameRate)
        assertIssueUnder("/rendering/0/frame_rate", "Error: frame_rate must be 1 to 240")
        compose.onNodeWithText("Error: duplicate variant_key x").assertExists()
        compose.onNodeWithTag(ConfigTags.issueCount(ConfigGroup.RENDERING), useUnmergedTree = true)
            .assertTextEquals("2")
        node(ConfigTags.SAVE).assertIsNotEnabled()
    }

    @Test
    fun theLimitsAndBehaviourAreEdited() {
        show()
        open(ConfigGroup.LIMITS)
        type(ConfigTags.field("/max_processes/ffmpeg"), "3")
        type(ConfigTags.field("/max_processes/exiftool"), "4")
        type(ConfigTags.field("/max_cuncurrent_renders"), "2")
        type(ConfigTags.field("/max_cuncurrent_renders"), "2x")
        node(ConfigTags.field("/max_cuncurrent_renders")).assertTextContains("2")
        assertEquals(3, config.maxProcesses.ffmpeg)
        assertEquals(4, config.maxProcesses.exiftool)
        assertEquals(2, config.maxConcurrentRenders)

        open(ConfigGroup.BEHAVIOUR)
        type(ConfigTags.field("/scan_interval_seconds"), "1.")
        node(ConfigTags.field("/scan_interval_seconds")).assertTextContains("1.")
        type(ConfigTags.field("/scan_interval_seconds"), "1.5")
        type(ConfigTags.field("/recent_render_history_days"), "7")
        click(ListTags.add("delete_sequence_after_render_regexes"))
        type(ConfigTags.field("/delete_sequence_after_render_regexes/0"), "^del")
        assertEquals(1.5, config.scanIntervalSeconds, 0.0)
        assertEquals(7, config.recentRenderHistoryDays)
        assertEquals(listOf("^del"), config.deleteSequenceAfterRenderRegexesList)
    }

    @Test
    fun pausesAreRemovable() {
        show()
        open(ConfigGroup.PAUSES)
        compose.onNodeWithText("LRT_2 · h264_ori_30").assertExists()
        click(ListTags.remove("/paused_sequence_names/0"))
        click(ListTags.remove("/paused_variants/0"))
        assertEquals(0, config.pausedSequenceNamesCount)
        assertEquals(0, config.pausedVariantsCount)
        compose.onNodeWithText("LRT_1").assertDoesNotExist()
    }

    @Test
    fun theLegacyKeysAndTheRawJsonAreInAdvanced() {
        show()
        open(ConfigGroup.ADVANCED)
        compose.onNodeWithText("{\"raw\": 1}").assertExists()
        node(ConfigTags.field("/sleep_time_seconds")).assertTextContains("12")
        click(ConfigTags.field("/log_in_color"))
        type(ConfigTags.field("/progress_bar_width"), "60")
        type(ConfigTags.field("/sleep_time_seconds"), "3")
        assertTrue(config.logInColor)
        assertEquals(60, config.progressBarWidth)
        assertEquals(3.0, config.sleepTimeSeconds, 0.0)
        node(ConfigTags.USE_PATH).assertIsNotEnabled()
        compose.onNodeWithText("Save or discard the changes before using another config file.").assertExists()
    }

    @Test
    fun anotherConfigFileIsChosenAndLoaded() {
        folders()
        show()
        open(ConfigGroup.ADVANCED)
        node(ConfigTags.USE_PATH).assertIsNotEnabled()
        click(ConfigTags.BROWSE_PATH)
        awaitTag(BrowserTags.entry("/etc/trd/other.json"))
        assertEquals("/etc/trd", fake.requestsOf<ListDirectoryRequest>().single().path)
        assertTrue(fake.requestsOf<ListDirectoryRequest>().single().includeFiles)
        node(BrowserTags.entry("/etc/trd/old")).performClick()
        node(BrowserTags.CHOOSE).assertIsNotEnabled()
        node(BrowserTags.entry("/etc/trd/other.json")).performClick()
        node(BrowserTags.CHOOSE).performClick()
        compose.waitForIdle()
        fake.configState = state().toBuilder().setPath("/etc/trd/other.json").build()
        click(ConfigTags.USE_PATH)
        await { controller.editor.state.value.path == "/etc/trd/other.json" }
        val sent = fake.requestsOf<SetConfigPathRequest>().single()
        assertEquals("/etc/trd/other.json", sent.path)
        assertFalse(sent.createIfMissing)
        assertEquals(2, fake.callCount("GetConfig"))
    }

    @Test
    fun aRefusedConfigPathShowsTheServersMessage() {
        show()
        open(ConfigGroup.ADVANCED)
        fake.failWith["SetConfigPath"] = Status.INVALID_ARGUMENT.withDescription("the config path must be absolute")
        type(ConfigTags.CONFIG_PATH, "relative.json")
        click(ConfigTags.USE_PATH)
        awaitText("the config path must be absolute")
        assertEquals(1, fake.callCount("GetConfig"))
    }

    @Test
    fun issuesWithoutAFieldAreListedInTheirGroup() {
        val warning = ConfigIssue.newBuilder().setSeverity(IssueSeverity.ISSUE_WARNING)
        show(
            state().toBuilder()
                .addIssues(error("/rendering/5/codec", "codec \"gone\" is not in the codec table"))
                .addIssues(warning.setField("").setMessage("the codec table did not load"))
                .build(),
        )
        node(ConfigTags.SAVE).assertIsNotEnabled()
        open(ConfigGroup.RENDERING)
        compose.onNodeWithText("Error: /rendering/5/codec: codec \"gone\" is not in the codec table").assertExists()
        open(ConfigGroup.ADVANCED)
        compose.onNodeWithText("Warning: the codec table did not load").assertExists()
    }

    @Test
    fun aConflictOffersReload() {
        show()
        fake.failWith["UpdateConfig"] = Status.FAILED_PRECONDITION.withDescription("stale")
        type(ConfigTags.field("/export_paths/0"), "/mine")
        settle()
        node(ConfigTags.SAVE).performClick()
        awaitText("The config changed on the server")
        compose.onNodeWithText("Reload").assertExists()
        fake.configState = state(baseConfig.toBuilder().setExportPaths(0, "/theirs").build(), revision = "rev9")
        node(ConfigTags.RELOAD).performClick()
        await { controller.editor.state.value.config?.getExportPaths(0) == "/theirs" }
        awaitNoText("The config changed on the server")
        assertFalse(controller.dirty)
    }

    @Test
    fun aFailedLoadCanBeRetried() {
        fake.failWith["GetConfig"] = Status.UNAVAILABLE.withDescription("trd is starting")
        show()
        awaitText("trd is starting")
        compose.onNodeWithText("The config could not be loaded.").assertExists()
        fake.failWith.clear()
        node(ConfigTags.RELOAD).performClick()
        awaitTag(ConfigTags.field("/export_paths/0"))
    }

    @Test
    fun closingWithoutChangesAsksNothing() {
        show()
        controller.requestClose()
        assertEquals(1, closed.get())
        assertFalse(controller.ui.value.closePrompt)
    }

    @Test
    fun closingWithChangesAsksAndCancelKeepsTheWindow() {
        show()
        type(ConfigTags.field("/export_paths/0"), "/new")
        controller.requestClose()
        awaitTag(ConfigTags.PROMPT_CANCEL)
        node(ConfigTags.PROMPT_CANCEL).performClick()
        compose.waitForIdle()
        compose.onNodeWithTag(ConfigTags.PROMPT_CANCEL).assertDoesNotExist()
        assertEquals(0, closed.get())
        assertEquals(1, stayed.get())
        assertTrue(controller.dirty)
    }

    @Test
    fun discardClosesWithoutSaving() {
        show()
        type(ConfigTags.field("/export_paths/0"), "/new")
        controller.requestClose()
        awaitTag(ConfigTags.PROMPT_DISCARD)
        node(ConfigTags.PROMPT_DISCARD).performClick()
        await { closed.get() == 1 }
        assertTrue(configUpdates().isEmpty())
    }

    @Test
    fun saveInThePromptSavesAndCloses() {
        show()
        type(ConfigTags.field("/export_paths/0"), "/new")
        settle()
        fake.updateConfigResult = state(config, revision = "rev2")
        controller.requestClose()
        awaitTag(ConfigTags.PROMPT_SAVE)
        node(ConfigTags.PROMPT_SAVE).performClick()
        await { closed.get() == 1 }
        assertEquals("/new", configUpdates().single().config.getExportPaths(0))
    }

    @Test
    fun aSaveThatFailsKeepsTheWindowOpen() {
        show()
        type(ConfigTags.field("/export_paths/0"), "/new")
        settle()
        fake.failWith["UpdateConfig"] = Status.INTERNAL.withDescription("disk full")
        controller.requestClose()
        awaitTag(ConfigTags.PROMPT_SAVE)
        node(ConfigTags.PROMPT_SAVE).performClick()
        await { stayed.get() == 1 }
        awaitText("disk full")
        assertEquals(0, closed.get())
    }

    @Test
    fun thePromptCannotSaveAConfigWithErrors() {
        show()
        answer(error("/export_paths/0", "not an absolute path"))
        type(ConfigTags.field("/export_paths/0"), "new")
        settle()
        controller.requestClose()
        awaitTag(ConfigTags.PROMPT_SAVE)
        node(ConfigTags.PROMPT_SAVE).assertIsNotEnabled()
        controller.saveAndClose()
        assertEquals(1, stayed.get())
        assertEquals(0, closed.get())
    }

    @Test
    fun saveDoesNothingWithoutChanges() {
        show()
        controller.save()
        compose.waitForIdle()
        assertNull(controller.editor.state.value.error)
        assertEquals(0, fake.callCount("UpdateConfig"))
    }

    @Test
    fun theSaveShortcutUsesCommandOnTheMacAndControlElsewhere() {
        assertTrue(isSaveShortcut(Key.S, meta = true, ctrl = false, mac = true))
        assertFalse(isSaveShortcut(Key.S, meta = false, ctrl = true, mac = true))
        assertTrue(isSaveShortcut(Key.S, meta = false, ctrl = true, mac = false))
        assertFalse(isSaveShortcut(Key.S, meta = true, ctrl = false, mac = false))
        assertFalse(isSaveShortcut(Key.D, meta = true, ctrl = true, mac = true))
    }

    @Test
    fun issuePointersBelongToTheirGroups() {
        val expected = mapOf(
            "/master_videos/0" to ConfigGroup.LOCATIONS,
            "/raw_types" to ConfigGroup.MATCHING,
            "/rendering/2/codec" to ConfigGroup.RENDERING,
            "/max_processes/ffmpeg" to ConfigGroup.LIMITS,
            "/max_cuncurrent_renders" to ConfigGroup.LIMITS,
            "/delete_sequence_after_render_regexes/1" to ConfigGroup.BEHAVIOUR,
            "/paused_variants/0" to ConfigGroup.PAUSES,
            "/progress_bar_width" to ConfigGroup.ADVANCED,
            "" to ConfigGroup.ADVANCED,
        )
        assertEquals(expected, expected.keys.associateWith { ConfigGroup.of(it) })
    }

    @Test
    fun theBrowserStartsAtTheFolderTheValueNames() {
        assertEquals("", browseStart("", pickFile = false))
        assertEquals("/a/b", browseStart("/a/b", pickFile = false))
        assertEquals("/a", browseStart("/a/\$YEAR/b", pickFile = false))
        assertEquals("/a", browseStart("/a/shots-\$YEAR", pickFile = false))
        assertEquals("/", browseStart("/\$YEAR", pickFile = false))
        assertEquals("C:\\", browseStart("C:\\\$YEAR", pickFile = false))
        assertEquals("", browseStart("\$YEAR", pickFile = false))
        assertEquals("/a", browseStart("/a/config.json", pickFile = true))
    }

    @Test
    fun resolutionModesKeepTheNumbersAlreadyThere() {
        assertEquals(emptyList<Int>(), resolutionFor(ResolutionMode.ORIGINAL, listOf(1, 2)))
        assertEquals(listOf(3840), resolutionFor(ResolutionMode.WIDTH, emptyList()))
        assertEquals(listOf(3840, 2160), resolutionFor(ResolutionMode.EXACT, emptyList()))
        assertEquals(listOf(100, 2160), resolutionFor(ResolutionMode.EXACT, listOf(100)))
        assertEquals(ResolutionMode.EXACT, ResolutionMode.of(listOf(1, 2, 3)))
    }

    @Test
    fun onlyASelectedFileCanBeChosenAsAFile() {
        val ui = DirectoryUi(path = "/d", entries = listOf(dir("/d/f", isDir = false)))
        assertNull(chosenPath(ui, pickFile = true))
        assertEquals("/d", chosenPath(ui, pickFile = false))
        assertEquals("/d/f", chosenPath(ui.copy(selected = "/d/f"), pickFile = true))
    }
}
