// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.ChipRole
import com.timelapserender.monitor.core.state.FailureRowUi
import com.timelapserender.monitor.core.state.HeaderUi
import com.timelapserender.monitor.core.state.ProblemRowUi
import com.timelapserender.monitor.core.state.RenderedRowUi
import com.timelapserender.monitor.core.state.SequenceCardUi
import com.timelapserender.trd.v1.ProblemKind
import com.timelapserender.trd.v1.Severity
import com.timelapserender.trd.v1.Stage

fun problemUi(id: String) = ProblemRowUi(
    id, Severity.SEVERITY_ERROR, ProblemKind.PROBLEM_TOOL_MISSING, "Problem $id", "detail $id", "fix $id",
)

fun failureUi(id: String) = FailureRowUi(id, "Disk full", "$id.mov", "No space left", null, null, id, "k", id)

fun cardUi(id: String, framesText: String = "1 / 10 frames", progress: Float = 0.1f) = SequenceCardUi(
    key = id, name = "seq $id", stage = Stage.STAGE_EXPORTING, stageText = "Exporting", stageRole = ChipRole.Active,
    paused = false, progress = progress, framesText = framesText, exportRateText = null, exportSizeText = null,
    secondaryText = "/exports", note = "", variants = emptyList(),
)

fun videoUi(id: String, hasThumbnail: Boolean = false) = RenderedRowUi(
    key = id, outputFilename = "$id.mov", variantLabel = "H264 · OriRes · 30 fps", sizeText = "1.0 KB",
    framesText = "300 frames", durationText = "10.0 s", relativeTimeText = "just now", absoluteTimeText = "14:00",
    exportPath = "/exports", outputPath = "/out/$id.mov", hasThumbnail = hasThumbnail, fileExists = true,
    missingText = null,
)

fun activityUi(
    problems: List<String> = emptyList(),
    failures: List<String> = emptyList(),
    cards: List<String> = emptyList(),
    videos: List<String> = emptyList(),
) = ActivityUi(
    header = HeaderUi("Studio", "Connected", "Idle", ChipRole.Neutral, "", globalPause = false, sandbox = false),
    problems = problems.map { problemUi(it) },
    failures = failures.map { failureUi(it) },
    inProgress = cards.map { cardUi(it) },
    recent = videos.map { videoUi(it) },
    inProgressEmptyText = if (cards.isEmpty()) "Nothing in progress" else null,
    recentEmptyText = if (videos.isEmpty()) "Nothing rendered in the last 30 days" else null,
)
