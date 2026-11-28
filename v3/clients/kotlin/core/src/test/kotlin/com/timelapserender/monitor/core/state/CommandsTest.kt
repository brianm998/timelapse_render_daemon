// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class CommandsTest {
    @Test
    fun everyControlCallsItsRpc() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val c = Commands(env.client)
            val results = listOf(
                c.setGlobalPause(true), c.pauseSequence("s"), c.resumeSequence("s"),
                c.pauseVariant("s", "v"), c.resumeVariant("s", "v"), c.suspendRender("r"),
                c.resumeRender("r"), c.stopRender("r"), c.retryVariant("s", "v"),
                c.dismissFailure("f"), c.dismissAllFailures(), c.rescanNow(),
            )
            assertTrue(results.all { it.isSuccess })
            assertEquals(
                setOf(
                    "SetGlobalPause", "PauseSequence", "ResumeSequence", "PauseVariant", "ResumeVariant",
                    "SuspendRender", "ResumeRender", "StopRender", "RetryVariant", "DismissFailure",
                    "DismissAllFailures", "RescanNow",
                ),
                env.fake.methodsCalled(),
            )
        }
    }

    @Test
    fun aFailureCarriesTheStatusDescription() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["PauseSequence"] = Status.FAILED_PRECONDITION.withDescription("sequence is busy")
            val result = Commands(env.client).pauseSequence("s")
            assertTrue(result.isFailure)
            val e = result.exceptionOrNull() as CommandException
            assertEquals("sequence is busy", e.message)
            assertEquals(Status.Code.FAILED_PRECONDITION, e.code)
        }
    }

    @Test
    fun aFailureWithoutDescriptionNamesTheStatusCode() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["StopRender"] = Status.UNAVAILABLE
            assertEquals("UNAVAILABLE", Commands(env.client).stopRender("r").exceptionOrNull()?.message)
        }
    }

    @Test
    fun cancellationIsRethrownNotReturnedAsAFailure() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.gates["RescanNow"] = CompletableDeferred()
            var result: Result<Unit>? = null
            val job = launch(start = CoroutineStart.UNDISPATCHED) { result = Commands(env.client).rescanNow() }
            job.cancelAndJoin()
            assertNull(result)
            assertTrue(job.isCancelled)
        }
    }
}
