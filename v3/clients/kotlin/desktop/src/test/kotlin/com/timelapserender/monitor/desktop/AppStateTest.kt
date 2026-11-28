// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.pairing.PairingException
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.desktop.platform.CommandResult
import com.timelapserender.monitor.desktop.platform.RELEASES_URL
import com.timelapserender.monitor.desktop.platform.WindowGeometry
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.WatchSnapshotsRequest
import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.job
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppStateTest {
    private var harness: Harness? = null

    private fun harness(h: Harness): Harness = h.also { harness = it }

    @After
    fun tearDown() {
        harness?.close()
    }

    private fun Harness.awaitConnectedTo(name: String) =
        await { it.connected && it.activity?.header?.serverName == name }

    @Test
    fun switchingServersReconnectsAndResetsTheListToTheTop() {
        val h = harness(Harness())
        h.store.save(record("other", Harness.OTHER_PORT))
        h.registry.refresh()
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        val first = h.app.session!!
        h.app.listState.requestScrollToItem(7, 3)
        assertEquals(7, h.app.listState.firstVisibleItemIndex)

        h.app.selectServer("other")

        assertEquals(0, h.app.listState.firstVisibleItemIndex)
        assertEquals(0, h.app.listState.firstVisibleItemScrollOffset)
        val ui = h.awaitConnectedTo("Other trd")
        assertEquals("other", ui.activeServerId)
        assertEquals("Timelapse Render Monitor — Other trd", ui.windowTitle)
        assertEquals("other", h.settings.lastServerId)
        assertEquals(1, h.other.callCount("GetServerInfo"))
        assertEquals("Bearer token-other", h.other.calls.first().authorization)
        assertFalse(first.scope.isActive)

        val before = h.local.callCount("GetServerInfo")
        h.app.listState.requestScrollToItem(4)
        h.app.selectServer("local")
        assertEquals(0, h.app.listState.firstVisibleItemIndex)
        h.awaitConnectedTo("Fake trd")
        assertTrue(h.local.callCount("GetServerInfo") > before)
    }

    @Test
    fun switchingServersClearsTheSelection() {
        val h = harness(Harness())
        h.store.save(record("other", Harness.OTHER_PORT))
        h.registry.refresh()
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        h.app.select("card:/exp/LRT_a")
        h.app.selectServer("local")
        assertEquals("card:/exp/LRT_a", h.app.ui.value.selectedKey)

        h.app.selectServer("other")

        assertNull(h.app.ui.value.selectedKey)
        assertNull(h.awaitConnectedTo("Other trd").selectedKey)
    }

    @Test
    fun selectingTheActiveServerKeepsTheConnectionAndTheScrollPosition() {
        val h = harness(Harness())
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        val session = h.app.session
        h.app.listState.requestScrollToItem(5)
        h.app.selectServer("local")
        assertTrue(session === h.app.session)
        assertEquals(5, h.app.listState.firstVisibleItemIndex)
    }

    @Test
    fun reconnectingPutsTheListBackAtTheTop() {
        val h = harness(Harness())
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        h.app.listState.requestScrollToItem(6)
        h.local.failWith["GetServerInfo"] = Status.UNAVAILABLE
        h.local.failWith["WatchSnapshots"] = Status.UNAVAILABLE
        val lost = h.await { !it.connected }
        // A lost connection keeps the main window and the scroll position.
        assertEquals(Screen.MAIN, lost.screen)
        assertEquals(6, h.app.listState.firstVisibleItemIndex)
        h.localUp()
        h.await { it.connected }
        h.awaitTrue { h.app.listState.firstVisibleItemIndex == 0 }
    }

    @Test
    fun startsOnTheLastActiveServer() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.app.start()
        h.awaitConnectedTo("Other trd")
        assertEquals(0, h.local.callCount("GetServerInfo"))
    }

    @Test
    fun theServerOverrideStartsOnTheLocalEntry() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store, localOverride = true))
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        assertEquals("other", h.settings.lastServerId)
        assertEquals(0, h.other.callCount("GetServerInfo"))
    }

    @Test
    fun aForgottenLastServerFallsBackToTheLocalOne() {
        val h = harness(Harness(FakeSettings(lastServerId = "gone")))
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        assertEquals("local", h.app.ui.value.activeServerId)
    }

    @Test
    fun theLocalServerIsProbedEveryTwoSecondsUntilItAnswers() {
        val h = harness(Harness())
        h.localDown()
        h.app.start()
        h.awaitTrue { h.delays.size >= 6 }
        assertEquals(Screen.NOT_RUNNING, h.app.ui.value.screen)
        assertTrue(h.delays.all { it == LOCAL_PROBE_MS })
        assertEquals(WINDOW_TITLE, h.app.ui.value.windowTitle)
        h.localUp()
        assertEquals(Screen.MAIN, h.awaitConnectedTo("Fake trd").screen)
    }

    @Test
    fun otherServersFollowTheClientBackOffAndStayInTheMainWindow() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.other.failWith["GetServerInfo"] = Status.UNAVAILABLE
        h.app.start()
        h.awaitTrue { h.delays.size >= 4 }
        assertEquals(listOf(1000L, 2000L, 5000L, 10000L), h.delays.take(4))
        assertEquals(Screen.MAIN, h.app.ui.value.screen)
    }

    @Test
    fun discoveryFeedsTheServerListAndIsStoppedOnClose() {
        val h = harness(Harness())
        h.app.start()
        h.awaitTrue { h.discovery.started }
        h.discovery.servers.value = listOf(discovered("new", Harness.OTHER_PORT))
        h.await { ui -> ui.servers.map { it.serverId } == listOf("local", "new") }
        h.app.close()
        assertTrue(h.discovery.stopped)
    }

    @Test
    fun pairingADiscoveredServerStoresItAndConnects() {
        val asked = mutableListOf<Pair<String, String>>()
        val h = harness(
            Harness(
                pairer = { endpoint: ServerEndpoint, code: String, device: String ->
                    asked += code to device
                    if (code == "000000") throw PairingException("That code is wrong.")
                    ServerRecord("other", "Other trd", endpoint.host, endpoint.port, "cd", "new-token", "linux-x64", 1L)
                },
            ),
        )
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        h.discovery.servers.value = listOf(discovered("other", Harness.OTHER_PORT))
        h.await { ui -> ui.servers.any { it.serverId == "other" } }

        h.app.selectServer("other")
        assertTrue(h.app.ui.value.dialog is AppDialog.Pair)
        assertEquals("local", h.app.ui.value.activeServerId)

        h.app.submitPairing("000000")
        h.await { (it.dialog as? AppDialog.Pair)?.error == "That code is wrong." }
        assertTrue(h.store.list().isEmpty())

        h.app.submitPairing("123 456")
        val ui = h.awaitConnectedTo("Other trd")
        assertNull(ui.dialog)
        assertEquals("new-token", h.store.list().single().token)
        assertEquals("other", h.settings.lastServerId)
        assertEquals(listOf("000000" to "test-desk", "123456" to "test-desk"), asked)
        assertEquals("Bearer new-token", h.other.calls.first().authorization)
    }

    @Test
    fun cancellingThePairingDialogStoresNothing() {
        val answer = CompletableDeferred<ServerRecord>()
        val entered = CompletableDeferred<Unit>()
        val cancelled = CompletableDeferred<Unit>()
        val h = harness(
            Harness(
                pairer = { _, _, _ ->
                    try {
                        entered.complete(Unit)
                        answer.await()
                    } finally {
                        cancelled.complete(Unit)
                    }
                },
            ),
        )
        h.app.start()
        h.discovery.servers.value = listOf(discovered("other", Harness.OTHER_PORT))
        h.await { ui -> ui.servers.any { it.serverId == "other" } }
        h.app.openPairing("other")
        h.app.submitPairing("123456")
        assertTrue((h.app.ui.value.dialog as AppDialog.Pair).busy)
        // A job cancelled before it starts never reaches the pairer.
        h.awaitTrue { entered.isCompleted }
        h.app.dismissDialog()
        h.awaitTrue { cancelled.isCompleted }
        assertNull(h.app.ui.value.dialog)
        assertTrue(h.store.list().isEmpty())
    }

    @Test
    fun anAnswerArrivingAfterTheDialogIsCancelledIsDropped() {
        for (succeeds in listOf(true, false)) {
            val entered = CompletableDeferred<Unit>()
            val answer = CompletableDeferred<Unit>()
            val returned = CompletableDeferred<Job>()
            val h = harness(
                Harness(
                    pairer = { _, _, _ ->
                        val job = currentCoroutineContext().job
                        // Like a call that is already past the point of being cancelled.
                        withContext(NonCancellable) {
                            entered.complete(Unit)
                            answer.await()
                        }
                        returned.complete(job)
                        if (!succeeds) throw PairingException("too late")
                        record("other", Harness.OTHER_PORT)
                    },
                ),
            )
            h.app.start()
            h.discovery.servers.value = listOf(discovered("other", Harness.OTHER_PORT))
            h.await { ui -> ui.servers.any { it.serverId == "other" } }
            h.app.openPairing("other")
            h.app.submitPairing("123456")
            h.awaitTrue { entered.isCompleted }
            h.app.dismissDialog()
            answer.complete(Unit)
            runBlocking { withTimeout(Harness.TIMEOUT_MS) { returned.await().join() } }

            assertNull(h.app.ui.value.dialog)
            assertTrue(h.store.list().isEmpty())
            assertEquals("local", h.app.ui.value.activeServerId)
            h.close()
        }
        harness = null
    }

    @Test
    fun aRevokedTokenShowsNeedsPairing() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.other.failWith["GetServerInfo"] = Status.UNAUTHENTICATED
        h.app.start()
        h.await { it.connection == ConnectionState.NeedsPairing }
        h.app.openPairing("other")
        assertEquals("other", (h.app.ui.value.dialog as AppDialog.Pair).server.serverId)
    }

    @Test
    fun theLocalServerIsNeverPairedOrForgotten() {
        val h = harness(Harness())
        h.app.start()
        h.app.openPairing("local")
        assertNull(h.app.ui.value.dialog)
        h.app.forgetServer("local")
        assertEquals(listOf("local"), h.app.ui.value.servers.map { it.serverId })
    }

    @Test
    fun forgettingTheActiveServerReturnsToTheLocalOne() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.app.start()
        h.awaitConnectedTo("Other trd")
        h.app.forgetServer("other")
        h.awaitConnectedTo("Fake trd")
        assertTrue(h.store.list().isEmpty())
        assertNull(h.settings.lastServerId)
        assertEquals(listOf("local"), h.app.ui.value.servers.map { it.serverId })
    }

    @Test
    fun filtersAreRememberedAndAppliedToTheList() {
        val h = harness(Harness(FakeSettings(search = "alpha")))
        h.local.snapshots = listOf(
            Snapshot.newBuilder()
                .addInProgress(SequenceCard.newBuilder().setId("1").setName("alpha").setExportPath("/a"))
                .addInProgress(SequenceCard.newBuilder().setId("2").setName("beta").setExportPath("/b"))
                .build(),
        )
        h.app.start()
        val ui = h.await { it.activity?.inProgress?.isNotEmpty() == true }
        assertEquals(listOf("alpha"), ui.activity!!.inProgress.map { it.name })
        assertEquals(listOf("/a", "/b"), ui.exportPaths)

        h.app.setSearch("")
        h.app.setExportPath("/b")
        h.await { state -> state.activity?.inProgress?.map { it.name } == listOf("beta") }
        assertEquals("", h.settings.search)
        assertEquals("/b", h.settings.exportPath)
    }

    @Test
    fun theLastScanIsDescribed() {
        val h = harness(Harness())
        h.local.snapshots = listOf(
            Snapshot.newBuilder().setLastScanCompletedMs(1000).setLastScanDurationMs(1250).build(),
        )
        assertEquals(AppUi.NOT_SCANNED, h.app.ui.value.lastScanText)
        h.app.start()
        val ui = h.await { it.lastScanText != AppUi.NOT_SCANNED }
        assertTrue(ui.lastScanText, ui.lastScanText.startsWith("Last scan "))
        assertTrue(ui.lastScanText, ui.lastScanText.endsWith("· took 1.3 s"))
    }

    @Test
    fun theDetailsPaneStateIsRemembered() {
        val h = harness(Harness(FakeSettings(detailsPaneVisible = false)))
        assertFalse(h.app.ui.value.detailsVisible)
        h.app.toggleDetails()
        assertTrue(h.app.ui.value.detailsVisible)
        assertTrue(h.settings.detailsPaneVisible)
    }

    @Test
    fun windowGeometryIsRememberedAndNeverBelowTheMinimum() {
        val h = harness(Harness())
        assertEquals(WindowGeometry(null, null, DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT), h.app.windowGeometry)
        h.app.saveWindowGeometry(WindowGeometry(10f, 20f, 1000f, 700f))
        assertEquals(WindowGeometry(10f, 20f, 1000f, 700f), h.app.windowGeometry)
        h.app.saveWindowGeometry(WindowGeometry(10f, 20f, 100f, 100f))
        assertEquals(WindowGeometry(10f, 20f, 900f, 600f), h.settings.windowGeometry)
    }

    @Test
    fun controlsReachTheServerAndAFailureBecomesANotice() {
        val h = harness(Harness())
        h.app.start()
        h.awaitConnectedTo("Fake trd")
        h.app.setGlobalPause(true)
        h.app.rescanNow()
        h.awaitTrue { h.local.callCount("SetGlobalPause") == 1 && h.local.callCount("RescanNow") == 1 }
        h.local.failWith["RescanNow"] = Status.FAILED_PRECONDITION.withDescription("no config")
        h.app.rescanNow()
        h.await { it.notice == "no config" }
        h.app.dismissNotice()
        assertNull(h.app.ui.value.notice)
    }

    @Test
    fun aFailedStartInTheMainWindowIsANotice() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.runner.results = h.runner.results + ("launchctl" to CommandResult(3, "no such service"))
        h.app.start()
        h.app.startTrd()
        h.await { it.notice == "Start trd failed: no such service" }
        assertEquals(StartState.Failed("no such service"), h.app.ui.value.start)
    }

    @Test
    fun theLogFolderOpensOnlyOverLoopbackAndTheReleasesPageAlways() {
        val store = MemoryStore().apply { save(record("other", Harness.OTHER_PORT)) }
        val h = harness(Harness(FakeSettings(lastServerId = "other"), store))
        h.app.start()
        h.awaitConnectedTo("Other trd")
        assertFalse(h.app.ui.value.loopback)
        h.app.openLogFolder()
        h.app.openReleasesPage()
        h.awaitTrue { h.runner.calls.isNotEmpty() }
        assertEquals(listOf(listOf("open", RELEASES_URL)), h.runner.calls.toList())

        h.app.selectServer("local")
        h.await { it.loopback }
        h.app.openLogFolder()
        h.awaitTrue { h.runner.calls.size == 2 }
        assertTrue(h.runner.calls[1][1].endsWith("TimelapseRenderDaemon"))
    }

    @Test
    fun closingTheWindowKeepsTheAppInTheTray() {
        val h = harness(Harness(traySupported = true))
        var quits = 0
        assertTrue(h.app.ui.value.windowVisible)
        h.app.closeWindow { quits++ }
        assertFalse(h.app.ui.value.windowVisible)
        assertEquals(0, quits)
        h.app.openWindow()
        assertTrue(h.app.ui.value.windowVisible)
        assertEquals(1, h.app.ui.value.openWindowRequests)
        // Opening a visible window still asks for it to come to the front.
        h.app.openWindow()
        assertEquals(2, h.app.ui.value.openWindowRequests)
    }

    @Test
    fun closingTheWindowQuitsWhenKeepRunningInTheTrayIsOff() {
        val h = harness(Harness(traySupported = true))
        h.app.setKeepRunningInTray(false)
        var quits = 0
        h.app.closeWindow { quits++ }
        assertEquals(1, quits)
        assertTrue(h.app.ui.value.windowVisible)
    }

    @Test
    fun closingTheWindowQuitsWithoutATray() {
        val h = harness(Harness(traySupported = false))
        assertTrue(h.preferences.keepRunningInTray)
        var quits = 0
        h.app.closeWindow { quits++ }
        assertEquals(1, quits)
        assertTrue(h.app.ui.value.windowVisible)
    }

    @Test
    fun snapshotsAreAskedForAtThePreferredRate() {
        val h = harness(Harness())
        fun rates() = h.local.requestsOf<WatchSnapshotsRequest>().map { it.maxPerSecond }
        h.app.start()
        h.awaitTrue { rates().isNotEmpty() }
        assertTrue(rates().all { it == DEFAULT_SNAPSHOT_RATE })

        h.app.setSnapshotRate(9)
        h.awaitTrue { rates().lastOrNull() == 9 }
        assertEquals(9, h.preferences.snapshotRate)
        assertEquals(9, h.app.ui.value.preferences.snapshotRate)

        h.app.setSnapshotRate(99)
        h.awaitTrue { rates().lastOrNull() == 10 }
        assertEquals(10, h.app.ui.value.preferences.snapshotRate)
    }

    @Test
    fun theStoredSnapshotRateIsUsedFromTheStart() {
        val preferences = InMemoryAppPreferences().apply { snapshotRate = 2 }
        val h = harness(Harness(preferences = preferences))
        h.app.start()
        h.awaitTrue { h.local.requestsOf<WatchSnapshotsRequest>().isNotEmpty() }
        assertEquals(2, h.local.requestsOf<WatchSnapshotsRequest>().first().maxPerSecond)
    }

    @Test
    fun aFailingEventWatcherLeavesTheSessionRunning() {
        val asked = CompletableDeferred<Unit>()
        val preferences = object : AppPreferences by InMemoryAppPreferences() {
            override fun get(serverId: String): Long? {
                asked.complete(Unit)
                error("preferences unreadable")
            }
        }
        val h = harness(Harness(preferences = preferences))
        h.app.start()
        h.awaitTrue { asked.isCompleted }
        h.awaitConnectedTo("Fake trd")
        assertTrue(h.app.session!!.scope.isActive)
        h.app.rescanNow()
        h.awaitTrue { h.local.callCount("RescanNow") == 1 }
    }

    @Test
    fun aThrowingOpenIsANotice() {
        val h = harness(Harness())
        h.runner.failure = java.io.IOException("no temporary file")
        h.app.openReleasesPage()
        h.await { it.notice == "no temporary file" }
    }
}
