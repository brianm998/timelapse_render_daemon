// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.pairing

import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.fake.TestCertificate
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.trd.v1.DevicePlatform
import io.grpc.Status
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class PairingClientTest {
    private val fake = FakeTrdService()

    private fun <T> withServer(block: suspend (ServerEndpoint) -> T): T = fake.startTls(cert).use { running ->
        val endpoint = ServerEndpoint("127.0.0.1", running.port, TlsMode.Pinned(running.fingerprintSha256Hex!!))
        runBlocking { block(endpoint) }
    }

    private fun pairer() = PairingClient(clock = { 99L }, timeoutMs = 60_000)

    private suspend fun failure(endpoint: ServerEndpoint): String = try {
        pairer().pair(endpoint, "000000", "Desk", DevicePlatform.DEVICE_PLATFORM_DESKTOP)
        fail("expected a PairingException")
        ""
    } catch (e: PairingException) {
        e.message!!
    }

    @Test
    fun pairingStoresAWorkingRecord() {
        fake.serverInfo = fake.serverInfo.toBuilder().setCallerIsLocal(false).setCallerIsPaired(true).build()
        withServer { endpoint ->
            val record = pairer().pair(endpoint, "123456", "Desk", DevicePlatform.DEVICE_PLATFORM_DESKTOP)
            assertEquals("fake-token", record.token)
            assertEquals("fake-server", record.serverId)
            assertEquals("Fake trd", record.name)
            assertEquals(endpoint.port, record.port)
            assertEquals((endpoint.tls as TlsMode.Pinned).fingerprintSha256Hex, record.fingerprint)
            assertEquals(99L, record.lastSeenMs)
            assertEquals(listOf("Pair", "GetServerInfo"), fake.calls.map { it.method })
            assertEquals(null, fake.calls[0].authorization)
            assertEquals("Bearer fake-token", fake.calls[1].authorization)
        }
    }

    @Test
    fun wrongCodeGivesTheInvalidCodeMessage() {
        fake.failWith["Pair"] = Status.INVALID_ARGUMENT
        withServer { assertEquals("That code is not valid. Ask for a new one on the trd machine.", failure(it)) }
    }

    @Test
    fun exhaustedCodeGivesTheTooManyMessage() {
        fake.failWith["Pair"] = Status.RESOURCE_EXHAUSTED
        withServer { assertEquals("Too many wrong codes. Issue a new code on the trd machine.", failure(it)) }
    }

    @Test
    fun unconfirmedPairingFails() {
        fake.serverInfo = fake.serverInfo.toBuilder().setCallerIsPaired(false).build()
        withServer { assertEquals(PairingClient.NOT_CONFIRMED, failure(it)) }
    }

    @Test
    fun unreachableServerGivesAConnectionMessage() {
        withServer { endpoint ->
            val wrong = endpoint.copy(tls = TlsMode.Pinned("0".repeat(64)))
            assertEquals(PairingClient.CONNECTION_FAILED, failure(wrong))
        }
    }

    @Test
    fun plaintextEndpointIsRefused() {
        withServer { endpoint ->
            assertEquals(PairingClient.CONNECTION_FAILED, failure(endpoint.copy(tls = TlsMode.Plaintext)))
            assertTrue(fake.calls.isEmpty())
            assertNotNull(endpoint)
        }
    }

    private companion object {
        val cert: TestCertificate = TestCertificate.generate()
    }
}
