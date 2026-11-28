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
    val error: String? = null,
)

/**
 * `ListLog` for the first page, `WatchLog` for live entries, older pages on demand; a level filter
 * (default Info) applied by the server and a search applied here. Rows are newest first.
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
    private var job: Job? = null
    private var olderJob: Job? = null

    /** Loads the first page and follows the log; also how a level change restarts it. */
    fun start() {
        job?.cancel()
        olderJob?.cancel()
        entries = emptyList()
        more = false
        publish(loading = true, error = null)
        job = scope.launch {
            val level = _state.value.minLevel
            var newest = 0L
            try {
                val page = client.listLog(0, pageSize, level)
                entries = page.entriesList
                more = page.more
                newest = entries.firstOrNull()?.seq ?: 0L
                publish(loading = false, error = null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                publish(loading = false, error = e.statusMessage())
            }
            follow(newest, level)
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
        publish(_state.value.loading, _state.value.error)
    }

    fun loadOlder() {
        val oldest = entries.lastOrNull()?.seq ?: return
        if (!more || olderJob?.isActive == true) return
        val level = _state.value.minLevel
        olderJob = scope.launch {
            try {
                val page = client.listLog(oldest, pageSize, level)
                val known = entries.mapTo(HashSet()) { it.seq }
                entries = entries + page.entriesList.filter { it.seq !in known }
                more = page.more
                publish(false, null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                publish(false, e.statusMessage())
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
                        publish(false, null)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // reconnect below
            }
            delayer(retryDelayMs)
        }
    }

    private fun publish(loading: Boolean, error: String?) {
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
        _state.update { it.copy(rows = rows, hasOlder = more, loading = loading, error = error) }
    }
}
