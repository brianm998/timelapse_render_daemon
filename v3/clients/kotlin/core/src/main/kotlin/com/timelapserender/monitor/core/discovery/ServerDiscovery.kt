// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.discovery

import kotlinx.coroutines.flow.StateFlow

/** Finds trds on the LAN. [servers] holds the ones currently visible. */
interface ServerDiscovery {
    val servers: StateFlow<List<DiscoveredServer>>

    fun start()

    fun stop()
}

/**
 * Parses trd's TXT records (spec 1 §8.4). Null when `id`, `fp` or `port` is missing or not usable, or
 * `api` is not `1`. `name` falls back to the id and `ver` to an empty string.
 */
fun parseTxt(txt: Map<String, String>, host: String = ""): DiscoveredServer? {
    val id = txt["id"]?.takeIf { it.isNotBlank() } ?: return null
    val fp = txt["fp"]?.takeIf { it.isNotBlank() } ?: return null
    val port = txt["port"]?.trim()?.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
    if (txt["api"]?.trim() != "1") return null
    return DiscoveredServer(
        serverId = id,
        name = txt["name"]?.takeIf { it.isNotBlank() } ?: id,
        host = host,
        port = port,
        fingerprint = fp,
        version = txt["ver"] ?: "",
        apiVersion = 1,
    )
}
