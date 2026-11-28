// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.state.ActivityFilter
import com.timelapserender.monitor.core.state.ActivityPresenter
import com.timelapserender.monitor.core.state.DetailsModel
import com.timelapserender.monitor.core.state.DetailsState
import com.timelapserender.monitor.core.state.DetailsTarget
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.trd.v1.RenderDetails
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionContractTest {
    @Test
    fun loopbackConnectionReportsApiVersionOneAndTheSandbox() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val client = trd.connected()
            val info = (client.state.value as ConnectionState.Connected).info
            assertEquals(1, info.apiVersion)
            assertTrue(info.sandbox)
            assertTrue(info.callerIsLocal)
            assertEquals(trd.lanPort, info.lanPort)
            assertTrue(info.serverId.isNotEmpty())
            assertEquals(setOf("ffmpeg", "ffprobe", "exiftool"), info.toolsList.map { it.name }.toSet())
            assertTrue(info.toolsList.toString(), info.toolsList.all { it.available })
            assertEquals(info.serverId, client.serverInfo().serverId)

            val repo = SnapshotRepository(client, trd.scope)
            repo.start()
            // The daemon's first snapshot, before its first scan, names no config yet.
            repo.state.reach("a scanned snapshot") { it.snapshot?.configPath == trd.configFile.toString() }
        }
    }

    @Test
    fun aSequenceMovesThroughItsStagesIntoRecentlyRendered() = TrdProcess.create().use { trd ->
        trd.keepSequences("a", renders = 1)
        trd.script(trd.held())
        trd.start()
        trd.test {
            val client = trd.connected()
            val repo = SnapshotRepository(client, trd.scope)
            repo.start()
            val presenter = ActivityPresenter(repo, MutableStateFlow(ActivityFilter()), client, trd.scope)
            val name = sequenceName('a')

            trd.exportFrames('a', 1..30)
            val exporting = presenter.ui.reach("half of A exported") { ui ->
                ui.inProgress.any { it.name == name && it.framesText == "30 / 60 frames" }
            }.inProgress.single()
            assertEquals(Stage.STAGE_EXPORTING, exporting.stage)
            assertEquals("Exporting", exporting.stageText)
            assertEquals(0.5f, exporting.progress, 0.001f)
            assertEquals("variants appear only once the sequence is complete", 0, exporting.variants.size)

            trd.exportFrames('a', 31..60)
            val rendering = presenter.ui.reach("a render of A is held") { ui ->
                ui.inProgress.any { card ->
                    card.name == name && card.variants.any { it.status == VariantStatus.VARIANT_RUNNING && it.progress != null }
                }
            }.inProgress.single()
            assertEquals(exporting.key, rendering.key)
            assertEquals(Stage.STAGE_RENDERING, rendering.stage)
            assertEquals("60 / 60 frames", rendering.framesText)
            assertEquals(listOf(PRORES_MQ, H264, PRORES_UHQ), rendering.variants.map { it.variantKey })
            assertEquals(listOf("rendering", "queued", "queued"), rendering.variants.map { it.statusText })
            val running = rendering.variants.first()
            assertTrue(running.progressText, running.progressText.startsWith("1 / 60"))

            val details = DetailsModel(client, trd.scope)
            details.load(DetailsTarget.Render(running.renderId))
            val live = loaded(details)
            assertEquals(name, live.sequenceName)
            assertEquals(VariantStatus.VARIANT_RUNNING, live.status)
            assertEquals(60, live.totalFrames)
            assertTrue(live.pid > 0)
            assertTrue(live.commandLine, live.commandLine.contains("-progress"))

            trd.release()
            // The first state without the card already lists its videos (overview §6.2 S7).
            val done = presenter.ui.reach("A leaves in progress") { ui -> ui.inProgress.none { it.name == name } }
            assertEquals(3, done.recent.size)
            assertTrue(done.recent.all { it.fileExists && it.framesText == "60 frames" })
            assertEquals("Nothing in progress", done.inProgressEmptyText)

            val video = done.recent.first()
            details.load(DetailsTarget.RenderedVideo(video.key))
            val finished = loaded(details)
            assertEquals(video.outputPath, finished.outputPath)
            assertEquals(VariantStatus.VARIANT_DONE, finished.status)
            assertTrue(finished.finishedAtMs > 0)
        }
    }

    @Test
    fun olderRenderedVideosArePagedAndSearched() = TrdProcess.create().use { trd ->
        // Three complete sequences times eleven frame rates: more videos than a snapshot lists.
        trd.keepSequences("bcd", renders = 8) { config ->
            config["rendering"] = (1..11).map { mapOf("codec" to "h264", "frame_rate" to it, "resolution" to listOf(720)) }
        }
        trd.start()
        trd.test {
            val client = trd.connected()
            val snapshot = client.awaitSnapshot("33 videos") {
                it.recentlyRenderedTotal == 33 && it.inProgressCount == 0
            }
            assertEquals(30, snapshot.recentlyRenderedCount)

            val repo = SnapshotRepository(client, trd.scope)
            repo.start()
            val presenter = ActivityPresenter(repo, MutableStateFlow(ActivityFilter()), client, trd.scope, olderPageSize = 2)
            val first = presenter.ui.reach("the snapshot's videos") { it.recent.size == 30 }
            assertTrue(first.canLoadOlder)

            presenter.loadOlder()
            val second = presenter.ui.reach("one older page") { it.recent.size == 32 && !it.loadingOlder }
            assertTrue(second.canLoadOlder)
            presenter.loadOlder()
            val last = presenter.ui.reach("the last older page") { it.recent.size == 33 && !it.loadingOlder }
            assertFalse(last.canLoadOlder)
            assertEquals(33, last.recent.map { it.key }.toSet().size)

            val page = client.listRenderedVideos(limit = 5)
            assertEquals(last.recent.take(5).map { it.key }, page.videosList.map { it.id })
            assertTrue(page.more)
            val found = client.listRenderedVideos(search = "SANDBOX-C")
            assertEquals(11, found.videosCount)
            assertTrue(found.videosList.all { it.sequenceName == sequenceName('c') })
        }
    }

    private suspend fun loaded(model: DetailsModel): RenderDetails {
        val state = model.state.reach("the details") { it !is DetailsState.Loading }
        assertTrue(state.toString(), state is DetailsState.Loaded)
        return (state as DetailsState.Loaded).details
    }
}
