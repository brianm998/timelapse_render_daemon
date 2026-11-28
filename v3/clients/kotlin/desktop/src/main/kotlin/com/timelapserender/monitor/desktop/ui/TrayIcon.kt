// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.window.ApplicationScope
import androidx.compose.ui.window.Tray
import androidx.compose.ui.window.TrayState
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.WINDOW_TITLE

/** What the tray icon's dot says (overview §6.8). */
enum class TrayStatus(val color: Color) {
    IDLE(Color(0xFF9E9E9E)),
    RENDERING(Color(0xFF2E9E44)),
    FAILURES(Color(0xFFD32F2F)),
}

enum class TrayAction { OPEN_WINDOW, PAUSE_ALL, QUIT }

data class TrayItemModel(val action: TrayAction, val label: String, val enabled: Boolean = true)

/** The counts line, or the app's name when there is none to show. */
fun trayTooltip(ui: AppUi): String =
    ui.activity?.header?.countsLine?.takeIf { ui.connected && it.isNotEmpty() } ?: WINDOW_TITLE

fun trayMenu(ui: AppUi): List<TrayItemModel> {
    val paused = ui.activity?.header?.globalPause == true
    return listOf(
        TrayItemModel(TrayAction.OPEN_WINDOW, "Open window"),
        TrayItemModel(
            TrayAction.PAUSE_ALL,
            if (paused) "Resume rendering" else "Pause all rendering",
            enabled = ui.connected,
        ),
        TrayItemModel(TrayAction.QUIT, "Quit"),
    )
}

fun AppState.performTray(action: TrayAction, quit: () -> Unit) {
    when (action) {
        TrayAction.OPEN_WINDOW -> openWindow()
        TrayAction.PAUSE_ALL -> setGlobalPause(ui.value.activity?.header?.globalPause != true)
        TrayAction.QUIT -> quit()
    }
}

/** A monitor on its stand with the status dot in the lower right corner, drawn to any size. */
class TrayIconPainter(val status: TrayStatus) : Painter() {
    override val intrinsicSize = Size(64f, 64f)

    override fun DrawScope.onDraw() {
        val w = size.width
        val h = size.height
        val frame = Color(0xFF424242)
        // The light edge keeps the dark monitor visible on a dark panel.
        drawRoundRect(Color(0xFFE0E0E0), Offset(0f, h * 0.06f), Size(w, h * 0.68f), CornerRadius(w * 0.10f))
        drawRoundRect(frame, Offset(w * 0.04f, h * 0.10f), Size(w * 0.92f, h * 0.60f), CornerRadius(w * 0.08f))
        drawRect(Color(0xFF90CAF9), Offset(w * 0.12f, h * 0.18f), Size(w * 0.76f, h * 0.44f))
        drawRect(frame, Offset(w * 0.42f, h * 0.72f), Size(w * 0.16f, h * 0.14f))
        drawRect(frame, Offset(w * 0.24f, h * 0.84f), Size(w * 0.52f, h * 0.10f))
        val center = Offset(w * 0.74f, h * 0.74f)
        drawCircle(Color.White, w * 0.26f, center)
        drawCircle(status.color, w * 0.22f, center)
    }

    override fun equals(other: Any?) = other is TrayIconPainter && other.status == status

    override fun hashCode() = status.hashCode()
}

/** The tray icon and its menu. Only called where the system has a tray. */
@Composable
fun ApplicationScope.AppTray(ui: AppUi, state: TrayState, onAction: (TrayAction) -> Unit) {
    val icon = remember(ui.trayStatus) { TrayIconPainter(ui.trayStatus) }
    Tray(
        icon = icon,
        state = state,
        tooltip = trayTooltip(ui),
        onAction = { onAction(TrayAction.OPEN_WINDOW) },
    ) {
        for (item in trayMenu(ui)) {
            Item(item.label, enabled = item.enabled, onClick = { onAction(item.action) })
        }
    }
}
