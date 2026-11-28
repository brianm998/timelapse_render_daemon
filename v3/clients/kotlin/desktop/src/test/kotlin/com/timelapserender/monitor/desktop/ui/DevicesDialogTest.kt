// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.DevicesModel
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.record
import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.PairedDevice
import com.timelapserender.trd.v1.StartPairingResponse
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestCoroutineScheduler
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.time.Instant
import java.time.ZoneOffset

@OptIn(ExperimentalCoroutinesApi::class)
class DevicesDialogTest {
    @get:Rule
    val compose = createComposeRule()

    private val base = Instant.parse("2025-11-27T15:00:00Z").toEpochMilli()
    private val fake = FakeTrdService()
    private val running = fake.startInProcess()

    /** The model runs and counts down in the virtual time of this scheduler. */
    private val scheduler = TestCoroutineScheduler()
    private val scope = CoroutineScope(SupervisorJob() + StandardTestDispatcher(scheduler))
    private var harness: Harness? = null

    @After
    fun tearDown() {
        scope.cancel()
        running.close()
        harness?.close()
    }

    private fun code(code: String, expiresAtMs: Long) = StartPairingResponse.newBuilder()
        .setCode(code).setExpiresAtMs(expiresAtMs).setServerName("Studio Mac").setLanPort(50072)
        .setCertFingerprintSha256("0123456789abcdef0123456789abcdef")
        .addAllLanAddresses(listOf("192.168.1.20", "10.0.0.5")).build()

    private fun phone(id: String, name: String) = PairedDevice.newBuilder().setDeviceId(id).setName(name)
        .setPlatform(DevicePlatform.DEVICE_PLATFORM_IOS).setPairedAtMs(base).build()

    private fun showDialog() {
        val model = DevicesModel(
            TrdClient(running.channel!!, null, scope),
            scope,
            clock = { base + scheduler.currentTime },
            zone = ZoneOffset.UTC,
        )
        compose.setContent { DevicesDialog(model, onDismiss = {}) }
    }

    /** Waits for the fake's answers, running what is due in virtual time and no more. */
    private fun await(condition: () -> Boolean) = compose.waitUntil(Harness.TIMEOUT_MS) {
        scheduler.runCurrent()
        condition()
    }

    private fun shows(text: String) = compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty()

    private fun exists(tag: String) = compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty()

    private fun advance(ms: Long) {
        scheduler.advanceTimeBy(ms)
        scheduler.runCurrent()
    }

    @Test
    fun theCodeCountsDownAndNewCodeReplacesIt() {
        fake.startPairingResponse = code("482913", base + 300_000)
        showDialog()
        await { shows("482 913") }
        compose.onNodeWithTag(DevicesTags.CODE).assertTextEquals("482 913")
        compose.onNodeWithTag(DevicesTags.COUNTDOWN).assertTextEquals("Expires in 5:00")
        compose.onNodeWithText("Server: Studio Mac").assertExists()
        compose.onNodeWithText("Addresses: 192.168.1.20, 10.0.0.5").assertExists()
        compose.onNodeWithText("Port: 50072").assertExists()
        compose.onNodeWithTag(DevicesTags.FINGERPRINT).assertTextEquals("Fingerprint: 0123 4567 89ab cdef")

        advance(61_000)
        await { shows("Expires in 3:59") }

        fake.startPairingResponse = code("105577", base + scheduler.currentTime + 300_000)
        compose.onNodeWithTag(DevicesTags.NEW_CODE).performScrollTo().performClick()
        await { shows("105 577") }
        compose.onNodeWithText("482 913").assertDoesNotExist()
        compose.onNodeWithTag(DevicesTags.COUNTDOWN).assertTextEquals("Expires in 5:00")
        assertEquals(2, fake.callCount("StartPairing"))
        // The replaced code's countdown has stopped: only the new one is counting.
        advance(1_000)
        await { shows("Expires in 4:59") }
        compose.onNodeWithText("105 577").assertExists()

        advance(299_000)
        await { shows(CODE_EXPIRED) }
        compose.onNodeWithTag(DevicesTags.CODE).assertDoesNotExist()
    }

    @Test
    fun aPhoneThatPairsAppearsAndRevokeAsksFirst() {
        fake.startPairingResponse = code("482913", base + 300_000)
        showDialog()
        await { shows("482 913") && shows("No paired devices.") }

        fake.pairedDevices = listOf(phone("d1", "Pat's phone"))
        // One second at a time: the dialog reads the list again as the code counts down.
        repeat(3) {
            advance(1_000)
            compose.waitForIdle()
        }
        await { shows("Pat's phone") }

        compose.onNodeWithTag(DevicesTags.revoke("d1")).performScrollTo().performClick()
        compose.onNodeWithText("Revoke Pat's phone?").assertExists()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("Revoke Pat's phone?").assertDoesNotExist()
        assertEquals(0, fake.callCount("RevokeDevice"))

        compose.onNodeWithTag(DevicesTags.revoke("d1")).performScrollTo().performClick()
        fake.pairedDevices = emptyList()
        compose.onNodeWithTag(DevicesTags.CONFIRM_REVOKE).performClick()
        await { shows("No paired devices.") }
        assertEquals(1, fake.callCount("RevokeDevice"))
    }

    @Test
    fun aFailureIsShown() {
        fake.failWith["StartPairing"] = Status.UNAVAILABLE.withDescription("the LAN listener is off")
        showDialog()
        await { shows("the LAN listener is off") }
        compose.onNodeWithTag(DevicesTags.CODE).assertDoesNotExist()
    }

    private fun items(h: Harness) = menuModel(h.app.ui.value).flatMap { it.items }.map { it.action }

    private fun start(local: Boolean = true): Harness {
        val h = Harness()
        harness = h
        h.local.serverInfo = Harness.info("studio", "Studio iMac", local = local)
        h.local.startPairingResponse = code("482913", System.currentTimeMillis() + 300_000)
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        compose.setContent { MainWindow(h.app) }
        h.app.start()
        h.await { it.connected }
        return h
    }

    private fun awaitUi(condition: () -> Boolean) = compose.waitUntil(Harness.TIMEOUT_MS, condition)

    @Test
    fun pairAPhoneOpensTheDialogOverLoopback() {
        val h = start()
        assertTrue(MenuAction.PAIR_PHONE in items(h))
        h.app.perform(MenuAction.PAIR_PHONE) {}
        awaitUi { shows("482 913") }
        compose.onNodeWithText("Paired devices").assertExists()
        compose.onNodeWithText("Close").performClick()
        awaitUi { !shows("Paired devices") }
        assertNull(h.app.ui.value.dialog)
    }

    @Test
    fun theDialogAndTheMenuItemAreAbsentOverTheLan() {
        val h = start()
        h.app.showDevices()
        assertTrue(h.app.ui.value.dialog is AppDialog.Devices)
        awaitUi { shows("482 913") }

        // Switching to another computer closes it.
        h.app.selectServer("other")
        h.await { it.activeServerId == "other" && it.connected }
        awaitUi { !shows("Paired devices") }
        assertNull(h.app.ui.value.dialog)
        assertTrue(MenuAction.PAIR_PHONE !in items(h))
        h.app.perform(MenuAction.PAIR_PHONE) {}
        assertNull(h.app.ui.value.dialog)
        compose.onNodeWithText("Paired devices").assertDoesNotExist()
        assertEquals(0, h.other.callCount("StartPairing"))
    }

    @Test
    fun aCallerThatIsNotLocalHasNoDialog() {
        val h = start(local = false)
        assertTrue(MenuAction.PAIR_PHONE !in items(h))
        h.app.showDevices()
        assertNull(h.app.ui.value.dialog)
        assertEquals(0, h.local.callCount("StartPairing"))
    }

    @Test
    fun theDialogClosesWhenTheConnectionIsLost() {
        val h = start()
        h.app.showDevices()
        awaitUi { shows("482 913") }
        h.localDown()
        h.local.failWith["StartPairing"] = Status.UNAVAILABLE
        compose.onNodeWithTag(DevicesTags.NEW_CODE).performScrollTo().performClick()
        h.await { !it.connected && it.dialog == null }
        awaitUi { !exists(DevicesTags.NEW_CODE) }
        assertTrue(MenuAction.PAIR_PHONE !in items(h))
    }
}
