// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

const val DONT_ASK_AGAIN = "Don't ask again"
const val CONFIRM_BUTTON_TAG = "confirm"
const val DONT_ASK_AGAIN_TAG = "dont-ask-again"

/**
 * A control that is confirmed first (spec 2 §6.4). [dontAskAgainByDefault] is how the checkbox
 * starts: Pause all rendering asks the first time only, so its box starts ticked.
 */
sealed interface Confirmation {
    val title: String
    val text: String
    val confirmLabel: String
    val dontAskAgainByDefault: Boolean

    data class StopRender(val renderId: String) : Confirmation {
        override val title get() = "Stop render"
        override val text
            get() = "Stop this render? ffmpeg is stopped, the partial file deleted and the variant paused."
        override val confirmLabel get() = "Stop"
        override val dontAskAgainByDefault get() = false
    }

    data object PauseAll : Confirmation {
        override val title = "Pause all rendering"
        override val text =
            "Pause all rendering? Running renders are suspended and no queued render starts until you resume."
        override val confirmLabel = "Pause"
        override val dontAskAgainByDefault = true
    }
}

/** [onConfirm] is told whether "Don't ask again" is ticked; cancelling remembers nothing. */
@Composable
fun ConfirmDialog(confirmation: Confirmation, onConfirm: (Boolean) -> Unit, onCancel: () -> Unit) {
    var dontAskAgain by remember(confirmation) { mutableStateOf(confirmation.dontAskAgainByDefault) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(confirmation.title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(confirmation.text)
                Row(
                    Modifier.toggleable(dontAskAgain, role = Role.Checkbox) { dontAskAgain = it }
                        .testTag(DONT_ASK_AGAIN_TAG),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Checkbox(checked = dontAskAgain, onCheckedChange = null)
                    Text(DONT_ASK_AGAIN)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(dontAskAgain) }, modifier = Modifier.testTag(CONFIRM_BUTTON_TAG)) {
                Text(confirmation.confirmLabel)
            }
        },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } },
    )
}
