// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import com.timelapserender.monitor.desktop.ui.MenuAction
import com.timelapserender.monitor.desktop.ui.perform
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/** The app's Config windows: one per server, closed with their server, and asked about on quit. */
class ConfigWindowsTest {
    private fun connected(h: Harness = Harness()): Harness {
        val state = ConfigState.newBuilder().setExists(true).setRevision("r").setConfig(Config.getDefaultInstance())
        h.local.configState = state.build()
        h.other.configState = state.build()
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        h.app.start()
        h.await { it.connected }
        return h
    }

    private fun Harness.openWindow(): ConfigWindowHandle {
        app.perform(MenuAction.CONFIG) {}
        val window = app.configWindows.value.last()
        awaitTrue { !window.controller.editor.state.value.loading }
        return window
    }

    private fun Harness.makeDirty(window: ConfigWindowHandle) {
        window.controller.edit { it.addExportPaths("/x") }
        assertTrue(window.controller.dirty)
    }

    @Test
    fun theMenuOpensOneWindowPerServer() {
        connected().use { h ->
            val first = h.openWindow()
            h.app.openConfig()
            assertEquals(listOf(first), h.app.configWindows.value)
            assertEquals(1, first.controller.ui.value.focusRequests)

            h.app.selectServer("other")
            h.await { it.activeServerId == "other" && it.connected }
            val second = h.openWindow()
            assertEquals(listOf("local", "other"), h.app.configWindows.value.map { it.serverId })
            // The first window still talks to its own server.
            val before = h.local.callCount("GetConfig")
            first.controller.reload()
            h.awaitTrue { h.local.callCount("GetConfig") == before + 1 }

            h.app.forgetServer("other")
            assertEquals(listOf(first), h.app.configWindows.value)
            second.controller.requestFocus()
            first.controller.requestClose()
            assertTrue(h.app.configWindows.value.isEmpty())
        }
    }

    @Test
    fun quittingWithNothingUnsavedIsImmediate() {
        connected().use { h ->
            h.openWindow()
            val quits = AtomicInteger()
            h.app.requestQuit { quits.incrementAndGet() }
            assertEquals(1, quits.get())
        }
    }

    @Test
    fun quittingAsksAboutUnsavedChangesAndCancelStays() {
        connected().use { h ->
            val window = h.openWindow()
            h.makeDirty(window)
            val quits = AtomicInteger()
            h.app.requestQuit { quits.incrementAndGet() }
            assertTrue(window.controller.ui.value.closePrompt)
            assertEquals(0, quits.get())
            window.controller.cancelClose()
            // Closing that window later is not a quit any more.
            window.controller.requestClose()
            window.controller.discardAndClose()
            assertTrue(h.app.configWindows.value.isEmpty())
            assertEquals(0, quits.get())
        }
    }

    @Test
    fun theAppQuitsOnceEveryUnsavedWindowIsAnswered() {
        connected().use { h ->
            val first = h.openWindow()
            h.app.selectServer("other")
            h.await { it.activeServerId == "other" && it.connected }
            val second = h.openWindow()
            h.makeDirty(first)
            h.makeDirty(second)
            val quits = AtomicInteger()
            h.app.requestQuit { quits.incrementAndGet() }
            first.controller.discardAndClose()
            assertEquals(0, quits.get())
            second.controller.discardAndClose()
            assertEquals(1, quits.get())
        }
    }
}
