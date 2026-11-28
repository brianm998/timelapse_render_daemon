// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.PairedDevice
import com.timelapserender.trd.v1.StartPairingResponse
import io.grpc.Status
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import kotlin.time.Duration.Companion.seconds

class DevicesModelTest {
    private val base = Instant.parse("2025-11-27T15:00:00Z").toEpochMilli()
    private val utc = ZoneId.of("UTC")

    private fun device(id: String, platform: DevicePlatform, pairedAt: String, lastSeenMs: Long) =
        PairedDevice.newBuilder().setDeviceId(id).setName("Phone $id").setPlatform(platform)
            .setPairedAtMs(Instant.parse(pairedAt).toEpochMilli()).setLastSeenMs(lastSeenMs).build()

    @Test
    fun pairingCountsDownFromTheExpiryByTheClock() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.startPairingResponse = StartPairingResponse.newBuilder()
                .setCode("482913").setExpiresAtMs(base + 300_000).setServerName("Studio Mac")
                .setCertFingerprintSha256("0123456789abcdef0123456789abcdef").setLanPort(50072)
                .addAllLanAddresses(listOf("192.168.1.20")).build()
            var now = base
            val ticks = Channel<Unit>(Channel.UNLIMITED)
            val m = DevicesModel(env.client, backgroundScope, clock = { now }, zone = utc, delayer = { ticks.receive() })
            assertNull(m.state.value.pairing)
            m.startPairing()
            val first = m.state.first { it.pairing != null }.pairing!!
            assertEquals("482913", first.code)
            assertEquals("Studio Mac", first.serverName)
            assertEquals(listOf("192.168.1.20"), first.lanAddresses)
            assertEquals(50072, first.lanPort)
            assertEquals("0123456789abcdef", first.fingerprintPrefix)
            assertEquals(300L, first.remainingSeconds)
            assertEquals("5:00", first.countdownText)
            assertFalse(first.expired)

            now = base + 61_000
            ticks.send(Unit)
            assertEquals("3:59", m.state.first { it.pairing?.remainingSeconds == 239L }.pairing!!.countdownText)

            now = base + 300_000
            ticks.send(Unit)
            val expired = m.state.first { it.pairing?.expired == true }.pairing!!
            assertEquals("0:00", expired.countdownText)
        }
    }

    @Test
    fun cancelPairingClearsTheCode() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            val ticks = Channel<Unit>(Channel.UNLIMITED)
            val m = DevicesModel(env.client, backgroundScope, clock = { base }, zone = utc, delayer = { ticks.receive() })
            m.startPairing()
            m.state.first { it.pairing != null }
            m.cancelPairing()
            assertNull(m.state.value.pairing)
        }
    }

    @Test
    fun listsPairedDevicesAndRevokeRefreshesTheList() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.pairedDevices = listOf(
                device("d1", DevicePlatform.DEVICE_PLATFORM_IOS, "2025-11-20T14:02:00Z", base - 5 * 60_000),
                device("d2", DevicePlatform.DEVICE_PLATFORM_ANDROID, "2025-11-27T09:30:00Z", 0),
            )
            val m = DevicesModel(env.client, backgroundScope, clock = { base }, zone = utc)
            m.refresh()
            val rows = m.state.first { it.devices.size == 2 }.devices
            assertEquals(listOf("d1", "d2"), rows.map { it.deviceId })
            assertEquals("Phone d1", rows[0].name)
            assertEquals("iOS", rows[0].platformText)
            assertEquals("Nov 20 14:02", rows[0].pairedText)
            assertEquals("5 min ago", rows[0].lastSeenText)
            assertEquals("Android", rows[1].platformText)
            assertEquals("09:30", rows[1].pairedText)
            assertEquals("never", rows[1].lastSeenText)

            env.fake.pairedDevices = env.fake.pairedDevices.drop(1) // what the server lists once d1 is revoked
            m.revoke("d1")
            val after = m.state.first { it.devices.size == 1 }
            assertEquals("d2", after.devices.single().deviceId)
            assertEquals(1, env.fake.callCount("RevokeDevice"))
        }
    }

    @Test
    fun failuresAreReported() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["StartPairing"] = Status.PERMISSION_DENIED.withDescription("loopback only")
            val m = DevicesModel(env.client, backgroundScope, clock = { base }, zone = utc)
            m.startPairing()
            assertEquals("loopback only", m.state.first { it.error != null }.error)
            env.fake.failWith.clear()
            env.fake.failWith["RevokeDevice"] = Status.NOT_FOUND.withDescription("no such device")
            m.revoke("x")
            assertTrue(m.state.first { it.error == "no such device" }.devices.isEmpty())
        }
    }
}
