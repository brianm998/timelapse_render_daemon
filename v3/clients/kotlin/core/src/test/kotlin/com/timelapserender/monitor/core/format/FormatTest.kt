// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.format

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.time.Instant
import java.time.ZoneId

class FormatTest {
    private val utc: ZoneId = ZoneId.of("UTC")
    private val now = ms("2025-11-27T15:00:00Z")

    private fun ms(iso: String) = Instant.parse(iso).toEpochMilli()

    private fun gb(v: Double) = (v * 1024 * 1024 * 1024).toLong()

    @Test
    fun bytesTableExamples() {
        assertEquals("512 B", Format.bytes(512))
        assertEquals("12.4 GB", Format.bytes(gb(12.4)))
        assertEquals("231 MB", Format.bytes(231L * 1024 * 1024))
    }

    @Test
    fun bytesUnitsAndBoundaries() {
        assertEquals("0 B", Format.bytes(0))
        assertEquals("1023 B", Format.bytes(1023))
        assertEquals("1.0 KB", Format.bytes(1024))
        assertEquals("1.5 KB", Format.bytes(1536))
        assertEquals("99.9 MB", Format.bytes((99.94 * 1024 * 1024).toLong()))
        assertEquals("100 MB", Format.bytes((99.96 * 1024 * 1024).toLong()))
        assertEquals("100 MB", Format.bytes(100L * 1024 * 1024))
        assertEquals("1.0 MB", Format.bytes((1023.96 * 1024).toLong()))
        assertEquals("2048 TB", Format.bytes(2048L * 1024 * 1024 * 1024 * 1024))
        assertEquals("—", Format.bytes(-1))
    }

    @Test
    fun frameCountsUseThousandsSeparator() {
        assertEquals("1,234 / 1,500 frames", Format.frames(1234, 1500))
        assertEquals("0 / 0 frames", Format.frames(0, 0))
        assertEquals("1,234,567 / 2,000,000 frames", Format.frames(1_234_567, 2_000_000))
    }

    @Test
    fun videoDurationTableExamples() {
        assertEquals("50.0 s", Format.videoDuration(50.0))
        assertEquals("3:20", Format.videoDuration(200.0))
        assertEquals("1:02:05", Format.videoDuration(3725.0))
    }

    @Test
    fun videoDurationBoundaries() {
        assertEquals("0.0 s", Format.videoDuration(0.0))
        assertEquals("59.9 s", Format.videoDuration(59.94))
        assertEquals("1:00", Format.videoDuration(59.96))
        assertEquals("1:00", Format.videoDuration(60.0))
        assertEquals("59:59", Format.videoDuration(3599.0))
        assertEquals("1:00:00", Format.videoDuration(3599.6))
        assertEquals("—", Format.videoDuration(-1.0))
    }

    @Test
    fun etaTableExamples() {
        assertEquals("~40 s", Format.eta(40))
        assertEquals("~6 min", Format.eta(360))
        assertEquals("~1 h 12 min", Format.eta(4320))
    }

    @Test
    fun etaBoundariesAndUnknown() {
        assertEquals("~0 s", Format.eta(0))
        assertEquals("~59 s", Format.eta(59))
        assertEquals("~1 min", Format.eta(60))
        assertEquals("~59 min", Format.eta(3599))
        assertEquals("~1 h 0 min", Format.eta(3600))
        assertNull(Format.eta(-1))
    }

    @Test
    fun exportRateTableExample() {
        assertEquals("41 frames/min", Format.exportRate(41.0))
        assertEquals("41 frames/min", Format.exportRate(40.6))
        assertNull(Format.exportRate(0.0))
    }

    private fun rel(then: String, zone: ZoneId = utc, at: Long = now) = Format.relativeTime(ms(then), at, zone)

    @Test
    fun relativeTimeBelowAnHour() {
        assertEquals("just now", rel("2025-11-27T14:59:30Z"))
        assertEquals("just now", rel("2025-11-27T14:59:01Z"))
        assertEquals("1 min ago", rel("2025-11-27T14:59:00Z"))
        assertEquals("12 min ago", rel("2025-11-27T14:48:00Z"))
        assertEquals("59 min ago", rel("2025-11-27T14:00:01Z"))
        assertEquals("just now", rel("2025-11-27T15:00:30Z"))
    }

    @Test
    fun relativeTimeHoursYesterdayAndDates() {
        assertEquals("1 h ago", rel("2025-11-27T14:00:00Z"))
        assertEquals("3 h ago", rel("2025-11-27T12:00:00Z"))
        assertEquals("23 h ago", rel("2025-11-26T15:00:01Z"))
        assertEquals("yesterday", rel("2025-11-26T15:00:00Z"))
        assertEquals("Nov 25", rel("2025-11-25T15:00:00Z"))
        assertEquals("Nov 20", rel("2025-11-20T14:02:00Z"))
        assertEquals("Nov 20, 2024", rel("2024-11-20T14:02:00Z"))
    }

    @Test
    fun relativeTimeUsesTheLocalDate() {
        val la = ZoneId.of("America/Los_Angeles")
        val at = ms("2025-11-27T03:00:00Z") // Nov 26, 19:00 in Los Angeles
        assertEquals("Nov 25", rel("2025-11-25T20:00:00Z", utc, at))
        assertEquals("yesterday", rel("2025-11-25T20:00:00Z", la, at))
    }

    @Test
    fun absoluteTimeTodayAndOtherDays() {
        assertEquals("14:02", Format.absoluteTime(ms("2025-11-27T14:02:00Z"), now, utc))
        assertEquals("Nov 20 14:02", Format.absoluteTime(ms("2025-11-20T14:02:00Z"), now, utc))
        assertEquals("Sep 3 09:05", Format.absoluteTime(ms("2025-09-03T09:05:00Z"), now, utc))
        assertEquals("Nov 26 23:59", Format.absoluteTime(ms("2025-11-26T23:59:00Z"), now, utc))
    }

    @Test
    fun variantLabelTableExample() {
        assertEquals("ProResMQ · 2160p · 30 fps", Format.variantLabel("ProResMQ", "2160p", 30))
    }

    @Test
    fun countdownIsMinutesAndSeconds() {
        assertEquals("4:59", Format.countdown(299))
        assertEquals("5:00", Format.countdown(300))
        assertEquals("0:00", Format.countdown(-3))
    }
}
