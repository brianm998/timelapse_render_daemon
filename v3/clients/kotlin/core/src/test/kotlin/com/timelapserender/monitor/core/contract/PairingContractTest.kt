// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.pairing.PairingClient
import com.timelapserender.monitor.core.pairing.PairingException
import com.timelapserender.monitor.core.state.DevicesModel
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.trd.v1.DevicePlatform
import io.grpc.Status
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** `StartPairing` on loopback, then everything else over the TLS listener, pinned to its certificate. */
class PairingContractTest {
    private suspend fun lanEndpoint(trd: TrdProcess, loopback: TrdClient) =
        ServerEndpoint("127.0.0.1", trd.lanPort, TlsMode.Pinned(loopback.serverInfo().certFingerprintSha256))

    /** The message `PairingClient` fails with. */
    private suspend fun refusal(endpoint: ServerEndpoint, code: String): String? {
        val result = runCatching {
            PairingClient().pair(endpoint, code, "contract phone", DevicePlatform.DEVICE_PLATFORM_ANDROID)
        }
        val failure = result.exceptionOrNull()
        assertTrue("$result", failure is PairingException)
        return failure?.message
    }

    @Test
    fun pairingGivesATokenThatStreamsUntilItIsRevoked() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val loopback = trd.connected()
            val endpoint = lanEndpoint(trd, loopback)

            val unpaired = trd.client(endpoint, null)
            unpaired.start()
            unpaired.state.reach("an unpaired caller") { it == ConnectionState.NeedsPairing }

            val devices = DevicesModel(loopback, trd.scope)
            devices.startPairing()
            val pairing = devices.state.reach("a pairing code") { it.pairing != null }.pairing!!
            assertTrue(pairing.code, Regex("\\d{6}").matches(pairing.code))
            assertEquals(trd.lanPort, pairing.lanPort)
            assertEquals((endpoint.tls as TlsMode.Pinned).fingerprintSha256Hex.take(16), pairing.fingerprintPrefix)
            assertTrue(pairing.remainingSeconds > 0)
            devices.cancelPairing()

            // A pin that is not the certificate's never reaches the server, so the code survives.
            val wrongPin = endpoint.copy(tls = TlsMode.Pinned("0".repeat(64)))
            assertEquals(PairingClient.CONNECTION_FAILED, refusal(wrongPin, pairing.code))

            val record = PairingClient().pair(endpoint, pairing.code, "contract phone", DevicePlatform.DEVICE_PLATFORM_ANDROID)
            assertEquals(loopback.serverInfo().serverId, record.serverId)
            assertEquals(trd.lanPort, record.port)
            assertTrue(!record.token.isNullOrEmpty())

            val paired = trd.client(endpoint, record.token)
            paired.start()
            val connected = paired.state.reach("the paired connection") { it is ConnectionState.Connected }
            val info = (connected as ConnectionState.Connected).info
            assertTrue(info.callerIsPaired)
            assertTrue(!info.callerIsLocal)
            val repo = SnapshotRepository(paired, trd.scope)
            repo.start()
            repo.state.reach("a snapshot over TLS") { it.snapshot != null }
            withTimeout(HANG_GUARD_MS) { paired.watchSnapshots().first() }

            // Device calls are served on loopback only.
            val onLan = runCatching { paired.startPairing() }
            assertEquals(Status.Code.PERMISSION_DENIED, Status.fromThrowable(onLan.exceptionOrNull()!!).code)

            devices.refresh()
            val row = devices.state.reach("the device list") { it.devices.isNotEmpty() }.devices.single()
            assertEquals("contract phone", row.name)
            assertEquals("Android", row.platformText)
            assertEquals(row.deviceId, loopback.listPairedDevices().devicesList.single().deviceId)

            // Revoking ends the open stream.
            devices.revoke(row.deviceId)
            repo.state.reach("the revoked stream") { it.connection == ConnectionState.NeedsPairing }
            val after = devices.state.reach("an empty device list") { it.devices.isEmpty() }
            assertEquals(null, after.error)

            val revoked = trd.client(endpoint, record.token)
            revoked.start()
            revoked.state.reach("a revoked token") { it == ConnectionState.NeedsPairing }
        }
    }

    @Test
    fun wrongCodesAreRefusedAndTheFifthInvalidatesTheCode() = TrdProcess.create().use { trd ->
        trd.keepSequences("")
        trd.start()
        trd.test {
            val loopback = trd.connected()
            val endpoint = lanEndpoint(trd, loopback)
            val issued = loopback.startPairing()
            val wrong = ((issued.code.toInt() + 1) % 1_000_000).toString().padStart(6, '0')

            repeat(4) { assertEquals("attempt ${it + 1}", PairingClient.BAD_CODE, refusal(endpoint, wrong)) }
            assertEquals(PairingClient.TOO_MANY, refusal(endpoint, wrong))
            assertEquals(PairingClient.BAD_CODE, refusal(endpoint, issued.code))
            assertEquals(0, loopback.listPairedDevices().devicesCount)

            // The `Pair` RPC itself, with a new code on the loopback listener.
            val next = loopback.startPairing()
            val response = loopback.pair(next.code, "contract desktop", DevicePlatform.DEVICE_PLATFORM_DESKTOP)
            assertTrue(response.token.isNotEmpty())
            loopback.revokeDevice(response.deviceId)
            assertEquals(0, loopback.listPairedDevices().devicesCount)
        }
    }
}
