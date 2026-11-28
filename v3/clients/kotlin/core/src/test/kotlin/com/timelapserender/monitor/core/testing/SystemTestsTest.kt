// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.testing

import org.junit.Assert.assertEquals
import org.junit.Test

class SystemTestsTest {
    private fun allowed(ci: String?, sw: String?): Boolean =
        SystemTests.allowed({ name -> mapOf("CI" to ci, "SW" to sw)[name] }, "SW")

    @Test
    fun trueOnlyForCiTrueAndSwitchOne() {
        assertEquals(true, allowed("true", "1"))
    }

    @Test
    fun ciValuesOtherThanTrueAreFalse() {
        for (ci in listOf(null, "", "false", "1", "TRUE")) assertEquals("CI=$ci", false, allowed(ci, "1"))
    }

    @Test
    fun switchValuesOtherThanOneAreFalse() {
        for (sw in listOf(null, "", "0", "true")) assertEquals("SW=$sw", false, allowed("true", sw))
    }
}
