// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigState
import com.timelapserender.trd.v1.Counts
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.ListRenderedVideosRequest
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.Problem
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Severity
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import io.grpc.Status
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import kotlin.time.Duration.Companion.seconds

class ActivityPresenterTest {
    private val now = Instant.parse("2025-11-27T15:00:00Z").toEpochMilli()
    private val utc = ZoneId.of("UTC")

    private fun ms(iso: String) = Instant.parse(iso).toEpochMilli()

    private fun variant(key: String, codec: String, res: String, fps: Int, status: VariantStatus, f: VariantRow.Builder.() -> Unit = {}) =
        VariantRow.newBuilder().setVariantKey(key).setCodec(codec).setResolutionLabel(res).setFrameRate(fps)
            .setStatus(status).setSizeBytes(-1).setEtaSeconds(-1).apply(f).build()

    private fun video(id: String, file: String, f: RenderedVideo.Builder.() -> Unit = {}) =
        RenderedVideo.newBuilder().setId(id).setOutputFilename(file).setSequenceName(file.substringBeforeLast('.'))
            .setExportPath("/exp").setCodec("H264").setResolutionLabel("OriRes").setFrameRate(30).setFrameCount(300)
            .setSizeBytes(1024).setFileExists(true).setRenderedAtMs(now - 60_000).apply(f).build()

    private val gb124 = (12.4 * 1024 * 1024 * 1024).toLong()

    private fun scripted(): Snapshot = Snapshot.newBuilder()
        .setRevision(5).setPhase(Phase.PHASE_RENDERING)
        .setCounts(Counts.newBuilder().setRendering(2).setQueued(1).setExporting(3).setFailures(1))
        .addProblems(Problem.newBuilder().setId("p-err").setSeverity(Severity.SEVERITY_ERROR).setTitle("ffmpeg missing").setDetail("ffmpeg was not found").setFix("Install ffmpeg"))
        .addProblems(Problem.newBuilder().setId("p-warn").setSeverity(Severity.SEVERITY_WARNING).setTitle("Disk almost full").setDetail("10 GB left").setFix("Free space"))
        .addFailures(
            FailureCard.newBuilder().setId("f-1").setKindLabel("Corrupt output").setOutputFilename("LRT_e.mov").setSequenceName("LRT_e")
                .setExportPath("/exp").setSummary("ffmpeg produced a corrupt file").setOccurrences(3).setVariantAutoPaused(true),
        )
        .addFailures(
            FailureCard.newBuilder().setId("f-2").setKindLabel("Disk full").setOutputFilename("LRT_f.mov").setSequenceName("LRT_f")
                .setExportPath("/exp").setSummary("No space left").setOccurrences(1),
        )
        .addInProgress(
            SequenceCard.newBuilder().setId("/exp/LRT_a").setName("LRT_a").setExportPath("/exp").setStage(Stage.STAGE_EXPORTING)
                .setExpectedFrames(1500).setExportedFrames(1234).setExportFramesPerMinute(41.0).setExportEtaSeconds(360)
                .setExportSizeBytes(gb124)
                .addVariants(variant("k-wait", "H264", "720w", 30, VariantStatus.VARIANT_WAITING)),
        )
        .addInProgress(
            SequenceCard.newBuilder().setId("/exp/LRT_b").setName("LRT_b").setExportPath("/exp").setStage(Stage.STAGE_RENDERING)
                .setExpectedFrames(1500).setExportedFrames(1500).setExportEtaSeconds(-1).setExportSizeBytes(-1)
                .setNote("3 frames missing")
                .addVariants(
                    variant("prores-2160-30", "ProResMQ", "2160p", 30, VariantStatus.VARIANT_RUNNING) {
                        setFrame(843).setTotalFrames(1500).setFps(4.2).setEtaSeconds(180).setProgressKnown(true).setRenderId("r-1")
                    },
                )
                .addVariants(variant("h264-720-24", "H264", "720w", 24, VariantStatus.VARIANT_DONE) { setSizeBytes(231L * 1024 * 1024) })
                .addVariants(variant("h264-ori-30", "H264", "OriRes", 30, VariantStatus.VARIANT_ALREADY_RENDERED) { setSizeBytes(1024) })
                .addVariants(variant("h265-720-30", "H265", "720w", 30, VariantStatus.VARIANT_PAUSED))
                .addVariants(variant("av1-720-30", "AV1", "720w", 30, VariantStatus.VARIANT_FAILED) { setMessage("ffmpeg exited 1").setFailureId("f-1") })
                .addVariants(variant("nocodec", "Foo", "720w", 30, VariantStatus.VARIANT_UNKNOWN_CODEC)),
        )
        .addRecentlyRendered(
            video("v-1", "LRT_c_ProResMQ.mov") {
                setSequenceName("LRT_c").setCodec("ProResMQ").setResolutionLabel("2160p").setFrameCount(1500).setSizeBytes(gb124)
                    .setRenderedAtMs(now - 12 * 60_000)
            },
        )
        .addRecentlyRendered(
            video("v-2", "LRT_d.mp4") {
                setSequenceName("LRT_d").setFrameCount(6000).setSizeBytes(231L * 1024 * 1024).setFileExists(false)
                    .setRenderedAtMs(ms("2025-11-20T14:02:00Z"))
            },
        )
        .setRecentlyRenderedTotal(2)
        .build()

    private fun describe(ui: ActivityUi): List<String> = buildList {
        val h = ui.header
        add("header ${h.serverName} | ${h.connectionText} | ${h.phaseText}/${h.phaseRole} | ${h.countsLine} | pause=${h.globalPause} sandbox=${h.sandbox}")
        ui.problems.forEach { add("problem ${it.key} ${it.severity} ${it.title} | ${it.detail} | ${it.fix}") }
        ui.failures.forEach {
            add("failure ${it.key} [${it.kindLabel}] ${it.outputFilename} | ${it.summary} | ${it.occurrencesText} | ${it.autoPausedText}")
        }
        ui.inProgressEmptyText?.let { add("empty: $it") }
        ui.inProgress.forEach { c ->
            add(
                "card ${c.key} ${c.name} [${c.stageText}/${c.stageRole}] paused=${c.paused} | ${c.framesText} | ${c.exportRateText} | " +
                    "${c.exportSizeText} | ${c.secondaryText} | note='${c.note}'",
            )
            c.variants.forEach { v ->
                add("variant ${v.key} ${v.label} [${v.statusText}/${v.statusRole}] '${v.progressText}' size=${v.sizeText} msg='${v.message}'")
            }
        }
        ui.recent.forEach {
            add(
                "recent ${it.key} ${it.outputFilename} | ${it.variantLabel} | ${it.sizeText} | ${it.framesText} | ${it.durationText} | " +
                    "${it.relativeTimeText} (${it.absoluteTimeText}) | ${it.exportPath} | missing=${it.missingText}",
            )
        }
        ui.recentEmptyText?.let { add("empty: $it") }
        add("older=${ui.canLoadOlder}")
    }

    private fun TestScope.presenter(
        env: Env,
        filter: MutableStateFlow<ActivityFilter> = MutableStateFlow(ActivityFilter()),
        repoClock: () -> Long = { now },
        delayer: suspend (Long) -> Unit = { delay(it) },
    ): ActivityPresenter {
        val repo = SnapshotRepository(env.client, backgroundScope, clock = repoClock)
        repo.start()
        env.client.start()
        return ActivityPresenter(repo, filter, env.client, backgroundScope, clock = { now }, zone = utc, delayer = delayer)
    }

    private fun Env.script(snapshot: Snapshot, recentDays: Int = 30) {
        fake.snapshots = listOf(snapshot)
        fake.configState = ConfigState.newBuilder().setConfig(Config.newBuilder().setRecentRenderHistoryDays(recentDays)).build()
    }

    @Test
    fun scriptedSnapshotEqualsTheGoldenList() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.script(scripted())
            val p = presenter(env)
            val ui = p.ui.first { it.header.connectionText == "Connected" && it.problems.isNotEmpty() }
            assertEquals(
                listOf(
                    "header Fake trd | Connected | Rendering/Active | 2 rendering · 1 queued · 3 exporting · 1 failed | pause=false sandbox=false",
                    "problem p-err SEVERITY_ERROR ffmpeg missing | ffmpeg was not found | Install ffmpeg",
                    "problem p-warn SEVERITY_WARNING Disk almost full | 10 GB left | Free space",
                    "failure f-1 [Corrupt output] LRT_e.mov | ffmpeg produced a corrupt file | ×3 | variant paused after repeated corrupt renders",
                    "failure f-2 [Disk full] LRT_f.mov | No space left | null | null",
                    "card /exp/LRT_a LRT_a [Exporting/Active] paused=false | 1,234 / 1,500 frames | 41 frames/min · ~6 min left | 12.4 GB | /exp | note=''",
                    "card /exp/LRT_b LRT_b [Rendering/Active] paused=false | 1,500 / 1,500 frames | null | null | /exp | note='3 frames missing'",
                    "variant /exp/LRT_b/prores-2160-30 ProResMQ · 2160p · 30 fps [rendering/Active] '843 / 1,500 · 4.2 fps · ~3 min' size=null msg=''",
                    "variant /exp/LRT_b/h264-720-24 H264 · 720w · 24 fps [done/Ok] '' size=231 MB msg=''",
                    "variant /exp/LRT_b/h264-ori-30 H264 · OriRes · 30 fps [exists/Ok] '' size=1.0 KB msg=''",
                    "variant /exp/LRT_b/h265-720-30 H265 · 720w · 30 fps [paused/Warning] '' size=null msg=''",
                    "variant /exp/LRT_b/av1-720-30 AV1 · 720w · 30 fps [failed/Error] '' size=null msg='ffmpeg exited 1'",
                    "variant /exp/LRT_b/nocodec Foo · 720w · 30 fps [unknown codec/Error] '' size=null msg=''",
                    "recent v-1 LRT_c_ProResMQ.mov | ProResMQ · 2160p · 30 fps | 12.4 GB | 1,500 frames | 50.0 s | 12 min ago (14:48) | /exp | missing=null",
                    "recent v-2 LRT_d.mp4 | H264 · OriRes · 30 fps | 231 MB | 6,000 frames | 3:20 | Nov 20 (Nov 20 14:02) | /exp | missing=moved or deleted",
                    "older=false",
                ),
                describe(ui),
            )
            assertEquals(
                listOf(
                    "p-err", "p-warn", "f-1", "f-2", "/exp/LRT_a", "/exp/LRT_b",
                    "/exp/LRT_b/prores-2160-30", "/exp/LRT_b/h264-720-24", "/exp/LRT_b/h264-ori-30",
                    "/exp/LRT_b/h265-720-30", "/exp/LRT_b/av1-720-30", "/exp/LRT_b/nocodec", "v-1", "v-2",
                ),
                ui.keys,
            )
        }
    }

    @Test
    fun progressBarsFollowTheCountsAndIndeterminateWhenUnknown() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val unknown = variant("u", "H264", "720w", 30, VariantStatus.VARIANT_RUNNING) { setFps(1.0) }
            val snapshot = scripted().toBuilder().setInProgress(1, scripted().getInProgress(1).toBuilder().addVariants(unknown)).build()
            env.script(snapshot)
            val ui = presenter(env).ui.first { it.inProgress.size == 2 }
            assertEquals(1234f / 1500f, ui.inProgress[0].progress, 0.0001f)
            val running = ui.inProgress[1].variants[0]
            assertEquals(843f / 1500f, running.progress!!, 0.0001f)
            assertFalse(running.indeterminate)
            val indeterminate = ui.inProgress[1].variants.last()
            assertNull(indeterminate.progress)
            assertTrue(indeterminate.indeterminate)
            assertEquals("1.0 fps", indeterminate.progressText)
        }
    }

    @Test
    fun phaseChipPauseSandboxAndCountsLine() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.serverInfo = env.fake.serverInfo.toBuilder().setSandbox(true).build()
            env.script(
                Snapshot.newBuilder().setPhase(Phase.PHASE_PAUSED).setGlobalPause(true)
                    .setCounts(Counts.newBuilder().setSuspended(1).setPausedSequences(2)).build(),
            )
            val h = presenter(env).ui.first { it.header.connectionText == "Connected" && it.header.phaseText == "Paused" }.header
            assertEquals(ChipRole.Warning, h.phaseRole)
            assertTrue(h.globalPause)
            assertTrue(h.sandbox)
            assertEquals("1 suspended · 2 paused", h.countsLine)
        }
    }

    @Test
    fun connectionTextForAnIncompatibleServer() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.serverInfo = env.fake.serverInfo.toBuilder().setApiVersion(2).build()
            val text = presenter(env).ui.first { it.header.connectionText.startsWith("This trd") }.header.connectionText
            assertEquals("This trd uses API version 2; this app supports version 1.", text)
        }
    }

    @Test
    fun connectionTextForAnOfflineServerNamesTheLastUpdate() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["GetServerInfo"] = Status.UNAVAILABLE
            val p = presenter(env, repoClock = { ms("2025-11-27T14:02:00Z") })
            p.ui.first { it.header.connectionText == "Offline — last update 14:02" }
        }
    }

    @Test
    fun filterIsACaseInsensitiveSubstringOfSequenceAndFileNamesAndAnExportPath() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.script(scripted())
            val filter = MutableStateFlow(ActivityFilter())
            val p = presenter(env, filter)
            p.ui.first { it.recent.size == 2 }
            filter.value = ActivityFilter(search = "lrt_c")
            assertEquals(listOf("p-err", "p-warn", "v-1"), p.ui.first { it.recent.size == 1 }.keys)
            filter.value = ActivityFilter(search = "LRT_E.MOV")
            assertEquals(listOf("p-err", "p-warn", "f-1"), p.ui.first { it.failures.size == 1 && it.recent.isEmpty() }.keys)
            filter.value = ActivityFilter(search = "lrt_b")
            val b = p.ui.first { it.inProgress.size == 1 }
            assertEquals("/exp/LRT_b", b.inProgress.single().key)
            assertEquals(6, b.inProgress.single().variants.size)
            filter.value = ActivityFilter(exportPath = "/elsewhere")
            val none = p.ui.first { it.inProgress.isEmpty() && it.failures.isEmpty() && it.recent.isEmpty() }
            assertEquals(listOf("p-err", "p-warn"), none.keys) // problems are never filtered
            assertEquals("Nothing in progress", none.inProgressEmptyText)
            filter.value = ActivityFilter(search = "", exportPath = "/exp")
            assertEquals(14, p.ui.first { it.recent.size == 2 }.keys.size)
        }
    }

    @Test
    fun emptyStatesUseTheConfiguredHistoryDays() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.script(Snapshot.newBuilder().setPhase(Phase.PHASE_IDLE).build(), recentDays = 14)
            // the config and the first snapshot arrive independently: wait for both
            val ui = presenter(env).ui.first {
                it.recentEmptyText == "Nothing rendered in the last 14 days" && it.header.phaseText == "Idle"
            }
            assertEquals("Nothing in progress", ui.inProgressEmptyText)
            assertEquals("", ui.header.countsLine)
            assertFalse(ui.canLoadOlder)
            assertTrue(ui.keys.isEmpty())
        }
    }

    @Test
    fun loadOlderPagesByBeforeId() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val all = (1..4).map { video("v-$it", "LRT_x$it.mov") }
            env.script(
                Snapshot.newBuilder().setPhase(Phase.PHASE_IDLE).addRecentlyRendered(all[0]).addRecentlyRendered(all[1])
                    .setRecentlyRenderedTotal(4).build(),
            )
            env.fake.renderedVideos = all
            val p = presenter(env)
            assertTrue(p.ui.first { it.recent.size == 2 }.canLoadOlder)
            p.loadOlder()
            val ui = p.ui.first { it.recent.size == 4 }
            assertEquals(listOf("v-1", "v-2", "v-3", "v-4"), ui.recent.map { it.key })
            assertFalse(ui.canLoadOlder)
            val request = env.fake.requestsOf<ListRenderedVideosRequest>().single()
            assertEquals("v-2", request.beforeId)
            assertEquals(50, request.limit)
        }
    }

    @Test
    fun s7_aFinishedSequenceLeavesInProgressAndItsVideosLeadRecentlyRendered() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.script(
                Snapshot.newBuilder().setPhase(Phase.PHASE_RENDERING)
                    .addInProgress(SequenceCard.newBuilder().setId("/exp/X").setName("X").setExportPath("/exp").setStage(Stage.STAGE_RENDERING))
                    .addRecentlyRendered(video("vy-1", "LRT_y1.mov"))
                    .addRecentlyRendered(video("vy-2", "LRT_y2.mov"))
                    .addRecentlyRendered(video("v-old", "LRT_old.mov"))
                    .setRecentlyRenderedTotal(3).build(),
            )
            val ui = presenter(env).ui.first { it.recent.size == 3 }
            assertEquals(listOf("/exp/X", "vy-1", "vy-2", "v-old"), ui.keys)
        }
    }

    @Test
    fun s4s5_aProgressUpdateChangesNoKey() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val before = scripted()
            val card = before.getInProgress(1)
            val after = before.toBuilder()
                .setInProgress(1, card.toBuilder().setVariants(0, card.getVariants(0).toBuilder().setFrame(1200).setFps(5.0)))
                .build()
            env.script(before)
            val p = presenter(env)
            fun progress(ui: ActivityUi) = ui.inProgress.getOrNull(1)?.variants?.firstOrNull()?.progressText.orEmpty()
            val first = p.ui.first { progress(it).startsWith("843") }
            // the next subscription gets the update; scripting it only now means neither state can be conflated away
            env.fake.snapshots = listOf(after)
            val second = p.ui.first { progress(it).startsWith("1,200") }
            assertEquals(first.keys, second.keys)
            assertTrue(first.keys.isNotEmpty())
        }
    }

    @Test
    fun aFailedLoadOlderIsReportedAndCanBeRetried() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val all = (1..2).map { video("v-$it", "LRT_x$it.mov") }
            env.script(Snapshot.newBuilder().addRecentlyRendered(all[0]).setRecentlyRenderedTotal(2).build())
            env.fake.renderedVideos = all
            env.fake.failWith["ListRenderedVideos"] = Status.INTERNAL.withDescription("history unavailable")
            val p = presenter(env)
            p.ui.first { it.recent.size == 1 }
            p.loadOlder()
            val failed = p.ui.first { it.olderError != null }
            assertEquals("history unavailable", failed.olderError)
            assertTrue(failed.canLoadOlder)
            assertFalse(failed.loadingOlder)
            env.fake.failWith.clear()
            p.loadOlder()
            assertNull(p.ui.first { it.recent.size == 2 }.olderError)
        }
    }

    @Test
    fun aFailedConfigLoadIsRetried() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.script(Snapshot.newBuilder().build(), recentDays = 14)
            env.fake.failWith["GetConfig"] = Status.INTERNAL
            val p = presenter(env, delayer = {
                env.fake.failWith.clear()
            })
            p.ui.first { it.recentEmptyText == "Nothing rendered in the last 14 days" }
            assertTrue(env.fake.callCount("GetConfig") >= 2)
        }
    }

    @Test
    fun aVideoPushedOutOfTheSnapshotStaysOnceOlderPagesAreShown() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val all = (1..3).map { video("v-$it", "LRT_x$it.mov") }
            env.script(
                Snapshot.newBuilder().addRecentlyRendered(all[0]).addRecentlyRendered(all[1]).setRecentlyRenderedTotal(3).build(),
            )
            env.fake.renderedVideos = all
            val p = presenter(env)
            p.ui.first { it.recent.size == 2 }
            p.loadOlder()
            p.ui.first { it.recent.size == 3 }
            // the next subscription brings a new render, which pushes v-2 out of the snapshot
            env.fake.snapshots = listOf(
                Snapshot.newBuilder().addRecentlyRendered(video("v-0", "LRT_new.mov")).addRecentlyRendered(all[0])
                    .setRecentlyRenderedTotal(4).build(),
            )
            val ui = p.ui.first { state -> state.recent.any { it.key == "v-0" } }
            assertEquals(listOf("v-0", "v-1", "v-2", "v-3"), ui.recent.map { it.key })
        }
    }
}
