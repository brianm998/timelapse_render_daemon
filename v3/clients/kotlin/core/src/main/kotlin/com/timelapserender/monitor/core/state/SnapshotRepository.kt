// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.Snapshot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The latest snapshot with the connection state and the time the snapshot arrived. */
data class RepoState(
    val snapshot: Snapshot? = null,
    val connection: ConnectionState = ConnectionState.Connecting,
    val lastUpdateMs: Long = 0,
    /** Why `WatchSnapshots` last failed; null again once a snapshot arrives. */
    val watchError: String? = null,
)

/**
 * Follows `WatchSnapshots` and the client's connection state. A stream that fails or ends is
 * restarted after [retryDelayMs] (taken through [delayer]); the last snapshot stays visible meanwhile.
 * The stream asks for [maxPerSecond] snapshots per second (0: the server's default) and is restarted
 * when that changes.
 */
class SnapshotRepository(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    private val retryDelayMs: Long = 1000,
    private val delayer: suspend (Long) -> Unit = { delay(it) },
    private val maxPerSecond: StateFlow<Int> = MutableStateFlow(0),
) {
    private val _state = MutableStateFlow(RepoState(connection = client.state.value))
    val state: StateFlow<RepoState> = _state.asStateFlow()

    private var jobs: List<Job> = emptyList()

    fun start() {
        if (jobs.any { it.isActive }) return
        jobs = listOf(
            scope.launch { client.state.collect { connection -> _state.update { it.copy(connection = connection) } } },
            scope.launch { maxPerSecond.collectLatest { watchLoop(it) } },
        )
    }

    fun stop() {
        jobs.forEach { it.cancel() }
        jobs = emptyList()
    }

    private suspend fun watchLoop(rate: Int) {
        while (true) {
            try {
                client.watchSnapshots(rate).collect { snapshot ->
                    _state.update { it.copy(snapshot = snapshot, lastUpdateMs = clock(), watchError = null) }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // The client's connection state reflects only some failures, so keep the reason too.
                _state.update { it.copy(watchError = e.statusMessage()) }
            }
            delayer(retryDelayMs)
        }
    }
}
