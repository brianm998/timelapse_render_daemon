// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.prefs.AbstractPreferences

class DesktopSettingsTest {
    /** A preferences node in memory: the owner's preferences are never touched. */
    private class MemoryPreferences : AbstractPreferences(null, "") {
        val values = HashMap<String, String>()
        override fun putSpi(key: String, value: String) {
            values[key] = value
        }
        override fun getSpi(key: String): String? = values[key]
        override fun removeSpi(key: String) {
            values.remove(key)
        }
        override fun removeNodeSpi() = Unit
        override fun keysSpi(): Array<String> = values.keys.toTypedArray()
        override fun childrenNamesSpi(): Array<String> = emptyArray()
        override fun childSpi(name: String): AbstractPreferences = throw UnsupportedOperationException()
        override fun syncSpi() = Unit
        override fun flushSpi() = Unit
    }

    private val node = MemoryPreferences()

    @Test
    fun anEmptyNodeHasTheDefaults() {
        val settings = PreferencesSettings(node)
        assertNull(settings.lastServerId)
        assertNull(settings.windowGeometry)
        assertEquals(true, settings.detailsPaneVisible)
        assertEquals("", settings.search)
        assertNull(settings.exportPath)
    }

    @Test
    fun valuesSurviveANewInstanceAndCanBeCleared() {
        PreferencesSettings(node).apply {
            lastServerId = "abc"
            windowGeometry = WindowGeometry(12f, 34f, 1000f, 700f)
            detailsPaneVisible = false
            search = "LRT"
            exportPath = "/exports"
        }
        val read = PreferencesSettings(node)
        assertEquals("abc", read.lastServerId)
        assertEquals(WindowGeometry(12f, 34f, 1000f, 700f), read.windowGeometry)
        assertEquals(false, read.detailsPaneVisible)
        assertEquals("LRT", read.search)
        assertEquals("/exports", read.exportPath)

        read.lastServerId = null
        read.windowGeometry = WindowGeometry(null, null, 950f, 650f)
        read.search = ""
        read.exportPath = null
        assertNull(read.lastServerId)
        assertEquals(WindowGeometry(null, null, 950f, 650f), read.windowGeometry)
        assertEquals("", read.search)
        assertNull(read.exportPath)
        read.windowGeometry = null
        assertNull(read.windowGeometry)
    }

    @Test
    fun anUnreadableGeometryIsIgnored() {
        node.values["window.width"] = "wide"
        node.values["window.height"] = "700"
        assertNull(PreferencesSettings(node).windowGeometry)
    }
}
