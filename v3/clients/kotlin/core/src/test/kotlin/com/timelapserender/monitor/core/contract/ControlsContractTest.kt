// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.CommandException
import com.timelapserender.monitor.core.state.Commands
import com.timelapserender.monitor.core.state.DetailsModel
import com.timelapserender.monitor.core.state.DetailsState
import com.timelapserender.monitor.core.state.DetailsTarget
import com.timelapserender.trd.v1.FailureKind
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantStatus
import com.timelapserender.trd.v1.VariantStatus.VARIANT_FAILED
import com.timelapserender.trd.v1.VariantStatus.VARIANT_PAUSED
import com.timelapserender.trd.v1.VariantStatus.VARIANT_QUEUED
import com.timelapserender.trd.v1.VariantStatus.VARIANT_RUNNING
import com.timelapserender.trd.v1.VariantStatus.VARIANT_SUSPENDED
import io.grpc.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Every control of overview §6.9, on sequence B's renders held by `wait_for_file`. */
class ControlsContractTest {
    private val b = sequenceName('b')

    /** B's three renders, held; with fewer than three [slots] the later variants stay queued. */
    private fun held(slots: Int, body: suspend (TrdProcess, TrdClient, Commands) -> Unit) =
        TrdProcess.create().use { trd ->
            trd.keepSequences("b", renders = slots)
            trd.script(trd.held())
            trd.start()
            trd.test {
                val client = trd.connected()
                body(trd, client, Commands(client))
            }
        }

    private suspend fun TrdClient.awaitB(what: String, vararg statuses: VariantStatus): Snapshot =
        awaitSnapshot(what) { snapshot ->
            listOf(PRORES_MQ, H264, PRORES_UHQ).map { snapshot.statusOf('b', it) } == statuses.toList()
        }

    private fun Snapshot.idOf(key: String): String = variant('b', key)!!.renderId

    private fun Result<Unit>.assertRefused(code: Status.Code) {
        val failure = exceptionOrNull()
        assertTrue("$this", failure is CommandException)
        assertEquals((failure as CommandException).message, code, failure.code)
    }

    @Test
    fun suspendResumeAndStopActOnOneRender() = held(slots = 2) { trd, client, commands ->
        val start = client.awaitB("two running", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_QUEUED)
        val mq = start.idOf(PRORES_MQ)
        commands.resumeRender(mq).assertRefused(Status.Code.FAILED_PRECONDITION)
        commands.stopRender("no-such-render").assertRefused(Status.Code.NOT_FOUND)

        commands.suspendRender(mq).getOrThrow()
        val suspended = client.awaitB("suspended", VARIANT_SUSPENDED, VARIANT_RUNNING, VARIANT_QUEUED)
        assertEquals(1, suspended.counts.suspended)
        assertEquals(1, suspended.counts.rendering)
        assertEquals(mq, suspended.idOf(PRORES_MQ))

        commands.resumeRender(mq).getOrThrow()
        client.awaitB("resumed", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_QUEUED)
        assertEquals("suspend and resume persist nothing in config.json", emptyList<Pair<String, String>>(), trd.pausedVariants())

        // Stop on a queued render.
        commands.stopRender(start.idOf(PRORES_UHQ)).getOrThrow()
        client.awaitB("queued stopped", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_PAUSED)
        assertEquals(listOf(b to PRORES_UHQ), trd.pausedVariants())

        // Stop on a suspended render.
        commands.suspendRender(mq).getOrThrow()
        client.awaitB("suspended again", VARIANT_SUSPENDED, VARIANT_RUNNING, VARIANT_PAUSED)
        commands.stopRender(mq).getOrThrow()
        client.awaitB("suspended stopped", VARIANT_PAUSED, VARIANT_RUNNING, VARIANT_PAUSED)

        // Stop on a running render.
        commands.stopRender(start.idOf(H264)).getOrThrow()
        val stopped = client.awaitB("all stopped", VARIANT_PAUSED, VARIANT_PAUSED, VARIANT_PAUSED)
        assertEquals(Stage.STAGE_BLOCKED, stopped.card('b')?.stage)
        assertEquals(0, stopped.counts.rendering + stopped.counts.suspended + stopped.counts.queued)
        assertEquals(0, stopped.failuresCount)
        assertEquals(setOf(b to PRORES_MQ, b to H264, b to PRORES_UHQ), trd.pausedVariants().toSet())

        // Retry also applies to a paused variant: it clears the pause and queues it again.
        commands.retryVariant(b, H264).getOrThrow()
        client.awaitB("retried", VARIANT_PAUSED, VARIANT_RUNNING, VARIANT_PAUSED)
        assertEquals(setOf(b to PRORES_MQ, b to PRORES_UHQ), trd.pausedVariants().toSet())
    }

    @Test
    fun pauseAndResumeAVariant() = held(slots = 3) { trd, client, commands ->
        val start = client.awaitB("running", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_RUNNING)

        commands.pauseVariant(b, H264).getOrThrow()
        client.awaitB("variant paused", VARIANT_RUNNING, VARIANT_PAUSED, VARIANT_RUNNING)
        assertEquals(listOf(b to H264), trd.pausedVariants())
        commands.pauseVariant(b, H264).assertRefused(Status.Code.FAILED_PRECONDITION)

        commands.resumeVariant(b, H264).getOrThrow()
        val resumed = client.awaitB("variant resumed", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_RUNNING)
        assertNotEquals("a new render", start.idOf(H264), resumed.idOf(H264))
        assertEquals(emptyList<Pair<String, String>>(), trd.pausedVariants())
    }

    @Test
    fun pauseAndResumeASequence() = held(slots = 3) { trd, client, commands ->
        val start = client.awaitB("running", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_RUNNING)

        commands.pauseSequence(b).getOrThrow()
        val paused = client.awaitB("sequence paused", VARIANT_PAUSED, VARIANT_PAUSED, VARIANT_PAUSED)
        assertEquals(Stage.STAGE_PAUSED, paused.card('b')?.stage)
        assertTrue(paused.card('b')!!.paused)
        assertEquals(1, paused.counts.pausedSequences)
        assertEquals(0, paused.counts.rendering)
        assertEquals(listOf(b), trd.pausedSequences())

        commands.resumeSequence(b).getOrThrow()
        val resumed = client.awaitB("sequence resumed", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_RUNNING)
        assertFalse(resumed.card('b')!!.paused)
        assertNotEquals(start.idOf(PRORES_MQ), resumed.idOf(PRORES_MQ))
        assertEquals(emptyList<String>(), trd.pausedSequences())
    }

    @Test
    fun pauseAllRenderingSuspendsAndResumesOnlyWhatItSuspended() = held(slots = 2) { trd, client, commands ->
        val start = client.awaitB("two running", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_QUEUED)
        val mq = start.idOf(PRORES_MQ)
        commands.suspendRender(mq).getOrThrow()
        val before = trd.config()

        commands.setGlobalPause(true).getOrThrow()
        val paused = client.awaitB("all paused", VARIANT_SUSPENDED, VARIANT_SUSPENDED, VARIANT_QUEUED)
        assertTrue(paused.globalPause)
        assertEquals(Phase.PHASE_PAUSED, paused.phase)

        commands.setGlobalPause(false).getOrThrow()
        val resumed = client.awaitB("rendering resumed", VARIANT_SUSPENDED, VARIANT_RUNNING, VARIANT_QUEUED)
        assertFalse(resumed.globalPause)
        assertEquals("the global pause is not in config.json", before, trd.config())

        trd.release()
        client.awaitSnapshot("the others finish") {
            it.statusOf('b', H264) == VariantStatus.VARIANT_DONE &&
                it.statusOf('b', PRORES_UHQ) == VariantStatus.VARIANT_DONE &&
                it.statusOf('b', PRORES_MQ) == VARIANT_SUSPENDED
        }
        commands.resumeRender(mq).getOrThrow()
        client.awaitSnapshot("B is finished") { it.card('b') == null && it.recentlyRenderedTotal == 3 }
    }

    @Test
    fun retryAndDismissActOnFailures() = TrdProcess.create().use { trd ->
        trd.keepSequences("b")
        trd.script(
            mapOf("exit_code" to 1, "write_output" to false, "stderr_lines" to listOf("Unknown encoder 'broken'")),
        )
        trd.start()
        trd.test {
            val client = trd.connected()
            val commands = Commands(client)
            val failed = client.awaitSnapshot("three failures") { it.failuresCount == 3 && it.counts.rendering == 0 }
            client.awaitB("failed rows", VARIANT_FAILED, VARIANT_FAILED, VARIANT_FAILED)
            assertEquals(3, failed.counts.failures)
            val card = failed.failuresList.single { it.variantKey == PRORES_MQ }
            assertEquals(b, card.sequenceName)
            assertEquals(FailureKind.FAILURE_FFMPEG_REJECTED_ARGUMENTS, card.kind)
            assertTrue(card.hasExitCode)
            assertEquals(1, card.exitCode)

            val details = DetailsModel(client, trd.scope)
            details.load(DetailsTarget.Failure(card.id))
            val state = details.state.reach("the failure's details") { it !is DetailsState.Loading }
            assertTrue(state.toString(), state is DetailsState.Loaded)
            val loaded = (state as DetailsState.Loaded).details
            assertEquals(VARIANT_FAILED, loaded.status)
            assertTrue(loaded.logTailList.toString(), loaded.logTailList.any { "Unknown encoder" in it })

            // From here on a render that starts is held.
            trd.script(trd.held())

            commands.retryVariant(b, PRORES_MQ).getOrThrow()
            val retried = client.awaitB("retried", VARIANT_RUNNING, VARIANT_FAILED, VARIANT_FAILED)
            assertEquals(setOf(H264, PRORES_UHQ), retried.failuresList.map { it.variantKey }.toSet())

            val h264 = retried.failuresList.single { it.variantKey == H264 }
            commands.dismissFailure(h264.id).getOrThrow()
            val dismissed = client.awaitB("dismissed", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_FAILED)
            assertEquals(listOf(PRORES_UHQ), dismissed.failuresList.map { it.variantKey })
            commands.dismissFailure(h264.id).assertRefused(Status.Code.NOT_FOUND)

            commands.dismissAllFailures().getOrThrow()
            val clear = client.awaitB("all dismissed", VARIANT_RUNNING, VARIANT_RUNNING, VARIANT_RUNNING)
            assertEquals(0, clear.failuresCount)
            assertEquals(0, clear.counts.failures)
            assertEquals(emptyList<Pair<String, String>>(), trd.pausedVariants())
        }
    }

    @Test
    fun rescanNowIsAppliedAndALaterScanSeesNewFrames() = TrdProcess.create().use { trd ->
        // The daemon scans for a RescanNow at its next tick, one scan interval after the control,
        // so this asserts that a later scan happens and not how soon.
        trd.keepSequences("a") { it["scan_interval_seconds"] = 5.0 }
        trd.start()
        trd.test {
            val client = trd.connected()
            val before = client.awaitSnapshot("the first scan") { it.lastScanCompletedMs > 0 }
            assertEquals(0, before.inProgressCount)

            trd.exportFrames('a', 1..10)
            Commands(client).rescanNow().getOrThrow()
            val after = client.awaitSnapshot("A is seen") { it.card('a') != null }
            assertTrue(after.lastScanCompletedMs > before.lastScanCompletedMs)
            assertEquals(10, after.card('a')?.exportedFrames)
            assertTrue(client.listLog().entriesList.any { it.message == "rescan now" })
        }
    }
}
