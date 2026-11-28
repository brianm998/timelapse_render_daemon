// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.WatchSnapshotsRequest
import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class SnapshotRepositoryTest {
    @Test
    fun publishesTheLatestSnapshotWithItsArrivalTime() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.snapshots = listOf(Snapshot.newBuilder().setRevision(7).build())
            val repo = SnapshotRepository(env.client, backgroundScope, clock = { 1234L })
            assertNull(repo.state.value.snapshot)
            repo.start()
            val s = repo.state.first { it.snapshot != null }
            assertEquals(7L, s.snapshot!!.revision)
            assertEquals(1234L, s.lastUpdateMs)
        }
    }

    @Test
    fun showsTheInitialSnapshotUntilTheFirstOneArrives() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.snapshots = listOf(Snapshot.newBuilder().setRevision(7).build())
            val initial = Snapshot.newBuilder().setRevision(1).build()
            val repo = SnapshotRepository(env.client, backgroundScope, initialSnapshot = initial)
            assertEquals(initial, repo.state.value.snapshot)
            repo.start()
            repo.state.first { it.snapshot?.revision == 7L }
        }
    }

    @Test
    fun aChangedRateRestartsTheWatchAndKeepsTheSnapshot() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.snapshots = listOf(Snapshot.newBuilder().setRevision(3).build())
            val rate = MutableStateFlow(4)
            val ended = Channel<Unit>(Channel.UNLIMITED)
            val repo = SnapshotRepository(
                env.client, backgroundScope,
                delayer = {
                    ended.send(Unit)
                    awaitCancellation()
                },
                maxPerSecond = rate,
            )
            repo.start()
            ended.receive() // the first stream has ended and the loop waits to restart it
            env.fake.failWith["WatchSnapshots"] = Status.UNAVAILABLE
            rate.value = 7
            ended.receive() // the new rate does not wait for the old loop
            assertEquals(3L, repo.state.value.snapshot!!.revision)
            env.fake.failWith.clear()
            rate.value = 2
            ended.receive()
            assertEquals(listOf(4, 2), env.fake.requestsOf<WatchSnapshotsRequest>().map { it.maxPerSecond })
            assertEquals(3, env.fake.callCount("WatchSnapshots"))
        }
    }

    @Test
    fun followsTheClientsConnectionState() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val repo = SnapshotRepository(env.client, backgroundScope)
            repo.start()
            env.client.start()
            val s = repo.state.first { it.connection is ConnectionState.Connected }
            assertEquals("Fake trd", (s.connection as ConnectionState.Connected).info.name)
        }
    }

    @Test
    fun restartsTheWatchAfterTheStreamFails() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["WatchSnapshots"] = Status.UNAVAILABLE
            val reached = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val repo = SnapshotRepository(
                env.client, backgroundScope,
                delayer = {
                    reached.complete(Unit)
                    release.await()
                },
            )
            repo.start()
            reached.await()
            assertNull(repo.state.value.snapshot)
            env.fake.failWith.clear()
            release.complete(Unit)
            val s = repo.state.first { it.snapshot != null }
            assertNotNull(s.snapshot)
            assertTrue(env.fake.callCount("WatchSnapshots") >= 2)
        }
    }

    @Test
    fun keepsTheLastSnapshotWhileTheStreamIsDown() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.snapshots = listOf(Snapshot.newBuilder().setRevision(3).build())
            val reached = CompletableDeferred<Unit>()
            val release = CompletableDeferred<Unit>()
            val repo = SnapshotRepository(env.client, backgroundScope, delayer = {
                reached.complete(Unit)
                release.await()
            })
            repo.start()
            reached.await() // the first stream has ended; the snapshot stays
            assertEquals(3L, repo.state.value.snapshot!!.revision)
        }
    }

    @Test
    fun reportsWhyTheStreamFailedUntilASnapshotArrives() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["WatchSnapshots"] = Status.INTERNAL.withDescription("snapshot failed")
            val release = CompletableDeferred<Unit>()
            val repo = SnapshotRepository(env.client, backgroundScope, delayer = { release.await() })
            repo.start()
            assertEquals("snapshot failed", repo.state.first { it.watchError != null }.watchError)
            env.fake.failWith.clear()
            release.complete(Unit)
            assertNull(repo.state.first { it.snapshot != null }.watchError)
        }
    }
}
