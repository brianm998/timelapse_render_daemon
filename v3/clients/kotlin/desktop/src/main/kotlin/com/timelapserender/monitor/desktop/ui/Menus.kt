// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyShortcut
import androidx.compose.ui.window.FrameWindowScope
import androidx.compose.ui.window.MenuBar
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.StartState

/** Every menu item of spec 2 §6.2. */
enum class MenuAction {
    CONNECT, PAIR_PHONE, CLOSE_WINDOW,
    CONFIG, LOG, DETAILS, PREFERENCES,
    PAUSE_ALL, RESCAN, START_TRD, LOG_FOLDER,
    ABOUT, RELEASES,
}

/** [key] is pressed with Cmd on macOS and Ctrl elsewhere. */
data class MenuItemModel(val action: MenuAction, val label: String, val enabled: Boolean = true, val key: Key? = null)

data class MenuModel(val title: String, val items: List<MenuItemModel>)

/**
 * The menus for [ui]. Pair a phone…, Log… and Preferences… are disabled until their windows exist.
 */
fun menuModel(ui: AppUi): List<MenuModel> {
    val paused = ui.activity?.header?.globalPause == true
    return listOf(
        MenuModel(
            "File",
            listOf(
                MenuItemModel(MenuAction.CONNECT, "Connect to another computer…"),
                MenuItemModel(MenuAction.PAIR_PHONE, "Pair a phone…", enabled = false),
                MenuItemModel(MenuAction.CLOSE_WINDOW, "Close window"),
            ),
        ),
        MenuModel(
            "View",
            listOf(
                MenuItemModel(MenuAction.CONFIG, "Config…", enabled = ui.connected, key = Key.E),
                MenuItemModel(MenuAction.LOG, "Log…", enabled = false, key = Key.L),
                MenuItemModel(MenuAction.DETAILS, "Details pane", key = Key.D),
                MenuItemModel(MenuAction.PREFERENCES, "Preferences…", enabled = false, key = Key.Comma),
            ),
        ),
        MenuModel(
            "trd",
            listOf(
                MenuItemModel(
                    MenuAction.PAUSE_ALL,
                    if (paused) "Resume rendering" else "Pause all rendering",
                    enabled = ui.connected,
                ),
                MenuItemModel(MenuAction.RESCAN, "Rescan now", enabled = ui.connected, key = Key.R),
                MenuItemModel(MenuAction.START_TRD, "Start trd", enabled = ui.start !is StartState.Running),
                MenuItemModel(MenuAction.LOG_FOLDER, "Show trd's log folder", enabled = ui.loopback),
            ),
        ),
        MenuModel(
            "Help",
            listOf(
                MenuItemModel(MenuAction.ABOUT, "About"),
                MenuItemModel(MenuAction.RELEASES, "Releases page"),
            ),
        ),
    )
}

fun AppState.perform(action: MenuAction, closeWindow: () -> Unit) {
    when (action) {
        MenuAction.CONNECT -> showServers()
        MenuAction.CONFIG -> openConfig()
        MenuAction.CLOSE_WINDOW -> closeWindow()
        MenuAction.DETAILS -> toggleDetails()
        MenuAction.PAUSE_ALL -> setGlobalPause(ui.value.activity?.header?.globalPause != true)
        MenuAction.RESCAN -> rescanNow()
        MenuAction.START_TRD -> startTrd()
        MenuAction.LOG_FOLDER -> openLogFolder()
        MenuAction.ABOUT -> showAbout()
        MenuAction.RELEASES -> openReleasesPage()
        // Disabled in the model.
        MenuAction.PAIR_PHONE, MenuAction.LOG, MenuAction.PREFERENCES -> Unit
    }
}

fun menuShortcut(key: Key, mac: Boolean) = KeyShortcut(key, meta = mac, ctrl = !mac)

/** Cmd+F on macOS, Ctrl+F elsewhere: focus the search field. */
fun isSearchShortcut(key: Key, meta: Boolean, ctrl: Boolean, mac: Boolean): Boolean =
    key == Key.F && (if (mac) meta else ctrl)

/** The Compose menu bar (the macOS menu bar on macOS). */
@Composable
fun FrameWindowScope.AppMenuBar(ui: AppUi, mac: Boolean, onAction: (MenuAction) -> Unit) {
    MenuBar {
        for (menu in menuModel(ui)) {
            Menu(menu.title) {
                for (item in menu.items) {
                    Item(
                        item.label,
                        enabled = item.enabled,
                        shortcut = item.key?.let { menuShortcut(it, mac) },
                        onClick = { onAction(item.action) },
                    )
                }
            }
        }
    }
}
