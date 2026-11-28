// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.Screen
import com.timelapserender.monitor.desktop.StartScreen

object MainWindowTags {
    const val ACTIVITY = "activity"
    const val DETAILS = "details"
    const val DIVIDER = "divider"
}

/** The main window's content: the start, not-running or connected view, with dialogs and notices. */
@Composable
fun MainWindow(app: AppState) {
    val ui by app.ui.collectAsState()
    MaterialTheme {
        Surface(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize()) {
                when (ui.screen) {
                    Screen.STARTING -> StartScreen()
                    Screen.NOT_RUNNING -> NotRunningScreen(
                        start = ui.start,
                        onStart = app::startTrd,
                        onGet = app::openReleasesPage,
                        onConnect = app::showServers,
                    )
                    Screen.MAIN -> ConnectedView(ui, app)
                }
                ui.notice?.let { notice ->
                    Snackbar(
                        modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                        action = { TextButton(onClick = app::dismissNotice) { Text("Dismiss") } },
                    ) { Text(notice) }
                }
                AppDialogs(ui, app)
            }
        }
    }
}

@Composable
private fun ConnectedView(ui: AppUi, app: AppState) {
    var detailsWidth by remember { mutableStateOf(DEFAULT_DETAILS_WIDTH) }
    val density = LocalDensity.current
    Column(Modifier.fillMaxSize()) {
        TopBar(
            ui,
            TopBarActions(
                selectServer = app::selectServer,
                setGlobalPause = app::setGlobalPause,
                setSearch = app::setSearch,
                setExportPath = app::setExportPath,
            ),
        )
        Row(Modifier.weight(1f)) {
            ActivityPlaceholder(ui, app, Modifier.weight(1f).fillMaxHeight().testTag(MainWindowTags.ACTIVITY))
            if (ui.detailsVisible) {
                VerticalDivider(
                    modifier = Modifier.fillMaxHeight().testTag(MainWindowTags.DIVIDER).draggable(
                        orientation = Orientation.Horizontal,
                        state = rememberDraggableState { pixels ->
                            val moved = with(density) { pixels.toDp() }
                            detailsWidth = (detailsWidth - moved).coerceIn(MIN_DETAILS_WIDTH, MAX_DETAILS_WIDTH)
                        },
                    ),
                    thickness = 4.dp,
                )
                // Nothing can be selected until the Activity list exists, so only the scan summary.
                Column(Modifier.width(detailsWidth).fillMaxHeight().padding(16.dp).testTag(MainWindowTags.DETAILS)) {
                    Text(ui.lastScanText)
                }
            }
        }
    }
}

/** Stands where the Activity list will be. */
@Composable
private fun ActivityPlaceholder(ui: AppUi, app: AppState, modifier: Modifier) {
    Column(
        modifier.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        if (ui.connection is ConnectionState.NeedsPairing) {
            Text("This computer is not paired with the server.")
            Button(onClick = { app.openPairing(ui.activeServerId) }) { Text("Pair…") }
        } else {
            Text("Activity")
        }
    }
}

private val DEFAULT_DETAILS_WIDTH = 320.dp
private val MIN_DETAILS_WIDTH = 220.dp
private val MAX_DETAILS_WIDTH = 520.dp
