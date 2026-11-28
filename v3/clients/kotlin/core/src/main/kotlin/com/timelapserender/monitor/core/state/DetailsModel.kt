// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.GetRenderDetailsRequest
import com.timelapserender.trd.v1.RenderDetails
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface DetailsTarget {
    data class Render(val renderId: String) : DetailsTarget

    data class RenderedVideo(val renderedVideoId: String) : DetailsTarget

    data class Failure(val failureId: String) : DetailsTarget
}

sealed interface DetailsState {
    data object Loading : DetailsState

    data class Loaded(val details: RenderDetails) : DetailsState

    data class Error(val message: String) : DetailsState
}

/** Loads `GetRenderDetails` for a render, a rendered video or a failure. */
class DetailsModel(private val client: TrdClient, private val scope: CoroutineScope) {
    private val _state = MutableStateFlow<DetailsState>(DetailsState.Loading)
    val state: StateFlow<DetailsState> = _state.asStateFlow()

    private var job: Job? = null
    private var target: DetailsTarget? = null

    fun load(target: DetailsTarget) {
        this.target = target
        job?.cancel()
        _state.value = DetailsState.Loading
        val request = GetRenderDetailsRequest.newBuilder().apply {
            when (target) {
                is DetailsTarget.Render -> renderId = target.renderId
                is DetailsTarget.RenderedVideo -> renderedVideoId = target.renderedVideoId
                is DetailsTarget.Failure -> failureId = target.failureId
            }
        }.build()
        job = scope.launch {
            _state.value = try {
                DetailsState.Loaded(client.getRenderDetails(request))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                DetailsState.Error(e.statusMessage())
            }
        }
    }

    fun reload() {
        target?.let { load(it) }
    }
}
