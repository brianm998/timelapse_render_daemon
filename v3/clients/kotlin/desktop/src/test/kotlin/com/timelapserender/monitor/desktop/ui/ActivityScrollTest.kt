// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ActivityFilter
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class ActivityScrollTest {
    @get:Rule
    val compose = createComposeRule()

    private val listState = LazyListState()
    private var activity by mutableStateOf(activityUi())

    private val cards = listOf("a", "b", "c")
    private val videos = (1..30).map { "v$it" }
    private val base = activityUi(cards = cards, videos = videos)

    private fun show(first: ActivityUi = base) {
        activity = first
        val keeper = ScrollKeeper()
        compose.setContent {
            MaterialTheme {
                Box(Modifier.size(800.dp, 500.dp)) {
                    ActivityList(
                        activity = activity,
                        listState = listState,
                        scrollKeeper = keeper,
                        posters = PosterLoader { null },
                        connected = true,
                        selectedKey = null,
                        onSelect = {},
                        onLoadOlder = {},
                    )
                }
            }
        }
        compose.waitForIdle()
    }

    private fun update(next: ActivityUi) {
        compose.runOnIdle { activity = next }
        compose.waitForIdle()
    }

    private fun scrollTo(key: String) {
        val index = activityItems(activity).indexOfFirst { it.key == key }
        compose.onNodeWithTag(ACTIVITY_LIST_TAG).performScrollToIndex(index)
        compose.waitForIdle()
    }

    private fun top(key: String) = compose.onNodeWithTag(key).fetchSemanticsNode().positionInRoot.y

    private fun firstKey() = compose.runOnIdle { activityItems(activity)[listState.firstVisibleItemIndex].key }

    private fun assertAtTop() = compose.runOnIdle {
        assertEquals(0, listState.firstVisibleItemIndex)
        assertEquals(0, listState.firstVisibleItemScrollOffset)
    }

    private fun assertPill(text: String) {
        compose.onNodeWithTag(NEW_ITEMS_PILL_TAG).assertIsDisplayed()
        compose.onNodeWithText(text).assertIsDisplayed()
    }

    private fun assertNoPill() = compose.onNodeWithTag(NEW_ITEMS_PILL_TAG).assertDoesNotExist()

    @Test
    fun atTheTopANewInProgressCardIsVisibleAfterTheUpdate() {
        show()
        update(activityUi(cards = listOf("new") + cards, videos = videos))
        compose.onNodeWithTag("card:new").assertIsDisplayed()
        assertAtTop()
        assertNoPill()
    }

    @Test
    fun atTheTopRowsInsertedAboveTheFirstRowAreSeenAtOnce() {
        show(activityUi(failures = listOf("f1"), cards = cards, videos = videos))
        update(activityUi(problems = listOf("p1"), failures = listOf("f2", "f1"), cards = cards, videos = videos))
        compose.onNodeWithTag("problem:p1").assertIsDisplayed()
        compose.onNodeWithTag("failure:f2").assertIsDisplayed()
        assertAtTop()
        assertNoPill()
    }

    @Test
    fun scrolledDownTheAnchorRowStaysAndThePillCountsTheNewRows() {
        show()
        scrollTo("video:v8")
        assertNoPill()
        val before = top("video:v8")
        // Two cards, a problem and a video arrive above; the new "Needs attention" header is not a row.
        update(
            activityUi(
                problems = listOf("p1"),
                failures = listOf("f1"),
                cards = listOf("n1") + cards,
                videos = listOf("w1") + videos,
            ),
        )
        assertEquals(before, top("video:v8"), 0.5f)
        assertEquals("video:v8", firstKey())
        assertPill("↑ 4 new")
        // A row below the viewport is not counted; one more above is added.
        update(
            activityUi(
                problems = listOf("p1"),
                failures = listOf("f1"),
                cards = listOf("n2", "n1") + cards,
                videos = listOf("w1") + videos + "below",
            ),
        )
        assertEquals(before, top("video:v8"), 0.5f)
        assertPill("↑ 5 new")
    }

    @Test
    fun clickingThePillGoesToTheTopAndRemovesIt() {
        show()
        scrollTo("video:v8")
        update(activityUi(cards = listOf("n1") + cards, videos = videos))
        compose.onNodeWithTag(NEW_ITEMS_PILL_TAG).performClick()
        compose.waitForIdle()
        assertAtTop()
        assertNoPill()
        compose.onNodeWithTag("card:n1").assertIsDisplayed()
    }

    @Test
    fun scrollingBackToTheTopStartsTheCountAgain() {
        show()
        scrollTo("video:v8")
        update(activityUi(cards = listOf("n1") + cards, videos = videos))
        assertPill("↑ 1 new")
        compose.onNodeWithTag(ACTIVITY_LIST_TAG).performScrollToIndex(0)
        compose.waitForIdle()
        assertNoPill()
        scrollTo("video:v8")
        assertNoPill()
        update(activityUi(cards = listOf("n2", "n1") + cards, videos = videos))
        assertPill("↑ 1 new")
    }

    @Test
    fun aProgressUpdateNeitherScrollsNorCounts() {
        show()
        scrollTo("card:b")
        val before = top("card:b")
        update(base.copy(inProgress = cards.map { cardUi(it, framesText = "9 / 10 frames", progress = 0.9f) }))
        assertEquals(before, top("card:b"), 0.5f)
        assertEquals("card:b", firstKey())
        assertNoPill()
    }

    @Test
    fun whenTheAnchorRowLeavesItsNeighbourTakesItsPlace() {
        show()
        scrollTo("card:b")
        val before = top("card:b")
        update(activityUi(cards = listOf("a", "c"), videos = listOf("b-video") + videos))
        assertEquals("card:c", firstKey())
        assertEquals(before, top("card:c"), 0.5f)
        assertNoPill()
    }

    @Test
    fun rowsBroughtBackByAFilterChangeAreNotNew() {
        val filtered = activityUi(videos = videos).copy(filter = ActivityFilter(search = "v"))
        show(filtered)
        scrollTo("video:v8")
        val before = top("video:v8")
        update(base)
        assertEquals(before, top("video:v8"), 0.5f)
        assertNoPill()
    }

    @Test
    fun thePillIsLabelledWithTheCount() {
        compose.setContent { NewItemsPill(12, onClick = {}) }
        compose.onNodeWithTag(NEW_ITEMS_PILL_TAG).assertTextEquals("↑ 12 new")
    }
}
