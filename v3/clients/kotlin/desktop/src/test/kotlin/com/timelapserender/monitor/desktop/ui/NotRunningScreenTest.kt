// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.platform.CommandResult
import com.timelapserender.monitor.desktop.platform.HostOs
import com.timelapserender.monitor.desktop.platform.RELEASES_URL
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class NotRunningScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun show(os: HostOs = HostOs.MAC): Harness {
        val h = Harness(os = os).also { harness = it }
        h.localDown()
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitText(NOT_RUNNING_TEXT)
        return h
    }

    private fun awaitText(text: String) {
        compose.waitUntil(Harness.TIMEOUT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun withNoServerTheScreenShowsItsTextAndButtons() {
        val h = show()
        compose.onNodeWithText(NOT_RUNNING_TEXT).assertExists()
        compose.onNodeWithText("Start trd").assertExists()
        compose.onNodeWithText("Get trd").assertExists()
        compose.onNodeWithText("Connect to another computer…").assertExists()
        compose.onNodeWithTag(TopBarTags.SERVER).assertDoesNotExist()
        assertEquals(emptyList<List<String>>(), h.runner.calls.toList())
    }

    @Test
    fun startTrdCallsTheMacCommand() {
        val h = show(HostOs.MAC)
        compose.onNodeWithText("Start trd").performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.runner.calls.size == 2 }
        assertEquals(
            listOf(
                listOf("/usr/bin/id", "-u"),
                listOf("launchctl", "kickstart", "-k", "gui/501/com.timelapserender.trd"),
            ),
            h.runner.calls.toList(),
        )
    }

    @Test
    fun startTrdCallsTheLinuxCommand() {
        val h = show(HostOs.LINUX)
        compose.onNodeWithText("Start trd").performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.runner.calls.isNotEmpty() }
        assertEquals(listOf(listOf("systemctl", "--user", "start", "trd")), h.runner.calls.toList())
    }

    @Test
    fun startTrdCallsTheWindowsCommand() {
        val h = show(HostOs.WINDOWS)
        compose.onNodeWithText("Start trd").performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.runner.calls.isNotEmpty() }
        assertEquals(listOf(listOf("schtasks", "/Run", "/TN", "TimelapseRenderDaemon")), h.runner.calls.toList())
    }

    @Test
    fun aFailedStartShowsTheOutputAndTheReleasePage() {
        val h = show(HostOs.LINUX)
        h.runner.results = mapOf("systemctl" to CommandResult(5, "Unit trd.service not found."))
        compose.onNodeWithText("Start trd").performClick()
        awaitText("Unit trd.service not found.")
        compose.onNodeWithText(RELEASES_URL).assertExists()
    }

    @Test
    fun getTrdOpensTheReleasesPage() {
        val h = show(HostOs.LINUX)
        compose.onNodeWithText("Get trd").performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.runner.calls.isNotEmpty() }
        assertEquals(listOf(listOf("xdg-open", RELEASES_URL)), h.runner.calls.toList())
    }

    @Test
    fun connectToAnotherComputerShowsTheServerList() {
        show()
        compose.onNodeWithText("Connect to another computer…").performClick()
        awaitText("No other computers found yet.")
    }

    @Test
    fun theMainWindowAppearsWhenTrdAnswers() {
        val h = show()
        h.localUp()
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithText(NOT_RUNNING_TEXT).fetchSemanticsNodes().isEmpty() && h.app.ui.value.connected
        }
        compose.onNodeWithTag(TopBarTags.SERVER).assertExists()
    }
}
