// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import io.grpc.Status
import kotlinx.coroutines.CancellationException

/** A failed command: [message] is the gRPC status description, [code] the status code. */
class CommandException(message: String, val code: Status.Code) : Exception(message)

/** The gRPC status description of [this], or the code name when the server sent none. */
internal fun Throwable.statusMessage(): String {
    val status = Status.fromThrowable(this)
    return status.description?.takeIf { it.isNotBlank() } ?: status.code.name
}

internal fun Throwable.statusCode(): Status.Code = Status.fromThrowable(this).code

/** One `suspend` function per control of overview §6.9. A failure carries the status description. */
class Commands(private val client: TrdClient) {
    suspend fun setGlobalPause(paused: Boolean): Result<Unit> = attempt { client.setGlobalPause(paused) }

    suspend fun pauseSequence(sequenceName: String): Result<Unit> = attempt { client.pauseSequence(sequenceName) }

    suspend fun resumeSequence(sequenceName: String): Result<Unit> = attempt { client.resumeSequence(sequenceName) }

    suspend fun pauseVariant(sequenceName: String, variantKey: String): Result<Unit> =
        run { client.pauseVariant(sequenceName, variantKey) }

    suspend fun resumeVariant(sequenceName: String, variantKey: String): Result<Unit> =
        run { client.resumeVariant(sequenceName, variantKey) }

    suspend fun suspendRender(renderId: String): Result<Unit> = attempt { client.suspendRender(renderId) }

    suspend fun resumeRender(renderId: String): Result<Unit> = attempt { client.resumeRender(renderId) }

    suspend fun stopRender(renderId: String): Result<Unit> = attempt { client.stopRender(renderId) }

    suspend fun retryVariant(sequenceName: String, variantKey: String): Result<Unit> =
        run { client.retryVariant(sequenceName, variantKey) }

    suspend fun dismissFailure(failureId: String): Result<Unit> = attempt { client.dismissFailure(failureId) }

    suspend fun dismissAllFailures(): Result<Unit> = attempt { client.dismissAllFailures() }

    suspend fun rescanNow(): Result<Unit> = attempt { client.rescanNow() }

    private suspend fun attempt(block: suspend () -> Unit): Result<Unit> =
        try {
            block()
            Result.success(Unit)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Result.failure(CommandException(e.statusMessage(), e.statusCode()))
        }
}
