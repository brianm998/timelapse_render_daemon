// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.EventKind
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import io.grpc.StatusException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.flow
import java.util.concurrent.ConcurrentHashMap

/** The last `seq` seen per server, kept by the client (preferences on desktop). */
interface LastSeqStore {
    fun get(serverId: String): Long?

    fun set(serverId: String, seq: Long)
}

class InMemoryLastSeqStore : LastSeqStore {
    private val map = ConcurrentHashMap<String, Long>()

    override fun get(serverId: String): Long? = map[serverId]

    override fun set(serverId: String, seq: Long) {
        map[serverId] = seq
    }
}

/** A log event worth a notification. */
data class ServerEvent(
    val seq: Long,
    val kind: EventKind,
    val atMs: Long,
    val sequenceName: String,
    val outputFilename: String,
    val message: String,
    val videoCount: Int,
    val totalBytes: Long,
    val failureId: String,
)

/**
 * Follows `WatchLog` and emits `SEQUENCE_FINISHED` and `RENDER_FAILED` entries. The last seen `seq`
 * is stored for every entry, so a reconnect (or a restart) repeats nothing. An event's `seq` is
 * stored once the collector has taken it: an event whose collector throws is delivered again.
 * On first contact with a server (nothing stored) it starts at the newest `seq`: no flood of old events.
 *
 * Only failed RPCs and streams are retried (each is passed to [onRpcFailure]); a failure of the
 * collector or of the [LastSeqStore] ends [events] with that exception.
 */
class EventWatcher(
    private val client: TrdClient,
    private val store: LastSeqStore,
    private val serverId: String,
    private val retryDelayMs: Long = 1000,
    private val delayer: suspend (Long) -> Unit = { delay(it) },
    private val onRpcFailure: (Throwable) -> Unit = {},
) {
    fun events(): Flow<ServerEvent> = flow {
        while (true) {
            val after = store.get(serverId) ?: newestSeq()?.also { store.set(serverId, it) }
            if (after != null) {
                // `catch` sits upstream of the collector, so it sees only the stream's own failures.
                client.watchLog(after)
                    .catch { if (it.isRpcFailure()) onRpcFailure(it) else throw it }
                    .collect { entry ->
                        if (entry.seq <= (store.get(serverId) ?: 0L)) return@collect
                        toEvent(entry)?.let { emit(it) }
                        store.set(serverId, entry.seq)
                    }
            }
            delayer(retryDelayMs)
        }
    }

    /** The newest `seq` of the server's log (0 when it is empty), or null when it could not be asked. */
    private suspend fun newestSeq(): Long? =
        try {
            client.listLog(0, 1, LogLevel.LOG_DEBUG).entriesList.firstOrNull()?.seq ?: 0L
        } catch (e: StatusException) {
            onRpcFailure(e)
            null
        }

    private fun toEvent(e: LogEntry): ServerEvent? =
        when (e.event) {
            EventKind.EVENT_SEQUENCE_FINISHED, EventKind.EVENT_RENDER_FAILED ->
                ServerEvent(
                    e.seq, e.event, e.atMs, e.sequenceName, e.outputFilename, e.message,
                    e.videoCount, e.totalBytes, e.failureId,
                )
            else -> null
        }
}
