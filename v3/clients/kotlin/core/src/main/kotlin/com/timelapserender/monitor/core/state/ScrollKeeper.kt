// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

sealed interface ScrollDecision {
    /** The list is at the top and stays there (S1, S2). */
    data object StayAtTop : ScrollDecision

    /** Keep [key] where it is; [insertedAbove] rows were inserted above the viewport since the user left the top (S3). */
    data class KeepAnchor(val key: String, val insertedAbove: Int) : ScrollDecision
}

/**
 * Overview §6.2 S1–S7 as a pure decision, used by both UIs. Row keys (S4) are stable across progress
 * updates (S5), so an update that adds nothing above the anchor changes nothing. The keeper never
 * asks for the bottom (S6). [reset] on first display, reconnect and server switch puts the list at
 * the top again (S1). S7 is an ordering rule of the presenter's output, not of this class.
 */
class ScrollKeeper {
    private var insertedAbove = 0

    fun reset() {
        insertedAbove = 0
    }

    /**
     * @param prevKeys the keys of the list before the update, top to bottom
     * @param newKeys the keys after it
     * @param firstVisibleKey the key of the first row in the viewport, or null when unknown
     * @param atTop whether the first row is visible
     * @param counts whether a new key above the anchor counts towards the pill; a list that also
     *   holds headers, or that changed because the filter did, leaves those out
     */
    fun decide(
        prevKeys: List<String>,
        newKeys: List<String>,
        firstVisibleKey: String?,
        atTop: Boolean,
        counts: (String) -> Boolean = { true },
    ): ScrollDecision {
        if (atTop || firstVisibleKey == null || newKeys.isEmpty()) {
            insertedAbove = 0
            return ScrollDecision.StayAtTop
        }
        val anchor = if (firstVisibleKey in newKeys) firstVisibleKey else survivingNeighbour(prevKeys, newKeys, firstVisibleKey)
            ?: return ScrollDecision.StayAtTop
        val known = prevKeys.toHashSet()
        insertedAbove += newKeys.takeWhile { it != anchor }.count { it !in known && counts(it) }
        return ScrollDecision.KeepAnchor(anchor, insertedAbove)
    }

    /** The nearest key around [removed] in the old list that is still in the new one (below first, then above). */
    private fun survivingNeighbour(prevKeys: List<String>, newKeys: List<String>, removed: String): String? {
        val index = prevKeys.indexOf(removed)
        if (index < 0) return null
        val alive = newKeys.toHashSet()
        for (distance in 1 until prevKeys.size) {
            prevKeys.getOrNull(index + distance)?.takeIf { it in alive }?.let { return it }
            prevKeys.getOrNull(index - distance)?.takeIf { it in alive }?.let { return it }
        }
        return null
    }
}
