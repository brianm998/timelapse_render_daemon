// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.platform.CommandResult
import com.timelapserender.monitor.desktop.platform.HostOs
import com.timelapserender.monitor.desktop.platform.TrdService
import com.timelapserender.monitor.desktop.record
import com.timelapserender.trd.v1.ConfigPathSource
import com.timelapserender.trd.v1.ConfigState
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.Problem
import com.timelapserender.trd.v1.ProblemKind
import com.timelapserender.trd.v1.SetConfigPathRequest
import com.timelapserender.trd.v1.Snapshot
import io.grpc.Status
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.nio.file.Paths
import java.util.concurrent.CopyOnWriteArrayList

private const val OTHER_PROBLEM = "Another problem"

class FirstRunSetupTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    /** What the file chooser answers, one per time it is opened. */
    private val chosen = CopyOnWriteArrayList<String?>()

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun snapshot(phase: Phase, diskAccessMissing: Boolean = false, revision: Long = 1): Snapshot {
        val builder = Snapshot.newBuilder().setPhase(phase).setRevision(revision)
        if (diskAccessMissing) {
            builder.addProblems(
                Problem.newBuilder().setId("fda").setKind(ProblemKind.PROBLEM_FULL_DISK_ACCESS_MISSING),
            )
            // Any other problem: its banner offers no How to fix.
            builder.addProblems(Problem.newBuilder().setId("other").setTitle(OTHER_PROBLEM))
        }
        return builder.build()
    }

    private fun show(first: Snapshot, os: HostOs = HostOs.MAC, local: Boolean = true): Harness {
        val h = Harness(os = os, configChooser = { chosen.removeAt(0) })
        harness = h
        h.local.serverInfo = Harness.info("studio", "Studio iMac", local = local)
        h.local.snapshots = listOf(first)
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        compose.setContent { MainWindow(h.app) }
        h.app.start()
        h.await { it.connected && it.phase == first.phase }
        return h
    }

    private fun shows(text: String) = compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty()

    private fun await(condition: () -> Boolean) = compose.waitUntil(Harness.TIMEOUT_MS, condition)

    private fun awaitText(text: String) = await { shows(text) }

    private fun click(text: String) = compose.onNodeWithText(text).performScrollTo().performClick()

    private fun Harness.pathRequests() = local.requestsOf<SetConfigPathRequest>()

    @Test
    fun choosingAFileSetsTheConfigPathAndOpensTheConfigWindow() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG))
        awaitText(SETUP_TITLE)
        compose.onNodeWithText(STEP_CONFIG).assertExists()
        compose.onNodeWithText(STEP_DISK_ACCESS).assertDoesNotExist()

        // A cancelled chooser changes nothing.
        chosen.add(null)
        click(USE_EXISTING_CONFIG)
        compose.waitForIdle()
        assertTrue(h.pathRequests().isEmpty())

        chosen += "/Users/tester/timelapse/config.json"
        click(USE_EXISTING_CONFIG)
        awaitText("✓ Config: /Users/tester/timelapse/config.json")
        val request = h.pathRequests().single()
        assertEquals("/Users/tester/timelapse/config.json", request.path)
        assertEquals(false, request.createIfMissing)
        h.awaitTrue { h.app.configWindows.value.map { it.serverId } == listOf("local") }
        // trd has not taken the file up yet.
        compose.onNodeWithText(RESTART_HINT).assertExists()

        h.local.snapshots = listOf(snapshot(Phase.PHASE_IDLE, revision = 2))
        await { !shows(RESTART_HINT) }
        compose.onNodeWithText(USE_EXISTING_CONFIG).assertDoesNotExist()
    }

    @Test
    fun createUsesTheDefaultPathTrdReports() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG))
        h.local.configState = ConfigState.newBuilder()
            .setPath("/trd/default/config.json").setPathSource(ConfigPathSource.CONFIG_PATH_DEFAULT).build()
        awaitText(SETUP_TITLE)
        click(CREATE_NEW_CONFIG)
        awaitText("✓ Config: /trd/default/config.json")
        val request = h.pathRequests().single()
        assertEquals("/trd/default/config.json", request.path)
        assertEquals(true, request.createIfMissing)
        h.awaitTrue { h.app.configWindows.value.size == 1 }
    }

    @Test
    fun createFallsBackToThisComputersDefaultWhenTrdUsesAnotherPath() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG))
        h.local.configState = ConfigState.newBuilder()
            .setPath("/gone/config.json").setPathSource(ConfigPathSource.CONFIG_PATH_SETTING).build()
        awaitText(SETUP_TITLE)
        click(CREATE_NEW_CONFIG)
        h.awaitTrue { h.pathRequests().size == 1 }
        val expected = Paths.get(
            "/home/tester", "Library", "Application Support", "TimelapseRenderDaemon", "config.json",
        )
        assertEquals(expected.toString(), h.pathRequests().single().path)
        assertEquals(true, h.pathRequests().single().createIfMissing)
    }

    @Test
    fun theDefaultConfigPathFollowsTheSystem() {
        fun path(os: HostOs, env: Map<String, String> = emptyMap()) =
            TrdService(os, { CommandResult(0, "") }, "/h") { env[it] }.defaultConfigPath()
        assertEquals(
            Paths.get("/h", "Library", "Application Support", "TimelapseRenderDaemon", "config.json"),
            path(HostOs.MAC),
        )
        assertEquals(Paths.get("/h", ".config", "trd", "config.json"), path(HostOs.LINUX))
        assertEquals(Paths.get("/x", "trd", "config.json"), path(HostOs.LINUX, mapOf("XDG_CONFIG_HOME" to "/x")))
        assertEquals(
            Paths.get("/h", "AppData", "Roaming", "TimelapseRenderDaemon", "config.json"),
            path(HostOs.WINDOWS),
        )
        assertEquals(
            Paths.get("/a", "TimelapseRenderDaemon", "config.json"),
            path(HostOs.WINDOWS, mapOf("APPDATA" to "/a")),
        )
    }

    @Test
    fun aRefusedPathShowsWhyAndCanBeTriedAgain() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG))
        h.local.failWith["SetConfigPath"] = Status.INVALID_ARGUMENT.withDescription("the config path must be absolute")
        awaitText(SETUP_TITLE)
        chosen += "config.json"
        click(USE_EXISTING_CONFIG)
        awaitText("the config path must be absolute")
        assertTrue(h.app.configWindows.value.isEmpty())

        h.local.failWith.clear()
        chosen += "/c/config.json"
        click(USE_EXISTING_CONFIG)
        awaitText("✓ Config: /c/config.json")
        compose.onNodeWithText("the config path must be absolute").assertDoesNotExist()
    }

    @Test
    fun theDiskAccessStepCompletesWhenTheProblemDisappears() {
        val h = show(snapshot(Phase.PHASE_WAITING_FOR_DISK_ACCESS, diskAccessMissing = true))
        // Setup comes by itself only without a config; the problem's How to fix opens it.
        awaitText("How to fix")
        awaitText(OTHER_PROBLEM)
        compose.onNodeWithText(SETUP_TITLE).assertDoesNotExist()
        // One button, on the Full Disk Access banner.
        compose.onAllNodesWithText("How to fix").assertCountEquals(1)
        compose.onNodeWithText("How to fix").performClick()
        awaitText(STEP_DISK_ACCESS)
        compose.onNodeWithText(DISK_ACCESS_DETAIL).assertExists()
        compose.onNodeWithText(DISK_ACCESS_FIX).assertExists()
        compose.onNodeWithText(STEP_CONFIG).assertDoesNotExist()

        click(OPEN_PRIVACY_SETTINGS)
        h.awaitTrue { h.runner.calls.size == 1 }
        click(SHOW_TRD_IN_FINDER)
        h.awaitTrue { h.runner.calls.size == 2 }
        assertEquals(
            listOf(
                listOf("open", "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"),
                listOf("open", "-R", "/Applications/Timelapse Render Daemon.app"),
            ),
            h.runner.calls.toList(),
        )
        compose.onNodeWithText(DISK_ACCESS_ALLOWED, substring = true).assertDoesNotExist()

        h.local.snapshots = listOf(snapshot(Phase.PHASE_IDLE, revision = 2))
        awaitText("✓ $DISK_ACCESS_ALLOWED")
        compose.onNodeWithText(OPEN_PRIVACY_SETTINGS).assertDoesNotExist()
        compose.onNodeWithText("How to fix").assertDoesNotExist()
    }

    @Test
    fun grantingAccessWithoutAConfigLeadsOnToChoosingOne() {
        val h = show(snapshot(Phase.PHASE_WAITING_FOR_DISK_ACCESS, diskAccessMissing = true))
        h.app.openFullDiskAccessHelp()
        awaitText(STEP_DISK_ACCESS)
        h.local.snapshots = listOf(snapshot(Phase.PHASE_NO_CONFIG, revision = 2))
        awaitText("✓ $DISK_ACCESS_ALLOWED")
        awaitText(USE_EXISTING_CONFIG)
    }

    @Test
    fun aFailedButtonSaysWhy() {
        val h = show(snapshot(Phase.PHASE_WAITING_FOR_DISK_ACCESS, diskAccessMissing = true))
        h.runner.results = mapOf("open" to CommandResult(1, "no such application"))
        h.app.showTrdInFinder()
        h.await { it.notice == "no such application" }
    }

    @Test
    fun thereIsNoDiskAccessStepOffTheMac() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG, diskAccessMissing = true), os = HostOs.LINUX)
        awaitText(SETUP_TITLE)
        compose.onNodeWithText(STEP_DISK_ACCESS).assertDoesNotExist()
        compose.onNodeWithText("How to fix").assertDoesNotExist()
        h.app.openFullDiskAccessHelp()
        compose.onNodeWithText(STEP_DISK_ACCESS).assertDoesNotExist()
    }

    @Test
    fun aClosedSetupStaysClosedUntilTheServerIsSelectedAgain() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG))
        awaitText(SETUP_TITLE)
        compose.onNodeWithText("Close").performClick()
        await { !shows(SETUP_TITLE) }
        h.local.snapshots = listOf(snapshot(Phase.PHASE_NO_CONFIG, revision = 2))
        h.awaitTrue { h.app.session?.repository?.state?.value?.snapshot?.revision == 2L }
        assertNull(h.app.ui.value.setup)

        h.app.selectServer("other")
        h.await { it.activeServerId == "other" && it.connected }
        h.app.selectServer("local")
        awaitText(SETUP_TITLE)
    }

    @Test
    fun thereIsNoSetupOverTheLan() {
        val h = show(snapshot(Phase.PHASE_NO_CONFIG, diskAccessMissing = true), local = false)
        h.app.openFullDiskAccessHelp()
        compose.waitForIdle()
        assertNull(h.app.ui.value.setup)
        compose.onNodeWithText(SETUP_TITLE).assertDoesNotExist()
        compose.onNodeWithText("How to fix").assertDoesNotExist()
    }
}
