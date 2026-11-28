// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.time.ZoneId

data class LogRowUi(
    /** The entry's `seq`, as text: stable, so the list follows the §6.2 scroll rules. */
    val key: String,
    val seq: Long,
    val atMs: Long,
    val timeText: String,
    val level: LogLevel,
    val subject: String,
    val message: String,
)

data class LogUi(
    /** Newest first, after the search filter. */
    val rows: List<LogRowUi> = emptyList(),
    val minLevel: LogLevel = LogLevel.LOG_INFO,
    val search: String = "",
    val hasOlder: Boolean = false,
    val loading: Boolean = true,
    /** Why the first page, the live stream or the last older page failed; null when none did. */
    val error: String? = null,
)

/**
 * `ListLog` for the first page, `WatchLog` for live entries, older pages on demand; a level filter
 * (default Info) applied by the server and a search applied here. Rows are newest first. A failed
 * first page is retried before the log is followed; a failed stream is reported and reopened.
 */
class LogModel(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val pageSize: Int = 200,
    private val retryDelayMs: Long = 1000,
    private val delayer: suspend (Long) -> Unit = { delay(it) },
) {
    private val _state = MutableStateFlow(LogUi())
    val state: StateFlow<LogUi> = _state.asStateFlow()

    private var entries: List<LogEntry> = emptyList() // newest first
    private var more = false
    private var streamError: String? = null // the first page or the live stream
    private var olderError: String? = null
    private var job: Job? = null
    private var olderJob: Job? = null

    /** Loads the first page and follows the log; also how a level change restarts it. */
    fun start() {
        job?.cancel()
        olderJob?.cancel()
        entries = emptyList()
        more = false
        streamError = null
        olderError = null
        publish(loading = true)
        job = scope.launch {
            val level = _state.value.minLevel
            follow(firstPage(level), level)
        }
    }

    /** Asks until the first page arrives, and returns its newest `seq`. */
    private suspend fun firstPage(level: LogLevel): Long {
        while (true) {
            try {
                val page = client.listLog(0, pageSize, level)
                entries = page.entriesList
                more = page.more
                streamError = null
                publish(loading = false)
                return entries.firstOrNull()?.seq ?: 0L
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                streamError = e.statusMessage()
                publish(loading = false)
            }
            delayer(retryDelayMs)
        }
    }

    fun stop() {
        job?.cancel()
        olderJob?.cancel()
    }

    fun setLevel(level: LogLevel) {
        if (level == _state.value.minLevel) return
        _state.update { it.copy(minLevel = level) }
        start()
    }

    fun setSearch(text: String) {
        _state.update { it.copy(search = text) }
        publish(_state.value.loading)
    }

    fun loadOlder() {
        val oldest = entries.lastOrNull()?.seq ?: return
        if (!more || olderJob?.isActive == true) return
        val level = _state.value.minLevel
        olderError = null
        publish(_state.value.loading)
        olderJob = scope.launch {
            try {
                val page = client.listLog(oldest, pageSize, level)
                val known = entries.mapTo(HashSet()) { it.seq }
                entries = entries + page.entriesList.filter { it.seq !in known }
                more = page.more
                publish(false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                olderError = e.statusMessage()
                publish(false)
            }
        }
    }

    private suspend fun follow(startSeq: Long, level: LogLevel) {
        var after = startSeq
        while (true) {
            try {
                client.watchLog(after, level).collect { entry ->
                    after = maxOf(after, entry.seq)
                    if (entries.none { it.seq == entry.seq }) {
                        entries = (listOf(entry) + entries).sortedByDescending { it.seq }
                        streamError = null
                        publish(false)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                streamError = e.statusMessage()
                publish(false)
            }
            delayer(retryDelayMs)
            // The stream is being reopened: the error stands again only if that fails too.
            if (streamError != null) {
                streamError = null
                publish(false)
            }
        }
    }

    private fun publish(loading: Boolean) {
        val now = clock()
        val needle = _state.value.search.trim()
        val rows = entries
            .filter { needle.isEmpty() || it.subject.contains(needle, true) || it.message.contains(needle, true) }
            .map {
                LogRowUi(
                    it.seq.toString(), it.seq, it.atMs, Format.absoluteTime(it.atMs, now, zone),
                    it.level, it.subject, it.message,
                )
            }
        _state.update { it.copy(rows = rows, hasOlder = more, loading = loading, error = streamError ?: olderError) }
    }
}
