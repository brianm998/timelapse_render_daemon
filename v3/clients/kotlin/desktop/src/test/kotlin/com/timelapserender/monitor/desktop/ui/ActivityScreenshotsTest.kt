// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.use
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.ChipRole
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.core.state.VariantRowUi
import com.timelapserender.monitor.desktop.jpeg
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.trd.v1.VariantStatus
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** PNG renders of each state under `desktop/build/screenshots/`, for review; they are not compared. */
class ActivityScreenshotsTest {
    private val poster = Image.makeFromEncoded(jpeg()).toComposeImageBitmap()

    private fun variant(status: VariantStatus, text: String, role: ChipRole, progress: Float? = null) = VariantRowUi(
        key = "a/$text", sequenceName = "a", variantKey = text, label = "ProResMQ · 2160p · 30 fps", status = status,
        statusText = text, statusRole = role, progress = progress, indeterminate = false,
        progressText = if (progress != null) "843 / 1,500 · 4.2 fps · ~3 min" else "",
        sizeText = if (status == VariantStatus.VARIANT_DONE) "231 MB" else null,
        message = if (status == VariantStatus.VARIANT_FAILED) "ffmpeg exited 1" else "", renderId = "", failureId = "",
    )

    private val full = activityUi(
        problems = listOf("p1"),
        failures = listOf("LRT_a_very_long_output_file_name_that_has_to_be_truncated_in_the_middle_ProResMQ_2160p_30fps_final"),
        cards = listOf("a"),
        videos = (1..12).map { "LRT_$it" },
    ).let { ui ->
        ui.copy(
            inProgress = listOf(
                cardUi("a").copy(
                    variants = listOf(
                        variant(VariantStatus.VARIANT_RUNNING, "rendering", ChipRole.Active, 0.56f),
                        variant(VariantStatus.VARIANT_DONE, "done", ChipRole.Ok),
                        variant(VariantStatus.VARIANT_FAILED, "failed", ChipRole.Error),
                        variant(VariantStatus.VARIANT_UNKNOWN_CODEC, "unknown codec", ChipRole.Error),
                    ),
                ),
            ),
            recent = ui.recent.map { it.copy(hasThumbnail = true) },
        )
    }

    private var activity by mutableStateOf(full)

    private fun render(name: String, first: ActivityUi, selected: String? = null, then: (LazyListState) -> ActivityUi? = { null }) {
        activity = first
        val listState = LazyListState()
        val keeper = ScrollKeeper()
        ImageComposeScene(1000, 900) {
            MaterialTheme {
                Surface {
                    ActivityList(activity, listState, keeper, PosterLoader { poster }, true, selected, {}, {})
                }
            }
        }.use { scene ->
            scene.render(0)
            val next = then(listState)
            if (next != null) {
                scene.render(16_000_000)
                activity = next
            }
            var image = scene.render(32_000_000)
            for (frame in 3..6) {
                Snapshot.sendApplyNotifications()
                image = scene.render(frame * 16_000_000L)
            }
            val file = File("build/screenshots/activity-$name.png")
            file.parentFile.mkdirs()
            file.writeBytes(checkNotNull(image.encodeToData(EncodedImageFormat.PNG)).bytes)
            assertTrue(file.length() > 0)
        }
    }

    @Test
    fun everyStateIsRendered() {
        render("full", full, selected = "card:a")
        render("empty", activityUi())
        render("older-loading", activityUi(videos = listOf("v1")).copy(canLoadOlder = true, loadingOlder = true))
        render("older-failed", activityUi(videos = listOf("v1")).copy(canLoadOlder = true, olderError = "history unavailable"))
        render("new-items", full) { listState ->
            listState.requestScrollToItem(8)
            Snapshot.sendApplyNotifications()
            full.copy(problems = listOf(problemUi("p2")) + full.problems)
        }
    }
}
