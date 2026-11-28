// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.DirEntry
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class DirectoryUi(
    /** "" while showing the roots. */
    val path: String = "",
    /** "" at a root (and while showing the roots). */
    val parent: String = "",
    val roots: List<String> = emptyList(),
    val entries: List<DirEntry> = emptyList(),
    /** What the user picked in the list, if anything. */
    val selected: String? = null,
    val loading: Boolean = false,
    val error: String? = null,
) {
    val atRoots: Boolean get() = path.isEmpty()

    /** The picked entry, else the folder being shown (nothing at the roots). */
    val selection: String? get() = selected ?: path.takeIf { it.isNotEmpty() }
}

/** `ListDirectory` navigation with roots, parent and a selection. */
class DirectoryBrowserModel(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val includeFiles: Boolean = false,
    private val showHidden: Boolean = false,
) {
    private val _state = MutableStateFlow(DirectoryUi())
    val state: StateFlow<DirectoryUi> = _state.asStateFlow()
    private var job: Job? = null

    /** Shows the roots. */
    fun openRoots() = navigate("")

    /** Shows [path]; a failure keeps the previous listing and sets `error`. */
    fun navigate(path: String) {
        job?.cancel()
        _state.update { it.copy(loading = true, error = null) }
        job = scope.launch {
            try {
                val r = client.listDirectory(path, includeFiles, showHidden)
                _state.update {
                    it.copy(
                        path = r.path, parent = r.parent, entries = r.entriesList,
                        roots = if (r.rootsCount > 0) r.rootsList else it.roots,
                        selected = null, loading = false, error = null,
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, error = e.statusMessage()) }
            }
        }
    }

    /** Goes to the parent folder, or to the roots from a root folder. */
    fun up() {
        val s = _state.value
        if (s.atRoots) return
        navigate(s.parent)
    }

    fun select(path: String?) {
        _state.update { it.copy(selected = path) }
    }
}
