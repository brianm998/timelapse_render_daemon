// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.input.pointer.PointerKeyboardModifiers
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.platform.WindowInfo
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTextInput
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.core.state.LogModel
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.monitor.desktop.record
import com.timelapserender.trd.v1.ListLogRequest
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.awt.datatransfer.Clipboard
import java.awt.datatransfer.DataFlavor
import java.time.Instant
import java.time.ZoneOffset

@OptIn(ExperimentalCoroutinesApi::class)
class LogWindowTest {
    @get:Rule
    val compose = createComposeRule()

    private val fake = FakeTrdService()
    private val running = fake.startInProcess()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default.limitedParallelism(1))
    private val scroll = LogScroll()
    private val copied = mutableListOf<String>()
    private var harness: Harness? = null

    @Volatile private var shift = false

    @After
    fun tearDown() {
        scope.cancel()
        running.close()
        harness?.close()
    }

    private val at = Instant.parse("2025-11-27T14:02:00Z").toEpochMilli()

    private fun entry(seq: Long, level: LogLevel = LogLevel.LOG_INFO, message: String = "message $seq") =
        LogEntry.newBuilder().setSeq(seq).setLevel(level).setSubject("subject").setMessage(message)
            .setAtMs(at).build()

    private fun entries(range: LongRange) = range.map { entry(it) }

    private fun append(range: LongRange) {
        fake.logEntries = fake.logEntries + entries(range)
    }

    private fun show(pageSize: Int = 200): LogModel {
        val model = LogModel(
            TrdClient(running.channel!!, null, scope),
            scope,
            clock = { at },
            zone = ZoneOffset.UTC,
            pageSize = pageSize,
            delayer = { delay(10) },
        )
        model.start()
        compose.setContent {
            val real = LocalWindowInfo.current
            val info = object : WindowInfo {
                override val isWindowFocused: Boolean get() = real.isWindowFocused
                override val keyboardModifiers: PointerKeyboardModifiers
                    get() = PointerKeyboardModifiers(isShiftPressed = shift)
            }
            CompositionLocalProvider(LocalWindowInfo provides info) {
                LogWindow(model, scroll, copy = { copied += it })
            }
        }
        return model
    }

    private fun node(tag: String) = compose.onNodeWithTag(tag)

    private fun row(seq: Long) = node(LogWindowTags.row(seq.toString()))

    private fun exists(tag: String) = compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty()

    private fun await(condition: () -> Boolean) = compose.waitUntil(Harness.TIMEOUT_MS, condition)

    private fun awaitRow(seq: Long) = await { exists(LogWindowTags.row(seq.toString())) }

    private fun awaitNoRow(seq: Long) = await { !exists(LogWindowTags.row(seq.toString())) }

    private fun awaitText(text: String) =
        await { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }

    private fun firstVisibleKey(): Any? = compose.runOnIdle { scroll.listState.layoutInfo.visibleItemsInfo.first().key }

    private fun scrollTo(index: Int) {
        node(LogWindowTags.LIST).performScrollToIndex(index)
        compose.waitForIdle()
    }

    private fun awaitPill(text: String) {
        await { exists(LogWindowTags.PILL) && compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }
        node(LogWindowTags.PILL).assertTextEquals(text)
    }

    @Test
    fun liveEntriesAppearAtTheTopWhileAtTheTop() {
        fake.logEntries = entries(1L..100L)
        show()
        awaitRow(100)
        append(101L..103L)
        awaitRow(103)
        compose.waitForIdle()
        row(103).assertIsDisplayed().assertTextContains("message 103")
        assertEquals("103", firstVisibleKey())
        assertTrue(compose.runOnIdle { scroll.atTop })
        node(LogWindowTags.PILL).assertDoesNotExist()
    }

    @Test
    fun thePillCountsLiveEntriesWhileScrolled() {
        fake.logEntries = entries(1L..100L)
        show()
        awaitRow(100)
        scrollTo(40)
        assertEquals("60", firstVisibleKey())
        node(LogWindowTags.PILL).assertDoesNotExist()
        append(101L..103L)
        awaitPill("↑ 3 new")
        assertEquals("60", firstVisibleKey())
        append(104L..105L)
        awaitPill("↑ 5 new")
        assertEquals("60", firstVisibleKey())

        node(LogWindowTags.PILL).performClick()
        await { !exists(LogWindowTags.PILL) }
        row(105).assertIsDisplayed()
        assertTrue(compose.runOnIdle { scroll.atTop })

        // Scrolling away again anchors the list as before.
        scrollTo(40)
        append(106L..106L)
        awaitPill("↑ 1 new")
        assertEquals("65", firstVisibleKey())
    }

    @Test
    fun scrollingBackToTheTopHidesThePillAndRestartsTheCount() {
        fake.logEntries = entries(1L..100L)
        show()
        awaitRow(100)
        scrollTo(40)
        append(101L..102L)
        awaitPill("↑ 2 new")
        scrollTo(0)
        node(LogWindowTags.PILL).assertDoesNotExist()
        scrollTo(40)
        node(LogWindowTags.PILL).assertDoesNotExist()
        append(103L..103L)
        awaitPill("↑ 1 new")
    }

    @Test
    fun aLongBurstKeepsTheFirstVisibleRow() {
        fake.logEntries = entries(1L..100L)
        show()
        awaitRow(100)
        scrollTo(40)
        append(101L..400L)
        awaitPill("↑ 300 new")
        assertEquals("60", firstVisibleKey())
    }

    @Test
    fun theLevelPickerHidesInfoLinesWhenSetToWarning() {
        fake.logEntries = listOf(
            entry(1, LogLevel.LOG_DEBUG), entry(2, LogLevel.LOG_INFO),
            entry(3, LogLevel.LOG_WARN), entry(4, LogLevel.LOG_ERROR),
        )
        show()
        awaitRow(2)
        node(LogWindowTags.level(LogLevel.LOG_INFO)).assertIsSelected()
        row(1).assertDoesNotExist()
        row(3).assertIsDisplayed()

        node(LogWindowTags.level(LogLevel.LOG_WARN)).performClick()
        awaitNoRow(2)
        awaitRow(3)
        row(4).assertIsDisplayed()
        row(1).assertDoesNotExist()
        node(LogWindowTags.level(LogLevel.LOG_WARN)).assertIsSelected()
        node(LogWindowTags.level(LogLevel.LOG_INFO)).assertIsNotSelected()
        assertEquals(LogLevel.LOG_WARN, fake.requestsOf<ListLogRequest>().last().minLevel)

        node(LogWindowTags.level(LogLevel.LOG_ERROR)).performClick()
        awaitNoRow(3)
        awaitRow(4)
        node(LogWindowTags.level(LogLevel.LOG_DEBUG)).performClick()
        awaitRow(1)
        row(2).assertIsDisplayed()
        for (name in listOf("Debug", "Info", "Warning", "Error")) {
            assertTrue(compose.onAllNodesWithText(name).fetchSemanticsNodes().isNotEmpty())
        }
    }

    @Test
    fun aNewLevelOrSearchPutsTheListBackAtTheTop() {
        fake.logEntries = entries(1L..100L)
        show()
        awaitRow(100)
        scrollTo(40)
        append(101L..101L)
        awaitPill("↑ 1 new")
        node(LogWindowTags.SEARCH).performTextInput("message")
        await { !exists(LogWindowTags.PILL) && compose.runOnIdle { scroll.atTop } }
        awaitRow(101)

        scrollTo(40)
        node(LogWindowTags.level(LogLevel.LOG_DEBUG)).performClick()
        await { compose.runOnIdle { scroll.atTop } }
        awaitRow(101)
        node(LogWindowTags.PILL).assertDoesNotExist()
    }

    @Test
    fun searchFiltersTheRows() {
        fake.logEntries = listOf(entry(1, message = "render started"), entry(2, message = "scan done"))
        show()
        awaitRow(2)
        node(LogWindowTags.SEARCH).performTextInput("RENDER")
        awaitNoRow(2)
        row(1).assertIsDisplayed()
        node(LogWindowTags.SEARCH).performTextInput(" nothing")
        awaitText("No log entries")
    }

    @Test
    fun selectedLinesAreCopiedAsListed() {
        fake.logEntries = listOf(entry(1), entry(2, LogLevel.LOG_WARN), entry(3, LogLevel.LOG_ERROR))
        show()
        awaitRow(3)
        node(LogWindowTags.COPY).assertIsNotEnabled()
        row(1).performClick()
        row(3).performClick()
        row(3).assertIsSelected()
        row(2).assertIsNotSelected()
        node(LogWindowTags.COPY).assertIsEnabled().performClick()
        compose.waitForIdle()
        assertEquals(
            listOf("14:02  Error  subject  message 3\n14:02  Info  subject  message 1"),
            copied,
        )
        row(3).performClick()
        row(3).assertIsNotSelected()
        row(1).performClick()
        node(LogWindowTags.COPY).assertIsNotEnabled()
    }

    @Test
    fun aSelectedLineTheSearchHidesIsNotCopied() {
        fake.logEntries = listOf(entry(1, message = "alpha"), entry(2, message = "beta"))
        show()
        awaitRow(2)
        row(1).performClick()
        node(LogWindowTags.SEARCH).performTextInput("beta")
        awaitNoRow(1)
        node(LogWindowTags.COPY).assertIsNotEnabled()
        row(2).performClick()
        node(LogWindowTags.COPY).performClick()
        compose.waitForIdle()
        assertEquals(listOf("14:02  Info  subject  beta"), copied)
    }

    @Test
    fun shiftClickSelectsTheLinesBetween() {
        fake.logEntries = entries(1L..5L)
        show()
        awaitRow(5)
        row(4).performClick()
        shift = true
        row(2).performClick()
        compose.waitForIdle()
        for (seq in 2L..4L) row(seq).assertIsSelected()
        row(5).assertIsNotSelected()
        row(1).assertIsNotSelected()
    }

    @Test
    fun theSelectionTogglesAndExtendsInEitherDirection() {
        val listed = listOf("e", "d", "c", "b", "a")
        val one = LogSelection().click("b", listed, extend = false)
        assertEquals(setOf("b"), one.keys)
        assertEquals(setOf("b", "c", "d"), one.click("d", listed, extend = true).keys)
        assertEquals(setOf("b", "a"), one.click("a", listed, extend = true).keys)
        assertEquals(emptySet<String>(), one.click("b", listed, extend = false).keys)
        // With nothing to extend from, a Shift-click is a click.
        assertEquals(setOf("c"), LogSelection().click("c", listed, extend = true).keys)
    }

    @Test
    fun copyingPutsTheTextOnTheClipboard() {
        val clipboard = Clipboard("test")
        copyToClipboard("two\nlines", clipboard)
        assertEquals("two\nlines", clipboard.getData(DataFlavor.stringFlavor))
    }

    @Test
    fun olderPagesAreLoadedAtTheEndOfTheList() {
        fake.logEntries = entries(1L..50L)
        val model = show(pageSize = 40)
        awaitRow(50)
        assertEquals(40, model.state.value.rows.size)
        scrollTo(40)
        await { model.state.value.rows.size == 50 }
        scrollTo(49)
        row(1).assertIsDisplayed()
        node(LogWindowTags.OLDER).assertDoesNotExist()
        node(LogWindowTags.PILL).assertDoesNotExist()
    }

    @Test
    fun aFailedOlderPageIsShownAndAskedForAgainByTheButton() {
        fake.logEntries = entries(1L..50L)
        val model = show(pageSize = 40)
        awaitRow(50)
        fake.failWith["ListLog"] = Status.INTERNAL.withDescription("log unavailable")
        scrollTo(40)
        awaitText("log unavailable")
        // The error line takes room from the list: bring the button fully back into view.
        scrollTo(40)
        fake.failWith.clear()
        node(LogWindowTags.OLDER).performClick()
        await { model.state.value.rows.size == 50 }
        await { !exists(LogWindowTags.ERROR) }
    }

    @Test
    fun aFailedFirstPageIsShownUntilItArrives() {
        fake.logEntries = entries(1L..1L)
        fake.failWith["ListLog"] = Status.INTERNAL.withDescription("log unavailable")
        show()
        awaitText("log unavailable")
        node(LogWindowTags.ERROR).assertIsDisplayed()
        fake.failWith.clear()
        awaitRow(1)
        await { !exists(LogWindowTags.ERROR) }
    }

    @Test
    fun anEmptyLogSaysSo() {
        show()
        awaitText("No log entries")
        compose.onNodeWithText("Loading the log…").assertDoesNotExist()
    }

    private fun connectedHarness(
        pairer: suspend (String) -> ServerRecord = { error("no pairing expected") },
    ): Harness {
        val h = Harness(pairer = { _, code, _ -> pairer(code) })
        harness = h
        h.store.save(record("other", Harness.OTHER_PORT, name = "Render box"))
        h.registry.refresh()
        h.local.serverInfo = Harness.info("studio", "Studio iMac")
        h.local.logEntries = entries(1L..2L)
        h.app.start()
        h.await { it.connected && it.activity?.header?.serverName == "Studio iMac" }
        return h
    }

    @Test
    fun eachServerHasOneLogWindowThatOutlivesAServerSwitch() {
        val h = connectedHarness()
        h.app.openLog()
        val first = h.app.logs.value.single()
        assertEquals("Log — Studio iMac", first.title)
        h.awaitTrue { first.model.state.value.rows.size == 2 }
        h.app.openLog()
        assertEquals(listOf(first), h.app.logs.value)
        assertEquals(1, first.focusRequests.value)

        h.app.selectServer("other")
        h.await { it.connected && it.activeServerId == "other" }
        h.local.logEntries = entries(1L..3L)
        h.awaitTrue { first.model.state.value.rows.size == 3 }
        h.app.openLog()
        assertEquals(listOf("local", "other"), h.app.logs.value.map { it.server.serverId })

        h.app.closeLog("local")
        assertEquals(listOf("other"), h.app.logs.value.map { it.server.serverId })
        h.app.forgetServer("other")
        assertTrue(h.app.logs.value.isEmpty())
    }

    @Test
    fun closingTheAppClosesTheLogWindows() {
        val h = connectedHarness()
        h.app.openLog()
        h.app.close()
        assertTrue(h.app.logs.value.isEmpty())
    }

    @Test
    fun aLogWindowIsBackAtTheTopAfterAReconnect() {
        val h = connectedHarness()
        h.app.openLog()
        val log = h.app.logs.value.single()
        h.awaitTrue { log.model.state.value.rows.size == 2 && log.client.state.value is ConnectionState.Connected }
        log.scroll.listState.requestScrollToItem(1)
        assertEquals(1, log.scroll.listState.firstVisibleItemIndex)
        h.local.failWith["WatchLog"] = Status.UNAVAILABLE
        h.awaitTrue { log.scroll.listState.firstVisibleItemIndex == 0 }
    }

    @Test
    fun pairingAgainMovesTheLogWindowToTheNewConnection() {
        val h = connectedHarness(pairer = { code -> record("other", Harness.OTHER_PORT).copy(token = code) })
        h.app.selectServer("other")
        h.await { it.connected && it.activeServerId == "other" }
        h.app.openLog()
        val before = h.app.logs.value.single()
        h.app.openPairing("other")
        h.app.submitPairing("123456")
        h.awaitTrue { h.app.logs.value.single() !== before }
        val after = h.app.logs.value.single()
        assertNotSame(before, after)
        assertEquals("123456", after.server.token)
        assertEquals(before.title, after.title)
    }
}
