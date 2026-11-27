// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.onNodeWithText
import org.junit.Test
import org.junit.Assert.assertEquals

@OptIn(ExperimentalTestApi::class)
class StartScreenTest {
    @Test
    fun startScreenSaysNotConnected() = runComposeUiTest {
        setContent { StartScreen() }
        onNodeWithText("Not connected").assertExists()
    }

    @Test
    fun windowTitleIsTheProductName() {
        assertEquals("Timelapse Render Monitor", WINDOW_TITLE)
    }
}
