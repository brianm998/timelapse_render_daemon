// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.Counts
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.Problem
import com.timelapserender.trd.v1.ProblemKind
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Severity
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.time.ZoneId
import java.util.Locale

/** Client-side filter (overview §6.1): case-insensitive substring of sequence name and output file name, and an export path. */
data class ActivityFilter(val search: String = "", val exportPath: String? = null)

/** The colour role of a chip; the UI maps it to its palette. */
enum class ChipRole { Neutral, Active, Ok, Warning, Error }

data class HeaderUi(
    val serverName: String,
    val connectionText: String,
    val phaseText: String,
    val phaseRole: ChipRole,
    val countsLine: String,
    val globalPause: Boolean,
    val sandbox: Boolean,
)

data class ProblemRowUi(
    val key: String,
    val severity: Severity,
    val kind: ProblemKind,
    val title: String,
    val detail: String,
    val fix: String,
)

data class FailureRowUi(
    val key: String,
    val kindLabel: String,
    val outputFilename: String,
    val summary: String,
    /** `×3` when the failure repeated, else null. */
    val occurrencesText: String?,
    /** "variant paused after repeated corrupt renders" when the variant was paused automatically, else null. */
    val autoPausedText: String?,
    val sequenceName: String,
    val variantKey: String,
    val failureId: String,
)

data class VariantRowUi(
    val key: String,
    val sequenceName: String,
    val variantKey: String,
    val label: String,
    val status: VariantStatus,
    val statusText: String,
    val statusRole: ChipRole,
    /** 0..1, or null when indeterminate or not applicable. */
    val progress: Float?,
    val indeterminate: Boolean,
    /** `843 / 1,500 · 4.2 fps · ~3 min` while running. */
    val progressText: String,
    /** The finished file's size, for finished and already-rendered variants. */
    val sizeText: String?,
    val message: String,
    val renderId: String,
    val failureId: String,
)

data class SequenceCardUi(
    val key: String,
    val name: String,
    val stage: Stage,
    val stageText: String,
    val stageRole: ChipRole,
    val paused: Boolean,
    val progress: Float,
    /** `1,234 / 1,500 frames`. */
    val framesText: String,
    /** While exporting: `41 frames/min · ~6 min left`, else null. */
    val exportRateText: String?,
    val exportSizeText: String?,
    /** The export path. */
    val secondaryText: String,
    val note: String,
    val variants: List<VariantRowUi>,
)

data class RenderedRowUi(
    val key: String,
    val outputFilename: String,
    val variantLabel: String,
    val sizeText: String,
    val framesText: String,
    val durationText: String,
    val relativeTimeText: String,
    val absoluteTimeText: String,
    val exportPath: String,
    val outputPath: String,
    val hasThumbnail: Boolean,
    val fileExists: Boolean,
    /** "moved or deleted" when the file is gone, else null. */
    val missingText: String?,
)

data class ActivityUi(
    val header: HeaderUi,
    val problems: List<ProblemRowUi> = emptyList(),
    val failures: List<FailureRowUi> = emptyList(),
    val inProgress: List<SequenceCardUi> = emptyList(),
    val recent: List<RenderedRowUi> = emptyList(),
    /** "Nothing in progress" when there are no cards. */
    val inProgressEmptyText: String? = null,
    /** "Nothing rendered in the last N days" when there are no rows. */
    val recentEmptyText: String? = null,
    /** Whether the "Show older" row is offered. */
    val canLoadOlder: Boolean = false,
    val loadingOlder: Boolean = false,
    /** Why the last "Show older" failed; the row stays offered for a retry. */
    val olderError: String? = null,
    /** Why the snapshot stream last failed, while the rows shown may be stale. */
    val watchError: String? = null,
    /** The filter these rows were built with, so a UI can tell a filter change from new rows. */
    val filter: ActivityFilter = ActivityFilter(),
) {
    /** Every row key in display order (S4): problems, failures, each card then its variants, recently rendered. */
    val keys: List<String>
        get() = problems.map { it.key } + failures.map { it.key } +
            inProgress.flatMap { card -> listOf(card.key) + card.variants.map { it.key } } + recent.map { it.key }
}

/**
 * Turns the repository's snapshot, the filter and older pages into ready-to-draw [ActivityUi]: row
 * keys follow overview §6.2 S4 and never change on progress (S5); every string is preformatted (§5.5).
 * Order is the server's: S7 holds because a finished sequence's videos are first in `recently_rendered`
 * in the snapshot that removes its card.
 */
class ActivityPresenter(
    private val repo: SnapshotRepository,
    private val filter: StateFlow<ActivityFilter>,
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val serverName: String = "",
    private val olderPageSize: Int = 50,
    private val retryDelayMs: Long = 1000,
    private val delayer: suspend (Long) -> Unit = { delay(it) },
) {
    private data class Older(
        val videos: List<RenderedVideo> = emptyList(),
        val more: Boolean? = null,
        val loading: Boolean = false,
        val error: String? = null,
    )

    /** The last snapshot's videos, and those that newer renders pushed out of it since paging began. */
    private var lastSnapshotVideos: List<RenderedVideo> = emptyList()
    private var pushedOut: List<RenderedVideo> = emptyList()

    private val older = MutableStateFlow(Older())
    private val historyDays = MutableStateFlow<Int?>(null)
    private var knownName = serverName

    val ui: StateFlow<ActivityUi> = combine(repo.state, filter, older, historyDays) { r, f, o, days ->
        build(r, f, o, days)
    }.stateIn(scope, SharingStarted.Eagerly, build(repo.state.value, filter.value, older.value, historyDays.value))

    init {
        scope.launch {
            repo.state.map { it.connection is ConnectionState.Connected }.distinctUntilChanged().collectLatest { connected ->
                if (connected) loadHistoryDays()
            }
        }
    }

    /** Asks until it answers; meanwhile the empty text is a general one. */
    private suspend fun loadHistoryDays() {
        while (true) {
            try {
                historyDays.value = client.getConfig().config.recentRenderHistoryDays.toInt()
                return
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                delayer(retryDelayMs)
            }
        }
    }

    /** Pages `ListRenderedVideos` by `before_id` (the oldest row shown). */
    fun loadOlder() {
        val o = older.value
        if (o.loading) return
        val last = o.videos.lastOrNull() ?: repo.state.value.snapshot?.recentlyRenderedList?.lastOrNull() ?: return
        older.update { it.copy(loading = true, error = null) }
        scope.launch {
            try {
                val r = client.listRenderedVideos(olderPageSize, last.id)
                older.update { cur ->
                    val known = cur.videos.mapTo(HashSet()) { it.id }
                    Older(cur.videos + r.videosList.filter { it.id !in known }, r.more, false)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                older.update { it.copy(loading = false, error = e.statusMessage()) }
            }
        }
    }

    private fun build(r: RepoState, f: ActivityFilter, o: Older, days: Int?): ActivityUi {
        val now = clock()
        val s = r.snapshot ?: Snapshot.getDefaultInstance()
        val connection = r.connection
        if (connection is ConnectionState.Connected && connection.info.name.isNotEmpty()) knownName = connection.info.name
        val header = HeaderUi(
            serverName = knownName,
            connectionText = connectionText(connection, r.lastUpdateMs, now),
            phaseText = phaseText(s.phase),
            phaseRole = phaseRole(s.phase),
            countsLine = countsLine(s.counts),
            globalPause = s.globalPause,
            sandbox = (connection as? ConnectionState.Connected)?.info?.sandbox ?: false,
        )
        val problems = s.problemsList.map { problemRow(it) }
        val failures = s.failuresList.filter { matches(f, it.exportPath, listOf(it.sequenceName, it.outputFilename)) }.map { failureRow(it) }
        val cards = s.inProgressList
            .filter { c -> matches(f, c.exportPath, listOf(c.name) + c.variantsList.map { it.outputFilename }) }
            .map { cardRow(it) }
        val snapshotIds = s.recentlyRenderedList.mapTo(HashSet()) { it.id }
        pushedOut = if (o.loading || o.more != null) {
            (lastSnapshotVideos + pushedOut).filter { it.id !in snapshotIds }.distinctBy { it.id }
        } else {
            emptyList()
        }
        lastSnapshotVideos = s.recentlyRenderedList
        val combined = (s.recentlyRenderedList + pushedOut + o.videos.filter { it.id !in snapshotIds }).distinctBy { it.id }
        val recent = combined.filter { matches(f, it.exportPath, listOf(it.sequenceName, it.outputFilename)) }.map { videoRow(it, now) }
        val more = o.more ?: (s.recentlyRenderedTotal.toLong() > combined.size)
        return ActivityUi(
            header = header,
            problems = problems,
            failures = failures,
            inProgress = cards,
            recent = recent,
            inProgressEmptyText = if (cards.isEmpty()) "Nothing in progress" else null,
            recentEmptyText = if (recent.isEmpty()) {
                days?.takeIf { it > 0 }?.let { "Nothing rendered in the last ${if (it == 1) "day" else "$it days"}" }
                    ?: "Nothing rendered recently"
            } else {
                null
            },
            canLoadOlder = more && combined.isNotEmpty(),
            loadingOlder = o.loading,
            olderError = o.error,
            watchError = r.watchError,
            filter = f,
        )
    }

    /** The export-path filter must equal the row's; the search must be a substring of one of [texts]. */
    private fun matches(f: ActivityFilter, exportPath: String, texts: List<String>): Boolean {
        if (f.exportPath != null && f.exportPath != exportPath) return false
        val needle = f.search.trim()
        return needle.isEmpty() || texts.any { it.contains(needle, ignoreCase = true) }
    }

    private fun connectionText(c: ConnectionState, lastUpdateMs: Long, now: Long): String = when (c) {
        ConnectionState.Connecting -> "Connecting…"
        is ConnectionState.Connected -> "Connected"
        is ConnectionState.Reconnecting -> "Reconnecting…"
        ConnectionState.NeedsPairing -> "Needs pairing"
        is ConnectionState.Incompatible -> c.message
        is ConnectionState.Offline -> {
            val last = if (c.lastUpdateMs > 0) c.lastUpdateMs else lastUpdateMs
            if (last > 0) "Offline — last update ${Format.absoluteTime(last, now, zone)}" else "Offline"
        }
    }

    private fun phaseText(p: Phase) = when (p) {
        Phase.PHASE_STARTING -> "Starting"
        Phase.PHASE_NO_CONFIG -> "No config"
        Phase.PHASE_WAITING_FOR_DISK_ACCESS -> "Waiting for disk access"
        Phase.PHASE_IDLE -> "Idle"
        Phase.PHASE_EXPORTING -> "Exporting"
        Phase.PHASE_RENDERING -> "Rendering"
        Phase.PHASE_PAUSED -> "Paused"
        else -> "Unknown"
    }

    private fun phaseRole(p: Phase) = when (p) {
        Phase.PHASE_NO_CONFIG -> ChipRole.Error
        Phase.PHASE_WAITING_FOR_DISK_ACCESS, Phase.PHASE_PAUSED -> ChipRole.Warning
        Phase.PHASE_EXPORTING, Phase.PHASE_RENDERING -> ChipRole.Active
        else -> ChipRole.Neutral
    }

    /** `2 rendering · 1 queued · 3 exporting · 1 failed`; zero parts are left out. */
    private fun countsLine(c: Counts): String {
        val parts = listOf(
            c.rendering to "rendering", c.suspended to "suspended", c.queued to "queued",
            c.exporting to "exporting", c.pausedSequences to "paused", c.failures to "failed",
        )
        return parts.filter { it.first > 0 }.joinToString(" · ") { "${it.first} ${it.second}" }
    }

    private fun problemRow(p: Problem) = ProblemRowUi(p.id, p.severity, p.kind, p.title, p.detail, p.fix)

    private fun failureRow(f: FailureCard) = FailureRowUi(
        key = f.id,
        kindLabel = f.kindLabel,
        outputFilename = f.outputFilename,
        summary = f.summary,
        occurrencesText = if (f.occurrences > 1) "×${f.occurrences}" else null,
        autoPausedText = if (f.variantAutoPaused) "variant paused after repeated corrupt renders" else null,
        sequenceName = f.sequenceName,
        variantKey = f.variantKey,
        failureId = f.id,
    )

    private fun cardRow(c: SequenceCard): SequenceCardUi {
        val exporting = c.stage == Stage.STAGE_EXPORTING
        val rate = Format.exportRate(c.exportFramesPerMinute)
        val eta = Format.eta(c.exportEtaSeconds)?.let { "$it left" }
        val rateText = if (exporting) listOfNotNull(rate, eta).joinToString(" · ").ifEmpty { null } else null
        val fraction = if (c.expectedFrames == 0) 0f else (c.exportedFrames.toFloat() / c.expectedFrames).coerceIn(0f, 1f)
        return SequenceCardUi(
            key = c.id,
            name = c.name,
            stage = c.stage,
            stageText = stageText(c.stage),
            stageRole = stageRole(c.stage),
            paused = c.paused,
            progress = fraction,
            framesText = Format.frames(c.exportedFrames.toLong(), c.expectedFrames.toLong()),
            exportRateText = rateText,
            exportSizeText = if (c.exportSizeBytes >= 0) Format.bytes(c.exportSizeBytes) else null,
            secondaryText = c.exportPath,
            note = c.note,
            variants = c.variantsList.filter { it.status != VariantStatus.VARIANT_WAITING }.map { variantRow(c, it) },
        )
    }

    private fun stageText(s: Stage) = when (s) {
        Stage.STAGE_EXPORTING -> "Exporting"
        Stage.STAGE_COUNT_MISMATCH -> "Count mismatch"
        Stage.STAGE_COLLECTING_METADATA -> "Collecting metadata"
        Stage.STAGE_QUEUED -> "Queued"
        Stage.STAGE_RENDERING -> "Rendering"
        Stage.STAGE_BLOCKED -> "Blocked"
        Stage.STAGE_PAUSED -> "Paused"
        else -> "Unknown"
    }

    private fun stageRole(s: Stage) = when (s) {
        Stage.STAGE_COUNT_MISMATCH, Stage.STAGE_BLOCKED, Stage.STAGE_PAUSED -> ChipRole.Warning
        Stage.STAGE_EXPORTING, Stage.STAGE_RENDERING, Stage.STAGE_COLLECTING_METADATA -> ChipRole.Active
        else -> ChipRole.Neutral
    }

    private fun variantRow(c: SequenceCard, v: VariantRow): VariantRowUi {
        val active = v.status == VariantStatus.VARIANT_RUNNING || v.status == VariantStatus.VARIANT_SUSPENDED
        val progressText = if (active) {
            listOfNotNull(
                if (v.progressKnown) "${Format.count(v.frame.toLong())} / ${Format.count(v.totalFrames.toLong())}" else null,
                if (v.fps > 0) String.format(Locale.ENGLISH, "%.1f fps", v.fps) else null,
                Format.eta(v.etaSeconds),
            ).joinToString(" · ")
        } else {
            ""
        }
        val (text, role) = when (v.status) {
            VariantStatus.VARIANT_ALREADY_RENDERED -> "exists" to ChipRole.Ok
            VariantStatus.VARIANT_QUEUED -> "queued" to ChipRole.Neutral
            VariantStatus.VARIANT_RUNNING -> "rendering" to ChipRole.Active
            VariantStatus.VARIANT_SUSPENDED -> "suspended" to ChipRole.Warning
            VariantStatus.VARIANT_DONE -> "done" to ChipRole.Ok
            VariantStatus.VARIANT_FAILED -> "failed" to ChipRole.Error
            VariantStatus.VARIANT_PAUSED -> "paused" to ChipRole.Warning
            VariantStatus.VARIANT_UNKNOWN_CODEC -> "unknown codec" to ChipRole.Error
            else -> "waiting" to ChipRole.Neutral
        }
        val finished = v.status == VariantStatus.VARIANT_DONE || v.status == VariantStatus.VARIANT_ALREADY_RENDERED
        return VariantRowUi(
            key = c.id + "/" + v.variantKey,
            sequenceName = c.name,
            variantKey = v.variantKey,
            label = Format.variantLabel(v.codec, v.resolutionLabel, v.frameRate),
            status = v.status,
            statusText = text,
            statusRole = role,
            progress = if (active && v.progressKnown && v.totalFrames > 0) (v.frame.toFloat() / v.totalFrames).coerceIn(0f, 1f) else null,
            indeterminate = active && !v.progressKnown,
            progressText = progressText,
            sizeText = if (finished && v.sizeBytes >= 0) Format.bytes(v.sizeBytes) else null,
            message = if (v.status == VariantStatus.VARIANT_FAILED) v.message else "",
            renderId = v.renderId,
            failureId = v.failureId,
        )
    }

    private fun videoRow(v: RenderedVideo, now: Long): RenderedRowUi {
        val seconds = if (v.frameRate > 0) v.frameCount.toDouble() / v.frameRate else v.durationSeconds
        return RenderedRowUi(
            key = v.id,
            outputFilename = v.outputFilename,
            variantLabel = Format.variantLabel(v.codec, v.resolutionLabel, v.frameRate),
            sizeText = Format.bytes(v.sizeBytes),
            framesText = "${Format.count(v.frameCount.toLong())} frames",
            durationText = Format.videoDuration(seconds),
            relativeTimeText = Format.relativeTime(v.renderedAtMs, now, zone),
            absoluteTimeText = Format.absoluteTime(v.renderedAtMs, now, zone),
            exportPath = v.exportPath,
            outputPath = v.outputPath,
            hasThumbnail = v.hasThumbnail,
            fileExists = v.fileExists,
            missingText = if (v.fileExists) null else "moved or deleted",
        )
    }
}
