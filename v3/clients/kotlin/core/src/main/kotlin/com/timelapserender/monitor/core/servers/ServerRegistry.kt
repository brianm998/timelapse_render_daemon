// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.servers

import com.timelapserender.monitor.core.discovery.DiscoveredServer
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class ServerSource { LOCAL, STORED, DISCOVERED }

/** One row of the server list. [token] is set for stored servers that are paired. */
data class RegisteredServer(
    val serverId: String,
    val name: String,
    val endpoint: ServerEndpoint,
    val token: String?,
    val source: ServerSource,
)

/**
 * Merges the local server (desktop only), the stored records and the discovered services, and holds
 * the active server id. A stored record beats a discovered service with the same server id.
 */
class ServerRegistry(
    private val store: ServerStore,
    private val localEndpoint: ServerEndpoint? = null,
) {
    private val lock = Any()
    private var discovered: List<DiscoveredServer> = emptyList()
    private val _servers = MutableStateFlow(merge())
    private val _activeServerId = MutableStateFlow<String?>(null)

    val servers: StateFlow<List<RegisteredServer>> = _servers.asStateFlow()
    val activeServerId: StateFlow<String?> = _activeServerId.asStateFlow()

    val active: RegisteredServer?
        get() = _servers.value.firstOrNull { it.serverId == _activeServerId.value }

    fun select(serverId: String?) {
        _activeServerId.value = serverId
    }

    fun updateDiscovered(list: List<DiscoveredServer>) = synchronized(lock) {
        discovered = list
        _servers.value = merge()
    }

    /** Re-reads the store after a change made elsewhere. */
    fun refresh() = synchronized(lock) { _servers.value = merge() }

    fun save(record: ServerRecord) = synchronized(lock) {
        store.save(record)
        _servers.value = merge()
    }

    fun remove(serverId: String) = synchronized(lock) {
        store.remove(serverId)
        _servers.value = merge()
        if (_activeServerId.value == serverId) _activeServerId.value = null
    }

    private fun merge(): List<RegisteredServer> {
        val out = ArrayList<RegisteredServer>()
        if (localEndpoint != null) {
            out += RegisteredServer(LOCAL_SERVER_ID, "This computer", localEndpoint, null, ServerSource.LOCAL)
        }
        val stored = store.list()
        for (r in stored) {
            val tls = if (r.fingerprint.isEmpty()) TlsMode.Plaintext else TlsMode.Pinned(r.fingerprint)
            out += RegisteredServer(r.serverId, r.name, ServerEndpoint(r.host, r.port, tls), r.token, ServerSource.STORED)
        }
        val seen = stored.mapTo(HashSet()) { it.serverId }
        for (d in discovered) {
            if (!seen.add(d.serverId)) continue
            out += RegisteredServer(
                d.serverId, d.name, ServerEndpoint(d.host, d.port, TlsMode.Pinned(d.fingerprint)), null,
                ServerSource.DISCOVERED,
            )
        }
        return out
    }

    companion object {
        const val LOCAL_SERVER_ID = "local"
        const val DEFAULT_LOCAL_PORT = 50071

        fun localEndpoint(override: ServerEndpoint? = null): ServerEndpoint =
            override ?: ServerEndpoint("127.0.0.1", DEFAULT_LOCAL_PORT, TlsMode.Plaintext)
    }
}
