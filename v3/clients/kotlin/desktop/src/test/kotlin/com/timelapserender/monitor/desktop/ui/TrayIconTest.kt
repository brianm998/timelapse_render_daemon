// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.WINDOW_TITLE
import com.timelapserender.trd.v1.Counts
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.Snapshot
import io.grpc.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TrayIconTest {
    private fun rendering(paused: Boolean = false) = Snapshot.newBuilder()
        .setCounts(Counts.newBuilder().setRendering(2)).setGlobalPause(paused).build()

    private val failing = Snapshot.newBuilder()
        .setCounts(Counts.newBuilder().setRendering(1).setFailures(1))
        .addFailures(FailureCard.newBuilder().setId("f1"))
        .build()

    /** Draws offscreen; no tray is created. */
    private fun dotColor(status: TrayStatus): Color {
        val bitmap = ImageBitmap(64, 64)
        val size = Size(64f, 64f)
        CanvasDrawScope().draw(Density(1f), LayoutDirection.Ltr, Canvas(bitmap), size) {
            with(TrayIconPainter(status)) { draw(size) }
        }
        return bitmap.toPixelMap()[47, 47]
    }

    @Test
    fun theDotIsGreyGreenOrRed() {
        for (status in TrayStatus.entries) assertEquals(status.color, dotColor(status))
        val (grey, green, red) = TrayStatus.entries.map { it.color }
        assertTrue(grey.red == grey.green && grey.green == grey.blue)
        assertTrue(green.green > green.red && green.green > green.blue)
        assertTrue(red.red > red.green && red.red > red.blue)
    }

    @Test
    fun theStatusFollowsTheServer() {
        Harness().use { h ->
            assertEquals(TrayStatus.IDLE, h.app.ui.value.trayStatus)
            h.app.start()
            h.await { it.connected }
            assertEquals(TrayStatus.IDLE, h.app.ui.value.trayStatus)
            h.local.snapshots = listOf(rendering())
            h.await { it.trayStatus == TrayStatus.RENDERING }
            h.local.snapshots = listOf(failing)
            h.await { it.trayStatus == TrayStatus.FAILURES }
            // A filtered-out failure still shows.
            h.app.setSearch("nothing matches this")
            assertEquals(TrayStatus.FAILURES, h.app.ui.value.trayStatus)
            h.local.failWith["GetServerInfo"] = Status.UNAVAILABLE
            h.local.failWith["WatchSnapshots"] = Status.UNAVAILABLE
            val lost = h.await { !it.connected && it.trayStatus == TrayStatus.IDLE }
            assertEquals(WINDOW_TITLE, trayTooltip(lost))
        }
    }

    @Test
    fun theTooltipIsTheCountsLine() {
        assertEquals(WINDOW_TITLE, trayTooltip(AppUi()))
        Harness().use { h ->
            h.local.snapshots = listOf(rendering())
            h.app.start()
            val ui = h.await { it.connected && it.activity?.header?.countsLine?.startsWith("2 rendering") == true }
            assertEquals(ui.activity!!.header.countsLine, trayTooltip(ui))
            assertTrue(trayTooltip(ui), trayTooltip(ui).startsWith("2 rendering"))
        }
    }

    @Test
    fun theMenuOffersOpenPauseAndQuit() {
        val menu = trayMenu(AppUi())
        assertEquals(listOf("Open window", "Pause all rendering", "Quit"), menu.map { it.label })
        assertEquals(listOf(true, false, true), menu.map { it.enabled })
        Harness().use { h ->
            h.local.snapshots = listOf(rendering(paused = true))
            h.app.start()
            val ui = h.await { it.connected && it.activity?.header?.globalPause == true }
            assertEquals(listOf("Open window", "Resume rendering", "Quit"), trayMenu(ui).map { it.label })
            assertTrue(trayMenu(ui).all { it.enabled })
        }
    }

    @Test
    fun theMenuActionsDriveTheApp() {
        Harness(traySupported = true).use { h ->
            h.app.start()
            h.await { it.connected }
            var quits = 0
            h.app.closeWindow { quits++ }
            h.app.performTray(TrayAction.OPEN_WINDOW) { quits++ }
            assertTrue(h.app.ui.value.windowVisible)
            h.app.performTray(TrayAction.PAUSE_ALL) { quits++ }
            h.app.confirm(false)
            h.awaitTrue { h.local.callCount("SetGlobalPause") == 1 }
            assertEquals(0, quits)
            h.app.performTray(TrayAction.QUIT) { quits++ }
            assertEquals(1, quits)
        }
    }
}
