// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.FileTime

/**
 * JPEG bytes by rendered-video id: a memory LRU of [memoryCapacity] entries, a disk cache of at most
 * [diskCapBytes] under `<cacheDir>/<serverId>` (oldest file evicted first), and `GetThumbnail`.
 * A `NOT_FOUND` is remembered for [notFoundMs] by [clock]; concurrent requests for one id share one fetch.
 */
class ThumbnailCache(
    private val client: TrdClient,
    serverId: String,
    cacheDir: Path,
    private val clock: () -> Long = System::currentTimeMillis,
    private val memoryCapacity: Int = 200,
    private val diskCapBytes: Long = 200L * 1024 * 1024,
    private val notFoundMs: Long = 10L * 60 * 1000,
) {
    private val dir: Path = cacheDir.resolve(fileSafe(serverId))
    private val lock = Any()
    private val memory = object : LinkedHashMap<String, ByteArray>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ByteArray>?): Boolean =
            size > memoryCapacity
    }
    private val notFoundUntil = HashMap<String, Long>()
    private val inflight = HashMap<String, CompletableDeferred<ByteArray?>>()

    /** The JPEG, or null when the server has none. Other failures are thrown. */
    suspend fun get(id: String): ByteArray? {
        while (true) {
            val mine = CompletableDeferred<ByteArray?>()
            var other: CompletableDeferred<ByteArray?>? = null
            synchronized(lock) {
                memory[id]?.let { return it }
                val until = notFoundUntil[id]
                if (until != null) {
                    if (clock() < until) return null
                    notFoundUntil.remove(id)
                }
                other = inflight[id]
                if (other == null) inflight[id] = mine
            }
            val shared = other
            if (shared != null) {
                val result = try {
                    Result.success(shared.await())
                } catch (e: CancellationException) {
                    Result.failure(e)
                }
                if (result.isSuccess) return result.getOrNull()
                // The fetching caller was cancelled: if we are still active, fetch ourselves.
                currentCoroutineContext().ensureActive()
                continue
            }
            try {
                val result = load(id)
                mine.complete(result)
                return result
            } catch (e: Throwable) {
                if (e is CancellationException) mine.cancel() else mine.completeExceptionally(e)
                throw e
            } finally {
                synchronized(lock) { inflight.remove(id) }
            }
        }
    }

    /** Forgets the cached copies of [id], for bytes that turned out to be unusable: the next [get] asks again. */
    fun discard(id: String) {
        synchronized(lock) { memory.remove(id) }
        try {
            Files.deleteIfExists(path(id))
        } catch (e: java.io.IOException) {
            // A cache that cannot be written is still a cache.
        }
    }

    /** Number of thumbnails on disk (for tests and diagnostics). */
    fun diskFiles(): List<Path> = listFiles()

    private suspend fun load(id: String): ByteArray? {
        readDisk(id)?.let {
            remember(id, it)
            return it
        }
        val bytes = try {
            client.getThumbnail(id).jpeg.toByteArray()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (e.statusCode() == Status.Code.NOT_FOUND) {
                synchronized(lock) { notFoundUntil[id] = clock() + notFoundMs }
                return null
            }
            throw e
        }
        if (bytes.isEmpty()) {
            synchronized(lock) { notFoundUntil[id] = clock() + notFoundMs }
            return null
        }
        remember(id, bytes)
        writeDisk(id, bytes)
        return bytes
    }

    private fun remember(id: String, bytes: ByteArray) {
        synchronized(lock) { memory[id] = bytes }
    }

    private fun path(id: String): Path = dir.resolve(fileSafe(id) + ".jpg")

    private fun readDisk(id: String): ByteArray? =
        try {
            val file = path(id)
            if (Files.isRegularFile(file)) {
                Files.readAllBytes(file).also { Files.setLastModifiedTime(file, FileTime.fromMillis(clock())) }
            } else {
                null
            }
        } catch (e: java.io.IOException) {
            null
        }

    private fun writeDisk(id: String, bytes: ByteArray) {
        try {
            Files.createDirectories(dir)
            val file = path(id)
            val tmp = dir.resolve(file.fileName.toString() + ".tmp")
            Files.write(tmp, bytes)
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING)
            Files.setLastModifiedTime(file, FileTime.fromMillis(clock()))
            enforceCap()
        } catch (e: java.io.IOException) {
            // A cache that cannot be written is still a cache.
        }
    }

    private fun listFiles(): List<Path> =
        if (Files.isDirectory(dir)) {
            Files.list(dir).use { s -> s.filter { it.fileName.toString().endsWith(".jpg") }.toList() }
        } else {
            emptyList()
        }

    private fun enforceCap() {
        val files = listFiles()
            .map { Triple(it, Files.size(it), Files.getLastModifiedTime(it).toMillis()) }
            .sortedWith(compareBy({ it.third }, { it.first.fileName.toString() }))
        var total = files.sumOf { it.second }
        for ((file, size, _) in files) {
            if (total <= diskCapBytes) break
            Files.deleteIfExists(file)
            total -= size
        }
    }

    private companion object {
        fun fileSafe(name: String): String = name.toByteArray().joinToString("") { "%02x".format(it) }
    }
}
