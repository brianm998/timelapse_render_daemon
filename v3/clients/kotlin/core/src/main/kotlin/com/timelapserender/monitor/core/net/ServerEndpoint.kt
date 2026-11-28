// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

/** How a client reaches a trd listener: loopback in plaintext, or LAN over TLS pinned to one certificate. */
sealed interface TlsMode {
    data object Plaintext : TlsMode

    /** [fingerprintSha256Hex] is the lowercase hex SHA-256 of the server certificate's DER. */
    data class Pinned(val fingerprintSha256Hex: String) : TlsMode
}

data class ServerEndpoint(val host: String, val port: Int, val tls: TlsMode)
