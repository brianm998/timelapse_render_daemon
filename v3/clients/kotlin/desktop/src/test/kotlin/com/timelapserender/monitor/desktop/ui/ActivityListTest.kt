// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertContentDescriptionContains
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.unit.dp
import com.google.protobuf.ByteString
import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.jpeg
import com.timelapserender.monitor.desktop.ui.rows.POSTER_PLACEHOLDER_TAG
import com.timelapserender.monitor.desktop.ui.rows.POSTER_TAG
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.Problem
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Severity
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.Thumbnail
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import io.grpc.Status
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.time.ZoneOffset

class ActivityListTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private val now = System.currentTimeMillis()
    private val renderedAt = now - 12 * 60_000 - 20_000

    private fun variant(key: String, codec: String, status: VariantStatus, f: VariantRow.Builder.() -> Unit = {}) =
        VariantRow.newBuilder().setVariantKey(key).setCodec(codec).setResolutionLabel("720w").setFrameRate(30)
            .setStatus(status).setSizeBytes(-1).setEtaSeconds(-1).apply(f).build()

    private fun video(id: String, f: RenderedVideo.Builder.() -> Unit = {}) =
        RenderedVideo.newBuilder().setId(id).setOutputFilename("$id.mov").setSequenceName(id).setExportPath("/exp")
            .setCodec("H264").setResolutionLabel("OriRes").setFrameRate(30).setFrameCount(1500).setSizeBytes(1024)
            .setFileExists(true).setRenderedAtMs(renderedAt).apply(f).build()

    private fun exporting(name: String, path: String = "/exp") = SequenceCard.newBuilder()
        .setId("$path/$name").setName(name).setExportPath(path).setStage(Stage.STAGE_EXPORTING).setExpectedFrames(1500)
        .setExportedFrames(1234).setExportFramesPerMinute(41.0).setExportEtaSeconds(360).setExportSizeBytes(1024)

    private fun rendering(name: String) = SequenceCard.newBuilder()
        .setId("/exp/$name").setName(name).setExportPath("/exp").setStage(Stage.STAGE_RENDERING).setExpectedFrames(1500)
        .setExportedFrames(1500).setExportEtaSeconds(-1).setExportSizeBytes(-1).setNote("3 frames missing")
        .addVariants(
            variant("run", "ProResMQ", VariantStatus.VARIANT_RUNNING) {
                setFrame(843).setTotalFrames(1500).setFps(4.2).setEtaSeconds(180).setProgressKnown(true)
            },
        )
        .addVariants(variant("blind", "VP9", VariantStatus.VARIANT_RUNNING))
        .addVariants(variant("done", "H264", VariantStatus.VARIANT_DONE) { setSizeBytes(231L * 1024 * 1024) })
        .addVariants(variant("exists", "H265", VariantStatus.VARIANT_ALREADY_RENDERED) { setSizeBytes(1024) })
        .addVariants(variant("paused", "AV1", VariantStatus.VARIANT_PAUSED))
        .addVariants(variant("failed", "DNxHR", VariantStatus.VARIANT_FAILED) { setMessage("ffmpeg exited 1") })
        .addVariants(variant("unknown", "Foo", VariantStatus.VARIANT_UNKNOWN_CODEC))

    private fun everySection(): Snapshot = Snapshot.newBuilder()
        .addProblems(
            Problem.newBuilder().setId("p-err").setSeverity(Severity.SEVERITY_ERROR).setTitle("ffmpeg missing")
                .setDetail("ffmpeg was not found").setFix("Install ffmpeg"),
        )
        .addProblems(
            Problem.newBuilder().setId("p-warn").setSeverity(Severity.SEVERITY_WARNING).setTitle("Disk almost full")
                .setDetail("10 GB left").setFix("Free space"),
        )
        .addFailures(
            FailureCard.newBuilder().setId("f-1").setKindLabel("Corrupt output").setOutputFilename("LRT_e.mov")
                .setSummary("ffmpeg produced a corrupt file").setOccurrences(3).setVariantAutoPaused(true),
        )
        .addInProgress(exporting("LRT_a"))
        .addInProgress(rendering("LRT_b"))
        .addRecentlyRendered(video("v-1"))
        .addRecentlyRendered(video("v-2") { setFileExists(false) })
        .setRecentlyRenderedTotal(2)
        .build()

    /** Connects an app to a fake server with [snapshot] and waits until [ready]. */
    private fun connect(snapshot: Snapshot, h: Harness = Harness(), ready: (ActivityUi) -> Boolean): ActivityUi {
        harness = h
        h.local.snapshots = listOf(snapshot)
        h.app.start()
        return h.await { ui -> ui.activity?.let(ready) == true }.activity!!
    }

    private var activity by mutableStateOf(activityUi())

    /** The list alone, tall enough that every row is composed. */
    private fun showList(first: ActivityUi, posters: PosterLoader = PosterLoader { null }) {
        activity = first
        val listState = LazyListState()
        val keeper = ScrollKeeper()
        compose.setContent {
            MaterialTheme {
                Box(Modifier.requiredSize(900.dp, 3000.dp)) {
                    ActivityList(activity, listState, keeper, posters, true, null, {}, {})
                }
            }
        }
    }

    private fun texts(tag: String): List<String> = compose.onNodeWithTag(tag).fetchSemanticsNode()
        .config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text }

    private fun top(tag: String) = compose.onNodeWithTag(tag).fetchSemanticsNode().positionInRoot.y

    private fun assertInOrder(vararg tags: String) {
        val tops = tags.map { top(it) }
        assertEquals(tags.toList(), tags.toList().zip(tops).sortedBy { it.second }.map { it.first })
        assertEquals(tops.size, tops.distinct().size)
    }

    private fun exists(tag: String) = compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty()

    private fun awaitTag(tag: String) = compose.waitUntil(Harness.TIMEOUT_MS) { exists(tag) }

    private fun awaitNoTag(tag: String) = compose.waitUntil(Harness.TIMEOUT_MS) { !exists(tag) }

    private fun awaitText(text: String) = compose.waitUntil(Harness.TIMEOUT_MS) {
        compose.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    fun aScriptedSnapshotRendersEverySectionInOrderWithItsTexts() {
        showList(connect(everySection()) { it.problems.isNotEmpty() })
        val card = "card:/exp/LRT_b"
        val variant = { key: String -> "variant:/exp/LRT_b/$key" }
        assertInOrder(
            "problem:p-err", "problem:p-warn", "header:attention", "failure:f-1", "header:progress", "card:/exp/LRT_a",
            card, variant("run"), variant("blind"), variant("done"), variant("exists"), variant("paused"),
            variant("failed"), variant("unknown"), "header:recent", "video:v-1", "video:v-2",
        )
        assertEquals(listOf("Needs attention"), texts("header:attention"))
        assertEquals(listOf("In progress"), texts("header:progress"))
        assertEquals(listOf("Recently rendered"), texts("header:recent"))
        assertEquals(listOf("ffmpeg missing", "ffmpeg was not found", "Install ffmpeg"), texts("problem:p-err"))
        compose.onNodeWithTag("problem:p-err").assertContentDescriptionContains("Error")
        compose.onNodeWithTag("problem:p-warn").assertContentDescriptionContains("Warning")
        assertEquals(
            listOf(
                "Corrupt output", "LRT_e.mov", "×3", "ffmpeg produced a corrupt file",
                "variant paused after repeated corrupt renders",
            ),
            texts("failure:f-1"),
        )
        assertEquals(
            listOf("LRT_a", "Exporting", "1,234 / 1,500 frames", "41 frames/min · ~6 min left", "1.0 KB", "/exp"),
            texts("card:/exp/LRT_a"),
        )
        assertEquals(
            listOf("LRT_b", "Rendering", "1,500 / 1,500 frames", "/exp", "3 frames missing"),
            texts(card),
        )
        fun label(codec: String) = "$codec · 720w · 30 fps"
        assertEquals(listOf(label("ProResMQ"), "rendering", "843 / 1,500 · 4.2 fps · ~3 min"), texts(variant("run")))
        assertEquals(listOf(label("VP9"), "rendering"), texts(variant("blind")))
        assertEquals(listOf(label("H264"), "231 MB"), texts(variant("done")))
        assertEquals(listOf(label("H265"), "exists", "1.0 KB"), texts(variant("exists")))
        assertEquals(listOf(label("AV1"), "paused"), texts(variant("paused")))
        assertEquals(listOf(label("DNxHR"), "failed", "ffmpeg exited 1"), texts(variant("failed")))
        assertEquals(listOf(label("Foo"), "unknown codec"), texts(variant("unknown")))
        // The finished variant's check mark.
        compose.onNode(
            hasContentDescription("Status: done") and hasAnyAncestor(hasTestTag(variant("done"))),
            useUnmergedTree = true,
        ).assertExists()
        val video = listOf("H264 · OriRes · 30 fps", "1.0 KB", "1,500 frames", "50.0 s", "12 min ago", "/exp")
        assertEquals(listOf("v-1.mov") + video, texts("video:v-1"))
        assertEquals(listOf("v-2.mov") + video + "moved or deleted", texts("video:v-2"))
        compose.onNodeWithText(SHOW_OLDER).assertDoesNotExist()
    }

    @Test
    fun progressBarsShowTheFractionOrAreIndeterminate() {
        showList(connect(everySection()) { it.problems.isNotEmpty() })
        fun bar(tag: String) = compose.onAllNodes(
            SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo) and hasAnyAncestor(hasTestTag(tag)),
            useUnmergedTree = true,
        ).fetchSemanticsNodes().map { it.config[SemanticsProperties.ProgressBarRangeInfo] }
        assertEquals(1234f / 1500f, bar("card:/exp/LRT_a").single().current, 0.001f)
        assertEquals(843f / 1500f, bar("variant:/exp/LRT_b/run").single().current, 0.001f)
        assertEquals(ProgressBarRangeInfo.Indeterminate, bar("variant:/exp/LRT_b/blind").single())
        assertTrue(bar("variant:/exp/LRT_b/done").isEmpty())
    }

    @Test
    fun emptySectionsSaySo() {
        val h = Harness()
        h.local.configState = h.local.configState.toBuilder()
            .setConfig(h.local.configState.config.toBuilder().setRecentRenderHistoryDays(14)).build()
        showList(connect(Snapshot.getDefaultInstance(), h) { it.recentEmptyText?.contains("14") == true })
        assertInOrder("header:progress", "empty:progress", "header:recent", "empty:recent")
        assertEquals(listOf("Nothing in progress"), texts("empty:progress"))
        assertEquals(listOf("Nothing rendered in the last 14 days"), texts("empty:recent"))
        compose.onNodeWithTag("header:attention").assertDoesNotExist()
        compose.onNodeWithText(SHOW_OLDER).assertDoesNotExist()
    }

    @Test
    fun aFinishedSequenceMovesToRecentlyRenderedInTheSameFrame() {
        val before = Snapshot.newBuilder()
            .addInProgress(exporting("LRT_a"))
            .addInProgress(rendering("LRT_b"))
            .addRecentlyRendered(video("old"))
            .setRecentlyRenderedTotal(1)
            .build()
        val after = Snapshot.newBuilder()
            .addInProgress(exporting("LRT_a"))
            .addRecentlyRendered(video("LRT_b-1"))
            .addRecentlyRendered(video("LRT_b-2"))
            .addRecentlyRendered(video("old"))
            .setRecentlyRenderedTotal(3)
            .build()
        val first = connect(before) { it.inProgress.size == 2 }
        val h = harness!!
        h.local.snapshots = listOf(after)
        val second = h.await { it.activity?.inProgress?.size == 1 }.activity!!
        compose.mainClock.autoAdvance = false
        showList(first)
        compose.mainClock.advanceTimeByFrame()
        compose.onNodeWithTag("card:/exp/LRT_b").assertExists()
        compose.onNodeWithTag("video:LRT_b-1").assertDoesNotExist()

        compose.runOnUiThread { activity = second }
        compose.mainClock.advanceTimeByFrame()

        compose.onNodeWithTag("card:/exp/LRT_b").assertDoesNotExist()
        compose.onNodeWithTag("variant:/exp/LRT_b/run").assertDoesNotExist()
        // Directly below the remaining card, ahead of the older video.
        assertInOrder("card:/exp/LRT_a", "header:recent", "video:LRT_b-1", "video:LRT_b-2", "video:old")
    }

    @Test
    fun showOlderPagesThroughTheHistory() {
        val h = Harness()
        h.local.renderedVideos = (1..4).map { video("v$it") }
        h.local.failWith["ListRenderedVideos"] = Status.INTERNAL.withDescription("history unavailable")
        harness = h
        h.local.snapshots = listOf(
            Snapshot.newBuilder().addAllRecentlyRendered(h.local.renderedVideos.take(2)).setRecentlyRenderedTotal(4).build(),
        )
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitTag("older")
        compose.onNodeWithText(SHOW_OLDER).performClick()
        awaitText("history unavailable")
        h.local.failWith.remove("ListRenderedVideos")
        compose.onNodeWithText(SHOW_OLDER).performClick()
        awaitNoTag("older")
        assertEquals(listOf("v3", "v4"), h.app.ui.value.activity!!.recent.drop(2).map { it.key })
    }

    @Test
    fun theTopBarSearchAndExportPathFilterTheList() {
        val h = Harness()
        harness = h
        h.local.snapshots = listOf(
            Snapshot.newBuilder().addInProgress(exporting("alpha", "/one")).addInProgress(exporting("beta", "/two")).build(),
        )
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitTag("card:/one/alpha")
        h.app.setSearch("BET")
        awaitNoTag("card:/one/alpha")
        compose.onNodeWithTag("card:/two/beta").assertExists()
        h.app.setSearch("")
        h.app.setExportPath("/one")
        awaitNoTag("card:/two/beta")
        awaitTag("card:/one/alpha")
    }

    @Test
    fun aClickSelectsTheRow() {
        val h = Harness()
        harness = h
        h.local.snapshots = listOf(everySection())
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitTag("failure:f-1")
        compose.onNodeWithTag("failure:f-1").assertIsNotSelected().performClick()
        compose.onNodeWithTag("failure:f-1").assertIsSelected()
        assertEquals("failure:f-1", h.app.ui.value.selectedKey)
        compose.onNodeWithTag("card:/exp/LRT_a").performClick()
        compose.onNodeWithTag("card:/exp/LRT_a").assertIsSelected()
        compose.onNodeWithTag("failure:f-1").assertIsNotSelected()
    }

    @OptIn(ExperimentalTestApi::class)
    @Test
    fun hoveringTheRelativeTimeShowsTheAbsoluteTime() {
        showList(connect(everySection().toBuilder().clearRecentlyRendered().addRecentlyRendered(video("v-1")).build()) {
            it.recent.size == 1
        })
        val absolute = Format.absoluteTime(renderedAt, System.currentTimeMillis(), ZoneOffset.UTC)
        compose.onNodeWithText(absolute).assertDoesNotExist()
        compose.onNodeWithText("12 min ago", useUnmergedTree = true).performMouseInput { moveTo(center) }
        awaitText(absolute)
    }

    @Test
    fun postersComeFromTheServerAndAreGreyOtherwise() {
        val h = Harness()
        harness = h
        h.local.thumbnails = mapOf("with" to Thumbnail.newBuilder().setJpeg(ByteString.copyFrom(jpeg())).build())
        h.local.snapshots = listOf(
            Snapshot.newBuilder()
                .addRecentlyRendered(video("with") { setHasThumbnail(true) })
                .addRecentlyRendered(video("gone") { setHasThumbnail(true) })
                .addRecentlyRendered(video("none"))
                .setRecentlyRenderedTotal(3)
                .build(),
        )
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        awaitTag("video:with")
        fun count(tag: String, row: String) = compose.onAllNodes(
            hasTestTag(tag) and hasAnyAncestor(hasTestTag(row)),
            useUnmergedTree = true,
        ).fetchSemanticsNodes().size
        compose.waitUntil(Harness.TIMEOUT_MS) { count(POSTER_TAG, "video:with") == 1 }
        compose.waitUntil(Harness.TIMEOUT_MS) { h.local.callCount("GetThumbnail") == 2 }
        compose.waitForIdle()
        assertEquals(1, count(POSTER_PLACEHOLDER_TAG, "video:gone"))
        assertEquals(1, count(POSTER_PLACEHOLDER_TAG, "video:none"))
        assertEquals(2, h.local.callCount("GetThumbnail"))
        val box = compose.onNode(hasTestTag(POSTER_TAG), useUnmergedTree = true).fetchSemanticsNode().size
        assertEquals(16f / 9f, box.width.toFloat() / box.height, 0.05f)
    }
}
