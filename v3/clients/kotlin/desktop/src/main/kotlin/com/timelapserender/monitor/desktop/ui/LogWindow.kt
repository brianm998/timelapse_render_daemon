// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.input.pointer.isShiftPressed
import com.timelapserender.monitor.core.state.LogModel
import com.timelapserender.monitor.core.state.LogRowUi
import com.timelapserender.monitor.core.state.LogUi
import com.timelapserender.monitor.core.state.ScrollDecision
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.trd.v1.LogLevel
import java.awt.Toolkit
import java.awt.datatransfer.Clipboard
import java.awt.datatransfer.StringSelection

object LogWindowTags {
    const val LIST = "log-list"
    const val SEARCH = "log-search"
    const val PILL = "log-pill"
    const val COPY = "log-copy"
    const val OLDER = "log-older"
    const val ERROR = "log-error"

    fun level(level: LogLevel) = "log-level-${level.name}"

    fun row(key: String) = "log-row-$key"
}

/** The level picker's choices (spec 2 §6.6). */
val LOG_LEVELS = listOf(LogLevel.LOG_DEBUG, LogLevel.LOG_INFO, LogLevel.LOG_WARN, LogLevel.LOG_ERROR)

fun levelName(level: LogLevel): String = when (level) {
    LogLevel.LOG_DEBUG -> "Debug"
    LogLevel.LOG_INFO -> "Info"
    LogLevel.LOG_WARN -> "Warning"
    LogLevel.LOG_ERROR -> "Error"
    else -> ""
}

/** The text "Copy selected lines" puts on the clipboard: the selected rows as listed, one per line. */
fun copyText(rows: List<LogRowUi>, selected: Set<String>): String =
    rows.filter { it.key in selected }
        .joinToString("\n") { "${it.timeText}  ${levelName(it.level)}  ${it.subject}  ${it.message}" }

fun copyToClipboard(text: String, clipboard: Clipboard = Toolkit.getDefaultToolkit().systemClipboard) {
    clipboard.setContents(StringSelection(text), null)
}

/**
 * The selected log lines. A click toggles a line; a Shift-click selects every listed line from the
 * one clicked before it to this one.
 */
data class LogSelection(val keys: Set<String> = emptySet(), val anchor: String? = null) {
    fun click(key: String, listed: List<String>, extend: Boolean): LogSelection {
        val from = listed.indexOf(anchor)
        val to = listed.indexOf(key)
        if (extend && from >= 0 && to >= 0) {
            return copy(keys = keys + listed.subList(minOf(from, to), maxOf(from, to) + 1))
        }
        return LogSelection(if (key in keys) keys - key else keys + key, key)
    }
}

/** The list's position and the pill's count: the overview §6.2 scroll rules over [ScrollKeeper]. */
class LogScroll {
    val listState = LazyListState()

    /** Rows inserted above the viewport since the user left the top (S3). */
    var inserted by mutableIntStateOf(0)
        private set

    private val keeper = ScrollKeeper()
    private var shown: LogUi? = null
    private var keys: List<String> = emptyList()

    val atTop: Boolean
        get() = listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset == 0

    /** Puts the list at the top: first display, reconnect, a new level or search (S1). */
    fun reset() {
        keeper.reset()
        keys = emptyList()
        inserted = 0
        listState.requestScrollToItem(0)
    }

    fun arrivedAtTop() {
        keeper.reset()
        inserted = 0
    }

    /** Called with every new state before the list is measured with it. */
    fun update(ui: LogUi) = Snapshot.withoutReadObservation {
        val before = shown
        shown = ui
        if (before != null && (before.minLevel != ui.minLevel || before.search != ui.search)) reset()
        val next = ui.rows.map { it.key }
        val index = listState.firstVisibleItemIndex
        val offset = listState.firstVisibleItemScrollOffset
        // Past the last row only the "load older" footer is left; the last row stands for it.
        val first = keys.getOrNull(index) ?: keys.lastOrNull()
        when (val decision = keeper.decide(keys, next, first, atTop)) {
            ScrollDecision.StayAtTop -> {
                inserted = 0
                listState.requestScrollToItem(0)
            }
            is ScrollDecision.KeepAnchor -> {
                inserted = decision.insertedAbove
                // LazyColumn follows the key only over short distances; a long one is asked for.
                val moved = next.indexOf(decision.key)
                if (moved - keys.indexOf(decision.key) > FOLLOWED_BY_KEY) listState.requestScrollToItem(moved, offset)
            }
        }
        keys = next
    }

    private companion object {
        const val FOLLOWED_BY_KEY = 50
    }
}

/**
 * The Log window's content (spec 2 §6.6): [model]'s entries newest first, a level picker, search
 * and "Copy selected lines". [model] is started and stopped by the caller.
 */
@Composable
fun LogWindow(
    model: LogModel,
    scroll: LogScroll = remember { LogScroll() },
    copy: (String) -> Unit = { copyToClipboard(it) },
) {
    val ui by model.state.collectAsState()
    var search by remember(model) { mutableStateOf(model.state.value.search) }
    var selection by remember(model) { mutableStateOf(LogSelection()) }
    val windowInfo = LocalWindowInfo.current
    MaterialTheme {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize()) {
                Row(
                    Modifier.fillMaxWidth().padding(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    LevelPicker(ui.minLevel, model::setLevel)
                    OutlinedTextField(
                        value = search,
                        onValueChange = {
                            search = it
                            model.setSearch(it)
                        },
                        placeholder = { Text("Search the log") },
                        singleLine = true,
                        modifier = Modifier.weight(1f).testTag(LogWindowTags.SEARCH),
                    )
                    Button(
                        onClick = { copy(copyText(ui.rows, selection.keys)) },
                        enabled = ui.rows.any { it.key in selection.keys },
                        modifier = Modifier.testTag(LogWindowTags.COPY),
                    ) { Text("Copy selected lines") }
                }
                ui.error?.let {
                    Text(
                        it,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(horizontal = 12.dp).testTag(LogWindowTags.ERROR),
                    )
                }
                LogList(ui, scroll, selection.keys, Modifier.weight(1f).fillMaxWidth(), model::loadOlder) { key ->
                    selection = selection.click(key, ui.rows.map { it.key }, windowInfo.keyboardModifiers.isShiftPressed)
                }
            }
        }
    }
}

@Composable
private fun LevelPicker(selected: LogLevel, onSelect: (LogLevel) -> Unit) {
    SingleChoiceSegmentedButtonRow {
        LOG_LEVELS.forEachIndexed { index, level ->
            SegmentedButton(
                selected = level == selected,
                onClick = { onSelect(level) },
                shape = SegmentedButtonDefaults.itemShape(index, LOG_LEVELS.size),
                modifier = Modifier.testTag(LogWindowTags.level(level)),
            ) { Text(levelName(level), maxLines = 1) }
        }
    }
}

@Composable
private fun LogList(
    ui: LogUi,
    scroll: LogScroll,
    selected: Set<String>,
    modifier: Modifier,
    loadOlder: () -> Unit,
    onClick: (String) -> Unit,
) {
    // During composition, so that the position is settled before the new rows are measured.
    remember(scroll, ui.rows, ui.minLevel, ui.search) { scroll.update(ui) }
    LaunchedEffect(scroll) {
        snapshotFlow { scroll.atTop }.collect { if (it) scroll.arrivedAtTop() }
    }
    Box(modifier) {
        LazyColumn(state = scroll.listState, modifier = Modifier.fillMaxSize().testTag(LogWindowTags.LIST)) {
            items(ui.rows, key = { it.key }) { row ->
                LogRow(row, row.key in selected) { onClick(row.key) }
            }
            if (ui.hasOlder) {
                item(key = OLDER_KEY) {
                    // Scrolling to the end asks for the next page; the button asks again after a failure.
                    LaunchedEffect(Unit) { loadOlder() }
                    TextButton(onClick = loadOlder, modifier = Modifier.testTag(LogWindowTags.OLDER)) {
                        Text("Load older entries")
                    }
                }
            }
        }
        if (ui.rows.isEmpty() && ui.error == null) {
            Text(
                if (ui.loading) "Loading the log…" else "No log entries",
                modifier = Modifier.align(Alignment.Center),
            )
        }
        NewRowsPill(scroll, Modifier.align(Alignment.TopCenter).padding(top = 8.dp))
    }
}

/** `↑ N new` while the user is away from the top and rows arrived above; a click goes back (S3). */
@Composable
private fun NewRowsPill(scroll: LogScroll, modifier: Modifier) {
    if (scroll.inserted > 0 && !scroll.atTop) {
        Button(onClick = scroll::reset, modifier = modifier.testTag(LogWindowTags.PILL)) {
            Text("↑ ${scroll.inserted} new")
        }
    }
}

@Composable
private fun LogRow(row: LogRowUi, selected: Boolean, onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    val levelColor = when (row.level) {
        LogLevel.LOG_ERROR -> colors.error
        LogLevel.LOG_WARN -> colors.tertiary
        else -> colors.onSurfaceVariant
    }
    Row(
        Modifier.fillMaxWidth()
            .background(if (selected) colors.secondaryContainer else colors.surface)
            .selectable(selected = selected, onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 4.dp)
            .testTag(LogWindowTags.row(row.key)),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(row.timeText, Modifier.width(TIME_WIDTH), fontFamily = FontFamily.Monospace)
        Text(levelName(row.level), Modifier.width(LEVEL_WIDTH), color = levelColor)
        Text(row.subject, Modifier.width(SUBJECT_WIDTH))
        Text(row.message, Modifier.weight(1f))
    }
}

private const val OLDER_KEY = "older"
private val TIME_WIDTH = 104.dp
private val LEVEL_WIDTH = 64.dp
private val SUBJECT_WIDTH = 160.dp
