// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.FailureRowUi
import com.timelapserender.monitor.core.state.ProblemRowUi
import com.timelapserender.monitor.core.state.RenderedRowUi
import com.timelapserender.monitor.core.state.ScrollDecision
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.core.state.SequenceCardUi
import com.timelapserender.monitor.core.state.VariantRowUi
import com.timelapserender.monitor.desktop.ui.rows.ERROR_RED
import com.timelapserender.monitor.desktop.ui.rows.FailureCardRow
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.monitor.desktop.ui.rows.ProblemBanner
import com.timelapserender.monitor.desktop.ui.rows.RenderedRow
import com.timelapserender.monitor.desktop.ui.rows.SecondaryText
import com.timelapserender.monitor.desktop.ui.rows.SequenceCardRow
import com.timelapserender.monitor.desktop.ui.rows.VariantRowView
import com.timelapserender.trd.v1.ProblemKind
import kotlinx.coroutines.launch

const val ACTIVITY_LIST_TAG = "activity-list"
const val SHOW_OLDER = "Show older"

/**
 * One item of the list. [key] is stable (overview §6.2 S4, S5): a row's id behind its kind, so that
 * ids of different kinds never collide. It is also the row's test tag.
 */
sealed interface ActivityItem {
    val key: String

    /** Whether the item is a row that the `↑ N new` pill counts. */
    val isRow: Boolean get() = true

    data class Problem(val row: ProblemRowUi) : ActivityItem {
        override val key get() = "problem:${row.key}"
    }

    data class Header(val id: String, val title: String) : ActivityItem {
        override val key get() = "header:$id"
        override val isRow get() = false
    }

    data class Failure(val row: FailureRowUi) : ActivityItem {
        override val key get() = "failure:${row.key}"
    }

    data class Card(val row: SequenceCardUi) : ActivityItem {
        override val key get() = "card:${row.key}"
    }

    data class Variant(val row: VariantRowUi) : ActivityItem {
        override val key get() = "variant:${row.key}"
    }

    data class Rendered(val row: RenderedRowUi) : ActivityItem {
        override val key get() = "video:${row.key}"
    }

    data class Empty(val id: String, val text: String) : ActivityItem {
        override val key get() = "empty:$id"
        override val isRow get() = false
    }

    data class Older(val loading: Boolean, val error: String?) : ActivityItem {
        override val key get() = "older"
        override val isRow get() = false
    }
}

/** The sections of overview §6.1 in order, flattened into list items. */
fun activityItems(ui: ActivityUi): List<ActivityItem> = buildList {
    ui.problems.forEach { add(ActivityItem.Problem(it)) }
    if (ui.failures.isNotEmpty()) {
        add(ActivityItem.Header("attention", "Needs attention"))
        ui.failures.forEach { add(ActivityItem.Failure(it)) }
    }
    add(ActivityItem.Header("progress", "In progress"))
    ui.inProgressEmptyText?.let { add(ActivityItem.Empty("progress", it)) }
    ui.inProgress.forEach { card ->
        add(ActivityItem.Card(card))
        card.variants.forEach { add(ActivityItem.Variant(it)) }
    }
    add(ActivityItem.Header("recent", "Recently rendered"))
    ui.recentEmptyText?.let { add(ActivityItem.Empty("recent", it)) }
    ui.recent.forEach { add(ActivityItem.Rendered(it)) }
    if (ui.canLoadOlder) add(ActivityItem.Older(ui.loadingOlder, ui.olderError))
}

/** What the list showed before the current update. */
private class Shown(var activity: ActivityUi? = null, var keys: List<String> = emptyList())

/**
 * The Activity list (spec 2 §6.3). When it was at the top before an update it asks to stay there
 * (overview §6.2 S2); otherwise the `LazyColumn` keeps the first visible row by its key and the pill
 * counts what [scrollKeeper] saw inserted above it (S3). Nothing scrolls to the bottom (S6).
 */
@Composable
fun ActivityList(
    activity: ActivityUi,
    listState: LazyListState,
    scrollKeeper: ScrollKeeper,
    posters: PosterLoader,
    connected: Boolean,
    selectedKey: String?,
    onSelect: (String) -> Unit,
    onLoadOlder: () -> Unit,
    modifier: Modifier = Modifier,
    /** The Full Disk Access banner's **How to fix**; null where it is not offered. */
    onDiskAccessHelp: (() -> Unit)? = null,
) {
    val items = remember(activity) { activityItems(activity) }
    val shown = remember(listState) { Shown() }
    var newAbove by remember(listState) { mutableStateOf(0) }
    val atTop by remember(listState) {
        derivedStateOf { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset == 0 }
    }

    // Runs after the new items are applied and before they are measured, so the list state still
    // describes the position among the old ones.
    SideEffect {
        val previous = shown.activity
        if (previous === activity) return@SideEffect
        val oldKeys = shown.keys
        val keys = items.map { it.key }
        shown.activity = activity
        shown.keys = keys
        if (previous == null || keys == oldKeys) return@SideEffect
        Snapshot.withoutReadObservation {
            val index = listState.firstVisibleItemIndex
            val top = index == 0 && listState.firstVisibleItemScrollOffset == 0
            val first = oldKeys.getOrNull(index)
            val rows = items.filter { it.isRow }.mapTo(HashSet()) { it.key }
            // Rows that a filter change brings back are not new.
            val filtered = previous.filter != activity.filter
            when (val decision = scrollKeeper.decide(oldKeys, keys, first, top) { !filtered && it in rows }) {
                ScrollDecision.StayAtTop -> {
                    scrollKeeper.reset()
                    newAbove = 0
                    listState.requestScrollToItem(0)
                }
                is ScrollDecision.KeepAnchor -> {
                    newAbove = decision.insertedAbove
                    // The LazyColumn follows a surviving first row by itself; a removed one is replaced.
                    if (decision.key != first) listState.requestScrollToItem(keys.indexOf(decision.key))
                }
            }
        }
    }
    LaunchedEffect(listState, scrollKeeper) {
        snapshotFlow { atTop }.collect {
            if (it) {
                scrollKeeper.reset()
                newAbove = 0
            }
        }
    }

    val scope = rememberCoroutineScope()
    Box(modifier) {
        LazyColumn(
            Modifier.fillMaxSize().testTag(ACTIVITY_LIST_TAG),
            state = listState,
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            items(items, key = { it.key }, contentType = { it::class }) { item ->
                val tag = Modifier.testTag(item.key)
                val selected = item.key == selectedKey
                val select = { onSelect(item.key) }
                when (item) {
                    is ActivityItem.Problem -> ProblemBanner(
                        item.row,
                        tag,
                        onHowToFix = onDiskAccessHelp
                            .takeIf { item.row.kind == ProblemKind.PROBLEM_FULL_DISK_ACCESS_MISSING },
                    )
                    is ActivityItem.Header ->
                        Text(item.title, tag.padding(top = 8.dp), style = MaterialTheme.typography.titleMedium)
                    is ActivityItem.Failure -> FailureCardRow(item.row, selected, select, tag)
                    is ActivityItem.Card -> SequenceCardRow(item.row, selected, select, tag)
                    is ActivityItem.Variant -> VariantRowView(item.row, selected, select, tag)
                    is ActivityItem.Rendered -> RenderedRow(item.row, posters, connected, selected, select, tag)
                    is ActivityItem.Empty -> SecondaryText(item.text, tag)
                    is ActivityItem.Older -> OlderRow(item, onLoadOlder, tag.fillMaxWidth())
                }
            }
        }
        if (newAbove > 0 && !atTop) {
            NewItemsPill(
                newAbove,
                onClick = { scope.launch { listState.animateScrollToItem(0) } },
                modifier = Modifier.align(Alignment.TopCenter).padding(top = 8.dp),
            )
        }
    }
}

@Composable
private fun OlderRow(item: ActivityItem.Older, onLoadOlder: () -> Unit, modifier: Modifier) {
    Box(modifier, contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            TextButton(onClick = onLoadOlder, enabled = !item.loading) {
                Text(if (item.loading) "Loading…" else SHOW_OLDER)
            }
            item.error?.let { Text(it, color = ERROR_RED) }
        }
    }
}
