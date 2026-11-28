// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.state.EventWatcher
import com.timelapserender.monitor.core.state.InMemoryLastSeqStore
import com.timelapserender.monitor.core.state.LogModel
import com.timelapserender.monitor.core.state.ServerEvent
import com.timelapserender.monitor.core.state.ThumbnailCache
import com.timelapserender.trd.v1.EventKind
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files

class EventsContractTest {
    @Test
    fun eventWatcherReceivesAFinishedSequenceAndAFailedRender() = TrdProcess.create().use { trd ->
        // One variant: B's render fails and C's succeeds, both once released.
        trd.keepSequences("bc") { config ->
            config["rendering"] = listOf(mapOf("codec" to "h264", "frame_rate" to 30, "resolution" to listOf(720)))
        }
        trd.script(
            trd.held(
                rules = listOf(
                    mapOf(
                        "match" to "sandbox-b",
                        "exit_code" to 1,
                        "write_output" to false,
                        "stderr_lines" to listOf("Unknown encoder 'broken'"),
                    ),
                ),
            ),
        )
        trd.start()
        trd.test {
            val client = trd.connected()
            client.awaitSnapshot("both renders held") { snapshot ->
                "bc".all { snapshot.statusOf(it, H264) == VariantStatus.VARIANT_RUNNING }
            }
            val serverId = client.serverInfo().serverId
            val store = InMemoryLastSeqStore()
            val events = Channel<ServerEvent>(Channel.UNLIMITED)
            val watching = launch { EventWatcher(client, store, serverId).events().collect { events.send(it) } }
            // The watcher starts at the newest entry: what happens from now on is delivered.
            await("the watcher's starting point") { store.get(serverId) }

            trd.release()
            val received = HashMap<EventKind, ServerEvent>()
            withTimeoutOrNull(HANG_GUARD_MS) {
                while (received.size < 2) events.receive().let { received[it.kind] = it }
            }
            watching.cancel()

            val finished = received[EventKind.EVENT_SEQUENCE_FINISHED]
            assertNotNull("$received", finished)
            assertEquals(sequenceName('c'), finished!!.sequenceName)
            assertEquals(1, finished.videoCount)
            assertTrue(finished.totalBytes > 0)
            val failed = received[EventKind.EVENT_RENDER_FAILED]
            assertNotNull("$received", failed)
            assertEquals(sequenceName('b'), failed!!.sequenceName)
            assertEquals(client.getSnapshot().failuresList.single().id, failed.failureId)
        }
    }

    @Test
    fun thumbnailCacheFetchesARealThumbnail() = TrdProcess.create().use { trd ->
        trd.keepSequences("b")
        trd.start()
        trd.test {
            val client = trd.connected()
            val video = client.awaitSnapshot("B is rendered") { it.recentlyRenderedTotal == 3 }.recentlyRenderedList.first()
            val serverId = client.serverInfo().serverId
            val dir = trd.root.resolve("thumbnail-cache")
            val cache = ThumbnailCache(client, serverId, dir)

            val jpeg = cache.get(video.id)!!
            assertArrayEquals("a JPEG", byteArrayOf(0xFF.toByte(), 0xD8.toByte()), jpeg.copyOf(2))
            assertArrayEquals(jpeg, Files.readAllBytes(cache.diskFiles().single()))
            assertArrayEquals(jpeg, ThumbnailCache(client, serverId, dir).get(video.id))
            val thumbnail = client.getThumbnail(video.id)
            assertArrayEquals(jpeg, thumbnail.jpeg.toByteArray())
            assertEquals(480, thumbnail.width)
            assertTrue(client.getSnapshot().recentlyRenderedList.first { it.id == video.id }.hasThumbnail)
            assertNull(cache.get("no-such-video"))
            assertEquals(1, cache.diskFiles().size)
        }
    }

    @Test
    fun logModelPagesTheLogAndFollowsIt() = TrdProcess.create().use { trd ->
        trd.keepSequences("b")
        trd.start()
        trd.test {
            val client = trd.connected()
            client.awaitSnapshot("B is rendered") { it.recentlyRenderedTotal == 3 && it.inProgressCount == 0 }
            val stored = client.listLog(limit = 1000).entriesList
            assertTrue("$stored", stored.size > 3)

            val model = LogModel(client, trd.scope, pageSize = 3)
            model.start()
            val first = model.state.reach("the first page") { !it.loading && it.rows.size >= 3 }
            assertTrue(first.hasOlder)
            await("the whole log", { model.state.value.toString() }) {
                model.loadOlder()
                model.state.value.takeIf { !it.hasOlder }
            }
            val rows = model.state.value.rows
            val seqs = rows.map { it.seq }
            assertEquals("newest first, no duplicates", seqs.distinct().sortedDescending(), seqs)
            assertTrue(seqs.containsAll(stored.map { it.seq }))
            assertNull(model.state.value.error)

            // A new entry arrives over WatchLog.
            client.rescanNow()
            val live = model.state.reach("the live entry") { ui -> ui.rows.any { it.message == "rescan now" } }
            assertTrue(live.rows.first { it.message == "rescan now" }.seq > seqs.first())
            model.stop()
        }
    }
}
