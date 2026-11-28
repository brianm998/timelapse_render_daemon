// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ChipRole
import com.timelapserender.monitor.core.state.FailureRowUi
import com.timelapserender.monitor.desktop.ui.ActionButtons
import com.timelapserender.monitor.desktop.ui.FAILURE_ACTIONS
import com.timelapserender.monitor.desktop.ui.RowAction

/** One failure of "Needs attention" (overview §6.1). */
@Composable
fun FailureCardRow(
    failure: FailureRowUi,
    selected: Boolean,
    onSelect: () -> Unit,
    onAction: (RowAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = rowColor(selected),
        shape = MaterialTheme.shapes.medium,
        modifier = modifier.fillMaxWidth().selectable(selected, onClick = onSelect),
    ) {
        Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Box(
                Modifier.size(20.dp).background(ERROR_RED, CircleShape)
                    .clearAndSetSemantics { contentDescription = "Failed" },
                contentAlignment = Alignment.Center,
            ) {
                Text("!", color = Color.White, style = MaterialTheme.typography.labelMedium)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Chip(failure.kindLabel, ChipRole.Error)
                    MiddleEllipsisText(failure.outputFilename, Modifier.weight(1f))
                    failure.occurrencesText?.let { Text(it) }
                }
                Text(failure.summary)
                failure.autoPausedText?.let { SecondaryText(it) }
                ActionButtons(FAILURE_ACTIONS, onAction)
            }
        }
    }
}

/** The background of a selectable row. */
@Composable
fun rowColor(selected: Boolean): Color =
    if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceVariant
