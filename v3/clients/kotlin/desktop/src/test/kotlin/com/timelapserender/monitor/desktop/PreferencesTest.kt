// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.prefs.AbstractPreferences
import java.util.prefs.Preferences

class PreferencesTest {
    /** A preferences node in memory: the owner's preferences are never touched. */
    private class MemoryNode : AbstractPreferences(null, "") {
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

    private val node = MemoryNode()

    /** Each implementation, as a source of instances over the same stored values. */
    private fun implementations(): List<() -> AppPreferences> {
        val memory = InMemoryAppPreferences()
        return listOf({ JavaAppPreferences(node) }, { memory })
    }

    @Test
    fun theDefaultsFollowTheSpec() {
        for (open in implementations()) {
            val preferences = open()
            assertTrue(preferences.notifyFinished)
            assertTrue(preferences.notifyFailed)
            assertTrue(preferences.keepRunningInTray)
            assertEquals(4, preferences.snapshotRate)
            assertFalse(preferences.skipStopConfirmation)
            assertFalse(preferences.skipPauseAllConfirmation)
            assertNull(preferences.get("a"))
        }
    }

    @Test
    fun choicesPersistAndConfirmationsCanBeReset() {
        for (open in implementations()) {
            open().apply {
                notifyFinished = false
                notifyFailed = false
                keepRunningInTray = false
                snapshotRate = 7
                skipStopConfirmation = true
                skipPauseAllConfirmation = true
                set("a", 5L)
                set("b", 9L)
            }
            val read = open()
            assertFalse(read.notifyFinished)
            assertFalse(read.notifyFailed)
            assertFalse(read.keepRunningInTray)
            assertEquals(7, read.snapshotRate)
            assertTrue(read.skipStopConfirmation)
            assertTrue(read.skipPauseAllConfirmation)
            assertEquals(5L, read.get("a"))
            assertEquals(9L, read.get("b"))

            read.resetConfirmations()
            assertFalse(open().skipStopConfirmation)
            assertFalse(open().skipPauseAllConfirmation)
            assertFalse(open().notifyFinished)
        }
    }

    @Test
    fun theSnapshotRateStaysWithinOneToTen() {
        for (open in implementations()) {
            val preferences = open()
            preferences.snapshotRate = 0
            assertEquals(1, preferences.snapshotRate)
            preferences.snapshotRate = 50
            assertEquals(10, preferences.snapshotRate)
        }
        node.values["snapshotRate"] = "400"
        assertEquals(10, JavaAppPreferences(node).snapshotRate)
        node.values["snapshotRate"] = "fast"
        assertEquals(DEFAULT_SNAPSHOT_RATE, JavaAppPreferences(node).snapshotRate)
    }

    @Test
    fun aLongServerIdFitsAKeyAndStaysDistinct() {
        val preferences = JavaAppPreferences(node)
        val first = "s".repeat(200) + "1"
        val second = "s".repeat(200) + "2"
        preferences.set(first, 1L)
        preferences.set(second, 2L)
        assertEquals(1L, preferences.get(first))
        assertEquals(2L, preferences.get(second))
        assertTrue(node.values.keys.all { it.length <= Preferences.MAX_KEY_LENGTH })
    }
}
