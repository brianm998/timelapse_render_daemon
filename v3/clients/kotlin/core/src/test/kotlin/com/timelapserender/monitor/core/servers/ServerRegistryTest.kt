// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.servers

import com.timelapserender.monitor.core.discovery.DiscoveredServer
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ServerRegistryTest {
    private class MemoryStore : ServerStore {
        val records = LinkedHashMap<String, ServerRecord>()
        override fun list() = records.values.toList()
        override fun save(record: ServerRecord) {
            records[record.serverId] = record
        }
        override fun remove(serverId: String) {
            records.remove(serverId)
        }
    }

    private fun record(id: String) = ServerRecord(id, "stored-$id", "10.0.0.1", 50072, "ff", "tok", "linux-x64", 1L)
    private fun found(id: String) = DiscoveredServer(id, "found-$id", "10.0.0.9", 50072, "ee", "3.0.0", 1)

    @Test
    fun mergesLocalStoredAndDiscoveredWithStoredWinning() {
        val store = MemoryStore().apply { save(record("a")) }
        val local = ServerRegistry.localEndpoint(ServerEndpoint("127.0.0.1", 1234, TlsMode.Plaintext))
        val registry = ServerRegistry(store, local)
        registry.updateDiscovered(listOf(found("a"), found("b")))
        val servers = registry.servers.value
        assertEquals(listOf("local", "a", "b"), servers.map { it.serverId })
        assertEquals(listOf(ServerSource.LOCAL, ServerSource.STORED, ServerSource.DISCOVERED), servers.map { it.source })
        assertEquals("stored-a", servers[1].name)
        assertEquals("tok", servers[1].token)
        assertEquals(TlsMode.Pinned("ff"), servers[1].endpoint.tls)
        assertEquals(local, servers[0].endpoint)
        assertEquals(TlsMode.Pinned("ee"), servers[2].endpoint.tls)
        assertNull(servers[2].token)
    }

    @Test
    fun defaultLocalEndpointIsLoopbackPlaintext() {
        assertEquals(ServerEndpoint("127.0.0.1", 50071, TlsMode.Plaintext), ServerRegistry.localEndpoint())
    }

    @Test
    fun withoutALocalServerOnlyStoredAndDiscoveredAppear() {
        val registry = ServerRegistry(MemoryStore())
        assertEquals(emptyList<RegisteredServer>(), registry.servers.value)
        registry.updateDiscovered(listOf(found("x")))
        assertEquals(listOf("x"), registry.servers.value.map { it.serverId })
    }

    @Test
    fun savingAndRemovingUpdatesTheListAndTheActiveId() {
        val registry = ServerRegistry(MemoryStore())
        registry.save(record("a"))
        registry.updateDiscovered(listOf(found("a")))
        assertEquals(listOf(ServerSource.STORED), registry.servers.value.map { it.source })
        assertNull(registry.activeServerId.value)
        registry.select("a")
        assertEquals("a", registry.activeServerId.value)
        assertEquals("stored-a", registry.active?.name)
        registry.remove("a")
        assertEquals(listOf(ServerSource.DISCOVERED), registry.servers.value.map { it.source })
        assertNull(registry.activeServerId.value)
    }
}
