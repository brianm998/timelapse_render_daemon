// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.google.protobuf.ByteString
import com.timelapserender.trd.v1.Thumbnail
import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import kotlin.time.Duration.Companion.seconds

class ThumbnailCacheTest {
    private val dir: Path = Files.createTempDirectory("trd-thumbs-")

    @After
    fun tearDown() {
        dir.toFile().deleteRecursively()
    }

    private fun thumb(vararg bytes: Int) =
        Thumbnail.newBuilder().setJpeg(ByteString.copyFrom(ByteArray(bytes.size) { bytes[it].toByte() })).build()

    @Test
    fun fetchesOnceThenServesFromMemoryAndDisk() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(1, 2, 3))
            val cache = ThumbnailCache(env.client, "srv", dir)
            assertArrayEquals(byteArrayOf(1, 2, 3), cache.get("a"))
            assertArrayEquals(byteArrayOf(1, 2, 3), cache.get("a"))
            assertEquals(1, env.fake.callCount("GetThumbnail"))
            // a new cache (a new process) finds it on disk
            val again = ThumbnailCache(env.client, "srv", dir)
            assertArrayEquals(byteArrayOf(1, 2, 3), again.get("a"))
            assertEquals(1, env.fake.callCount("GetThumbnail"))
            // the cache of another server is separate
            val other = ThumbnailCache(env.client, "other", dir)
            assertArrayEquals(byteArrayOf(1, 2, 3), other.get("a"))
            assertEquals(2, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun memoryIsAnLruOfTheConfiguredSize() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(1), "b" to thumb(2), "c" to thumb(3))
            // a disk cap of zero keeps nothing on disk, so only the memory LRU can answer
            val cache = ThumbnailCache(env.client, "srv", dir, memoryCapacity = 2, diskCapBytes = 0)
            cache.get("a")
            cache.get("b")
            cache.get("a") // a is now the most recently used
            assertEquals(2, env.fake.callCount("GetThumbnail"))
            cache.get("c") // evicts b
            assertEquals(3, env.fake.callCount("GetThumbnail"))
            cache.get("a")
            assertEquals(3, env.fake.callCount("GetThumbnail"))
            cache.get("b")
            assertEquals(4, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun diskIsCappedAndEvictsTheOldestFirst() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(1, 1, 1, 1, 1, 1), "b" to thumb(2, 2, 2, 2, 2, 2))
            var now = 1_000L
            val cache = ThumbnailCache(env.client, "srv", dir, clock = { now }, diskCapBytes = 10)
            cache.get("a")
            assertEquals(1, cache.diskFiles().size)
            now = 2_000L
            cache.get("b") // 12 bytes > 10: a, the oldest, goes
            assertEquals(1, cache.diskFiles().size)
            val fresh = ThumbnailCache(env.client, "srv", dir, clock = { now }, diskCapBytes = 10)
            assertArrayEquals(byteArrayOf(2, 2, 2, 2, 2, 2), fresh.get("b"))
            assertEquals(2, env.fake.callCount("GetThumbnail"))
            fresh.get("a")
            assertEquals(3, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun notFoundIsRememberedForTenMinutes() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("other" to thumb(9))
            var now = 0L
            val cache = ThumbnailCache(env.client, "srv", dir, clock = { now })
            assertNull(cache.get("missing"))
            assertNull(cache.get("missing"))
            assertEquals(1, env.fake.callCount("GetThumbnail"))
            now = 9 * 60_000L
            assertNull(cache.get("missing"))
            assertEquals(1, env.fake.callCount("GetThumbnail"))
            now = 10 * 60_000L + 1
            assertNull(cache.get("missing"))
            assertEquals(2, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun concurrentRequestsForOneIdShareOneFetch() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(5))
            val gate = CompletableDeferred<Unit>()
            env.fake.gates["GetThumbnail"] = gate
            val cache = ThumbnailCache(env.client, "srv", dir)
            val first = async(start = CoroutineStart.UNDISPATCHED) { cache.get("a") }
            val second = async(start = CoroutineStart.UNDISPATCHED) { cache.get("a") }
            gate.complete(Unit)
            assertArrayEquals(byteArrayOf(5), first.await())
            assertArrayEquals(byteArrayOf(5), second.await())
            assertEquals(1, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun aDiscardedThumbnailIsFetchedAgain() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(5))
            val cache = ThumbnailCache(env.client, "srv", dir)
            cache.get("a")
            cache.discard("a")
            assertTrue(cache.diskFiles().isEmpty())
            env.fake.thumbnails = mapOf("a" to thumb(6))
            assertArrayEquals(byteArrayOf(6), cache.get("a"))
            assertEquals(2, env.fake.callCount("GetThumbnail"))
        }
    }

    @Test
    fun otherFailuresArePropagatedAndNotRemembered() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.thumbnails = mapOf("a" to thumb(5))
            env.fake.failWith["GetThumbnail"] = Status.UNAVAILABLE
            val cache = ThumbnailCache(env.client, "srv", dir)
            try {
                cache.get("a")
                fail("expected a failure")
            } catch (e: io.grpc.StatusException) {
                assertEquals(Status.Code.UNAVAILABLE, e.status.code)
            }
            env.fake.failWith.clear()
            assertTrue(cache.get("a") != null)
        }
    }
}
