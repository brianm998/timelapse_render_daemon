// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.TooltipArea
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.desktop.platform.FileOpener
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.withTimeoutOrNull

/** Every control of a row (spec 2 §6.4). */
enum class RowAction(val label: String) {
    PAUSE_SEQUENCE("Pause sequence"),
    RESUME_SEQUENCE("Resume sequence"),
    SUSPEND("Suspend"),
    RESUME_RENDER("Resume"),
    STOP("Stop"),
    PAUSE_VARIANT("Pause variant"),
    RESUME_VARIANT("Resume variant"),
    RETRY("Retry"),
    DISMISS("Dismiss"),
    OPEN("Open"),
    REVEAL("Reveal"),
    COPY_PATH("Copy path"),
    DETAILS("Details"),
}

/** What an action acts on. [key] is the row's Activity list key, which Details selects. */
sealed interface ActionTarget {
    val key: String

    data class Sequence(override val key: String, val sequenceName: String) : ActionTarget

    data class Variant(
        override val key: String,
        val sequenceName: String,
        val variantKey: String,
        val renderId: String,
    ) : ActionTarget

    data class Failure(
        override val key: String,
        val failureId: String,
        val sequenceName: String,
        val variantKey: String,
    ) : ActionTarget

    data class Video(override val key: String, val outputPath: String) : ActionTarget
}

fun sequenceActions(paused: Boolean): List<RowAction> =
    listOf(if (paused) RowAction.RESUME_SEQUENCE else RowAction.PAUSE_SEQUENCE, RowAction.DETAILS)

fun variantActions(status: VariantStatus): List<RowAction> = when (status) {
    VariantStatus.VARIANT_QUEUED -> listOf(RowAction.STOP)
    VariantStatus.VARIANT_RUNNING -> listOf(RowAction.SUSPEND, RowAction.STOP)
    VariantStatus.VARIANT_SUSPENDED -> listOf(RowAction.RESUME_RENDER, RowAction.STOP)
    VariantStatus.VARIANT_PAUSED -> listOf(RowAction.RESUME_VARIANT, RowAction.RETRY)
    VariantStatus.VARIANT_FAILED -> listOf(RowAction.PAUSE_VARIANT, RowAction.RETRY)
    else -> listOf(RowAction.PAUSE_VARIANT)
}

val FAILURE_ACTIONS: List<RowAction> = listOf(RowAction.RETRY, RowAction.DISMISS, RowAction.DETAILS)

fun videoActions(loopback: Boolean, fileExists: Boolean): List<RowAction> = buildList {
    if (FileOpener.available(loopback, fileExists)) {
        add(RowAction.OPEN)
        add(RowAction.REVEAL)
    }
    add(RowAction.COPY_PATH)
    add(RowAction.DETAILS)
}

/** Actions as text buttons, wrapping where the width ends. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ActionButtons(actions: List<RowAction>, onAction: (RowAction) -> Unit, modifier: Modifier = Modifier) {
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        for (action in actions) {
            TextButton(
                onClick = { onAction(action) },
                modifier = Modifier.defaultMinSize(minHeight = 32.dp),
                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            ) { Text(action.label) }
        }
    }
}

/** A variant's actions as icon buttons, each named by its tooltip and content description. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun IconActionButtons(actions: List<RowAction>, onAction: (RowAction) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier) {
        for (action in actions) {
            TooltipArea(
                tooltip = {
                    Surface(shadowElevation = 4.dp, shape = MaterialTheme.shapes.small) {
                        Text(action.label, Modifier.padding(horizontal = 8.dp, vertical = 4.dp))
                    }
                },
            ) {
                IconButton(
                    onClick = { onAction(action) },
                    modifier = Modifier.size(32.dp).semantics { contentDescription = action.label },
                ) { ActionIcon(action) }
            }
        }
    }
}

/** The "⋮" button of a sequence card with its menu. */
@Composable
fun ActionMenu(actions: List<RowAction>, onAction: (RowAction) -> Unit, modifier: Modifier = Modifier) {
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        IconButton(
            onClick = { open = true },
            modifier = Modifier.size(32.dp).semantics { contentDescription = SEQUENCE_MENU },
        ) { Text("⋮", style = MaterialTheme.typography.titleMedium) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (action in actions) {
                DropdownMenuItem(
                    text = { Text(action.label) },
                    onClick = {
                        open = false
                        onAction(action)
                    },
                )
            }
        }
    }
}

const val SEQUENCE_MENU = "Sequence actions"

/** Drawn rather than typed, so that no icon depends on the fonts of the computer. */
@Composable
private fun ActionIcon(action: RowAction) {
    val color = LocalContentColor.current
    Canvas(Modifier.size(16.dp)) {
        val w = size.width
        val stroke = Stroke(w * 0.12f)
        fun bars(left: Float, top: Float, side: Float) {
            drawRect(color, Offset(left, top), Size(side * 0.32f, side))
            drawRect(color, Offset(left + side * 0.68f, top), Size(side * 0.32f, side))
        }
        when (action) {
            RowAction.SUSPEND -> bars(w * 0.15f, w * 0.15f, w * 0.7f)
            RowAction.RESUME_RENDER -> triangle(color, w * 0.2f, w * 0.1f, w * 0.8f)
            RowAction.STOP -> drawRect(color, Offset(w * 0.18f, w * 0.18f), Size(w * 0.64f, w * 0.64f))
            // The variant controls are the render's inside a ring.
            RowAction.PAUSE_VARIANT -> {
                drawCircle(color, w * 0.44f, style = stroke)
                bars(w * 0.34f, w * 0.34f, w * 0.32f)
            }
            RowAction.RESUME_VARIANT -> {
                drawCircle(color, w * 0.44f, style = stroke)
                triangle(color, w * 0.39f, w * 0.31f, w * 0.38f)
            }
            RowAction.RETRY -> {
                drawArc(
                    color, startAngle = 30f, sweepAngle = 290f, useCenter = false,
                    topLeft = Offset(w * 0.14f, w * 0.14f), size = Size(w * 0.72f, w * 0.72f), style = stroke,
                )
                drawPath(
                    Path().apply {
                        moveTo(w * 0.62f, w * 0.34f)
                        lineTo(w * 0.96f, w * 0.40f)
                        lineTo(w * 0.86f, w * 0.06f)
                        close()
                    },
                    color,
                )
            }
            else -> Unit
        }
    }
}

private fun DrawScope.triangle(color: Color, left: Float, top: Float, height: Float) {
    drawPath(
        Path().apply {
            moveTo(left, top)
            lineTo(left + height * 0.85f, top + height / 2)
            lineTo(left, top + height)
            close()
        },
        color,
    )
}

/**
 * Calls [onDoubleClick] on the second of two quick clicks without delaying or consuming either, so
 * the first selects at once. It goes after the row's `selectable`; presses that a button inside the
 * row took are left out.
 */
fun Modifier.onDoubleClick(onDoubleClick: () -> Unit): Modifier = composed {
    val action by rememberUpdatedState(onDoubleClick)
    pointerInput(Unit) {
        awaitEachGesture {
            awaitFirstDown()
            if (waitForUpOrCancellation() == null) return@awaitEachGesture
            val second = withTimeoutOrNull(viewConfiguration.doubleTapTimeoutMillis) { awaitFirstDown() }
            if (second != null && waitForUpOrCancellation() != null) action()
        }
    }
}
