// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ConfigEditorUi
import com.timelapserender.trd.v1.CodecFfmpeg
import com.timelapserender.trd.v1.CodecFilename
import com.timelapserender.trd.v1.VideoCodec

object CodecTags {
    const val NEW_NAME = "codec-new-name"
    const val ADD = "codec-add"
    const val MESSAGE = "codec-message"

    fun name(codec: String) = "codec-name-$codec"

    fun rename(codec: String) = "codec-rename-$codec"

    fun remove(codec: String) = "codec-remove-$codec"

    /** A field of a codec, such as `ffmpeg.pixel_format`. */
    fun field(codec: String, field: String) = "codec-field-$codec-$field"

    fun addArgument(codec: String) = "codec-add-arg-$codec"
}

/** The `video_codecs.json` table (overview §6.4 Codecs). */
@Composable
internal fun CodecsGroup(controller: ConfigController, editor: ConfigEditorUi, window: ConfigWindowUi) {
    window.codecMessage?.let {
        Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag(CodecTags.MESSAGE))
    }
    for (codec in editor.codecs) {
        key(codec.name) {
            CodecCard(controller, codec, only = editor.codecs.size == 1, checking = window.checkingCodec)
        }
    }
    var newName by remember { mutableStateOf("") }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(
            value = newName,
            onValueChange = { newName = it },
            label = { Text("New codec name") },
            singleLine = true,
            modifier = Modifier.testTag(CodecTags.NEW_NAME),
        )
        TextButton(
            onClick = {
                controller.addCodec(newName)
                if (controller.ui.value.codecMessage == null) newName = ""
            },
            enabled = newName.isNotBlank(),
            modifier = Modifier.testTag(CodecTags.ADD),
        ) { Text("Add codec") }
    }
}

@Composable
private fun CodecCard(controller: ConfigController, codec: VideoCodec, only: Boolean, checking: Boolean) {
    val name = codec.name
    var typedName by remember { mutableStateOf(name) }
    val ffmpeg = codec.ffmpeg
    val filename = codec.filename
    fun setFfmpeg(change: (CodecFfmpeg.Builder) -> Unit) =
        controller.editCodec(name) { it.ffmpeg = it.ffmpeg.toBuilder().also(change).build() }
    fun setFilename(change: (CodecFilename.Builder) -> Unit) =
        controller.editCodec(name) { it.filename = it.filename.toBuilder().also(change).build() }

    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = typedName,
                    onValueChange = { typedName = it },
                    label = { Text("Name") },
                    singleLine = true,
                    modifier = Modifier.weight(1f).testTag(CodecTags.name(name)),
                )
                TextButton(
                    onClick = { controller.renameCodec(name, typedName) },
                    enabled = !checking && typedName.isNotBlank() && typedName.trim() != name,
                    modifier = Modifier.testTag(CodecTags.rename(name)),
                ) { Text("Rename") }
                TextButton(
                    onClick = { controller.removeCodec(name) },
                    // The table keeps at least one codec.
                    enabled = !checking && !only,
                    modifier = Modifier.testTag(CodecTags.remove(name)),
                ) { Text("Remove") }
            }
            Text("ffmpeg", style = MaterialTheme.typography.titleSmall)
            CodecField(name, "ffmpeg.codec", "Codec", ffmpeg.codec) { v -> setFfmpeg { it.codec = v } }
            CodecField(name, "ffmpeg.pixel_format", "Pixel format", ffmpeg.pixelFormat) { v ->
                setFfmpeg { it.pixelFormat = v }
            }
            CodecField(name, "ffmpeg.color_range", "Color range", ffmpeg.colorRange) { v ->
                setFfmpeg { it.colorRange = v }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = ffmpeg.hasProfile,
                    onCheckedChange = { v ->
                        setFfmpeg {
                            it.hasProfile = v
                            if (!v) it.clearProfile()
                        }
                    },
                    modifier = Modifier.testTag(CodecTags.field(name, "ffmpeg.has_profile")),
                )
                Text("Profile")
            }
            if (ffmpeg.hasProfile) {
                CodecField(name, "ffmpeg.profile", "Profile", ffmpeg.profile) { v -> setFfmpeg { it.profile = v } }
            }
            Text("Extra arguments", style = MaterialTheme.typography.bodyMedium)
            ffmpeg.extraArgsList.forEachIndexed { i, argument ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = argument,
                        onValueChange = { v -> setFfmpeg { if (i < it.extraArgsCount) it.setExtraArgs(i, v) } },
                        singleLine = true,
                        modifier = Modifier.weight(1f).testTag(CodecTags.field(name, "ffmpeg.extra_args.$i")),
                    )
                    OrderButtons(
                        "/video_codecs/$name/extra_args/$i", i, ffmpeg.extraArgsCount,
                        move = { to ->
                            setFfmpeg {
                                val next = swapped(it.extraArgsList.toList(), i, to)
                                it.clearExtraArgs().addAllExtraArgs(next)
                            }
                        },
                        remove = {
                            setFfmpeg {
                                val next = it.extraArgsList.filterIndexed { j, _ -> j != i }
                                it.clearExtraArgs().addAllExtraArgs(next)
                            }
                        },
                    )
                }
            }
            TextButton(
                onClick = { setFfmpeg { it.addExtraArgs("") } },
                modifier = Modifier.testTag(CodecTags.addArgument(name)),
            ) { Text("Add argument") }
            Text("File name", style = MaterialTheme.typography.titleSmall)
            CodecField(name, "filename.pixel_format", "Pixel format", filename.pixelFormat) { v ->
                setFilename { it.pixelFormat = v }
            }
            CodecField(name, "filename.codec", "Codec", filename.codec) { v -> setFilename { it.codec = v } }
            CodecField(name, "filename.quality", "Quality", filename.quality) { v -> setFilename { it.quality = v } }
        }
    }
}

@Composable
private fun CodecField(codec: String, field: String, label: String, value: String, set: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = set,
        label = { Text(label) },
        singleLine = true,
        modifier = Modifier.fillMaxWidth().testTag(CodecTags.field(codec, field)),
    )
}
