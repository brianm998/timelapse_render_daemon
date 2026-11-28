// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class MiddleTruncateTest {
    @get:Rule
    val compose = createComposeRule()

    private fun atMost(length: Int): (String) -> Boolean = { it.codePointCount(0, it.length) <= length }

    @Test
    fun aTextThatFitsIsUnchanged() {
        assertEquals("short.mov", middleTruncate("short.mov", atMost(9)))
    }

    @Test
    fun aLongNameKeepsItsHeadAndExtension() {
        assertEquals("LRT_0…8.mov", middleTruncate("LRT_00001_ProResMQ_8.mov", atMost(11)))
    }

    @Test
    fun withNoRoomOnlyTheEllipsisIsLeft() {
        assertEquals("…", middleTruncate("abcdef", atMost(0)))
    }

    @Test
    fun surrogatePairsAreNotSplit() {
        assertEquals("🎬…🎞", middleTruncate("🎬🎬🎬🎞🎞🎞", atMost(3)))
    }

    @Test
    fun aNarrowFileNameKeepsItsWholeTextForSemantics() {
        val name = "a_very_long_sequence_name_that_cannot_possibly_fit_ProResMQ.mov"
        compose.setContent { MiddleEllipsisText(name, Modifier.width(80.dp)) }
        compose.onNodeWithText(name).assertExists()
    }
}
