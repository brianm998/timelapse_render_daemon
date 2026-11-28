// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyShortcut
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.StartState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MenusTest {
    private fun item(ui: AppUi, action: MenuAction) =
        menuModel(ui).flatMap { it.items }.single { it.action == action }

    private val connected = AppUi(connection = ConnectionState.Connected(Harness.info("a", "A")))

    @Test
    fun theMenusFollowTheSpec() {
        val menus = menuModel(connected)
        assertEquals(listOf("File", "View", "trd", "Help"), menus.map { it.title })
        assertEquals(
            listOf(
                listOf("Connect to another computer…", "Pair a phone…", "Close window"),
                listOf("Config…", "Log…", "Details pane", "Preferences…"),
                listOf("Pause all rendering", "Rescan now", "Start trd", "Show trd's log folder"),
                listOf("About", "Releases page"),
            ),
            menus.map { menu -> menu.items.map { it.label } },
        )
        assertEquals(
            mapOf(
                MenuAction.CONFIG to Key.E, MenuAction.LOG to Key.L, MenuAction.DETAILS to Key.D,
                MenuAction.PREFERENCES to Key.Comma, MenuAction.RESCAN to Key.R,
            ),
            menus.flatMap { it.items }.filter { it.key != null }.associate { it.action to it.key!! },
        )
    }

    @Test
    fun itemsAreEnabledByTheState() {
        val offline = AppUi(connection = ConnectionState.Offline(0))
        assertFalse(item(offline, MenuAction.PAUSE_ALL).enabled)
        assertFalse(item(offline, MenuAction.RESCAN).enabled)
        assertFalse(item(offline, MenuAction.LOG_FOLDER).enabled)
        assertTrue(item(offline, MenuAction.START_TRD).enabled)
        assertFalse(item(offline.copy(start = StartState.Running), MenuAction.START_TRD).enabled)
        assertTrue(item(connected, MenuAction.PAUSE_ALL).enabled)
        assertTrue(item(connected, MenuAction.RESCAN).enabled)
        assertTrue(item(connected, MenuAction.LOG_FOLDER).enabled)
        assertFalse(item(connected.copy(activeServerId = "other"), MenuAction.LOG_FOLDER).enabled)
        assertTrue(item(connected, MenuAction.CONFIG).enabled)
        assertFalse(item(offline, MenuAction.CONFIG).enabled)
        assertTrue(item(connected, MenuAction.LOG).enabled)
        assertFalse(item(offline, MenuAction.LOG).enabled)
        assertTrue(item(connected, MenuAction.PAIR_PHONE).enabled)
        assertTrue(item(connected, MenuAction.PREFERENCES).enabled)
        assertTrue(item(offline, MenuAction.PREFERENCES).enabled)
    }

    @Test
    fun menuActionsDriveTheApp() {
        Harness().use { h ->
            h.app.start()
            h.await { it.connected }
            var closed = 0
            val perform = { action: MenuAction -> h.app.perform(action) { closed++ } }
            perform(MenuAction.CLOSE_WINDOW)
            assertEquals(1, closed)
            perform(MenuAction.CONNECT)
            assertEquals(AppDialog.Servers, h.app.ui.value.dialog)
            perform(MenuAction.ABOUT)
            assertEquals(AppDialog.About, h.app.ui.value.dialog)
            perform(MenuAction.DETAILS)
            assertFalse(h.app.ui.value.detailsVisible)
            perform(MenuAction.PREFERENCES)
            assertTrue(h.app.ui.value.preferencesOpen)
            h.app.closePreferences()
            assertFalse(h.app.ui.value.preferencesOpen)
            perform(MenuAction.PAUSE_ALL)
            h.app.confirm(false)
            perform(MenuAction.RESCAN)
            perform(MenuAction.START_TRD)
            perform(MenuAction.LOG_FOLDER)
            perform(MenuAction.RELEASES)
            perform(MenuAction.LOG)
            assertEquals(listOf("local"), h.app.logs.value.map { it.server.serverId })
            perform(MenuAction.CONFIG)
            assertEquals(listOf("local"), h.app.configWindows.value.map { it.serverId })
            // id, launchctl, the log folder and the releases page.
            h.awaitTrue {
                h.local.callCount("SetGlobalPause") == 1 && h.local.callCount("RescanNow") == 1 &&
                    h.runner.calls.size == 4
            }
        }
    }

    @Test
    fun shortcutsUseCommandOnTheMacAndControlElsewhere() {
        for (key in listOf(Key.E, Key.L, Key.D, Key.Comma, Key.R)) {
            assertEquals(KeyShortcut(key, meta = true), menuShortcut(key, mac = true))
            assertEquals(KeyShortcut(key, ctrl = true), menuShortcut(key, mac = false))
            assertNotEquals(menuShortcut(key, mac = true), menuShortcut(key, mac = false))
        }
        assertTrue(isSearchShortcut(Key.F, meta = true, ctrl = false, mac = true))
        assertFalse(isSearchShortcut(Key.F, meta = false, ctrl = true, mac = true))
        assertTrue(isSearchShortcut(Key.F, meta = false, ctrl = true, mac = false))
        assertFalse(isSearchShortcut(Key.F, meta = true, ctrl = false, mac = false))
        assertFalse(isSearchShortcut(Key.G, meta = true, ctrl = true, mac = false))
    }
}
