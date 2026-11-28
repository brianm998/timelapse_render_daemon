// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.text
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ChipRole

val ERROR_RED = Color(0xFFC62828)
val WARNING_AMBER = Color(0xFF8D5A00)

fun roleColor(role: ChipRole): Color = when (role) {
    ChipRole.Neutral -> Color(0xFF616161)
    ChipRole.Active -> Color(0xFF1565C0)
    ChipRole.Ok -> Color(0xFF2E7D32)
    ChipRole.Warning -> WARNING_AMBER
    ChipRole.Error -> ERROR_RED
}

/** A small coloured label. */
@Composable
fun Chip(text: String, role: ChipRole, modifier: Modifier = Modifier) {
    Surface(color = roleColor(role), contentColor = Color.White, shape = MaterialTheme.shapes.small, modifier = modifier) {
        Text(text, Modifier.padding(horizontal = 8.dp, vertical = 2.dp), style = MaterialTheme.typography.labelMedium)
    }
}

/** Secondary text: smaller and muted. */
@Composable
fun SecondaryText(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        style = MaterialTheme.typography.bodySmall,
    )
}

/**
 * [text] itself when [fits] accepts it, else its longest head and tail around `…` that fit, so a file
 * name keeps its extension. Never splits a surrogate pair.
 */
fun middleTruncate(text: String, fits: (String) -> Boolean): String {
    if (fits(text)) return text
    val points = text.codePoints().toArray()
    fun shortened(keep: Int): String {
        val tail = keep / 2
        return String(points, 0, keep - tail) + "…" + String(points, points.size - tail, tail)
    }
    var low = 0
    var high = points.size - 1
    while (low < high) {
        val mid = (low + high + 1) / 2
        if (fits(shortened(mid))) low = mid else high = mid - 1
    }
    return shortened(maxOf(low, 0))
}

/** One line truncated in the middle to the width it gets; its semantics keep the whole [text]. */
@Composable
fun MiddleEllipsisText(text: String, modifier: Modifier = Modifier, style: TextStyle = LocalTextStyle.current) {
    val measurer = rememberTextMeasurer()
    BoxWithConstraints(modifier) {
        val width = constraints.maxWidth
        val shown = remember(text, width, style, measurer) {
            if (width == Constraints.Infinity || text.isEmpty()) {
                text
            } else {
                middleTruncate(text) { measurer.measure(it, style, softWrap = false, maxLines = 1).size.width <= width }
            }
        }
        Text(
            shown,
            Modifier.clearAndSetSemantics { this.text = AnnotatedString(text) },
            style = style,
            softWrap = false,
            maxLines = 1,
        )
    }
}
