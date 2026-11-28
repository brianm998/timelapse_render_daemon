// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.InMemoryAppPreferences
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class PreferencesWindowTest {
    @get:Rule
    val compose = createComposeRule()

    private var harness: Harness? = null

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun show(preferences: InMemoryAppPreferences = InMemoryAppPreferences()): InMemoryAppPreferences {
        val h = Harness(preferences = preferences)
        harness = h
        compose.setContent { PreferencesContent(h.app) }
        return preferences
    }

    private fun node(tag: String) = compose.onNodeWithTag(tag)

    @Test
    fun theSwitchesChangeThePreferences() {
        val preferences = show()
        for (tag in listOf(PreferencesTags.NOTIFY_FINISHED, PreferencesTags.NOTIFY_FAILED, PreferencesTags.KEEP_IN_TRAY)) {
            node(tag).assertIsOn()
        }
        node(PreferencesTags.NOTIFY_FINISHED).performClick()
        node(PreferencesTags.NOTIFY_FINISHED).assertIsOff()
        assertFalse(preferences.notifyFinished)
        assertTrue(preferences.notifyFailed)
        node(PreferencesTags.NOTIFY_FAILED).performClick()
        node(PreferencesTags.NOTIFY_FAILED).assertIsOff()
        assertFalse(preferences.notifyFailed)
        assertTrue(preferences.keepRunningInTray)
        node(PreferencesTags.KEEP_IN_TRAY).performClick()
        node(PreferencesTags.KEEP_IN_TRAY).assertIsOff()
        assertFalse(preferences.keepRunningInTray)
        node(PreferencesTags.NOTIFY_FINISHED).performClick()
        node(PreferencesTags.NOTIFY_FINISHED).assertIsOn()
        assertTrue(preferences.notifyFinished)
    }

    @Test
    fun theRateStepsBetweenOneAndTen() {
        val preferences = show()
        node(PreferencesTags.RATE).assertTextEquals("4")
        node(PreferencesTags.RATE_UP).performClick()
        node(PreferencesTags.RATE).assertTextEquals("5")
        assertEquals(5, preferences.snapshotRate)
        repeat(6) { node(PreferencesTags.RATE_UP).performClick() }
        node(PreferencesTags.RATE).assertTextEquals("10")
        node(PreferencesTags.RATE_UP).assertIsNotEnabled()
        repeat(10) { node(PreferencesTags.RATE_DOWN).performClick() }
        node(PreferencesTags.RATE).assertTextEquals("1")
        node(PreferencesTags.RATE_DOWN).assertIsNotEnabled()
        assertEquals(1, preferences.snapshotRate)
    }

    @Test
    fun theDontAskAgainChoicesAreReset() {
        val preferences = show(
            InMemoryAppPreferences().apply {
                skipStopConfirmation = true
                skipPauseAllConfirmation = true
            },
        )
        node(PreferencesTags.RESET).assertIsEnabled()
        node(PreferencesTags.RESET).performClick()
        node(PreferencesTags.RESET).assertIsNotEnabled()
        assertFalse(preferences.skipStopConfirmation)
        assertFalse(preferences.skipPauseAllConfirmation)
    }
}
