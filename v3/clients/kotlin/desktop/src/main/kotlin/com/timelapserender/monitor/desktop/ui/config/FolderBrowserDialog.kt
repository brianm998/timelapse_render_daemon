// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.DirectoryUi
import kotlinx.coroutines.flow.first

/**
 * A request for a path on the server, browsing from [current] (the roots when it is empty).
 * [pickFile] asks for a file instead of a folder.
 */
class BrowseRequest(val current: String, val pickFile: Boolean = false, val pick: (String) -> Unit)

/**
 * The folder to open for the value [current]: the folder itself, the part before `$YEAR`, or for a
 * file its folder. "" means the roots.
 */
fun browseStart(current: String, pickFile: Boolean): String {
    val year = current.indexOf("\$YEAR")
    if (year < 0 && !pickFile) return current
    val head = if (year < 0) current else current.substring(0, year)
    val cut = maxOf(head.lastIndexOf('/'), head.lastIndexOf('\\'))
    if (cut < 0) return ""
    val folder = head.substring(0, cut)
    // A root keeps its separator: "/" and "C:\".
    return if (folder.isEmpty() || folder.endsWith(':')) head.substring(0, cut + 1) else folder
}

/** What Choose would pick, if anything: a folder, or with [pickFile] a selected file. */
fun chosenPath(ui: DirectoryUi, pickFile: Boolean): String? {
    if (!pickFile) return ui.selection
    return ui.entries.firstOrNull { it.path == ui.selected && !it.isDir }?.path
}

object BrowserTags {
    const val UP = "browser-up"
    const val CHOOSE = "browser-choose"
    const val CANCEL = "browser-cancel"
    const val PATH = "browser-path"

    fun entry(path: String) = "browser-entry-$path"

    fun open(path: String) = "browser-open-$path"
}

/** The server's folders (`ListDirectory`): the roots, a folder's entries, Up and Choose. */
@Composable
fun FolderBrowserDialog(controller: ConfigController, request: BrowseRequest, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val model = remember(request) { controller.directoryBrowser(scope, request.pickFile) }
    val ui by model.state.collectAsState()
    LaunchedEffect(model) {
        val start = browseStart(request.current, request.pickFile)
        model.navigate(start)
        // A value that is no folder on the server starts at the roots.
        if (start.isNotEmpty()) {
            val first = model.state.first { !it.loading }
            if (first.error != null) model.openRoots()
        }
    }
    val chosen = chosenPath(ui, request.pickFile)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (request.pickFile) "Choose a file" else "Choose a folder") },
        text = {
            Column(Modifier.width(480.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextButton(
                        onClick = model::up,
                        enabled = !ui.atRoots,
                        modifier = Modifier.testTag(BrowserTags.UP),
                    ) { Text("Up") }
                    Text(if (ui.atRoots) "Roots" else ui.path, Modifier.testTag(BrowserTags.PATH))
                }
                ui.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                if (ui.loading) Text("Loading…")
                LazyColumn(Modifier.fillMaxWidth().height(300.dp)) {
                    if (ui.atRoots) {
                        items(ui.roots) { root ->
                            EntryRow(root, root, true, ui.selected == root, { model.select(root) }) {
                                model.navigate(root)
                            }
                        }
                    } else {
                        items(ui.entries) { e ->
                            EntryRow(e.name, e.path, e.isDir, ui.selected == e.path, { model.select(e.path) }) {
                                model.navigate(e.path)
                            }
                        }
                    }
                }
                Text(chosen.orEmpty(), style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    if (chosen != null) {
                        request.pick(chosen)
                        onDismiss()
                    }
                },
                enabled = chosen != null && !ui.loading,
                modifier = Modifier.testTag(BrowserTags.CHOOSE),
            ) { Text("Choose") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, modifier = Modifier.testTag(BrowserTags.CANCEL)) { Text("Cancel") }
        },
    )
}

@Composable
private fun EntryRow(
    name: String,
    path: String,
    folder: Boolean,
    selected: Boolean,
    select: () -> Unit,
    open: () -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            name,
            Modifier.weight(1f)
                .selectable(selected = selected, onClick = select)
                .testTag(BrowserTags.entry(path))
                .padding(vertical = 8.dp),
            color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
        )
        if (folder) {
            TextButton(onClick = open, modifier = Modifier.testTag(BrowserTags.open(path))) { Text("Open") }
        }
    }
}
