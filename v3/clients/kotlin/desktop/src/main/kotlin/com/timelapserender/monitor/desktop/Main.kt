// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application

const val WINDOW_TITLE = "Timelapse Render Monitor"

@Composable
fun StartScreen() {
    MaterialTheme {
        Text("Not connected")
    }
}

fun main() = application {
    Window(onCloseRequest = ::exitApplication, title = WINDOW_TITLE) {
        StartScreen()
    }
}
