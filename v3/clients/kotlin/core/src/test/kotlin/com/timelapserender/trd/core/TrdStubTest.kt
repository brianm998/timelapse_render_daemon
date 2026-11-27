// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.trd.core

import com.timelapserender.trd.v1.TrdServiceGrpcKt
import org.junit.Assert.assertEquals
import org.junit.Test

class TrdStubTest {
    @Test
    fun generatedCoroutineStubExists() {
        val stub = TrdServiceGrpcKt.TrdServiceCoroutineStub::class
        assertEquals("TrdServiceCoroutineStub", stub.simpleName)
    }
}
