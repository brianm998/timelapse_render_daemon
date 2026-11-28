// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.ListLogRequest
import com.timelapserender.trd.v1.LogEntry
import com.timelapserender.trd.v1.LogLevel
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import kotlin.time.Duration.Companion.seconds

class LogModelTest {
    private val now = Instant.parse("2025-11-27T15:00:00Z").toEpochMilli()

    private fun entry(seq: Long, level: LogLevel = LogLevel.LOG_INFO, message: String = "message $seq") =
        LogEntry.newBuilder().setSeq(seq).setLevel(level).setSubject("subject").setMessage(message)
            .setAtMs(Instant.parse("2025-11-27T14:02:00Z").toEpochMilli()).build()

    private fun model(env: Env, scope: CoroutineScope, pageSize: Int = 200) =
        LogModel(env.client, scope, clock = { now }, zone = ZoneId.of("UTC"), pageSize = pageSize)

    @Test
    fun firstPageIsNewestFirstWithTheDefaultInfoFilter() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, LogLevel.LOG_DEBUG), entry(2), entry(3), entry(4, LogLevel.LOG_ERROR))
            val m = model(env, backgroundScope)
            m.start()
            val ui = m.state.first { !it.loading }
            assertEquals(listOf("4", "3", "2"), ui.rows.map { it.key })
            assertEquals(LogLevel.LOG_INFO, ui.minLevel)
            assertEquals("14:02", ui.rows[0].timeText)
            assertEquals(LogLevel.LOG_INFO, env.fake.requestsOf<ListLogRequest>().first().minLevel)
        }
    }

    @Test
    fun liveEntriesArriveAtTheTop() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1), entry(2))
            val m = model(env, backgroundScope)
            m.start()
            m.state.first { !it.loading && it.rows.size == 2 }
            env.fake.logEntries = env.fake.logEntries + entry(3) + entry(4)
            val ui = m.state.first { it.rows.size == 4 }
            assertEquals(listOf("4", "3", "2", "1"), ui.rows.map { it.key })
        }
    }

    @Test
    fun olderPagesAreLoadedOnDemand() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = (1L..5L).map { entry(it) }
            val m = model(env, backgroundScope, pageSize = 2)
            m.start()
            val first = m.state.first { !it.loading }
            assertEquals(listOf("5", "4"), first.rows.map { it.key })
            assertTrue(first.hasOlder)
            m.loadOlder()
            val second = m.state.first { it.rows.size == 4 }
            assertEquals(listOf("5", "4", "3", "2"), second.rows.map { it.key })
            assertEquals(4L, env.fake.requestsOf<ListLogRequest>().last { it.beforeSeq != 0L }.beforeSeq)
            m.loadOlder()
            val third = m.state.first { it.rows.size == 5 }
            assertFalse(third.hasOlder)
        }
    }

    @Test
    fun levelFilterReloadsAndSearchFiltersLocally() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.logEntries = listOf(entry(1, LogLevel.LOG_DEBUG, "needle in debug"), entry(2, message = "plain"), entry(3, message = "a Needle"))
            val m = model(env, backgroundScope)
            m.start()
            m.state.first { !it.loading && it.rows.size == 2 }
            m.setSearch("needle")
            assertEquals(listOf("3"), m.state.value.rows.map { it.key })
            m.setLevel(LogLevel.LOG_DEBUG)
            val ui = m.state.first { it.minLevel == LogLevel.LOG_DEBUG && !it.loading && it.rows.size == 2 }
            assertEquals(listOf("3", "1"), ui.rows.map { it.key })
            m.setSearch("")
            assertEquals(3, m.state.value.rows.size)
        }
    }

    @Test
    fun aFailedFirstPageIsReported() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            env.fake.failWith["ListLog"] = Status.INTERNAL.withDescription("log unavailable")
            val m = model(env, backgroundScope)
            m.start()
            val ui = m.state.first { it.error != null }
            assertEquals("log unavailable", ui.error)
            assertFalse(ui.loading)
        }
    }
}
