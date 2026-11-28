// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import com.timelapserender.trd.v1.ServerInfo

const val SUPPORTED_API_VERSION = 1

sealed interface ConnectionState {
    data object Connecting : ConnectionState

    data class Connected(val info: ServerInfo) : ConnectionState

    data class Reconnecting(val attempt: Int, val lastUpdateMs: Long) : ConnectionState

    /** UNAUTHENTICATED: the token is missing, wrong or revoked. */
    data object NeedsPairing : ConnectionState

    data class Incompatible(val serverVersion: Int) : ConnectionState {
        val message: String
            get() = "This trd uses API version $serverVersion; this app supports version $SUPPORTED_API_VERSION."
    }

    /** After the fourth failed attempt; the client keeps retrying. */
    data class Offline(val lastUpdateMs: Long) : ConnectionState
}
