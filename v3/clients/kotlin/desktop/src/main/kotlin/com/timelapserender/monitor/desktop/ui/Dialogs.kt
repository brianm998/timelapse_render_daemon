// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.servers.ServerSource
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.WINDOW_TITLE

/** The dialog of [AppUi.dialog], if any. */
@Composable
fun AppDialogs(ui: AppUi, app: AppState) {
    when (val dialog = ui.dialog) {
        null -> Unit
        AppDialog.Servers -> ServersDialog(ui, app)
        is AppDialog.Pair -> PairServerDialog(dialog, app)
        is AppDialog.PairByAddress -> PairByAddressDialog(dialog, app)
        is AppDialog.Devices -> DevicesDialog(dialog.model, app::dismissDialog)
        is AppDialog.Confirm -> ConfirmDialog(dialog.confirmation, app::confirm, app::dismissDialog)
        AppDialog.About -> AlertDialog(
            onDismissRequest = app::dismissDialog,
            title = { Text(WINDOW_TITLE) },
            text = { Text("Monitors and controls the Timelapse Render Daemon (trd).\nGPL-3.0-or-later") },
            confirmButton = { TextButton(onClick = app::dismissDialog) { Text("Close") } },
        )
    }
}

/** Overview §6.6: paired servers can be connected to or forgotten; discovered ones paired. */
@Composable
private fun ServersDialog(ui: AppUi, app: AppState) {
    AlertDialog(
        onDismissRequest = app::dismissDialog,
        title = { Text("Connect to another computer") },
        text = {
            Column(
                Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (ui.servers.all { it.source == ServerSource.LOCAL }) {
                    Text("No other computers found yet.")
                }
                for (server in ui.servers) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(server.name)
                            Text(
                                "${server.endpoint.host}:${server.endpoint.port}",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        if (server.source == ServerSource.DISCOVERED) {
                            TextButton(onClick = { app.openPairing(server.serverId) }) { Text("Pair…") }
                        } else {
                            TextButton(onClick = { app.selectServer(server.serverId) }) { Text("Connect") }
                        }
                        if (server.source == ServerSource.STORED) {
                            TextButton(onClick = { app.forgetServer(server.serverId) }) { Text("Forget") }
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = app::dismissDialog) { Text("Close") } },
        dismissButton = { TextButton(onClick = app::showAddServer) { Text("Add a server…") } },
    )
}
