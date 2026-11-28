// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.google.protobuf.ByteString
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.jpeg
import com.timelapserender.trd.v1.GetRenderDetailsRequest
import com.timelapserender.trd.v1.RenderDetails
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Thumbnail
import com.timelapserender.trd.v1.VariantRef
import com.timelapserender.trd.v1.VariantStatus
import io.grpc.Status
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class DetailsPaneTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private val command = "ffmpeg -start_number 7 -i /exp/LRT_b/LRT_%05d.tif /exp/out/LRT_b_running.mov"

    private fun details() = RenderDetails.newBuilder()
        .setRenderId("r-running").setSequenceName("LRT_b").setExportPath("/exp").setVariantKey("running")
        .setInputPattern("/exp/LRT_b/LRT_%05d.tif").setStartNumber(7).setTotalFrames(1500)
        .setOutputPath("/exp/out/LRT_b_running.mov").setPartialPath("/exp/out/.partial.mov").setCommandLine(command)
        .addLogTail("frame=  842 fps=4.2").addLogTail("frame=  843 fps=4.2").setStartedAtMs(1_000).setPid(4321)
        .setAdopted(true).setStatus(VariantStatus.VARIANT_RUNNING).build()

    private fun show(h: Harness = Harness(), snapshot: Snapshot = ControlFixtures.snapshot()): Harness {
        harness = h
        h.local.snapshots = listOf(snapshot)
        h.local.renderDetails = details()
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithTag(ControlFixtures.FAILURE).fetchSemanticsNodes().isNotEmpty()
        }
        return h
    }

    private fun inPane(text: String) =
        hasText(text, substring = true) and hasAnyAncestor(hasTestTag(MainWindowTags.DETAILS))

    private fun awaitInPane(text: String) = compose.waitUntil(Harness.TIMEOUT_MS) {
        compose.onAllNodes(inPane(text), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun assertInPane(vararg texts: String) {
        for (text in texts) {
            assertTrue(text, compose.onAllNodes(inPane(text), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        }
    }

    private fun paneButton(name: String) = compose.onNode(
        (hasText(name) or hasContentDescription(name)) and hasClickAction() and
            hasAnyAncestor(hasTestTag(MainWindowTags.DETAILS)),
    )

    private fun requests(h: Harness) = h.local.requestsOf<GetRenderDetailsRequest>()

    @Test
    fun aRenderShowsTheCommandLineAndTheLogTailFromGetRenderDetails() {
        val h = show()
        h.app.select(ControlFixtures.variantTag("running"))
        awaitInPane(command)
        assertEquals("r-running", requests(h).single().renderId)
        compose.onNodeWithTag(DetailsTags.COMMAND_LINE, useUnmergedTree = true).assertExists()
        compose.onNodeWithTag(DetailsTags.LOG_TAIL, useUnmergedTree = true).assertExists()
        assertInPane(
            "frame=  842 fps=4.2\nframe=  843 fps=4.2", "/exp/LRT_b/LRT_%05d.tif", "1,500", "/exp/out/LRT_b_running.mov",
            "/exp/out/.partial.mov", "4321", "Input pattern", "Start number", "Started", "Finished", "Adopted",
        )
        paneButton(COPY_COMMAND_LINE).performScrollTo().performClick()
        assertEquals(listOf(command), h.copies.toList())
    }

    @Test
    fun theVariantsControlsWorkFromThePane() {
        val h = show()
        h.app.select(ControlFixtures.variantTag("failed"))
        awaitInPane("Failure id")
        paneButton("Retry").performClick()
        h.awaitTrue { h.local.requestsOf<VariantRef>().isNotEmpty() }
        assertEquals("failed", h.local.requestsOf<VariantRef>().single().variantKey)
    }

    @Test
    fun aVariantWithoutARenderSaysSoAndAsksForNothing() {
        val h = show()
        h.app.select(ControlFixtures.variantTag("running"))
        awaitInPane(command)
        h.app.select(ControlFixtures.variantTag("paused"))
        awaitInPane("this variant has no render")
        assertEquals(1, requests(h).size)
        compose.onNodeWithTag(DetailsTags.COMMAND_LINE, useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun aSequenceShowsItsFieldsVariantsAndControls() {
        val h = show()
        h.app.select(ControlFixtures.CARD)
        awaitInPane("Raw sequence location")
        assertInPane("raw_LRT_b", "/raw/location", "LRT_b_queued.mov", "LRT_b_unknown.mov", "H264 · 720w · 30 fps")
        paneButton("Pause sequence").performClick()
        h.awaitTrue { h.local.callCount("PauseSequence") == 1 }
        assertTrue(requests(h).isEmpty())
    }

    @Test
    fun aFailureShowsTheFullReportAndCopiesIt() {
        val h = show()
        h.app.select(ControlFixtures.FAILURE)
        awaitInPane("frame=  843")
        assertEquals("f-1", requests(h).single().failureId)
        assertInPane(
            "LRT_e", "k-e", "disk full", "Disk full", "No space left", "The disk filled up", "28",
            "av_interleaved_write_frame(): no space\nConversion failed!", "ffmpeg -i failing.tif LRT_e.mov",
        )
        paneButton(COPY_REPORT).performClick()
        val report = h.copies.single()
        assertEquals(failureReport(ControlFixtures.failure, h.app::formatTime), report)
        for (field in failureFields(ControlFixtures.failure, h.app::formatTime)) {
            assertTrue(field.label, report.contains("${field.label}: ${field.value}\n"))
        }
        assertTrue(report.contains("Tool output:\nav_interleaved_write_frame(): no space\nConversion failed!\n"))
        assertTrue(report.endsWith("Command line:\nffmpeg -i failing.tif LRT_e.mov"))
    }

    @Test
    fun aRenderedVideoShowsEveryFieldAndALargePoster() {
        val h = Harness()
        h.local.thumbnail = Thumbnail.newBuilder().setJpeg(ByteString.copyFrom(jpeg())).build()
        val video = ControlFixtures.video("v-1").toBuilder().setHasThumbnail(true).build()
        show(h, ControlFixtures.snapshot().toBuilder().setRecentlyRendered(0, video).build())
        h.app.select(ControlFixtures.VIDEO)
        compose.waitUntil(Harness.TIMEOUT_MS) {
            compose.onAllNodesWithTag(DetailsTags.POSTER, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        awaitInPane(command)
        assertEquals("v-1", requests(h).single().renderedVideoId)
        for (field in videoFields(video, h.app::formatTime)) assertInPane(field.label, field.value)
        assertInPane("3840 px", "2160 px", "1,234 s", "50.0 s", "trd")
        paneButton("Reveal").performClick()
        h.awaitTrue { h.runner.calls.size == 1 }
        val poster = compose.onNodeWithTag(DetailsTags.POSTER, useUnmergedTree = true).fetchSemanticsNode().size
        assertEquals(16f / 9f, poster.width.toFloat() / poster.height, 0.05f)
    }

    @Test
    fun everyFieldOfEachItemIsListed() {
        val time = { ms: Long -> "at $ms" }
        val card = ControlFixtures.snapshot().getInProgress(0)
        // The sequence card's variants and the failure's tool output and command line are shown apart.
        assertEquals(16, sequenceFields(card, time).size)
        assertEquals(17, variantFields(card.getVariants(0)).size)
        assertEquals(14, renderFields(details(), time).size)
        assertEquals(20, videoFields(ControlFixtures.video("v"), time).size)
        assertEquals(16, failureFields(ControlFixtures.failure, time).size)
        val render = renderFields(details(), time).associate { it.label to it.value }
        assertEquals("at 1000", render["Started"])
        assertEquals("—", render["Finished"])
        val idle = renderFields(details().toBuilder().setStartNumber(-1).setPid(0).build(), time)
            .associate { it.label to it.value }
        assertEquals("not passed", idle["Start number"])
        assertEquals("not running", idle["Process id"])
    }

    @Test
    fun aFailedRequestShowsItsMessageAndRefreshAsksAgain() {
        val h = Harness()
        h.local.failWith["GetRenderDetails"] = Status.NOT_FOUND.withDescription("the render log is gone")
        show(h)
        h.app.select(ControlFixtures.variantTag("running"))
        awaitInPane("the render log is gone")
        h.local.failWith.remove("GetRenderDetails")
        paneButton(REFRESH_DETAILS).performScrollTo().performClick()
        awaitInPane(command)
        assertEquals(2, h.local.callCount("GetRenderDetails"))
    }

    @Test
    fun aNewStatusAsksAgainAndProgressDoesNot() {
        val h = show()
        val key = ControlFixtures.variantTag("running")
        h.app.select(key)
        awaitInPane(command)
        val snapshot = ControlFixtures.snapshot()
        fun withRunning(change: com.timelapserender.trd.v1.VariantRow.Builder.() -> Unit): Snapshot {
            val card = snapshot.getInProgress(0)
            val variant = card.getVariants(1).toBuilder().apply(change).build()
            return snapshot.toBuilder().setInProgress(0, card.toBuilder().setVariants(1, variant)).build()
        }
        h.local.snapshots = listOf(withRunning { setFrame(900) })
        awaitInPane("900")
        assertEquals(1, h.local.callCount("GetRenderDetails"))
        h.local.snapshots = listOf(withRunning { setStatus(VariantStatus.VARIANT_SUSPENDED) })
        h.awaitTrue { h.local.callCount("GetRenderDetails") == 2 }
    }

    @Test
    fun anItemThatLeavesTheSnapshotLeavesThePane() {
        val h = show()
        h.app.select(ControlFixtures.FAILURE)
        awaitInPane("The disk filled up")
        h.local.snapshots = listOf(ControlFixtures.snapshot().toBuilder().clearFailures().build())
        awaitInPane("Not scanned yet")
        assertNull(h.app.ui.value.selection)
    }
}
