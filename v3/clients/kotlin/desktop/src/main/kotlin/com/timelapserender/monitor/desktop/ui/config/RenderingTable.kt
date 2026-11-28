// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ConfigEditorUi
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.RenderVariant

internal val RENDERING_FIELDS = listOf("codec", "frame_rate", "resolution")

private const val DEFAULT_FRAME_RATE = 30
private const val DEFAULT_WIDTH = 3840
private const val DEFAULT_HEIGHT = 2160

/** How a variant's `resolution` list is read: [size] is its number of entries. */
enum class ResolutionMode(val label: String, val size: Int) {
    ORIGINAL("Original", 0),
    WIDTH("Width", 1),
    EXACT("Width × Height", 2),
    ;

    companion object {
        fun of(resolution: List<Int>): ResolutionMode = entries.firstOrNull { it.size == resolution.size } ?: EXACT
    }
}

/** The `resolution` list for [mode], keeping the numbers [current] already has. */
fun resolutionFor(mode: ResolutionMode, current: List<Int>): List<Int> =
    List(mode.size) { current.getOrNull(it) ?: if (it == 0) DEFAULT_WIDTH else DEFAULT_HEIGHT }

/** The server's computed row (variant key, example file name) for the variant [row], if it has one yet. */
fun computedRow(row: RenderVariant, computed: List<RenderVariant>): RenderVariant? = computed.firstOrNull {
    it.codec == row.codec && it.frameRate == row.frameRate && it.resolutionList == row.resolutionList
}

object RenderingTags {
    const val ADD = "rendering-add"

    fun codec(row: Int) = "rendering-codec-$row"

    fun mode(row: Int) = "rendering-mode-$row"

    /** Entry [index] of the open menu of the picker tagged [picker]. */
    fun choice(picker: String, index: Int) = "$picker-choice-$index"

    fun computed(row: Int) = "rendering-computed-$row"
}

private fun ConfigController.editRow(index: Int, change: (RenderVariant.Builder) -> Unit) = edit { b ->
    if (index < b.renderingCount) b.setRendering(index, b.getRendering(index).toBuilder().also(change).build())
}

private fun RenderVariant.Builder.replaceResolution(values: List<Int>) {
    clearResolution().addAllResolution(values)
}

/** The variants table (overview §6.4 Rendering), edited inline. */
@Composable
internal fun RenderingGroup(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    preview: List<RenderVariant>,
) {
    val computed = editor.rendering.ifEmpty { preview }
    IssueLines(editor, "/rendering")
    config.renderingList.forEachIndexed { i, row ->
        VariantEditor(controller, editor, config, i, row, computedRow(row, computed))
        HorizontalDivider()
    }
    TextButton(
        onClick = {
            val codec = editor.codecs.firstOrNull()?.name.orEmpty()
            controller.edit {
                it.addRendering(RenderVariant.newBuilder().setCodec(codec).setFrameRate(DEFAULT_FRAME_RATE))
            }
        },
        modifier = Modifier.testTag(RenderingTags.ADD),
    ) { Text("Add variant") }
}

@Composable
private fun VariantEditor(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    index: Int,
    row: RenderVariant,
    computed: RenderVariant?,
) {
    val field = "/rendering/$index"
    val mode = ResolutionMode.of(row.resolutionList)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Column(Modifier.weight(1f)) {
                Picker(
                    RenderingTags.codec(index),
                    row.codec.ifEmpty { "Choose a codec" },
                    editor.codecs.map { it.name },
                ) { picked -> controller.editRow(index) { it.codec = editor.codecs[picked].name } }
                IssueLines(editor, "$field/codec")
            }
            CountSetting(editor, "Frame rate", "$field/frame_rate", row.frameRate, Modifier.width(110.dp)) { v ->
                controller.editRow(index) { it.frameRate = v }
            }
            Column {
                Picker(RenderingTags.mode(index), mode.label, ResolutionMode.entries.map { it.label }) { picked ->
                    controller.editRow(index) {
                        it.replaceResolution(resolutionFor(ResolutionMode.entries[picked], it.resolutionList))
                    }
                }
                IssueLines(editor, "$field/resolution")
            }
            for ((j, label) in listOf("Width", "Height").withIndex()) {
                if (j < row.resolutionCount) {
                    CountSetting(
                        editor, label, "$field/resolution/$j", row.getResolution(j), Modifier.width(100.dp),
                    ) { v -> controller.editRow(index) { if (j < it.resolutionCount) it.setResolution(j, v) } }
                }
            }
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).testTag(RenderingTags.computed(index))) {
                if (computed != null) {
                    Text(computed.variantKey, style = MaterialTheme.typography.bodyMedium)
                    Text(computed.exampleFilename, style = MaterialTheme.typography.bodySmall)
                }
            }
            OrderButtons(
                field, index, config.renderingCount,
                move = { to ->
                    controller.edit {
                        val rows = swapped(it.renderingList.toList(), index, to)
                        it.clearRendering().addAllRendering(rows)
                    }
                },
                remove = { controller.edit { if (index < it.renderingCount) it.removeRendering(index) } },
            )
        }
        IssueLines(editor, field)
    }
}

/** A button showing [text] that opens a menu of [choices]; [pick] gets the chosen index. */
@Composable
private fun Picker(tag: String, text: String, choices: List<String>, pick: (Int) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, modifier = Modifier.testTag(tag)) { Text("$text ▾") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            choices.forEachIndexed { i, choice ->
                DropdownMenuItem(
                    text = { Text(choice) },
                    modifier = Modifier.testTag(RenderingTags.choice(tag, i)),
                    onClick = {
                        open = false
                        pick(i)
                    },
                )
            }
        }
    }
}
