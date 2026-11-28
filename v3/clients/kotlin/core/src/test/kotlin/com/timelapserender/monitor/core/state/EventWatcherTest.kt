// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.EventKind
import com.timelapserender.trd.v1.ListLogRequest
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import com.timelapserender.trd.v1.WatchLogRequest
import io.grpc.Status
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class EventWatcherTest {
    private fun entry(seq: Long, event: EventKind = EventKind.EVENT_NONE) =
        LogEntry.newBuilder().setSeq(seq).setLevel(LogLevel.LOG_INFO).setEvent(event)
            .setSequenceName("LRT_$seq").setOutputFilename("out_$seq.mov").setVideoCount(3).setTotalBytes(1000).build()

    private val finished = EventKind.EVENT_SEQUENCE_FINISHED
    private val failed = EventKind.EVENT_RENDER_FAILED

    @Test
    fun firstContactStartsAtTheNewestSeqAndEmitsNoOldEvents() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, finished), entry(2, failed), entry(3, finished))
            val store = InMemoryLastSeqStore()
            val passes = Channel<Unit>(Channel.UNLIMITED)
            val release = Channel<Unit>()
            val watcher = EventWatcher(env.client, store, "srv", delayer = {
                passes.send(Unit)
                release.receive()
            })
            val events = Channel<ServerEvent>(Channel.UNLIMITED)
            backgroundScope.launch { watcher.events().collect { events.send(it) } }
            passes.receive() // first contact done, the first (empty) stream ended
            assertEquals(3L, store.get("srv"))
            assertTrue(events.tryReceive().isFailure)
            val list = env.fake.requestsOf<ListLogRequest>().single()
            assertEquals(1, list.limit)
            assertEquals(LogLevel.LOG_DEBUG, list.minLevel)
            assertEquals(listOf(3L), env.fake.requestsOf<WatchLogRequest>().map { it.afterSeq })
        }
    }

    @Test
    fun reconnectsRepeatNothingAndOnlyNotifiableEventsAreEmitted() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, finished), entry(2, failed))
            val store = InMemoryLastSeqStore()
            val passes = Channel<Unit>(Channel.UNLIMITED)
            val release = Channel<Unit>()
            val watcher = EventWatcher(env.client, store, "srv", delayer = {
                passes.send(Unit)
                release.receive()
            })
            val events = Channel<ServerEvent>(Channel.UNLIMITED)
            backgroundScope.launch { watcher.events().collect { events.send(it) } }
            passes.receive()

            // while "disconnected": a finished sequence, a non-notifiable entry, a failure, another non-notifiable entry
            env.fake.logEntries = env.fake.logEntries + entry(3, finished) + entry(4, EventKind.EVENT_RENDER_STARTED) +
                entry(5, failed) + entry(6, EventKind.EVENT_SEQUENCE_READY)
            release.send(Unit)
            passes.receive()
            val a = events.receive()
            val b = events.receive()
            assertEquals(listOf(3L, 5L), listOf(a.seq, b.seq))
            assertEquals(finished, a.kind)
            assertEquals("LRT_3", a.sequenceName)
            assertEquals(3, a.videoCount)
            assertEquals(1000L, a.totalBytes)
            assertEquals(failed, b.kind)
            assertEquals("out_5.mov", b.outputFilename)
            assertEquals(6L, store.get("srv")) // every entry seen is stored, notifiable or not

            // a further reconnect repeats nothing
            release.send(Unit)
            passes.receive()
            assertTrue(events.tryReceive().isFailure)
            assertEquals(listOf(2L, 2L, 6L), env.fake.requestsOf<WatchLogRequest>().map { it.afterSeq })
        }
    }

    @Test
    fun aStoredSeqIsResumedWithoutAskingForTheNewest() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, finished), entry(2, finished), entry(3, failed))
            val store = InMemoryLastSeqStore().also { it.set("srv", 1L) }
            val passes = Channel<Unit>(Channel.UNLIMITED)
            val release = Channel<Unit>()
            val watcher = EventWatcher(env.client, store, "srv", delayer = {
                passes.send(Unit)
                release.receive()
            })
            val events = Channel<ServerEvent>(Channel.UNLIMITED)
            backgroundScope.launch { watcher.events().collect { events.send(it) } }
            passes.receive()
            assertEquals(listOf(2L, 3L), listOf(events.receive().seq, events.receive().seq))
            assertEquals(0, env.fake.callCount("ListLog"))
            assertEquals(3L, store.get("srv"))
        }
    }

    @Test
    fun seqsAreKeptPerServer() = runTest(timeout = 60.seconds) {
        val store = InMemoryLastSeqStore()
        store.set("a", 5L)
        store.set("b", 9L)
        assertEquals(5L, store.get("a"))
        assertEquals(9L, store.get("b"))
        assertEquals(null, store.get("c"))
    }

    @Test
    fun aThrowingCollectorEndsTheFlowAndItsEventIsDeliveredAgain() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, finished), entry(2, failed))
            val store = InMemoryLastSeqStore().also { it.set("srv", 1L) }
            val watcher = EventWatcher(env.client, store, "srv", delayer = { error("must not retry") })
            val thrown = runCatching { watcher.events().collect { throw IllegalStateException("notify failed") } }
            assertEquals("notify failed", thrown.exceptionOrNull()?.message)
            assertEquals(1L, store.get("srv"))
            assertEquals(2L, watcher.events().first().seq)
        }
    }

    @Test
    fun aThrowingStoreEndsTheFlow() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val store = object : LastSeqStore {
                override fun get(serverId: String): Long? = null

                override fun set(serverId: String, seq: Long): Unit = throw IllegalStateException("store failed")
            }
            val watcher = EventWatcher(env.client, store, "srv", delayer = { error("must not retry") })
            val thrown = runCatching { watcher.events().collect {} }
            assertEquals("store failed", thrown.exceptionOrNull()?.message)
        }
    }

    @Test
    fun failedRpcsAreReportedAndRetried() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, finished))
            env.fake.failWith["ListLog"] = Status.INTERNAL
            env.fake.failWith["WatchLog"] = Status.UNAVAILABLE
            val store = InMemoryLastSeqStore()
            val failures = mutableListOf<Status.Code>()
            val passes = Channel<Unit>(Channel.UNLIMITED)
            val release = Channel<Unit>()
            val watcher = EventWatcher(
                env.client, store, "srv",
                delayer = {
                    passes.send(Unit)
                    release.receive()
                },
                onRpcFailure = { failures += Status.fromThrowable(it).code },
            )
            val events = Channel<ServerEvent>(Channel.UNLIMITED)
            backgroundScope.launch { watcher.events().collect { events.send(it) } }
            passes.receive()
            assertEquals(null, store.get("srv"))
            assertEquals(0, env.fake.callCount("WatchLog"))
            env.fake.failWith.remove("ListLog")
            release.send(Unit)
            passes.receive()
            assertEquals(listOf(Status.Code.INTERNAL, Status.Code.UNAVAILABLE), failures)
            env.fake.failWith.clear()
            env.fake.logEntries = env.fake.logEntries + entry(2, failed)
            release.send(Unit)
            assertEquals(2L, events.receive().seq)
        }
    }
}
