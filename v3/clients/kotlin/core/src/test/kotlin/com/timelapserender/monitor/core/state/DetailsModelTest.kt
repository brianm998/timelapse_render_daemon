// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.GetRenderDetailsRequest
import com.timelapserender.trd.v1.RenderDetails
import io.grpc.Status
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class DetailsModelTest {
    @Test
    fun loadsDetailsForARender() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.renderDetails = RenderDetails.newBuilder().setRenderId("r1").setCommandLine("ffmpeg -i x").build()
            val model = DetailsModel(env.client, backgroundScope)
            assertEquals(DetailsState.Loading, model.state.value)
            model.load(DetailsTarget.Render("r1"))
            val loaded = model.state.first { it is DetailsState.Loaded } as DetailsState.Loaded
            assertEquals("ffmpeg -i x", loaded.details.commandLine)
            val request = env.fake.requestsOf<GetRenderDetailsRequest>().single()
            assertEquals(GetRenderDetailsRequest.TargetCase.RENDER_ID, request.targetCase)
            assertEquals("r1", request.renderId)
        }
    }

    @Test
    fun loadsDetailsForARenderedVideoAndAFailure() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val model = DetailsModel(env.client, backgroundScope)
            model.load(DetailsTarget.RenderedVideo("v1"))
            model.state.first { it is DetailsState.Loaded }
            model.load(DetailsTarget.Failure("f1"))
            model.state.first { it is DetailsState.Loaded && env.fake.callCount("GetRenderDetails") == 2 }
            val requests = env.fake.requestsOf<GetRenderDetailsRequest>()
            assertEquals("v1", requests[0].renderedVideoId)
            assertEquals(GetRenderDetailsRequest.TargetCase.FAILURE_ID, requests[1].targetCase)
            assertEquals("f1", requests[1].failureId)
        }
    }

    @Test
    fun reportsAnErrorAndReloadRetries() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["GetRenderDetails"] = Status.NOT_FOUND.withDescription("no such render")
            val model = DetailsModel(env.client, backgroundScope)
            model.load(DetailsTarget.Render("gone"))
            val error = model.state.first { it is DetailsState.Error } as DetailsState.Error
            assertEquals("no such render", error.message)
            env.fake.failWith.clear()
            model.reload()
            model.state.first { it is DetailsState.Loaded }
            assertTrue(env.fake.callCount("GetRenderDetails") == 2)
        }
    }
}
