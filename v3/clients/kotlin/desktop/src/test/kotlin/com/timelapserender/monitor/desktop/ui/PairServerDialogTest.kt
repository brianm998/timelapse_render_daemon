// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.fake.TestCertificate
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.pairing.PairingClient
import com.timelapserender.monitor.core.pairing.PairingException
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.core.servers.ServerRegistry
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.FakeDiscovery
import com.timelapserender.monitor.desktop.FakeSettings
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.MemoryStore
import com.timelapserender.monitor.desktop.RecordingRunner
import com.timelapserender.monitor.desktop.createClient
import com.timelapserender.monitor.desktop.platform.FileOpener
import com.timelapserender.monitor.desktop.platform.HostOs
import com.timelapserender.monitor.desktop.platform.TrdService
import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.PairRequest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList

class PairServerDialogTest {
    @get:Rule
    val compose = createComposeRule()

    private val closing = mutableListOf<AutoCloseable>()

    @After
    fun tearDown() {
        closing.asReversed().forEach { it.close() }
    }

    private val full = "0123456789abcdef".repeat(4)

    @Test
    fun theDeviceNameSaysItIsADesktop() {
        assertEquals("studio (desktop)", desktopDeviceName("studio"))
    }

    @Test
    fun aTypedFingerprintMayBeGroupedButNotShort() {
        assertEquals("0123456789abcdef", normalizeFingerprint(" 0123 4567 89AB CDEF "))
        assertEquals(full, normalizeFingerprint(full.chunked(2).joinToString(":")))
        assertNull(normalizeFingerprint("0123 4567 89ab cde"))
        assertNull(normalizeFingerprint("0123 4567 89ab cdeg"))
        assertNull(normalizeFingerprint(full + "0"))
    }

    @Test
    fun theProbeReturnsTheWholeFingerprintOfTheMatchingCertificate() = runBlocking {
        val certificate = TestCertificate.generate()
        val running = FakeTrdService().startTls(certificate).also { closing += it }
        val fingerprint = certificate.fingerprintSha256Hex
        // A terminated server no longer tells its port.
        val port = running.port
        assertEquals(fingerprint, probeFingerprint("127.0.0.1", port, fingerprint.take(16)))

        val other = (if (fingerprint[0] == '0') "1" else "0") + fingerprint.substring(1, 16)
        val mismatch = assertThrows(PairingException::class.java) {
            runBlocking { probeFingerprint("127.0.0.1", port, other) }
        }
        assertEquals(FINGERPRINT_MISMATCH, mismatch.message)

        running.close()
        val unreachable = assertThrows(PairingException::class.java) {
            runBlocking { probeFingerprint("127.0.0.1", port, fingerprint.take(16)) }
        }
        assertEquals(PairingClient.CONNECTION_FAILED, unreachable.message)
    }

    /** The app's own pairing, against a fake trd over TLS: nothing is replaced but the connections after it. */
    @Test
    fun thisDesktopPairsAsADesktopWithAHandEnteredServer() {
        val certificate = TestCertificate.generate()
        val remote = FakeTrdService()
        remote.serverInfo = Harness.info("fake-server", "Fake trd", local = false)
        val tls = remote.startTls(certificate).also { closing += it }
        val inProcess = FakeTrdService().startInProcess().also { closing += it }
        val store = MemoryStore()
        val app = AppState(
            registry = ServerRegistry(store, ServerEndpoint("127.0.0.1", Harness.LOCAL_PORT, TlsMode.Plaintext)),
            discovery = FakeDiscovery(),
            settings = FakeSettings(),
            service = TrdService(HostOs.LINUX, RecordingRunner(), "/home/tester") { null },
            opener = FileOpener(HostOs.LINUX, RecordingRunner()),
            scope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
            clientFactory = { server, scope -> createClient(server, scope, channelFor = { inProcess.newChannel() }) },
            deviceName = { desktopDeviceName("studio") },
        )
        closing += app
        app.start()
        app.showAddServer()
        app.submitPairingByAddress(" 127.0.0.1 ", tls.port, certificate.fingerprintSha256Hex.take(16), "123 456")
        runBlocking { withTimeout(Harness.TIMEOUT_MS) { app.ui.first { it.activeServerId == "fake-server" } } }

        val request = remote.requestsOf<PairRequest>().single()
        assertEquals("123456", request.code)
        assertEquals("studio (desktop)", request.deviceName)
        assertEquals(DevicePlatform.DEVICE_PLATFORM_DESKTOP, request.platform)
        val record = store.list().single()
        assertEquals(certificate.fingerprintSha256Hex, record.fingerprint)
        assertEquals("127.0.0.1", record.host)
        assertEquals(tls.port, record.port)
        assertEquals("fake-token", record.token)
    }

    private fun awaitText(text: String) =
        compose.waitUntil(Harness.TIMEOUT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }

    private fun type(tag: String, text: String) = compose.onNodeWithTag(tag).performScrollTo().performTextInput(text)

    @Test
    fun aServerIsAddedByHandFromTheServerList() {
        val probes = CopyOnWriteArrayList<String>()
        val paired = CopyOnWriteArrayList<ServerEndpoint>()
        val h = Harness(
            fingerprintProbe = { host, port, prefix ->
                probes += "$host $port $prefix"
                if (probes.size == 1) throw PairingException(FINGERPRINT_MISMATCH)
                full
            },
            pairer = { endpoint, _, _ ->
                paired += endpoint
                ServerRecord("other", "Other trd", endpoint.host, endpoint.port, full, "t", "linux-x64", 1L)
            },
        )
        closing += h
        h.app.start()
        compose.setContent { MainWindow(h.app) }
        h.app.showServers()
        awaitText("Add a server…")
        compose.onNodeWithText("Add a server…").performClick()
        awaitText("Add a server")

        compose.onNodeWithText("Pair").assertIsNotEnabled()
        type(PairServerTags.HOST, "192.168.1.9")
        type(PairServerTags.FINGERPRINT, "0123 4567 89ab cdef")
        type(PAIR_CODE_TAG, "123456")
        compose.onNodeWithText("Pair").assertIsEnabled().performClick()
        awaitText(FINGERPRINT_MISMATCH)
        assertTrue(paired.isEmpty())
        assertTrue(h.store.list().isEmpty())

        compose.onNodeWithText("Pair").performClick()
        h.await { it.dialog == null && it.activeServerId == "other" }
        assertEquals(List(2) { "192.168.1.9 $DEFAULT_LAN_PORT 0123456789abcdef" }, probes.toList())
        assertEquals(TlsMode.Pinned(full), paired.single().tls)
    }

    @Test
    fun aShortFingerprintIsRefusedBeforeAnythingIsDialled() {
        val h = Harness()
        closing += h
        h.app.showAddServer()
        h.app.submitPairingByAddress("192.168.1.9", DEFAULT_LAN_PORT, "0123", "123456")
        assertEquals(BAD_FINGERPRINT, (h.app.ui.value.dialog as AppDialog.PairByAddress).error)
    }
}
