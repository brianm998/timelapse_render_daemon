// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.desktop.StartState
import com.timelapserender.monitor.desktop.platform.RELEASES_URL

const val NOT_RUNNING_TEXT = "trd is not running on this computer."

/** Spec 2 §6.1: the local server does not answer. */
@Composable
fun NotRunningScreen(
    start: StartState,
    onStart: () -> Unit,
    onGet: () -> Unit,
    onConnect: () -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(NOT_RUNNING_TEXT, style = MaterialTheme.typography.headlineSmall)
        Button(onClick = onStart, enabled = start !is StartState.Running) { Text("Start trd") }
        OutlinedButton(onClick = onGet) { Text("Get trd") }
        OutlinedButton(onClick = onConnect) { Text("Connect to another computer…") }
        if (start is StartState.Failed) {
            Text("Start trd failed:", color = MaterialTheme.colorScheme.error)
            SelectionContainer { Text(start.output, fontFamily = FontFamily.Monospace) }
            TextButton(onClick = onGet) { Text(RELEASES_URL) }
        }
    }
}
