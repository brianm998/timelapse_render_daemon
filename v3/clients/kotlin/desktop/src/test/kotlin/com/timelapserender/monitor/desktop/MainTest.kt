// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.WindowPlacement
import androidx.compose.ui.window.WindowPosition
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.desktop.platform.WindowGeometry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class MainTest {
    @Test
    fun theServerOptionIsAPlaintextLoopbackEndpoint() {
        assertNull(parseOptions(emptyList()).server)
        assertNull(parseOptions(listOf("-psn_0_123")).server)
        assertEquals(
            ServerEndpoint("127.0.0.1", 50081, TlsMode.Plaintext),
            parseOptions(listOf("--server", "127.0.0.1:50081")).server,
        )
        assertEquals("::1", parseOptions(listOf("--server", "[::1]:50081")).server?.host)
    }

    @Test
    fun badOptionsAreRefused() {
        for (args in listOf(
            listOf("--server"),
            listOf("--server", "127.0.0.1"),
            listOf("--server", "127.0.0.1:0"),
            listOf("--server", "10.0.0.2:50081"),
            listOf("--verbose"),
        )) {
            assertThrows(args.toString(), IllegalArgumentException::class.java) { parseOptions(args) }
        }
    }

    @Test
    fun onlyAFloatingWindowsGeometryIsRemembered() {
        val size = DpSize(1000.dp, 700.dp)
        val position = WindowPosition(10.dp, 20.dp)
        assertEquals(WindowGeometry(10f, 20f, 1000f, 700f), geometryOf(size, position, WindowPlacement.Floating))
        assertEquals(
            WindowGeometry(null, null, 1000f, 700f),
            geometryOf(size, WindowPosition.PlatformDefault, WindowPlacement.Floating),
        )
        assertNull(geometryOf(size, position, WindowPlacement.Maximized))
        assertNull(geometryOf(size, position, WindowPlacement.Fullscreen))
        assertNull(geometryOf(DpSize.Unspecified, position, WindowPlacement.Floating))
    }

    @Test
    fun theRememberedPositionIsRestored() {
        assertEquals(WindowPosition(10.dp, 20.dp), positionOf(WindowGeometry(10f, 20f, 1000f, 700f)))
        assertEquals(WindowPosition.PlatformDefault, positionOf(WindowGeometry(null, null, 1000f, 700f)))
    }
}
