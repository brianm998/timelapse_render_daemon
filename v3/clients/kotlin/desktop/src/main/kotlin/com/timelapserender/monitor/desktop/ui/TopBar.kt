// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.servers.RegisteredServer
import com.timelapserender.monitor.core.servers.ServerSource
import com.timelapserender.monitor.core.state.ChipRole
import com.timelapserender.monitor.desktop.AppUi

const val ALL_EXPORT_PATHS = "All export paths"

object TopBarTags {
    const val SERVER = "server-switcher"
    const val CONNECTION = "connection"
    const val PHASE = "phase"
    const val COUNTS = "counts"
    const val PAUSE = "pause-all"
    const val SEARCH = "search"
    const val EXPORT_PATH = "export-path"
}

data class TopBarActions(
    val selectServer: (String) -> Unit,
    val setGlobalPause: (Boolean) -> Unit,
    val setSearch: (String) -> Unit,
    val setExportPath: (String?) -> Unit,
)

/** The fixed header of spec 2 §6.2. */
@Composable
fun TopBar(ui: AppUi, actions: TopBarActions) {
    val header = ui.activity?.header
    Surface(tonalElevation = 2.dp) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                ServerSwitcher(ui, actions.selectServer)
                if (header != null) {
                    Text(
                        header.connectionText,
                        Modifier.weight(1f, fill = false).testTag(TopBarTags.CONNECTION),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Chip(header.phaseText, header.phaseRole, Modifier.testTag(TopBarTags.PHASE))
                    if (header.sandbox) Chip("SANDBOX", ChipRole.Warning)
                    Text(header.countsLine, Modifier.testTag(TopBarTags.COUNTS), maxLines = 1)
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Pause all rendering")
                Switch(
                    checked = header?.globalPause == true,
                    onCheckedChange = actions.setGlobalPause,
                    enabled = ui.connected,
                    modifier = Modifier.testTag(TopBarTags.PAUSE),
                )
                SearchField(ui, actions.setSearch, Modifier.weight(1f))
                ExportPathFilter(ui, actions.setExportPath)
            }
        }
    }
}

@Composable
private fun ServerSwitcher(ui: AppUi, onSelect: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    fun pick(id: String) {
        open = false
        onSelect(id)
    }
    // The connected server's own name when known; the local entry is otherwise "This computer".
    val name = ui.activity?.header?.serverName?.takeIf { it.isNotEmpty() } ?: ui.activeServer?.name ?: "No server"
    Box {
        OutlinedButton(onClick = { open = true }, modifier = Modifier.testTag(TopBarTags.SERVER)) {
            Text("$name ▾", maxLines = 1)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            // The registry lists the local server first, then paired ones, then discovered ones.
            val (discovered, known) = ui.servers.partition { it.source == ServerSource.DISCOVERED }
            for (server in known) {
                DropdownMenuItem(text = { Text(serverLabel(server, ui)) }, onClick = { pick(server.serverId) })
            }
            if (discovered.isNotEmpty()) {
                Text(
                    "Discovered",
                    Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                )
                for (server in discovered) {
                    DropdownMenuItem(text = { Text("${server.name} — Pair…") }, onClick = { pick(server.serverId) })
                }
            }
        }
    }
}

private fun serverLabel(server: RegisteredServer, ui: AppUi) =
    if (server.serverId == ui.activeServerId) "✓ ${server.name}" else server.name

@Composable
private fun SearchField(ui: AppUi, onSearch: (String) -> Unit, modifier: Modifier) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(ui.searchFocusRequests) {
        if (ui.searchFocusRequests > 0) focus.requestFocus()
    }
    OutlinedTextField(
        value = ui.filter.search,
        onValueChange = onSearch,
        singleLine = true,
        placeholder = { Text("Search") },
        modifier = modifier.focusRequester(focus).testTag(TopBarTags.SEARCH),
    )
}

@Composable
private fun ExportPathFilter(ui: AppUi, onSelect: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    fun pick(path: String?) {
        open = false
        onSelect(path)
    }
    Box {
        OutlinedButton(
            onClick = { open = true },
            modifier = Modifier.widthIn(max = 280.dp).testTag(TopBarTags.EXPORT_PATH),
        ) {
            Text("${ui.filter.exportPath ?: ALL_EXPORT_PATHS} ▾", maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Text(ALL_EXPORT_PATHS) }, onClick = { pick(null) })
            for (path in ui.exportPaths) {
                DropdownMenuItem(text = { Text(path) }, onClick = { pick(path) })
            }
        }
    }
}

@Composable
private fun Chip(text: String, role: ChipRole, modifier: Modifier = Modifier) {
    val color = when (role) {
        ChipRole.Neutral -> Color(0xFF616161)
        ChipRole.Active -> Color(0xFF1565C0)
        ChipRole.Ok -> Color(0xFF2E7D32)
        ChipRole.Warning -> Color(0xFF8D5A00)
        ChipRole.Error -> Color(0xFFC62828)
    }
    Surface(color = color, contentColor = Color.White, shape = MaterialTheme.shapes.small, modifier = modifier) {
        Text(text, Modifier.padding(horizontal = 8.dp, vertical = 2.dp), style = MaterialTheme.typography.labelMedium)
    }
}
