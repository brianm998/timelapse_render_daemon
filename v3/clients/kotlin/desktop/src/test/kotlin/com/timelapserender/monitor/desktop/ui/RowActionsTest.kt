// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.record
import com.timelapserender.trd.v1.FailureRef
import com.timelapserender.trd.v1.RenderRef
import com.timelapserender.trd.v1.SequenceRef
import com.timelapserender.trd.v1.SetGlobalPauseRequest
import com.timelapserender.trd.v1.VariantRef
import com.timelapserender.trd.v1.VariantStatus
import io.grpc.Status
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class RowActionsTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun show(): Harness {
        val h = Harness()
        harness = h
        h.local.snapshots = listOf(ControlFixtures.snapshot())
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithTag(ControlFixtures.FAILURE).fetchSemanticsNodes().isNotEmpty()
        }
        return h
    }

    private fun scrollTo(row: String) {
        compose.onNodeWithTag(ACTIVITY_LIST_TAG).performScrollToNode(hasTestTag(row))
    }

    /** The names of the buttons inside a row: their text, or an icon button's description. */
    private fun buttons(row: String): List<String> {
        scrollTo(row)
        return compose.onAllNodes(hasClickAction() and hasAnyAncestor(hasTestTag(row)))
            .fetchSemanticsNodes()
            .map { node ->
                node.config.getOrNull(SemanticsProperties.ContentDescription)?.single()
                    ?: node.config[SemanticsProperties.Text].single().text
            }
    }

    private fun button(row: String, name: String) {
        scrollTo(row)
        compose.onNode(
            (hasText(name) or hasContentDescription(name)) and hasClickAction() and hasAnyAncestor(hasTestTag(row)),
        ).performClick()
    }

    private fun menuItem(card: String, name: String) {
        button(card, SEQUENCE_MENU)
        compose.onNode(hasText(name) and hasAnyAncestor(isPopup())).performClick()
    }

    private inline fun <reified T : Any> Harness.sent(method: String): T {
        awaitTrue { local.callCount(method) == 1 && local.requestsOf<T>().isNotEmpty() }
        return local.requestsOf<T>().last()
    }

    private fun variantRef(sequence: String, key: String) =
        VariantRef.newBuilder().setSequenceName(sequence).setVariantKey(key).build()

    private fun renderRef(id: String) = RenderRef.newBuilder().setRenderId(id).build()

    private fun awaitDialog() = compose.waitUntil(Harness.TIMEOUT_MS) {
        compose.onAllNodesWithTag(CONFIRM_BUTTON_TAG).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    fun everyStatusHasExactlyItsActions() {
        val expected = mapOf(
            VariantStatus.VARIANT_QUEUED to listOf(RowAction.STOP),
            VariantStatus.VARIANT_RUNNING to listOf(RowAction.SUSPEND, RowAction.STOP),
            VariantStatus.VARIANT_SUSPENDED to listOf(RowAction.RESUME_RENDER, RowAction.STOP),
            VariantStatus.VARIANT_DONE to listOf(RowAction.PAUSE_VARIANT),
            VariantStatus.VARIANT_ALREADY_RENDERED to listOf(RowAction.PAUSE_VARIANT),
            VariantStatus.VARIANT_PAUSED to listOf(RowAction.RESUME_VARIANT, RowAction.RETRY),
            VariantStatus.VARIANT_FAILED to listOf(RowAction.PAUSE_VARIANT, RowAction.RETRY),
            VariantStatus.VARIANT_UNKNOWN_CODEC to listOf(RowAction.PAUSE_VARIANT),
            VariantStatus.VARIANT_WAITING to listOf(RowAction.PAUSE_VARIANT),
        )
        for ((status, actions) in expected) assertEquals(status.name, actions, variantActions(status))
        assertEquals(listOf(RowAction.PAUSE_SEQUENCE, RowAction.DETAILS), sequenceActions(paused = false))
        assertEquals(listOf(RowAction.RESUME_SEQUENCE, RowAction.DETAILS), sequenceActions(paused = true))
        assertEquals(listOf(RowAction.RETRY, RowAction.DISMISS, RowAction.DETAILS), FAILURE_ACTIONS)
        val local = listOf(RowAction.OPEN, RowAction.REVEAL, RowAction.COPY_PATH, RowAction.DETAILS)
        val other = listOf(RowAction.COPY_PATH, RowAction.DETAILS)
        assertEquals(local, videoActions(loopback = true, fileExists = true))
        assertEquals(other, videoActions(loopback = true, fileExists = false))
        assertEquals(other, videoActions(loopback = false, fileExists = true))
    }

    @Test
    fun everyRowShowsExactlyItsButtons() {
        show()
        for ((key, status) in ControlFixtures.STATUSES) {
            assertEquals(key, variantActions(status).map { it.label }, buttons(ControlFixtures.variantTag(key)))
        }
        assertEquals(listOf(SEQUENCE_MENU), buttons(ControlFixtures.CARD))
        assertEquals(listOf("Retry", "Dismiss", "Details"), buttons(ControlFixtures.FAILURE))
        assertEquals(listOf("Open", "Reveal", "Copy path", "Details"), buttons(ControlFixtures.VIDEO))
        assertEquals(listOf("Copy path", "Details"), buttons(ControlFixtures.MISSING_VIDEO))
    }

    private fun menu(card: String): List<String> {
        button(card, SEQUENCE_MENU)
        return compose.onAllNodes(hasClickAction() and hasAnyAncestor(isPopup())).fetchSemanticsNodes()
            .map { node -> node.config[SemanticsProperties.Text].single().text }
    }

    @Test
    fun aSequenceMenuPausesTheSequence() {
        val h = show()
        assertEquals(listOf("Pause sequence", "Details"), menu(ControlFixtures.CARD))
        compose.onNode(hasText("Pause sequence") and hasAnyAncestor(isPopup())).performClick()
        assertEquals("LRT_b", h.sent<SequenceRef>("PauseSequence").sequenceName)
    }

    @Test
    fun aPausedSequenceMenuResumesTheSequence() {
        val h = show()
        assertEquals(listOf("Resume sequence", "Details"), menu(ControlFixtures.PAUSED_CARD))
        compose.onNode(hasText("Resume sequence") and hasAnyAncestor(isPopup())).performClick()
        assertEquals("LRT_p", h.sent<SequenceRef>("ResumeSequence").sequenceName)
    }

    @Test
    fun suspendSendsTheRender() {
        val h = show()
        button(ControlFixtures.variantTag("running"), "Suspend")
        assertEquals(renderRef("r-running"), h.sent<RenderRef>("SuspendRender"))
    }

    @Test
    fun resumeSendsTheRender() {
        val h = show()
        button(ControlFixtures.variantTag("suspended"), "Resume")
        assertEquals(renderRef("r-suspended"), h.sent<RenderRef>("ResumeRender"))
    }

    @Test
    fun pauseVariantSendsTheSequenceAndVariant() {
        val h = show()
        button(ControlFixtures.variantTag("done"), "Pause variant")
        assertEquals(variantRef("LRT_b", "done"), h.sent<VariantRef>("PauseVariant"))
    }

    @Test
    fun resumeVariantSendsTheSequenceAndVariant() {
        val h = show()
        button(ControlFixtures.variantTag("paused"), "Resume variant")
        assertEquals(variantRef("LRT_b", "paused"), h.sent<VariantRef>("ResumeVariant"))
    }

    @Test
    fun retrySendsTheSequenceAndVariant() {
        val h = show()
        button(ControlFixtures.variantTag("failed"), "Retry")
        assertEquals(variantRef("LRT_b", "failed"), h.sent<VariantRef>("RetryVariant"))
    }

    @Test
    fun aFailureCardRetriesItsVariant() {
        val h = show()
        button(ControlFixtures.FAILURE, "Retry")
        assertEquals(variantRef("LRT_e", "k-e"), h.sent<VariantRef>("RetryVariant"))
    }

    @Test
    fun aFailureCardIsDismissed() {
        val h = show()
        button(ControlFixtures.FAILURE, "Dismiss")
        assertEquals("f-1", h.sent<FailureRef>("DismissFailure").failureId)
    }

    @Test
    fun detailsSelectsTheRowAndShowsTheClosedPane() {
        val h = show()
        for (row in listOf(ControlFixtures.FAILURE, ControlFixtures.VIDEO)) {
            h.app.toggleDetails()
            compose.waitForIdle()
            button(row, "Details")
            compose.waitForIdle()
            assertEquals(row, h.app.ui.value.selectedKey)
            assertTrue(h.app.ui.value.detailsVisible)
            compose.onNodeWithTag(row).assertIsSelected()
            compose.onNodeWithTag(MainWindowTags.DETAILS).assertExists()
        }
        menuItem(ControlFixtures.CARD, "Details")
        compose.waitForIdle()
        assertEquals(ControlFixtures.CARD, h.app.ui.value.selectedKey)
    }

    @Test
    fun copyPathPutsThePathOnTheClipboard() {
        val h = show()
        button(ControlFixtures.MISSING_VIDEO, "Copy path")
        assertEquals(listOf("/exp/out/v-gone.mov"), h.copies.toList())
    }

    @Test
    fun stopAsksFirstAndCancellingSendsAndRemembersNothing() {
        val h = show()
        button(ControlFixtures.variantTag("queued"), "Stop")
        awaitDialog()
        compose.onNodeWithText(
            "Stop this render? ffmpeg is stopped, the partial file deleted and the variant paused.",
        ).assertExists()
        compose.onNodeWithTag(DONT_ASK_AGAIN_TAG).assertIsOff().performClick().assertIsOn()
        compose.onNodeWithText("Cancel").performClick()
        compose.waitForIdle()
        assertNull(h.app.ui.value.dialog)
        assertFalse(h.preferences.skipStopConfirmation)
        assertEquals(0, h.local.callCount("StopRender"))
    }

    @Test
    fun aConfirmedStopSendsTheRenderAndAsksAgainNextTime() {
        val h = show()
        button(ControlFixtures.variantTag("running"), "Stop")
        awaitDialog()
        compose.onNodeWithTag(CONFIRM_BUTTON_TAG).performClick()
        assertEquals(renderRef("r-running"), h.sent<RenderRef>("StopRender"))
        assertFalse(h.preferences.skipStopConfirmation)
        button(ControlFixtures.variantTag("suspended"), "Stop")
        awaitDialog()
    }

    @Test
    fun dontAskAgainStopsAtOnceFromThenOn() {
        val h = show()
        button(ControlFixtures.variantTag("running"), "Stop")
        awaitDialog()
        compose.onNodeWithTag(DONT_ASK_AGAIN_TAG).performClick()
        compose.onNodeWithTag(CONFIRM_BUTTON_TAG).performClick()
        h.sent<RenderRef>("StopRender")
        assertTrue(h.preferences.skipStopConfirmation)
        assertTrue(h.app.ui.value.preferences.confirmationsSkipped)
        button(ControlFixtures.variantTag("queued"), "Stop")
        h.awaitTrue { h.local.callCount("StopRender") == 2 }
        assertNull(h.app.ui.value.dialog)
        assertFalse(h.preferences.skipPauseAllConfirmation)
    }

    @Test
    fun pauseAllAsksTheFirstTimeOnlyAndResumingNever() {
        val h = show()
        compose.onNodeWithTag(TopBarTags.PAUSE).performClick()
        awaitDialog()
        assertEquals(0, h.local.callCount("SetGlobalPause"))
        compose.onNodeWithTag(DONT_ASK_AGAIN_TAG).assertIsOn()
        compose.onNodeWithTag(CONFIRM_BUTTON_TAG).performClick()
        assertTrue(h.sent<SetGlobalPauseRequest>("SetGlobalPause").paused)
        assertTrue(h.preferences.skipPauseAllConfirmation)
        assertFalse(h.preferences.skipStopConfirmation)
        h.app.setGlobalPause(true)
        h.awaitTrue { h.local.callCount("SetGlobalPause") == 2 }
        assertNull(h.app.ui.value.dialog)
    }

    @Test
    fun pauseAllKeepsAskingWhenTheBoxIsCleared() {
        val h = show()
        h.app.setGlobalPause(true)
        awaitDialog()
        compose.onNodeWithTag(DONT_ASK_AGAIN_TAG).performClick().assertIsOff()
        compose.onNodeWithTag(CONFIRM_BUTTON_TAG).performClick()
        h.sent<SetGlobalPauseRequest>("SetGlobalPause")
        assertFalse(h.preferences.skipPauseAllConfirmation)
        h.app.setGlobalPause(true)
        awaitDialog()
    }

    @Test
    fun resumingAllRenderingIsNotConfirmed() {
        val h = show()
        h.app.setGlobalPause(false)
        assertFalse(h.sent<SetGlobalPauseRequest>("SetGlobalPause").paused)
        assertNull(h.app.ui.value.dialog)
    }

    @Test
    fun pausingFromTheTrayOpensTheWindowToAsk() {
        val h = Harness(traySupported = true)
        harness = h
        h.app.start()
        h.await { it.connected }
        h.app.closeWindow { error("must not quit") }
        h.app.performTray(TrayAction.PAUSE_ALL) { error("must not quit") }
        assertTrue(h.app.ui.value.windowVisible)
        assertEquals(AppDialog.Confirm(Confirmation.PauseAll), h.app.ui.value.dialog)
    }

    @Test
    fun switchingServersDropsAConfirmation() {
        val h = Harness()
        harness = h
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        h.app.start()
        h.await { it.connected }
        h.app.perform(RowAction.STOP, ActionTarget.Variant("variant:x", "x", "k", "r-1"))
        assertEquals(AppDialog.Confirm(Confirmation.StopRender("r-1")), h.app.ui.value.dialog)
        h.app.selectServer("other")
        h.await { it.activeServerId == "other" }
        assertNull(h.app.ui.value.dialog)
        h.app.confirm(true)
        assertFalse(h.preferences.skipStopConfirmation)
    }

    @Test
    fun aFailedCommandShowsASnackbarWithTheMessage() {
        val h = show()
        h.local.failWith["SuspendRender"] = Status.FAILED_PRECONDITION.withDescription("the render has ended")
        button(ControlFixtures.variantTag("running"), "Suspend")
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithText("the render has ended").fetchSemanticsNodes().isNotEmpty()
        }
    }
}
