// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.format

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToLong

/** The §5.5 display rules, identical on every Kotlin client. Pure: the clock and zone are parameters. */
object Format {
    private val LOCALE: Locale = Locale.ENGLISH
    private val UNITS = listOf("B", "KB", "MB", "GB", "TB")

    private val HH_MM = DateTimeFormatter.ofPattern("HH:mm", LOCALE)
    private val MONTH_DAY = DateTimeFormatter.ofPattern("MMM d", LOCALE)
    private val MONTH_DAY_YEAR = DateTimeFormatter.ofPattern("MMM d, yyyy", LOCALE)
    private val MONTH_DAY_TIME = DateTimeFormatter.ofPattern("MMM d HH:mm", LOCALE)

    /** Binary units; one decimal below 100, none from 100 (`512 B`, `12.4 GB`, `231 MB`). */
    fun bytes(count: Long): String {
        if (count < 0) return "—"
        var index = 0
        var value = count.toDouble()
        while (index < UNITS.size - 1 && value >= 1024.0) {
            value /= 1024.0
            index++
        }
        if (index == 0) return "$count B"
        // Rounding can reach the next unit (1023.96 KB shows as 1024 KB): promote it.
        if (index < UNITS.size - 1 && roundForDisplay(value) >= 1024.0) {
            value /= 1024.0
            index++
        }
        return if (value < 99.95) {
            String.format(LOCALE, "%.1f %s", value, UNITS[index])
        } else {
            "${value.roundToLong()} ${UNITS[index]}"
        }
    }

    private fun roundForDisplay(value: Double): Double =
        if (value < 99.95) Math.round(value * 10) / 10.0 else value.roundToLong().toDouble()

    /** `1,234 / 1,500 frames`. */
    fun frames(done: Long, total: Long): String = "${count(done)} / ${count(total)} frames"

    /** A count with the thousands separator. */
    fun count(value: Long): String = String.format(LOCALE, "%,d", value)

    /** `50.0 s` below a minute, `3:20` below an hour, `1:02:05` from an hour. */
    fun videoDuration(seconds: Double): String {
        if (seconds < 0 || seconds.isNaN()) return "—"
        if (seconds < 59.95) return String.format(LOCALE, "%.1f s", seconds)
        val total = seconds.roundToLong()
        val h = total / 3600
        val m = (total % 3600) / 60
        val s = total % 60
        return if (h > 0) String.format(LOCALE, "%d:%02d:%02d", h, m, s) else String.format(LOCALE, "%d:%02d", m, s)
    }

    /** `~40 s`, `~6 min`, `~1 h 12 min`; null when unknown (negative). */
    fun eta(seconds: Long): String? = when {
        seconds < 0 -> null
        seconds < 60 -> "~$seconds s"
        seconds < 3600 -> "~${seconds / 60} min"
        else -> "~${seconds / 3600} h ${(seconds % 3600) / 60} min"
    }

    /** `41 frames/min`; null when unknown (zero or less). */
    fun exportRate(framesPerMinute: Double): String? =
        if (framesPerMinute <= 0.0 || framesPerMinute.isNaN()) null else "${framesPerMinute.roundToLong()} frames/min"

    /** `just now`, `12 min ago`, `3 h ago`, `yesterday`, `Nov 20`, `Nov 20, 2025`. */
    fun relativeTime(thenMs: Long, nowMs: Long, zone: ZoneId): String {
        val diffSeconds = (nowMs - thenMs) / 1000
        if (diffSeconds < 60) return "just now"
        if (diffSeconds < 3600) return "${diffSeconds / 60} min ago"
        if (diffSeconds < 86_400) return "${diffSeconds / 3600} h ago"
        val then = date(thenMs, zone)
        val today = date(nowMs, zone)
        return when {
            then == today.minusDays(1) -> "yesterday"
            then.year == today.year -> MONTH_DAY.format(then)
            else -> MONTH_DAY_YEAR.format(then)
        }
    }

    /** `14:02` today, else `Nov 20 14:02`. */
    fun absoluteTime(thenMs: Long, nowMs: Long, zone: ZoneId): String {
        val then = Instant.ofEpochMilli(thenMs).atZone(zone)
        return if (then.toLocalDate() == date(nowMs, zone)) HH_MM.format(then) else MONTH_DAY_TIME.format(then)
    }

    /** `ProResMQ · 2160p · 30 fps`. */
    fun variantLabel(codec: String, resolutionLabel: String, frameRate: Int): String =
        "$codec · $resolutionLabel · $frameRate fps"

    /** Minutes and seconds, `4:59`, for countdowns. */
    fun countdown(seconds: Long): String {
        val s = maxOf(0, seconds)
        return String.format(LOCALE, "%d:%02d", s / 60, s % 60)
    }

    private fun date(ms: Long, zone: ZoneId): LocalDate = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
}
