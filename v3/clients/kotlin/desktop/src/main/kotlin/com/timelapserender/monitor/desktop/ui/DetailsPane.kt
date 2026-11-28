// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.state.DetailsState
import com.timelapserender.monitor.core.state.DetailsTarget
import com.timelapserender.monitor.desktop.ui.rows.ERROR_RED
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.RenderDetails
import com.timelapserender.trd.v1.RenderedSource
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import java.util.Locale

object DetailsTags {
    const val POSTER = "details-poster"
    const val POSTER_PLACEHOLDER = "details-poster-placeholder"
    const val COMMAND_LINE = "details-command-line"
    const val LOG_TAIL = "details-log-tail"
    const val TOOL_OUTPUT = "details-tool-output"
    const val CONTENT = "details-content"
}

const val COPY_COMMAND_LINE = "Copy command line"
const val COPY_REPORT = "Copy report"
const val REFRESH_DETAILS = "Refresh"

/** The item the Details pane shows (overview §6.3), as the last snapshot has it. */
sealed interface DetailsSelection {
    /** The selected row's Activity list key. */
    val key: String

    /** What `GetRenderDetails` is asked for, or null when the item has no render. */
    val target: DetailsTarget?

    data class Sequence(override val key: String, val card: SequenceCard) : DetailsSelection {
        override val target: DetailsTarget? get() = null
    }

    data class Variant(override val key: String, val card: SequenceCard, val variant: VariantRow) : DetailsSelection {
        override val target: DetailsTarget?
            get() = variant.renderId.takeIf { it.isNotEmpty() }?.let(DetailsTarget::Render)
    }

    data class Failure(override val key: String, val failure: FailureCard) : DetailsSelection {
        override val target: DetailsTarget get() = DetailsTarget.Failure(failure.id)
    }

    data class Video(override val key: String, val video: RenderedVideo) : DetailsSelection {
        override val target: DetailsTarget get() = DetailsTarget.RenderedVideo(video.id)
    }
}

/** A named value. */
data class DetailsField(val label: String, val value: String)

private const val NONE = "—"

private fun text(value: String) = value.ifEmpty { NONE }

private fun yesNo(value: Boolean) = if (value) "yes" else "no"

private fun size(bytes: Long) = if (bytes < 0) "unknown" else Format.bytes(bytes)

private fun eta(seconds: Long) = Format.eta(seconds) ?: "unknown"

private fun decimal(value: Double) = String.format(Locale.ENGLISH, "%.1f", value)

/** `VARIANT_ALREADY_RENDERED` reads "already rendered". */
private fun enumText(name: String, prefix: String) = name.removePrefix(prefix).lowercase().replace('_', ' ')

fun statusText(status: VariantStatus) = enumText(status.name, "VARIANT_")

fun stageText(stage: Stage) = enumText(stage.name, "STAGE_")

private fun sourceText(source: RenderedSource) = enumText(source.name, "RENDERED_SOURCE_")

/** Every field of a sequence card but its variants. [time] formats a time in milliseconds. */
fun sequenceFields(card: SequenceCard, time: (Long) -> String): List<DetailsField> = listOf(
    DetailsField("Id", text(card.id)),
    DetailsField("Name", text(card.name)),
    DetailsField("Export path", text(card.exportPath)),
    DetailsField("Raw sequence", text(card.rawSequenceName)),
    DetailsField("Raw sequence location", text(card.rawSequenceLocation)),
    DetailsField("Raw from backup", yesNo(card.rawFromBackup)),
    DetailsField("Expected frames", Format.count(card.expectedFrames.toLong())),
    DetailsField("Exported frames", Format.count(card.exportedFrames.toLong())),
    DetailsField("Stage", stageText(card.stage)),
    DetailsField("Paused", yesNo(card.paused)),
    DetailsField("Export rate", Format.exportRate(card.exportFramesPerMinute) ?: "unknown"),
    DetailsField("Export time left", eta(card.exportEtaSeconds)),
    DetailsField("Export size", size(card.exportSizeBytes)),
    DetailsField("First seen", moment(card.firstSeenMs, time)),
    DetailsField("Last frame", moment(card.lastFrameMs, time)),
    DetailsField("Note", text(card.note)),
)

private fun moment(ms: Long, time: (Long) -> String) = if (ms == 0L) NONE else time(ms)

fun variantFields(variant: VariantRow): List<DetailsField> = listOf(
    DetailsField("Variant key", text(variant.variantKey)),
    DetailsField("Codec", text(variant.codec)),
    DetailsField("Resolution", text(variant.resolutionLabel)),
    DetailsField("Frame rate", "${variant.frameRate} fps"),
    DetailsField("Output file", text(variant.outputFilename)),
    DetailsField("Output file path", text(variant.outputPath)),
    DetailsField("Status", statusText(variant.status)),
    DetailsField("Render id", text(variant.renderId)),
    DetailsField("Frame", Format.count(variant.frame.toLong())),
    DetailsField("Frames to render", Format.count(variant.totalFrames.toLong())),
    DetailsField("Speed", if (variant.fps > 0) "${decimal(variant.fps)} fps" else "unknown"),
    DetailsField("Time left", eta(variant.etaSeconds)),
    DetailsField("Progress known", yesNo(variant.progressKnown)),
    DetailsField("Adopted process", yesNo(variant.adopted)),
    DetailsField("Size", size(variant.sizeBytes)),
    DetailsField("Message", text(variant.message)),
    DetailsField("Failure id", text(variant.failureId)),
)

/** Everything `GetRenderDetails` returns but the command line and the log, which have places of their own. */
fun renderFields(details: RenderDetails, time: (Long) -> String): List<DetailsField> = listOf(
    DetailsField("Render", text(details.renderId)),
    DetailsField("Sequence", text(details.sequenceName)),
    DetailsField("Sequence export path", text(details.exportPath)),
    DetailsField("Render variant", text(details.variantKey)),
    DetailsField("Render status", statusText(details.status)),
    DetailsField("Input pattern", text(details.inputPattern)),
    DetailsField("Start number", if (details.startNumber < 0) "not passed" else details.startNumber.toString()),
    DetailsField("Total frames", Format.count(details.totalFrames.toLong())),
    DetailsField("Output path", text(details.outputPath)),
    DetailsField("Partial path", text(details.partialPath)),
    DetailsField("Started", moment(details.startedAtMs, time)),
    DetailsField("Finished", moment(details.finishedAtMs, time)),
    DetailsField("Process id", if (details.pid == 0) "not running" else details.pid.toString()),
    DetailsField("Adopted", yesNo(details.adopted)),
)

fun videoFields(video: RenderedVideo, time: (Long) -> String): List<DetailsField> = listOf(
    DetailsField("Id", text(video.id)),
    DetailsField("Rendered", moment(video.renderedAtMs, time)),
    DetailsField("Sequence", text(video.sequenceName)),
    DetailsField("Export path", text(video.exportPath)),
    DetailsField("File name", text(video.outputFilename)),
    DetailsField("Path", text(video.outputPath)),
    DetailsField("Variant key", text(video.variantKey)),
    DetailsField("Codec", text(video.codec)),
    DetailsField("Resolution", text(video.resolutionLabel)),
    DetailsField("Frame rate", "${video.frameRate} fps"),
    DetailsField("Frame count", Format.count(video.frameCount.toLong())),
    DetailsField("Size", size(video.sizeBytes)),
    DetailsField("Width", "${video.width} px"),
    DetailsField("Height", "${video.height} px"),
    DetailsField("Duration", Format.videoDuration(video.durationSeconds)),
    DetailsField("Render time", if (video.renderSeconds < 0) "unknown" else "${Format.count(video.renderSeconds)} s"),
    DetailsField("Poster", yesNo(video.hasThumbnail)),
    DetailsField("File exists", yesNo(video.fileExists)),
    DetailsField("XMP embedded", yesNo(video.xmpEmbedded)),
    DetailsField("Source", sourceText(video.source)),
)

/** Every field of a failure card but the tool output and the command line. */
fun failureFields(failure: FailureCard, time: (Long) -> String): List<DetailsField> = listOf(
    DetailsField("Id", text(failure.id)),
    DetailsField("First seen", moment(failure.firstSeenMs, time)),
    DetailsField("Last seen", moment(failure.lastSeenMs, time)),
    DetailsField("Sequence", text(failure.sequenceName)),
    DetailsField("Export path", text(failure.exportPath)),
    DetailsField("File name", text(failure.outputFilename)),
    DetailsField("Path", text(failure.outputPath)),
    DetailsField("Variant key", text(failure.variantKey)),
    DetailsField("Kind", enumText(failure.kind.name, "FAILURE_")),
    DetailsField("Kind label", text(failure.kindLabel)),
    DetailsField("Summary", text(failure.summary)),
    DetailsField("Detail", text(failure.detail)),
    DetailsField("Exit code", if (failure.hasExitCode) failure.exitCode.toString() else NONE),
    DetailsField("Occurrences", failure.occurrences.toString()),
    DetailsField("Variant paused automatically", yesNo(failure.variantAutoPaused)),
    DetailsField("Render id", text(failure.renderId)),
)

/** What "Copy report" puts on the clipboard. */
fun failureReport(failure: FailureCard, time: (Long) -> String): String = buildString {
    failureFields(failure, time).forEach { appendLine("${it.label}: ${it.value}") }
    appendLine("Tool output:")
    failure.toolOutputList.forEach { appendLine(it) }
    appendLine("Command line:")
    append(text(failure.commandLine))
}

/**
 * The Details pane (spec 2 §6.4): the selected item, or [emptyText] with nothing selected.
 * [details] is the `GetRenderDetails` state of the selection's target.
 */
@Composable
fun DetailsPane(
    selection: DetailsSelection?,
    details: DetailsState?,
    emptyText: String,
    loopback: Boolean,
    posters: PosterLoader,
    time: (Long) -> String,
    onAction: (RowAction, ActionTarget) -> Unit,
    onCopy: (String) -> Unit,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier) {
        if (selection == null) {
            Text(emptyText, Modifier.padding(16.dp))
            return@Box
        }
        // Another item starts at its top.
        key(selection.key) {
            SelectionContainer {
                Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)
                        .testTag(DetailsTags.CONTENT),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    when (selection) {
                        is DetailsSelection.Sequence -> SequenceDetails(selection, time, onAction)
                        is DetailsSelection.Variant -> {
                            VariantDetails(selection, onAction)
                            if (selection.target == null) {
                                Text("No render details: this variant has no render.")
                            } else {
                                RenderSection(details, selection.target, time, onCopy, onRefresh)
                            }
                        }
                        is DetailsSelection.Failure -> FailureDetails(selection, details, time, onAction, onCopy, onRefresh)
                        is DetailsSelection.Video -> {
                            VideoDetails(selection, loopback, posters, time, onAction)
                            RenderSection(details, selection.target, time, onCopy, onRefresh)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Title(title: String) {
    Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
}

@Composable
private fun Heading(heading: String) {
    Text(heading, Modifier.padding(top = 8.dp), style = MaterialTheme.typography.titleSmall)
}

@Composable
private fun Fields(fields: List<DetailsField>) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        for (field in fields) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    field.label,
                    Modifier.width(110.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                )
                Text(field.value, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

@Composable
private fun Monospaced(content: String, tag: String) {
    Text(
        content,
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(8.dp).testTag(tag),
        fontFamily = FontFamily.Monospace,
        style = MaterialTheme.typography.bodySmall,
    )
}

@Composable
private fun CommandLine(command: String, onCopy: (String) -> Unit) {
    Heading("Command line")
    if (command.isEmpty()) {
        Text(NONE)
        return
    }
    Monospaced(command, DetailsTags.COMMAND_LINE)
    TextButton(onClick = { onCopy(command) }) { Text(COPY_COMMAND_LINE) }
}

@Composable
private fun SequenceDetails(
    selection: DetailsSelection.Sequence,
    time: (Long) -> String,
    onAction: (RowAction, ActionTarget) -> Unit,
) {
    val card = selection.card
    val target = ActionTarget.Sequence(selection.key, card.name)
    Title(card.name)
    ActionButtons(sequenceActions(card.paused) - RowAction.DETAILS, { onAction(it, target) })
    Fields(sequenceFields(card, time))
    Heading("Variants")
    if (card.variantsCount == 0) Text(NONE)
    for (variant in card.variantsList) {
        val label = Format.variantLabel(variant.codec, variant.resolutionLabel, variant.frameRate)
        val variantTarget = ActionTarget.Variant(
            "variant:${card.id}/${variant.variantKey}", card.name, variant.variantKey, variant.renderId,
        )
        Column {
            Text(label)
            Text(
                "${statusText(variant.status)} · ${text(variant.outputFilename)}",
                style = MaterialTheme.typography.bodySmall,
            )
            // A variant of an incomplete sequence has no row, so nothing to act on yet.
            if (variant.status != VariantStatus.VARIANT_WAITING) {
                IconActionButtons(variantActions(variant.status), { onAction(it, variantTarget) })
            }
        }
    }
}

@Composable
private fun VariantDetails(selection: DetailsSelection.Variant, onAction: (RowAction, ActionTarget) -> Unit) {
    val card = selection.card
    val variant = selection.variant
    val target = ActionTarget.Variant(selection.key, card.name, variant.variantKey, variant.renderId)
    Title(card.name)
    Text(Format.variantLabel(variant.codec, variant.resolutionLabel, variant.frameRate))
    ActionButtons(variantActions(variant.status), { onAction(it, target) })
    Fields(variantFields(variant))
}

@Composable
private fun RenderSection(
    state: DetailsState?,
    target: DetailsTarget?,
    time: (Long) -> String,
    onCopy: (String) -> Unit,
    onRefresh: () -> Unit,
    showCommandLine: Boolean = true,
) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Heading("Render details")
        if (target != null) TextButton(onClick = onRefresh) { Text(REFRESH_DETAILS) }
    }
    when (state) {
        null, DetailsState.Loading -> Text("Loading…")
        is DetailsState.Error -> Text(state.message, color = ERROR_RED)
        is DetailsState.Loaded -> {
            val details = state.details
            Fields(renderFields(details, time))
            if (showCommandLine) CommandLine(details.commandLine, onCopy)
            Heading("Render log")
            if (details.logTailCount == 0) {
                Text(NONE)
            } else {
                Monospaced(details.logTailList.joinToString("\n"), DetailsTags.LOG_TAIL)
            }
        }
    }
}

@Composable
private fun FailureDetails(
    selection: DetailsSelection.Failure,
    details: DetailsState?,
    time: (Long) -> String,
    onAction: (RowAction, ActionTarget) -> Unit,
    onCopy: (String) -> Unit,
    onRefresh: () -> Unit,
) {
    val failure = selection.failure
    val target = ActionTarget.Failure(selection.key, failure.id, failure.sequenceName, failure.variantKey)
    Title(failure.outputFilename.ifEmpty { failure.sequenceName })
    ActionButtons(FAILURE_ACTIONS - RowAction.DETAILS, { onAction(it, target) })
    TextButton(onClick = { onCopy(failureReport(failure, time)) }) { Text(COPY_REPORT) }
    Fields(failureFields(failure, time))
    Heading("Tool output")
    if (failure.toolOutputCount == 0) {
        Text(NONE)
    } else {
        Monospaced(failure.toolOutputList.joinToString("\n"), DetailsTags.TOOL_OUTPUT)
    }
    // The card's command line; the render's only when the card has none.
    val rendered = (details as? DetailsState.Loaded)?.details?.commandLine.orEmpty()
    CommandLine(failure.commandLine.ifEmpty { rendered }, onCopy)
    RenderSection(details, selection.target, time, onCopy, onRefresh, showCommandLine = false)
}

@Composable
private fun VideoDetails(
    selection: DetailsSelection.Video,
    loopback: Boolean,
    posters: PosterLoader,
    time: (Long) -> String,
    onAction: (RowAction, ActionTarget) -> Unit,
) {
    val video = selection.video
    val target = ActionTarget.Video(selection.key, video.outputPath)
    Title(video.outputFilename)
    var poster by remember(video.id) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(video.id, video.hasThumbnail, posters) {
        if (video.hasThumbnail) posters.load(video.id)?.let { poster = it }
    }
    Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f)) {
        val shown = poster
        if (shown == null) {
            Box(Modifier.fillMaxSize().background(Color(0xFF9E9E9E)).testTag(DetailsTags.POSTER_PLACEHOLDER))
        } else {
            Image(
                shown,
                contentDescription = "Large poster",
                modifier = Modifier.fillMaxSize().background(Color.Black).testTag(DetailsTags.POSTER),
                contentScale = ContentScale.Fit,
            )
        }
    }
    ActionButtons(videoActions(loopback, video.fileExists) - RowAction.DETAILS, { onAction(it, target) })
    Fields(videoFields(video, time))
}
