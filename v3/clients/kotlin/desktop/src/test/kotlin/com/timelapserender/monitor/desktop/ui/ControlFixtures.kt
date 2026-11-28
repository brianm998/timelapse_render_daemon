// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.FailureKind
import com.timelapserender.trd.v1.RenderedSource
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus

/** One snapshot with a row of every kind that has controls, for the control and Details tests. */
object ControlFixtures {
    const val CARD = "card:/exp/LRT_b"
    const val PAUSED_CARD = "card:/exp/LRT_p"
    const val FAILURE = "failure:f-1"
    const val VIDEO = "video:v-1"
    const val MISSING_VIDEO = "video:v-gone"
    const val VIDEO_PATH = "/exp/out/v-1.mov"

    fun variantTag(key: String) = "variant:/exp/LRT_b/$key"

    /** The variants of [CARD] by key; each key names its status. */
    val STATUSES = linkedMapOf(
        "queued" to VariantStatus.VARIANT_QUEUED,
        "running" to VariantStatus.VARIANT_RUNNING,
        "suspended" to VariantStatus.VARIANT_SUSPENDED,
        "done" to VariantStatus.VARIANT_DONE,
        "exists" to VariantStatus.VARIANT_ALREADY_RENDERED,
        "paused" to VariantStatus.VARIANT_PAUSED,
        "failed" to VariantStatus.VARIANT_FAILED,
        "unknown" to VariantStatus.VARIANT_UNKNOWN_CODEC,
    )

    private val withRender = setOf("queued", "running", "suspended", "done", "failed")

    fun variant(key: String, status: VariantStatus): VariantRow = VariantRow.newBuilder()
        .setVariantKey(key).setCodec("H264").setResolutionLabel("720w").setFrameRate(30).setStatus(status)
        .setOutputFilename("LRT_b_$key.mov").setOutputPath("/exp/out/LRT_b_$key.mov")
        .setRenderId(if (key in withRender) "r-$key" else "").setSizeBytes(-1).setEtaSeconds(-1).build()

    private fun card(name: String) = SequenceCard.newBuilder().setId("/exp/$name").setName(name).setExportPath("/exp")
        .setRawSequenceName("raw_$name").setRawSequenceLocation("/raw/location").setStage(Stage.STAGE_RENDERING)
        .setExpectedFrames(1500).setExportedFrames(1500).setExportEtaSeconds(-1).setExportSizeBytes(-1)

    val failure: FailureCard = FailureCard.newBuilder()
        .setId("f-1").setFirstSeenMs(1_000).setLastSeenMs(2_000).setSequenceName("LRT_e").setExportPath("/exp")
        .setOutputFilename("LRT_e.mov").setOutputPath("/exp/out/LRT_e.mov").setVariantKey("k-e")
        .setKind(FailureKind.FAILURE_DISK_FULL).setKindLabel("Disk full").setSummary("No space left")
        .setDetail("The disk filled up").setHasExitCode(true).setExitCode(28)
        .setCommandLine("ffmpeg -i failing.tif LRT_e.mov").addToolOutput("av_interleaved_write_frame(): no space")
        .addToolOutput("Conversion failed!").setOccurrences(2).setRenderId("r-e").build()

    fun video(id: String, exists: Boolean = true): RenderedVideo = RenderedVideo.newBuilder()
        .setId(id).setRenderedAtMs(3_000).setSequenceName("LRT_v").setExportPath("/exp").setOutputFilename("$id.mov")
        .setOutputPath("/exp/out/$id.mov").setVariantKey("k-v").setCodec("ProResMQ").setResolutionLabel("2160p")
        .setFrameRate(30).setFrameCount(1500).setSizeBytes(1024).setWidth(3840).setHeight(2160)
        .setDurationSeconds(50.0).setRenderSeconds(1234).setFileExists(exists).setXmpEmbedded(true)
        .setSource(RenderedSource.RENDERED_SOURCE_TRD).build()

    fun snapshot(): Snapshot = Snapshot.newBuilder()
        .addFailures(failure)
        .addInProgress(card("LRT_b").addAllVariants(STATUSES.map { (key, status) -> variant(key, status) }))
        .addInProgress(card("LRT_p").setPaused(true).setStage(Stage.STAGE_PAUSED))
        .addRecentlyRendered(video("v-1"))
        .addRecentlyRendered(video("v-gone", exists = false))
        .setRecentlyRenderedTotal(2)
        .build()
}
