// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.discovery

import com.timelapserender.monitor.core.testing.SystemTests
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.net.InetAddress
import javax.jmdns.JmDNS
import javax.jmdns.ServiceInfo

/** System test: real multicast. Runs only with CI=true and TRM_MDNS_TEST=1; otherwise every case is skipped. */
class JmdnsLiveTest {
    private var registrar: JmDNS? = null
    private var discovery: JmdnsDiscovery? = null

    @Before
    fun requireSystemTestSwitch() {
        assumeTrue(SystemTests.allowed(System::getenv, "TRM_MDNS_TEST"))
    }

    @After
    fun tearDown() {
        discovery?.stop()
        registrar?.close()
    }

    @Test
    fun registeredServiceIsDiscoveredWithEveryFieldThenRemoved() = runBlocking<Unit> {
        val address = JmdnsDiscovery.defaultAddresses().firstOrNull() ?: InetAddress.getLoopbackAddress()
        val found = JmdnsDiscovery { listOf(address) }.also { discovery = it }
        found.start()
        val reg = JmDNS.create(address).also { registrar = it }
        val txt = mapOf("id" to "live-1", "name" to "Live trd", "fp" to "abcd1234", "port" to "50071", "ver" to "3.0.0", "api" to "1")
        val info = ServiceInfo.create(JmdnsDiscovery.SERVICE_TYPE, "live-trd", 50071, 0, 0, txt)
        reg.registerService(info)

        val server = withTimeout(120_000) { found.servers.first { list -> list.any { it.serverId == "live-1" } } }
            .first { it.serverId == "live-1" }
        assertEquals("Live trd", server.name)
        assertEquals(50071, server.port)
        assertEquals("abcd1234", server.fingerprint)
        assertEquals("3.0.0", server.version)
        assertEquals(1, server.apiVersion)
        assertEquals(true, server.host.isNotEmpty())

        reg.unregisterService(info)
        withTimeout(120_000) { found.servers.first { list -> list.none { it.serverId == "live-1" } } }
    }
}
