// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import java.util.prefs.Preferences

/** The main window's size and position in dp; [x] and [y] are null when the platform chooses. */
data class WindowGeometry(val x: Float?, val y: Float?, val width: Float, val height: Float)

/** What the desktop app remembers between runs. */
interface DesktopSettings {
    var lastServerId: String?
    var windowGeometry: WindowGeometry?
    var detailsPaneVisible: Boolean
    var search: String
    var exportPath: String?
}

/** [DesktopSettings] in a `java.util.prefs` node (`com/timelapserender/monitor`, spec 2 §6.8). */
class PreferencesSettings(private val node: Preferences) : DesktopSettings {
    override var lastServerId: String?
        get() = node.get(LAST_SERVER, null)
        set(value) = putOrRemove(LAST_SERVER, value)

    override var windowGeometry: WindowGeometry?
        get() {
            val width = float(WIDTH) ?: return null
            val height = float(HEIGHT) ?: return null
            return WindowGeometry(float(X), float(Y), width, height)
        }
        set(value) {
            putOrRemove(X, value?.x?.toString())
            putOrRemove(Y, value?.y?.toString())
            putOrRemove(WIDTH, value?.width?.toString())
            putOrRemove(HEIGHT, value?.height?.toString())
        }

    override var detailsPaneVisible: Boolean
        get() = node.getBoolean(DETAILS, true)
        set(value) = node.putBoolean(DETAILS, value)

    override var search: String
        get() = node.get(SEARCH, "")
        set(value) = putOrRemove(SEARCH, value.take(Preferences.MAX_VALUE_LENGTH).ifEmpty { null })

    override var exportPath: String?
        get() = node.get(EXPORT_PATH, null)
        set(value) = putOrRemove(EXPORT_PATH, value?.takeIf { it.length <= Preferences.MAX_VALUE_LENGTH })

    private fun float(key: String): Float? = node.get(key, null)?.toFloatOrNull()?.takeIf { it.isFinite() }

    private fun putOrRemove(key: String, value: String?) {
        if (value == null) node.remove(key) else node.put(key, value)
    }

    companion object {
        const val NODE = "com/timelapserender/monitor"
        private const val LAST_SERVER = "lastServerId"
        private const val X = "window.x"
        private const val Y = "window.y"
        private const val WIDTH = "window.width"
        private const val HEIGHT = "window.height"
        private const val DETAILS = "detailsPaneVisible"
        private const val SEARCH = "search"
        private const val EXPORT_PATH = "exportPath"
    }
}
