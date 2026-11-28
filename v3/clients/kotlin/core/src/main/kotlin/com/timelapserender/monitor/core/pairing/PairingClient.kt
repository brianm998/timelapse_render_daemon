// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.pairing

import com.timelapserender.monitor.core.net.ChannelFactory
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.trd.v1.DevicePlatform
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.withTimeout

/** Pairing failed; [message] is ready to show to the user. */
class PairingException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** Pairs with a trd over pinned TLS (spec 2 §5.3). */
class PairingClient(
    private val clock: () -> Long = System::currentTimeMillis,
    private val timeoutMs: Long = 60_000,
) {
    /**
     * Calls `Pair` with the pin and no token, then `GetServerInfo` with the new token to confirm
     * `caller_is_paired`, and returns the record to store.
     */
    suspend fun pair(endpoint: ServerEndpoint, code: String, deviceName: String, platform: DevicePlatform): ServerRecord {
        val pin = (endpoint.tls as? TlsMode.Pinned)?.fingerprintSha256Hex
            ?: throw PairingException(CONNECTION_FAILED)
        val scope = CoroutineScope(Dispatchers.IO + Job())
        val first = TrdClient(ChannelFactory.create(endpoint), null, scope, lan = true, clock = clock)
        var second: TrdClient? = null
        try {
            val response = try {
                withTimeout(timeoutMs) { first.pair(code, deviceName, platform) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                throw PairingException(messageFor(e), e)
            }
            second = TrdClient(ChannelFactory.create(endpoint), response.token, scope, lan = true, clock = clock)
            val info = try {
                withTimeout(timeoutMs) { second.serverInfo() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                throw PairingException(messageFor(e), e)
            }
            if (!info.callerIsPaired) throw PairingException(NOT_CONFIRMED)
            return ServerRecord(
                serverId = response.serverId.ifEmpty { info.serverId },
                name = response.serverName.ifEmpty { info.name },
                host = endpoint.host,
                port = endpoint.port,
                fingerprint = pin,
                token = response.token,
                platform = info.platform,
                lastSeenMs = clock(),
            )
        } finally {
            first.close()
            second?.close()
            scope.cancel()
        }
    }

    private fun messageFor(e: Throwable): String = when (Status.fromThrowable(e).code) {
        Status.Code.RESOURCE_EXHAUSTED -> TOO_MANY
        Status.Code.INVALID_ARGUMENT, Status.Code.PERMISSION_DENIED, Status.Code.UNAUTHENTICATED,
        Status.Code.NOT_FOUND -> BAD_CODE
        else -> CONNECTION_FAILED
    }

    companion object {
        const val BAD_CODE = "That code is not valid. Ask for a new one on the trd machine."
        const val TOO_MANY = "Too many wrong codes. Issue a new code on the trd machine."
        const val CONNECTION_FAILED = "Could not connect to the trd machine. Check that it is on the network."
        const val NOT_CONFIRMED = "The trd machine did not confirm the pairing. Ask for a new code and try again."
    }
}
