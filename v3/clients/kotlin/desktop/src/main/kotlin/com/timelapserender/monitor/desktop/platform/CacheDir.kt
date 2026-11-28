// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import java.nio.file.Path
import java.nio.file.Paths

/**
 * The client's cache directory: macOS `~/Library/Caches/TimelapseRenderMonitor`, Linux
 * `$XDG_CACHE_HOME/trm`, Windows `%LOCALAPPDATA%\TimelapseRenderMonitor\Cache`. [osName] is `os.name`.
 */
fun defaultCacheDir(osName: String, env: (String) -> String?, home: String): Path {
    val os = osName.lowercase()
    fun variable(name: String) = env(name)?.takeIf { it.isNotEmpty() }
    return when {
        os.contains("mac") || os.contains("darwin") -> Paths.get(home, "Library", "Caches", "TimelapseRenderMonitor")
        os.contains("win") ->
            Paths.get(variable("LOCALAPPDATA") ?: Paths.get(home, "AppData", "Local").toString())
                .resolve("TimelapseRenderMonitor").resolve("Cache")
        else -> Paths.get(variable("XDG_CACHE_HOME") ?: Paths.get(home, ".cache").toString()).resolve("trm")
    }
}
