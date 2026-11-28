// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ConfigEditorUi
import com.timelapserender.trd.v1.Config

/** One list of strings in the config; [key] is its JSON key. */
class ConfigList(
    val key: String,
    val title: String,
    val get: (Config) -> List<String>,
    val set: (Config.Builder, List<String>) -> Unit,
)

internal val LOCATION_LISTS = listOf(
    ConfigList("source_paths", "Source paths", { it.sourcePathsList }) { b, v ->
        b.clearSourcePaths().addAllSourcePaths(v)
    },
    ConfigList("backup_paths", "Backup paths", { it.backupPathsList }) { b, v ->
        b.clearBackupPaths().addAllBackupPaths(v)
    },
    ConfigList("export_paths", "Export paths", { it.exportPathsList }) { b, v ->
        b.clearExportPaths().addAllExportPaths(v)
    },
    ConfigList("master_videos", "Master videos", { it.masterVideosList }) { b, v ->
        b.clearMasterVideos().addAllMasterVideos(v)
    },
)

internal val MATCHING_LISTS = listOf(
    ConfigList("raw_types", "Raw types", { it.rawTypesList }) { b, v -> b.clearRawTypes().addAllRawTypes(v) },
    ConfigList("image_extensions", "Image extensions", { it.imageExtensionsList }) { b, v ->
        b.clearImageExtensions().addAllImageExtensions(v)
    },
    ConfigList("ignore_sequence_regexes", "Ignore sequences matching", { it.ignoreSequenceRegexesList }) { b, v ->
        b.clearIgnoreSequenceRegexes().addAllIgnoreSequenceRegexes(v)
    },
)

internal val DELETE_REGEXES = ConfigList(
    "delete_sequence_after_render_regexes",
    "Delete sequences after rendering when they match",
    { it.deleteSequenceAfterRenderRegexesList },
) { b, v -> b.clearDeleteSequenceAfterRenderRegexes().addAllDeleteSequenceAfterRenderRegexes(v) }

/** Test tags of a list's controls; an entry is named by its JSON pointer, such as `/raw_types/0`. */
object ListTags {
    fun add(key: String) = "list-add-$key"

    fun up(field: String) = "list-up-$field"

    fun down(field: String) = "list-down-$field"

    fun remove(field: String) = "list-remove-$field"

    fun browse(field: String) = "list-browse-$field"
}

/** [list] with the entries at [a] and [b] swapped; unchanged when either is outside it. */
internal fun <T> swapped(list: List<T>, a: Int, b: Int): List<T> =
    if (a !in list.indices || b !in list.indices) {
        list
    } else {
        list.toMutableList().also {
            it[a] = list[b]
            it[b] = list[a]
        }
    }

private fun ConfigController.editList(list: ConfigList, change: (List<String>) -> List<String>) =
    edit { builder -> list.set(builder, change(list.get(builder.build()))) }

/** A list of typed entries with add, remove and reorder. */
@Composable
internal fun StringListEditor(controller: ConfigController, editor: ConfigEditorUi, config: Config, list: ConfigList) {
    ListEditor(controller, editor, config, list, addLabel = "Add", add = { controller.editList(list) { it + "" } })
}

/** A list of folders on the server: added with the folder browser, and editable for `$YEAR`. */
@Composable
internal fun PathListEditor(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    list: ConfigList,
    browse: (BrowseRequest) -> Unit,
) {
    ListEditor(
        controller, editor, config, list,
        addLabel = "Add folder…",
        add = { browse(BrowseRequest("") { picked -> controller.editList(list) { it + picked } }) },
        browseEntry = { i, current ->
            browse(
                BrowseRequest(current) { picked ->
                    controller.editList(list) { paths -> paths.mapIndexed { j, p -> if (i == j) picked else p } }
                },
            )
        },
    )
}

@Composable
private fun ListEditor(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    list: ConfigList,
    addLabel: String,
    add: () -> Unit,
    browseEntry: ((Int, String) -> Unit)? = null,
) {
    val entries = list.get(config)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(list.title, style = MaterialTheme.typography.titleMedium)
        IssueLines(editor, "/${list.key}")
        entries.forEachIndexed { i, entry ->
            val field = "/${list.key}/$i"
            Column {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = entry,
                        onValueChange = { typed ->
                            controller.editList(list) { all -> all.mapIndexed { j, v -> if (i == j) typed else v } }
                        },
                        singleLine = true,
                        isError = hasError(editor, field),
                        modifier = Modifier.weight(1f).testTag(ConfigTags.field(field)),
                    )
                    if (browseEntry != null) {
                        TextButton(
                            onClick = { browseEntry(i, entry) },
                            modifier = Modifier.testTag(ListTags.browse(field)),
                        ) { Text("Browse…") }
                    }
                    OrderButtons(
                        field, i, entries.size,
                        move = { to -> controller.editList(list) { swapped(it, i, to) } },
                        remove = { controller.editList(list) { all -> all.filterIndexed { j, _ -> j != i } } },
                    )
                }
                IssueLines(editor, field)
            }
        }
        TextButton(onClick = add, modifier = Modifier.testTag(ListTags.add(list.key))) { Text(addLabel) }
    }
}

/** Move up, move down and remove, for entry [index] of [size]. */
@Composable
internal fun OrderButtons(field: String, index: Int, size: Int, move: (Int) -> Unit, remove: () -> Unit) {
    TextButton(
        onClick = { move(index - 1) },
        enabled = index > 0,
        modifier = Modifier.testTag(ListTags.up(field)),
    ) { Text("↑") }
    TextButton(
        onClick = { move(index + 1) },
        enabled = index < size - 1,
        modifier = Modifier.testTag(ListTags.down(field)),
    ) { Text("↓") }
    TextButton(onClick = remove, modifier = Modifier.testTag(ListTags.remove(field))) { Text("Remove") }
}
