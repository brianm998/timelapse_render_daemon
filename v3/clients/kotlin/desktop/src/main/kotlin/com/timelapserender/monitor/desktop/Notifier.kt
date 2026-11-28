// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.ui.window.Notification
import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.ServerEvent
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.trd.v1.EventKind
import com.timelapserender.trd.v1.FailureKind
import com.timelapserender.trd.v1.Snapshot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull

/** How long a failure's card is waited for in the watched snapshots before the server is asked. */
const val FAILURE_CARD_WAIT_MS = 2000L

const val UNKNOWN_FAILURE = "unknown failure"

/** The labels of spec 1's failure kinds, for a log entry whose card is already gone. */
private val KIND_LABELS = mapOf(
    FailureKind.FAILURE_INPUT_FRAMES_NOT_FOUND to "no input frames",
    FailureKind.FAILURE_INPUT_UNREADABLE to "input unreadable",
    FailureKind.FAILURE_SOURCE_FRAME_UNREADABLE to "bad source frame",
    FailureKind.FAILURE_FFMPEG_REJECTED_ARGUMENTS to "ffmpeg rejected arguments",
    FailureKind.FAILURE_OUTPUT_NOT_WRITABLE to "output not writable",
    FailureKind.FAILURE_DISK_FULL to "disk full",
    FailureKind.FAILURE_TERMINATED_BY_SIGNAL to "killed",
    FailureKind.FAILURE_PRODUCED_NO_OUTPUT to "no output produced",
    FailureKind.FAILURE_STRUCTURAL_CORRUPTION to "corrupt output",
    FailureKind.FAILURE_LAUNCH_FAILED to "ffmpeg would not start",
    FailureKind.FAILURE_NOT_QUEUEABLE to "cannot start render",
    FailureKind.FAILURE_UNKNOWN to "render failed",
)

/** The label for a failure's log message, which starts `FAILURE_X: `. */
fun kindLabelOfMessage(message: String): String {
    val code = message.substringBefore(':')
    return KIND_LABELS.entries.firstOrNull { it.key.name == code }?.value ?: UNKNOWN_FAILURE
}

/**
 * The kind label of a failure event: that of its card, waited for in the watched snapshots (the
 * snapshot may arrive after the log entry), then asked of the server, then read from the message.
 */
suspend fun failureKindLabel(
    event: ServerEvent,
    repository: SnapshotRepository,
    client: TrdClient,
    waitMs: Long = FAILURE_CARD_WAIT_MS,
): String {
    fun Snapshot.label() = failuresList.firstOrNull { it.id == event.failureId }?.kindLabel?.ifEmpty { null }
    if (event.failureId.isNotEmpty()) {
        val watched = withTimeoutOrNull(waitMs) {
            repository.state.first { it.snapshot?.label() != null }.snapshot?.label()
        }
        if (watched != null) return watched
        try {
            client.getSnapshot().label()?.let { return it }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // The message still names the kind.
        }
    }
    return kindLabelOfMessage(event.message)
}

/**
 * Turns server events into notifications with the overview §6.8 texts, as the preferences allow.
 * A failed lookup or send is logged and dropped: it must not end the event stream.
 */
class Notifier(
    private val preferences: AppPreferences,
    private val send: (Notification) -> Unit,
    private val kindLabel: suspend (ServerEvent) -> String,
) {
    suspend fun handle(event: ServerEvent) {
        try {
            val text = when (event.kind) {
                EventKind.EVENT_SEQUENCE_FINISHED -> if (preferences.notifyFinished) finished(event) else null
                EventKind.EVENT_RENDER_FAILED -> if (preferences.notifyFailed) failed(event) else null
                else -> null
            } ?: return
            send(Notification(WINDOW_TITLE, text))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            System.err.println("Notification failed: ${e.message ?: e}")
        }
    }

    private fun finished(event: ServerEvent): String {
        val videos = if (event.videoCount == 1) "1 video" else "${Format.count(event.videoCount.toLong())} videos"
        return "${event.sequenceName} rendered — $videos (${Format.bytes(event.totalBytes)})"
    }

    private suspend fun failed(event: ServerEvent): String {
        val file = event.outputFilename.ifEmpty { event.sequenceName }
        return "Render failed: $file — ${kindLabel(event)}"
    }
}
