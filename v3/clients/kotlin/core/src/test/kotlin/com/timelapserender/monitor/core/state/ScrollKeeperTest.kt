// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import org.junit.Assert.assertEquals
import org.junit.Test

class ScrollKeeperTest {
    private fun keys(vararg k: String) = k.toList()

    @Test
    fun s1_firstDisplayAndResetAreAtTheTop() {
        val keeper = ScrollKeeper()
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(emptyList(), keys("a", "b"), null, true))
        // scrolled away, then a reconnect or server switch resets
        keeper.decide(keys("a", "b", "c"), keys("x", "a", "b", "c"), "b", false)
        keeper.reset()
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("a"), keys("a"), null, true))
        // the pill counts from zero again
        assertEquals(
            ScrollDecision.KeepAnchor("b", 1),
            keeper.decide(keys("a", "b"), keys("y", "a", "b"), "b", false),
        )
    }

    @Test
    fun s2_atTopRowsInsertedMovedOrRemovedStayAtTop() {
        val keeper = ScrollKeeper()
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("a", "b"), keys("n", "a", "b"), "a", true))
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("n", "a", "b"), keys("b", "n", "a"), "n", true))
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("b", "n", "a"), keys("n", "a"), "b", true))
    }

    @Test
    fun s3_scrolledDownTheFirstVisibleRowIsAnchoredAndInsertionsAboveAreCounted() {
        val keeper = ScrollKeeper()
        assertEquals(
            ScrollDecision.KeepAnchor("b", 2),
            keeper.decide(keys("a", "b", "c"), keys("x", "y", "a", "b", "c"), "b", false),
        )
        // the count accumulates while the user stays away from the top
        assertEquals(
            ScrollDecision.KeepAnchor("b", 3),
            keeper.decide(keys("x", "y", "a", "b", "c"), keys("z", "x", "y", "a", "b", "c"), "b", false),
        )
        // rows inserted below the anchor and rows that merely moved are not counted
        assertEquals(
            ScrollDecision.KeepAnchor("b", 3),
            keeper.decide(keys("z", "x", "y", "a", "b", "c"), keys("x", "z", "y", "a", "b", "c", "d"), "b", false),
        )
        // back at the top the pill disappears
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("x", "z"), keys("x", "z"), "x", true))
        assertEquals(
            ScrollDecision.KeepAnchor("b", 1),
            keeper.decide(keys("a", "b"), keys("q", "a", "b"), "b", false),
        )
    }

    @Test
    fun s3_aRemovedAnchorFallsBackToTheNearestSurvivor() {
        val keeper = ScrollKeeper()
        assertEquals(
            ScrollDecision.KeepAnchor("c", 0),
            keeper.decide(keys("a", "b", "c"), keys("a", "c"), "b", false),
        )
        assertEquals(
            ScrollDecision.KeepAnchor("a", 0),
            keeper.decide(keys("a", "b", "c"), keys("a"), "b", false),
        )
    }

    @Test
    fun s4s5_progressUpdatesKeepTheKeysAndNeverMoveTheList() {
        val keeper = ScrollKeeper()
        val rows = keys("p1", "card/1", "card/1/v1", "video/9")
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(rows, rows, "p1", true))
        assertEquals(ScrollDecision.KeepAnchor("card/1/v1", 0), keeper.decide(rows, rows, "card/1/v1", false))
        assertEquals(ScrollDecision.KeepAnchor("card/1/v1", 0), keeper.decide(rows, rows, "card/1/v1", false))
    }

    @Test
    fun s6_nothingScrollsToTheBottom() {
        val keeper = ScrollKeeper()
        // the anchor and every neighbour vanish: stay at the top rather than jump to the last row
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("a", "b", "c"), keys("d", "e"), "b", false))
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("a", "b"), emptyList(), "b", false))
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(keys("a", "b"), keys("a", "b"), null, false))
    }

    @Test
    fun s7_aFinishedSequenceReplacedByItsVideosBelowTheRemainingCards() {
        val keeper = ScrollKeeper()
        val before = keys("card/x", "card/y", "video/1")
        val after = keys("card/x", "video/new", "video/1")
        // at the top: stays; scrolled to the older video: the new video is inserted above it and counted once
        assertEquals(ScrollDecision.StayAtTop, keeper.decide(before, after, "card/x", true))
        assertEquals(ScrollDecision.KeepAnchor("video/1", 1), keeper.decide(before, after, "video/1", false))
    }

    @Test
    fun onlyTheKeysThatCountAreAddedToThePill() {
        val keeper = ScrollKeeper()
        assertEquals(
            ScrollDecision.KeepAnchor("b", 1),
            keeper.decide(keys("a", "b"), keys("header", "x", "a", "b"), "b", false) { it != "header" },
        )
        // A later update adds to what was counted; one that counts nothing leaves it.
        assertEquals(
            ScrollDecision.KeepAnchor("b", 1),
            keeper.decide(keys("header", "x", "a", "b"), keys("y", "header", "x", "a", "b"), "b", false) { false },
        )
    }
}
