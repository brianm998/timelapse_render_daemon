// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.fake.TestCertificate
import com.timelapserender.trd.v1.GetServerInfoRequest
import com.timelapserender.trd.v1.TrdServiceGrpcKt
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.util.concurrent.TimeUnit

class PinnedTlsTest {
    private fun stub(endpoint: ServerEndpoint) =
        TrdServiceGrpcKt.TrdServiceCoroutineStub(ChannelFactory.create(endpoint))
            .withDeadlineAfter(60, TimeUnit.SECONDS)

    @Test
    fun rightFingerprintConnects() = runBlocking {
        val fake = FakeTrdService()
        fake.startTls(cert).use { running ->
            val channel = ChannelFactory.create(
                ServerEndpoint("127.0.0.1", running.port, TlsMode.Pinned(running.fingerprintSha256Hex!!.uppercase())),
            )
            try {
                val info = TrdServiceGrpcKt.TrdServiceCoroutineStub(channel)
                    .withDeadlineAfter(60, TimeUnit.SECONDS)
                    .getServerInfo(GetServerInfoRequest.getDefaultInstance())
                assertEquals("fake-server", info.serverId)
                assertEquals(listOf("GetServerInfo"), fake.calls.map { it.method })
            } finally {
                channel.shutdownNow()
            }
        }
    }

    @Test
    fun wrongFingerprintFailsBeforeAnyRpc() = runBlocking {
        val fake = FakeTrdService()
        fake.startTls(cert).use { running ->
            val wrong = "0".repeat(64)
            try {
                stub(ServerEndpoint("127.0.0.1", running.port, TlsMode.Pinned(wrong)))
                    .getServerInfo(GetServerInfoRequest.getDefaultInstance())
                fail("The call must fail")
            } catch (e: StatusException) {
                assertEquals(Status.Code.UNAVAILABLE, e.status.code)
            }
            assertTrue(fake.calls.isEmpty())
        }
    }

    @Test
    fun hostnameIsNotChecked() = runBlocking {
        val fake = FakeTrdService()
        fake.startTls(cert).use { running ->
            // The certificate is for CN=trd-test; "localhost" does not match it.
            val info = stub(ServerEndpoint("localhost", running.port, TlsMode.Pinned(running.fingerprintSha256Hex!!)))
                .getServerInfo(GetServerInfoRequest.getDefaultInstance())
            assertEquals("Fake trd", info.name)
        }
    }

    @Test
    fun trustManagerRejectsAnEmptyChain() {
        val tm = PinnedTrustManager(cert.fingerprintSha256Hex)
        try {
            tm.checkServerTrusted(emptyArray(), "RSA")
            fail("An empty chain must be rejected")
        } catch (_: java.security.cert.CertificateException) {
        }
        tm.checkServerTrusted(arrayOf(cert.certificate), "RSA")
    }

    companion object {
        private val cert: TestCertificate = TestCertificate.generate()
    }
}
