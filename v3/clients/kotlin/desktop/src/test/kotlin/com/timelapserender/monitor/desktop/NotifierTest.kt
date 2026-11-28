// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.ui.window.Notification
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.EventWatcher
import com.timelapserender.monitor.core.state.ServerEvent
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.trd.v1.EventKind
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.FailureKind
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.WatchLogRequest
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class NotifierTest {
    private val finished = EventKind.EVENT_SEQUENCE_FINISHED
    private val failed = EventKind.EVENT_RENDER_FAILED
    private val gigabytes = 13_314_398_618L // 12.4 GB

    private fun entry(seq: Long, event: EventKind, message: String = "", failureId: String = "") =
        LogEntry.newBuilder().setSeq(seq).setLevel(LogLevel.LOG_INFO).setEvent(event)
            .setSequenceName("LRT_$seq").setOutputFilename("out_$seq.mov").setVideoCount(3)
            .setTotalBytes(gigabytes).setMessage(message).setFailureId(failureId).build()

    private fun event(kind: EventKind, videos: Int = 3, file: String = "a.mov", message: String = "", id: String = "") =
        ServerEvent(1, kind, 0, "LRT_1", file, message, videos, gigabytes, id)

    private fun snapshot(id: String, label: String) =
        Snapshot.newBuilder().addFailures(FailureCard.newBuilder().setId(id).setKindLabel(label)).build()

    /** A client of an in-process fake. */
    private class Fake(scope: CoroutineScope) : AutoCloseable {
        val service = FakeTrdService()
        private val running = service.startInProcess()
        val client = TrdClient(running.channel!!, null, scope)

        override fun close() = running.close()
    }

    /** An [EventWatcher] feeding a [Notifier]; each [pass] lets it ask for the log once more. */
    private class Watching(scope: CoroutineScope, fake: Fake, preferences: AppPreferences, sent: MutableList<Notification>) {
        private val passes = Channel<Unit>(Channel.UNLIMITED)
        private val release = Channel<Unit>()
        private val notifier = Notifier(preferences, { sent += it }) { "disk full" }
        private val watcher = EventWatcher(fake.client, preferences, "srv", delayer = {
            passes.send(Unit)
            release.receive()
        })
        val job = scope.launch { watcher.events().collect { notifier.handle(it) } }

        suspend fun firstPass() = passes.receive()

        suspend fun pass() {
            release.send(Unit)
            passes.receive()
        }
    }

    @Test
    fun aFinishedSequenceNotifiesOnceAndNotAgainAfterAReconnectOrInANewSession() = runTest(timeout = 60.seconds) {
        Fake(backgroundScope).use { fake ->
            val preferences = InMemoryAppPreferences()
            val sent = mutableListOf<Notification>()
            val first = Watching(backgroundScope, fake, preferences, sent)
            first.firstPass()

            fake.service.logEntries = listOf(entry(1, finished))
            first.pass()
            val expected = Notification(WINDOW_TITLE, "LRT_1 rendered — 3 videos (12.4 GB)")
            assertEquals(listOf(expected), sent)

            first.pass() // a reconnect
            assertEquals(listOf(expected), sent)

            first.job.cancel()
            val second = Watching(backgroundScope, fake, preferences, sent)
            second.firstPass()
            second.pass()
            assertEquals(listOf(expected), sent)
            assertEquals(1L, fake.service.requestsOf<WatchLogRequest>().last().afterSeq)
        }
    }

    @Test
    fun theAppNotifiesOnceAcrossSessions() {
        Harness().use { h ->
            h.store.save(record("other", Harness.OTHER_PORT))
            h.registry.refresh()
            h.app.start()
            h.awaitTrue { h.preferences.get("fake-server") == 0L }
            h.local.logEntries = listOf(entry(1, finished))
            h.awaitTrue { h.notifications.isNotEmpty() }
            fun asked() = h.local.requestsOf<WatchLogRequest>().count { it.afterSeq == 1L }
            h.awaitTrue { asked() >= 2 }

            h.app.selectServer("other")
            h.awaitTrue { h.preferences.get("other") == 0L }
            val before = asked()
            h.app.selectServer("local")
            h.awaitTrue { asked() >= before + 2 }

            assertEquals(
                listOf(Notification(WINDOW_TITLE, "LRT_1 rendered — 3 videos (12.4 GB)")),
                h.notifications.toList(),
            )
        }
    }

    @Test
    fun theAppNamesTheKindOfAFailureFromItsCard() {
        Harness().use { h ->
            h.local.snapshots = listOf(snapshot("f1", "disk full"))
            h.app.start()
            h.awaitTrue { h.preferences.get("fake-server") == 0L }
            h.local.logEntries = listOf(entry(1, failed, failureId = "f1"))
            h.awaitTrue { h.notifications.isNotEmpty() }
            assertEquals("Render failed: out_1.mov — disk full", h.notifications.single().message)
        }
    }

    @Test
    fun theTextsFollowTheOverview() = runTest(timeout = 60.seconds) {
        val sent = mutableListOf<Notification>()
        val notifier = Notifier(InMemoryAppPreferences(), { sent += it }) { "killed" }
        notifier.handle(event(finished, videos = 1))
        notifier.handle(event(failed))
        notifier.handle(event(failed, file = ""))
        notifier.handle(event(EventKind.EVENT_RENDER_STARTED))
        assertEquals(
            listOf(
                "LRT_1 rendered — 1 video (12.4 GB)",
                "Render failed: a.mov — killed",
                "Render failed: LRT_1 — killed",
            ),
            sent.map { it.message },
        )
    }

    private fun sentWith(change: AppPreferences.() -> Unit): Pair<List<String>, Long?> {
        var result: Pair<List<String>, Long?>? = null
        runTest(timeout = 60.seconds) {
            Fake(backgroundScope).use { fake ->
                val preferences = InMemoryAppPreferences().apply(change)
                val sent = mutableListOf<Notification>()
                val watching = Watching(backgroundScope, fake, preferences, sent)
                watching.firstPass()
                fake.service.logEntries = listOf(entry(1, finished), entry(2, failed))
                watching.pass()
                result = sent.map { it.message } to preferences.get("srv")
            }
        }
        return result!!
    }

    @Test
    fun turningOffFailureNotificationsSuppressesOnlyThem() {
        val (sent, seq) = sentWith { notifyFailed = false }
        assertEquals(listOf("LRT_1 rendered — 3 videos (12.4 GB)"), sent)
        assertEquals(2L, seq) // a suppressed event is still consumed
    }

    @Test
    fun turningOffFinishedNotificationsSuppressesOnlyThem() {
        val (sent, seq) = sentWith { notifyFinished = false }
        assertEquals(listOf("Render failed: out_2.mov — disk full"), sent)
        assertEquals(2L, seq)
    }

    @Test
    fun aPreferenceChangeAppliesToTheNextEvent() = runTest(timeout = 60.seconds) {
        val preferences = InMemoryAppPreferences()
        val sent = mutableListOf<Notification>()
        val notifier = Notifier(preferences, { sent += it }) { "killed" }
        preferences.notifyFailed = false
        notifier.handle(event(failed))
        preferences.notifyFailed = true
        notifier.handle(event(failed))
        assertEquals(1, sent.size)
    }

    @Test
    fun aFailingSendOrLookupDoesNotEndTheEvents() = runTest(timeout = 60.seconds) {
        var attempts = 0
        val sending = Notifier(InMemoryAppPreferences(), { attempts++; error("no tray") }) { "killed" }
        sending.handle(event(finished))
        sending.handle(event(finished))
        assertEquals(2, attempts)
        val sent = mutableListOf<Notification>()
        val looking = Notifier(InMemoryAppPreferences(), { sent += it }) { error("no label") }
        looking.handle(event(failed))
        looking.handle(event(finished))
        assertEquals(1, sent.size)
    }

    @Test
    fun theKindLabelWaitsForTheCardInTheWatchedSnapshots() = runBlocking<Unit> {
        Fake(this).use { fake ->
            fake.service.snapshots = listOf(snapshot("f1", "corrupt output"))
            val repository = SnapshotRepository(fake.client, this, delayer = { awaitCancellation() })
            withTimeout(60_000) {
                val label = async(start = CoroutineStart.UNDISPATCHED) {
                    failureKindLabel(event(failed, id = "f1"), repository, fake.client, waitMs = 60_000)
                }
                repository.start()
                assertEquals("corrupt output", label.await())
                repository.state.first { it.snapshot != null }
            }
            assertEquals(0, fake.service.callCount("GetSnapshot"))
            repository.stop()
        }
    }

    @Test
    fun theKindLabelAsksTheServerThenReadsTheMessage() = runTest(timeout = 60.seconds) {
        Fake(backgroundScope).use { fake ->
            val repository = SnapshotRepository(fake.client, backgroundScope)
            suspend fun label(id: String, message: String = "FAILURE_DISK_FULL: no space") =
                failureKindLabel(event(failed, message = message, id = id), repository, fake.client)
            fake.service.snapshots = listOf(snapshot("f1", "bad source frame"))
            assertEquals("bad source frame", label("f1"))
            assertEquals("disk full", label("gone"))
            assertEquals("disk full", label(""))
            assertEquals(UNKNOWN_FAILURE, label("gone", "something else"))
            fake.service.failWith["GetSnapshot"] = Status.UNAVAILABLE
            assertEquals("disk full", label("f1"))
        }
    }

    @Test
    fun everyFailureKindHasALabel() {
        val kinds = FailureKind.entries.filter {
            it != FailureKind.FAILURE_KIND_UNSPECIFIED && it != FailureKind.UNRECOGNIZED
        }
        assertEquals(emptyList<FailureKind>(), kinds.filter { kindLabelOfMessage("${it.name}: x") == UNKNOWN_FAILURE })
        assertEquals("cannot start render", kindLabelOfMessage("FAILURE_NOT_QUEUEABLE: no images (x3)"))
    }
}
