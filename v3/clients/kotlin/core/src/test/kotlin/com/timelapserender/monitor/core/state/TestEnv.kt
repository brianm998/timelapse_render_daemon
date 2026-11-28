// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.TrdClient
import kotlinx.coroutines.CoroutineScope

/** A [FakeTrdService] with an in-process channel and a client over it. */
internal class Env(scope: CoroutineScope) : AutoCloseable {
    val fake = FakeTrdService()
    private val running = fake.startInProcess()
    val client = TrdClient(running.channel!!, null, scope)

    override fun close() {
        running.close()
    }
}
