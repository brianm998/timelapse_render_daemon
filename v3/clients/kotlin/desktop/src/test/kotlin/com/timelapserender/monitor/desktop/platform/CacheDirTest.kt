// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import org.junit.Assert.assertEquals
import org.junit.Test
import java.nio.file.Paths

class CacheDirTest {
    private fun dir(os: String, vararg env: Pair<String, String>) = defaultCacheDir(os, mapOf(*env)::get, "/home/t")

    @Test
    fun eachSystemHasItsCacheDirectory() {
        assertEquals(Paths.get("/home/t", "Library", "Caches", "TimelapseRenderMonitor"), dir("Mac OS X"))
        assertEquals(Paths.get("/home/t", ".cache", "trm"), dir("Linux"))
        assertEquals(Paths.get("/xdg", "trm"), dir("Linux", "XDG_CACHE_HOME" to "/xdg"))
        assertEquals(
            Paths.get("/home/t", "AppData", "Local", "TimelapseRenderMonitor", "Cache"),
            dir("Windows 11", "LOCALAPPDATA" to ""),
        )
        assertEquals(Paths.get("/local", "TimelapseRenderMonitor", "Cache"), dir("Windows 11", "LOCALAPPDATA" to "/local"))
    }
}
