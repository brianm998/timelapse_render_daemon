// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.doubleClick
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performScrollToNode
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.RecordingRunner
import com.timelapserender.monitor.desktop.record
import com.timelapserender.monitor.desktop.ui.ACTIVITY_LIST_TAG
import com.timelapserender.monitor.desktop.ui.ActionTarget
import com.timelapserender.monitor.desktop.ui.ControlFixtures
import com.timelapserender.monitor.desktop.ui.MainWindow
import com.timelapserender.monitor.desktop.ui.RowAction
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalTestApi::class)
class FileOpenerTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private val path = "/exports/a b/LRT_1.mov"

    private fun ran(os: HostOs, action: FileOpener.() -> ActionResult): List<String> {
        val runner = RecordingRunner()
        assertTrue(FileOpener(os, runner).action().ok)
        return runner.calls.single()
    }

    @Test
    fun macOsOpensAndRevealsWithOpen() {
        assertEquals(listOf("open", path), ran(HostOs.MAC) { open(path) })
        assertEquals(listOf("open", "-R", path), ran(HostOs.MAC) { reveal(path) })
    }

    @Test
    fun linuxOpensTheFileOrItsDirectory() {
        assertEquals(listOf("xdg-open", path), ran(HostOs.LINUX) { open(path) })
        assertEquals(listOf("xdg-open", "/exports/a b"), ran(HostOs.LINUX) { reveal(path) })
    }

    @Test
    fun windowsOpensAndSelectsWithExplorer() {
        val file = "C:\\exports\\LRT_1.mov"
        assertEquals(listOf("explorer.exe", file), ran(HostOs.WINDOWS) { open(file) })
        assertEquals(listOf("explorer.exe", "/select,$file"), ran(HostOs.WINDOWS) { reveal(file) })
    }

    @Test
    fun aFailureCarriesTheOutputAndExplorersExitCodeOneIsSuccess() {
        val runner = RecordingRunner()
        runner.results = mapOf(
            "open" to CommandResult(1, "The file does not exist.\n"),
            "xdg-open" to CommandResult(4, ""),
            "explorer.exe" to CommandResult(1, ""),
        )
        assertEquals("The file does not exist.", FileOpener(HostOs.MAC, runner).open(path).error)
        assertEquals("The command failed with exit code 4.", FileOpener(HostOs.LINUX, runner).reveal(path).error)
        assertTrue(FileOpener(HostOs.WINDOWS, runner).open(path).ok)
    }

    @Test
    fun onlyALocalServersExistingFileIsAvailable() {
        assertTrue(FileOpener.available(loopback = true, fileExists = true))
        assertFalse(FileOpener.available(loopback = true, fileExists = false))
        assertFalse(FileOpener.available(loopback = false, fileExists = true))
        assertFalse(FileOpener.available(loopback = false, fileExists = false))
    }

    private fun show(h: Harness): Harness {
        harness = h
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithTag(ControlFixtures.FAILURE).fetchSemanticsNodes().isNotEmpty()
        }
        return h
    }

    private fun local(): Harness {
        val h = Harness()
        h.local.snapshots = listOf(ControlFixtures.snapshot())
        return show(h)
    }

    private fun remote(): Harness {
        val h = Harness()
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        h.settings.lastServerId = "other"
        h.other.snapshots = listOf(ControlFixtures.snapshot())
        return show(h)
    }

    private fun scrollTo(row: String) {
        compose.onNodeWithTag(ACTIVITY_LIST_TAG).performScrollToNode(hasTestTag(row))
    }

    private fun button(row: String, name: String) = compose.onNode(hasText(name) and hasAnyAncestor(hasTestTag(row)))

    private fun doubleClickRow(row: String) {
        scrollTo(row)
        // On the file name, clear of the buttons.
        compose.onNodeWithTag(row).performMouseInput { doubleClick(Offset(centerX, 16f)) }
        compose.waitForIdle()
    }

    @Test
    fun theButtonsRunTheCommandsOfTheOs() {
        val h = local()
        scrollTo(ControlFixtures.VIDEO)
        button(ControlFixtures.VIDEO, "Open").performClick()
        h.awaitTrue { h.runner.calls.size == 1 }
        button(ControlFixtures.VIDEO, "Reveal").performClick()
        h.awaitTrue { h.runner.calls.size == 2 }
        assertEquals(
            listOf(listOf("open", ControlFixtures.VIDEO_PATH), listOf("open", "-R", ControlFixtures.VIDEO_PATH)),
            h.runner.calls.toList(),
        )
    }

    @Test
    fun aMissingFileHasNeitherButtonNorDoubleClick() {
        val h = local()
        scrollTo(ControlFixtures.MISSING_VIDEO)
        button(ControlFixtures.MISSING_VIDEO, "Open").assertDoesNotExist()
        button(ControlFixtures.MISSING_VIDEO, "Reveal").assertDoesNotExist()
        doubleClickRow(ControlFixtures.MISSING_VIDEO)
        compose.onNodeWithTag(ControlFixtures.MISSING_VIDEO).assertIsSelected()
        assertTrue(h.runner.calls.isEmpty())
    }

    @Test
    fun anotherComputersServerHasNeitherButtonNorDoubleClick() {
        val h = remote()
        scrollTo(ControlFixtures.VIDEO)
        button(ControlFixtures.VIDEO, "Copy path").assertExists()
        button(ControlFixtures.VIDEO, "Open").assertDoesNotExist()
        button(ControlFixtures.VIDEO, "Reveal").assertDoesNotExist()
        doubleClickRow(ControlFixtures.VIDEO)
        // Asked for directly, it still runs nothing: the file is on the other computer.
        val target = ActionTarget.Video(ControlFixtures.VIDEO, ControlFixtures.VIDEO_PATH)
        h.app.perform(RowAction.OPEN, target)
        h.app.perform(RowAction.REVEAL, target)
        assertTrue(h.runner.calls.isEmpty())
    }

    @Test
    fun aDoubleClickOpensTheVideoOnce() {
        val h = local()
        doubleClickRow(ControlFixtures.VIDEO)
        h.awaitTrue { h.runner.calls.isNotEmpty() }
        compose.onNodeWithTag(ControlFixtures.VIDEO).assertIsSelected()
        assertEquals(listOf(listOf("open", ControlFixtures.VIDEO_PATH)), h.runner.calls.toList())
    }

    @Test
    fun aDoubleClickOnAButtonDoesNotOpenTheVideo() {
        val h = local()
        scrollTo(ControlFixtures.VIDEO)
        button(ControlFixtures.VIDEO, "Copy path").performMouseInput { doubleClick() }
        compose.waitForIdle()
        assertEquals(listOf(ControlFixtures.VIDEO_PATH, ControlFixtures.VIDEO_PATH), h.copies.toList())
        assertTrue(h.runner.calls.isEmpty())
    }

    @Test
    fun aFailedOpenIsANotice() {
        val h = local()
        h.runner.results = mapOf("open" to CommandResult(1, "no application can open it"))
        scrollTo(ControlFixtures.VIDEO)
        button(ControlFixtures.VIDEO, "Open").performClick()
        h.await { it.notice == "no application can open it" }
    }
}
