// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.discovered
import com.timelapserender.monitor.desktop.record
import com.timelapserender.trd.v1.Counts
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Snapshot
import io.grpc.Status
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class TopBarTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun show(h: Harness = Harness(), sandbox: Boolean = false): Harness {
        harness = h
        h.local.serverInfo = Harness.info("studio", "Studio iMac", sandbox = sandbox)
        h.local.snapshots = listOf(
            Snapshot.newBuilder()
                .setPhase(Phase.PHASE_RENDERING)
                .setCounts(Counts.newBuilder().setRendering(2).setQueued(1).setExporting(3).setFailures(1))
                .addInProgress(SequenceCard.newBuilder().setId("1").setName("LRT_1").setExportPath("/exports/a"))
                .build(),
        )
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitText("Rendering")
        return h
    }

    private fun awaitText(text: String) {
        compose.waitUntil(Harness.TIMEOUT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun awaitNoText(text: String) {
        compose.waitUntil(Harness.TIMEOUT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isEmpty() }
    }

    @Test
    fun theTopBarShowsTheNamePhaseAndCounts() {
        show()
        compose.onNodeWithTag(TopBarTags.SERVER).assertTextContains("Studio iMac ▾")
        compose.onNodeWithTag(TopBarTags.CONNECTION).assertTextEquals("Connected")
        compose.onNodeWithTag(TopBarTags.PHASE, useUnmergedTree = true).assertExists()
        compose.onNodeWithText("Rendering").assertExists()
        compose.onNodeWithTag(TopBarTags.COUNTS)
            .assertTextEquals("2 rendering · 1 queued · 3 exporting · 1 failed")
        compose.onNodeWithText("Pause all rendering").assertExists()
        compose.onNodeWithText("SANDBOX").assertDoesNotExist()
        compose.onNodeWithTag(ACTIVITY_LIST_TAG).assertExists()
        compose.onNodeWithText("LRT_1").assertExists()
    }

    @Test
    fun aSandboxServerHasTheBadge() {
        show(sandbox = true)
        compose.onNodeWithText("SANDBOX").assertExists()
    }

    @Test
    fun thePauseSwitchCallsTheServer() {
        val h = show()
        compose.onNodeWithTag(TopBarTags.PAUSE).performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.local.callCount("SetGlobalPause") == 1 }
    }

    @Test
    fun typingInTheSearchFieldSetsTheFilter() {
        val h = show()
        compose.onNodeWithTag(TopBarTags.SEARCH).performTextInput("LRT")
        compose.waitUntil(Harness.TIMEOUT_MS) { h.settings.search == "LRT" }
    }

    @Test
    fun theSearchShortcutFocusesTheSearchField() {
        val h = show()
        h.app.requestSearchFocus()
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onNodeWithTag(TopBarTags.SEARCH).fetchSemanticsNode()
                .config.getOrElse(SemanticsProperties.Focused) { false }
        }
        compose.onNodeWithTag(TopBarTags.SEARCH).assertIsFocused()
    }

    @Test
    fun theExportPathFilterOffersTheSnapshotPaths() {
        val h = show()
        compose.onNodeWithTag(TopBarTags.EXPORT_PATH).performClick()
        // The card shows the path too; pick the menu's entry.
        compose.onNode(hasText("/exports/a") and hasAnyAncestor(isPopup())).performClick()
        awaitText("/exports/a ▾")
        assertEquals("/exports/a", h.settings.exportPath)
        compose.onNodeWithTag(TopBarTags.EXPORT_PATH).assertTextContains("/exports/a ▾")
    }

    @Test
    fun theSwitcherListsLocalThenPairedThenDiscoveredAndSwitches() {
        val h = Harness()
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        show(h)
        h.discovery.servers.value = listOf(discovered("lan", Harness.OTHER_PORT, name = "Laptop"))
        compose.waitUntil(Harness.TIMEOUT_MS) { h.app.ui.value.servers.size == 3 }
        compose.onNodeWithTag(TopBarTags.SERVER).performClick()
        val top = { text: String -> compose.onNodeWithText(text).fetchSemanticsNode().positionInRoot.y }
        val local = top("✓ This computer")
        val paired = top("Render box")
        val group = top("Discovered")
        val found = top("Laptop — Pair…")
        assert(local < paired && paired < group && group < found)
        compose.onNodeWithText("Render box").performClick()
        awaitText("Other trd ▾")
        assertEquals("other", h.app.ui.value.activeServerId)
    }

    @Test
    fun pairingFromTheSwitcherAsksForTheCode() {
        val h = Harness(
            pairer = { endpoint, code, _ ->
                ServerRecord("other", "Other trd", endpoint.host, endpoint.port, "cd", code, "linux-x64", 1L)
            },
        )
        show(h)
        h.discovery.servers.value = listOf(discovered("other", Harness.OTHER_PORT, name = "Laptop"))
        compose.waitUntil(Harness.TIMEOUT_MS) { h.app.ui.value.servers.size == 2 }
        compose.onNodeWithTag(TopBarTags.SERVER).performClick()
        compose.onNodeWithText("Laptop — Pair…").performClick()
        awaitText("Pair with Laptop")
        compose.onNodeWithTag(PAIR_CODE_TAG).performTextInput("123456")
        compose.onNodeWithText("Pair").performClick()
        awaitText("Other trd ▾")
        assertEquals("123456", h.store.list().single().token)
    }

    @Test
    fun theServerListForgetsAPairedServer() {
        val h = Harness()
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        show(h)
        h.app.showServers()
        awaitText("Render box")
        compose.onNodeWithText("Forget").performClick()
        compose.waitUntil(Harness.TIMEOUT_MS) { h.store.list().isEmpty() }
    }

    @Test
    fun anUnpairedConnectionOffersPairing() {
        val h = Harness()
        harness = h
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        h.settings.lastServerId = "other"
        h.other.failWith["GetServerInfo"] = Status.UNAUTHENTICATED
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitText("Needs pairing")
        compose.onNodeWithText("Pair…").performClick()
        awaitText("Pair with Render box")
    }

    @Test
    fun theDetailsPaneShowsTheLastScanAndCollapses() {
        val h = show()
        compose.onNodeWithTag(MainWindowTags.DETAILS).assertExists()
        compose.onNodeWithText("Not scanned yet").assertExists()
        h.app.toggleDetails()
        awaitNoText("Not scanned yet")
        compose.onNodeWithTag(MainWindowTags.DETAILS).assertDoesNotExist()
    }

    @Test
    fun theAboutDialogAndANoticeCanBeDismissed() {
        val h = show()
        h.app.showAbout()
        awaitText("Close")
        compose.onNodeWithText("Close").performClick()
        awaitNoText("Close")
        h.local.failWith["RescanNow"] = Status.FAILED_PRECONDITION.withDescription("rescan failed")
        h.app.rescanNow()
        awaitText("rescan failed")
        compose.onNodeWithText("Dismiss").performClick()
        awaitNoText("rescan failed")
    }
}
