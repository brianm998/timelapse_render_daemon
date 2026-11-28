// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.discovery

/** A trd found on the LAN, from the TXT records `id`, `name`, `fp`, `port`, `ver`, `api`. */
data class DiscoveredServer(
    val serverId: String,
    val name: String,
    val host: String,
    val port: Int,
    val fingerprint: String,
    val version: String,
    val apiVersion: Int,
)
