// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.discovery

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.net.Inet4Address
import java.net.InetAddress
import java.net.NetworkInterface
import javax.jmdns.JmDNS
import javax.jmdns.ServiceEvent
import javax.jmdns.ServiceListener

/** Browses `_trd._tcp.local.` on every up, multicast, non-loopback interface address. */
class JmdnsDiscovery(
    private val addresses: () -> List<InetAddress> = ::defaultAddresses,
) : ServerDiscovery {
    private val _servers = MutableStateFlow<List<DiscoveredServer>>(emptyList())
    private val lock = Any()
    private val instances = ArrayList<Pair<JmDNS, ServiceListener>>()

    /** Service name -> server id, so a removal event (which carries no TXT records) can drop the entry. */
    private val idsByService = HashMap<String, String>()

    override val servers: StateFlow<List<DiscoveredServer>> = _servers.asStateFlow()

    override fun start() {
        synchronized(lock) {
            if (instances.isNotEmpty()) return
            for (address in addresses()) {
                try {
                    val jmdns = JmDNS.create(address)
                    val listener = Listener()
                    jmdns.addServiceListener(SERVICE_TYPE, listener)
                    instances += jmdns to listener
                } catch (_: java.io.IOException) {
                    // An interface that cannot join multicast is skipped.
                }
            }
        }
    }

    override fun stop() {
        val closing = synchronized(lock) {
            val copy = instances.toList()
            instances.clear()
            idsByService.clear()
            copy
        }
        for ((jmdns, listener) in closing) {
            try {
                jmdns.removeServiceListener(SERVICE_TYPE, listener)
                jmdns.close()
            } catch (_: java.io.IOException) {
            }
        }
        _servers.value = emptyList()
    }

    private inner class Listener : ServiceListener {
        override fun serviceAdded(event: ServiceEvent) {
            event.dns.requestServiceInfo(event.type, event.name, true)
        }

        override fun serviceRemoved(event: ServiceEvent) {
            val id = synchronized(lock) { idsByService.remove(event.name) } ?: return
            _servers.update { list -> list.filter { it.serverId != id } }
        }

        override fun serviceResolved(event: ServiceEvent) {
            val info = event.info
            val txt = HashMap<String, String>()
            for (key in info.propertyNames) {
                txt[key] = info.getPropertyString(key) ?: ""
            }
            val host = info.inet4Addresses.firstOrNull()?.hostAddress
                ?: info.server.orEmpty().trimEnd('.')
            val server = parseTxt(txt, host) ?: return
            synchronized(lock) { idsByService[event.name] = server.serverId }
            _servers.update { list -> list.filter { it.serverId != server.serverId } + server }
        }
    }

    companion object {
        const val SERVICE_TYPE = "_trd._tcp.local."

        fun defaultAddresses(): List<InetAddress> =
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { runCatching { it.isUp && !it.isLoopback && it.supportsMulticast() }.getOrDefault(false) }
                .flatMap { nic -> nic.inetAddresses.toList().filterIsInstance<Inet4Address>() }
    }
}
