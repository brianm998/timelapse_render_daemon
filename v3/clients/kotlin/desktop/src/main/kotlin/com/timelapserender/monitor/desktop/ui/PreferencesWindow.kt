// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.rememberWindowState
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.SNAPSHOT_RATES

object PreferencesTags {
    const val NOTIFY_FINISHED = "prefs.notifyFinished"
    const val NOTIFY_FAILED = "prefs.notifyFailed"
    const val KEEP_IN_TRAY = "prefs.keepInTray"
    const val RATE = "prefs.rate"
    const val RATE_DOWN = "prefs.rateDown"
    const val RATE_UP = "prefs.rateUp"
    const val RESET = "prefs.reset"
}

/** The preferences of spec 2 §6.8; every change is stored at once. */
@Composable
fun PreferencesContent(app: AppState) {
    val ui by app.ui.collectAsState()
    val prefs = ui.preferences
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SwitchRow(
            "Notify when a sequence has finished rendering",
            prefs.notifyFinished,
            PreferencesTags.NOTIFY_FINISHED,
            app::setNotifyFinished,
        )
        SwitchRow(
            "Notify when a render fails",
            prefs.notifyFailed,
            PreferencesTags.NOTIFY_FAILED,
            app::setNotifyFailed,
        )
        SwitchRow(
            "Keep running in the tray",
            prefs.keepRunningInTray,
            PreferencesTags.KEEP_IN_TRAY,
            app::setKeepRunningInTray,
        )
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Snapshots per second", Modifier.weight(1f))
            OutlinedButton(
                onClick = { app.setSnapshotRate(prefs.snapshotRate - 1) },
                enabled = prefs.snapshotRate > SNAPSHOT_RATES.first,
                modifier = Modifier.testTag(PreferencesTags.RATE_DOWN),
            ) { Text("−") }
            Text(prefs.snapshotRate.toString(), Modifier.testTag(PreferencesTags.RATE))
            OutlinedButton(
                onClick = { app.setSnapshotRate(prefs.snapshotRate + 1) },
                enabled = prefs.snapshotRate < SNAPSHOT_RATES.last,
                modifier = Modifier.testTag(PreferencesTags.RATE_UP),
            ) { Text("+") }
        }
        OutlinedButton(
            onClick = app::resetConfirmations,
            enabled = prefs.confirmationsSkipped,
            modifier = Modifier.testTag(PreferencesTags.RESET),
        ) { Text("Reset “Don't ask again” choices") }
    }
}

@Composable
private fun SwitchRow(label: String, checked: Boolean, tag: String, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked, onChange, Modifier.testTag(tag))
    }
}

@Composable
fun PreferencesWindow(app: AppState) {
    Window(
        onCloseRequest = app::closePreferences,
        title = "Preferences",
        state = rememberWindowState(size = DpSize(480.dp, 340.dp)),
    ) {
        MaterialTheme {
            Surface { PreferencesContent(app) }
        }
    }
}
