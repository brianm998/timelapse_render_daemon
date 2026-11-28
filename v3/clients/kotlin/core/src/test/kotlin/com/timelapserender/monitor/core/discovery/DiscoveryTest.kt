// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.discovery

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DiscoveryTest {
    private val valid = mapOf("id" to "srv-1", "name" to "Studio", "fp" to "ab12", "port" to "50071", "ver" to "3.0.0", "api" to "1")

    @Test
    fun validRecordParsesWithEveryField() {
        assertEquals(
            DiscoveredServer("srv-1", "Studio", "10.0.0.5", 50071, "ab12", "3.0.0", 1),
            parseTxt(valid, "10.0.0.5"),
        )
    }

    @Test
    fun eachMissingRequiredFieldIsRejected() {
        for (key in listOf("id", "fp", "port", "api")) assertNull(key, parseTxt(valid - key))
    }

    @Test
    fun otherApiVersionsAndBadPortsAreRejected() {
        assertNull(parseTxt(valid + ("api" to "2")))
        assertNull(parseTxt(valid + ("port" to "abc")))
        assertNull(parseTxt(valid + ("port" to "0")))
    }

    @Test
    fun nameAndVersionFallBack() {
        val s = parseTxt(valid - "name" - "ver")!!
        assertEquals("srv-1", s.name)
        assertEquals("", s.version)
    }
}
