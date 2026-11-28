// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ChipRole
import com.timelapserender.monitor.core.state.SequenceCardUi
import com.timelapserender.monitor.core.state.VariantRowUi
import com.timelapserender.monitor.desktop.ui.ActionMenu
import com.timelapserender.monitor.desktop.ui.IconActionButtons
import com.timelapserender.monitor.desktop.ui.RowAction
import com.timelapserender.monitor.desktop.ui.sequenceActions
import com.timelapserender.monitor.desktop.ui.variantActions
import com.timelapserender.trd.v1.VariantStatus

/** The head of an in-progress card (overview §6.1, lines 1–3). Its variants are rows of their own. */
@Composable
fun SequenceCardRow(
    card: SequenceCardUi,
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
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(
                    Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        card.name,
                        Modifier.weight(1f, fill = false),
                        fontFamily = FontFamily.Monospace,
                        fontWeight = FontWeight.Bold,
                    )
                    Chip(card.stageText, card.stageRole)
                    if (card.paused) Chip("paused", ChipRole.Warning)
                }
                ActionMenu(sequenceActions(card.paused), onAction)
            }
            LinearProgressIndicator(progress = { card.progress }, modifier = Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(card.framesText)
                card.exportRateText?.let { Text(it) }
                card.exportSizeText?.let { Text(it) }
            }
            SecondaryText(card.secondaryText)
            if (card.note.isNotEmpty()) SecondaryText(card.note)
        }
    }
}

/** One variant of a complete sequence: status icon, label, progress or result. */
@Composable
fun VariantRowView(
    variant: VariantRowUi,
    selected: Boolean,
    onSelect: () -> Unit,
    onAction: (RowAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = rowColor(selected),
        shape = MaterialTheme.shapes.small,
        modifier = modifier.fillMaxWidth().padding(start = 24.dp).selectable(selected, onClick = onSelect),
    ) {
        Column(Modifier.padding(horizontal = 12.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    statusGlyph(variant.status),
                    Modifier.width(20.dp).clearAndSetSemantics { contentDescription = "Status: ${variant.statusText}" },
                    color = roleColor(variant.statusRole),
                    fontWeight = FontWeight.Bold,
                )
                Text(variant.label, Modifier.weight(1f))
                variantStatusLabel(variant)
                variant.sizeText?.let { Text(it) }
                IconActionButtons(variantActions(variant.status), onAction)
            }
            val progress = variant.progress
            if (progress != null) {
                LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
            } else if (variant.indeterminate) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            if (variant.progressText.isNotEmpty()) SecondaryText(variant.progressText)
            if (variant.message.isNotEmpty()) Text(variant.message, color = ERROR_RED)
        }
    }
}

/** An unknown codec is an error chip; a finished render needs no word beside its check mark. */
@Composable
private fun variantStatusLabel(variant: VariantRowUi) {
    when (variant.status) {
        VariantStatus.VARIANT_UNKNOWN_CODEC -> Chip(variant.statusText, variant.statusRole)
        VariantStatus.VARIANT_DONE -> Unit
        else -> Text(variant.statusText, color = roleColor(variant.statusRole))
    }
}

private fun statusGlyph(status: VariantStatus): String = when (status) {
    VariantStatus.VARIANT_DONE, VariantStatus.VARIANT_ALREADY_RENDERED -> "✓"
    VariantStatus.VARIANT_RUNNING -> "▶"
    VariantStatus.VARIANT_SUSPENDED, VariantStatus.VARIANT_PAUSED -> "‖"
    VariantStatus.VARIANT_FAILED, VariantStatus.VARIANT_UNKNOWN_CODEC -> "✕"
    else -> "…"
}
