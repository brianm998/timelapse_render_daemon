// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.desktop.Harness
import com.timelapserender.trd.v1.CodecFfmpeg
import com.timelapserender.trd.v1.CodecFilename
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigIssue
import com.timelapserender.trd.v1.ConfigState
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.MaxProcesses
import com.timelapserender.trd.v1.PausedVariant
import com.timelapserender.trd.v1.RenderVariant
import com.timelapserender.trd.v1.UpdateCodecsRequest
import com.timelapserender.trd.v1.UpdateConfigRequest
import com.timelapserender.trd.v1.ValidateConfigResponse
import com.timelapserender.trd.v1.VideoCodec
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.swing.Swing
import org.junit.After
import org.junit.Rule
import java.util.concurrent.atomic.AtomicInteger

/** The Config window's content, offscreen, over an in-process [FakeTrdService]. */
abstract class ConfigTestBase {
    @get:Rule
    val compose = createComposeRule()

    protected val fake = FakeTrdService()
    private val running = fake.startInProcess()

    // The Swing dispatcher, as in the app: the editor is used from one thread.
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Swing)
    private val client = TrdClient(running.newChannel(), null, scope)

    protected val closed = AtomicInteger()
    protected val stayed = AtomicInteger()
    protected lateinit var controller: ConfigController

    @After
    fun tearDown() {
        scope.cancel()
        client.close()
        running.close()
    }

    protected fun codec(name: String): VideoCodec = VideoCodec.newBuilder()
        .setName(name)
        .setFfmpeg(CodecFfmpeg.newBuilder().setCodec("lib$name").setPixelFormat("yuv420p").setColorRange("tv"))
        .setFilename(CodecFilename.newBuilder().setPixelFormat("420").setCodec(name).setQuality("hq"))
        .build()

    protected val baseConfig: Config = Config.newBuilder()
        .addSourcePaths("/src")
        .addExportPaths("/exp")
        .addRawTypes("ARW")
        .addImageExtensions("tif")
        .addIgnoreSequenceRegexes("^tmp")
        .setImageNamePrefix("IMG_")
        .setMaxProcesses(MaxProcesses.newBuilder().setFfmpeg(1).setExiftool(10))
        .setMaxConcurrentRenders(1)
        .setScanIntervalSeconds(2.0)
        .setRecentRenderHistoryDays(30)
        .setProgressBarWidth(40)
        .setSleepTimeSeconds(12.0)
        .addRendering(RenderVariant.newBuilder().setCodec("h264").setFrameRate(30).setVariantKey("h264_ori_30"))
        .addPausedSequenceNames("LRT_1")
        .addPausedVariants(PausedVariant.newBuilder().setSequenceName("LRT_2").setVariantKey("h264_ori_30"))
        .build()

    protected fun state(
        config: Config = baseConfig,
        codecs: List<VideoCodec> = listOf(codec("h264"), codec("prores")),
        revision: String = "rev1",
    ): ConfigState = ConfigState.newBuilder()
        .setPath("/etc/trd/config.json").setExists(true).setConfig(config).setRevision(revision)
        .addAllCodecs(codecs).setCodecsRevision("codecs1").setRawJson("{\"raw\": 1}")
        .build()

    protected fun error(field: String, message: String): ConfigIssue = ConfigIssue.newBuilder()
        .setSeverity(IssueSeverity.ISSUE_ERROR).setField(field).setMessage(message).build()

    protected fun answer(vararg issues: ConfigIssue) {
        fake.validateResponse = ValidateConfigResponse.newBuilder().addAllIssues(issues.toList()).build()
    }

    protected fun show(configState: ConfigState = state()) {
        fake.configState = configState
        controller = ConfigController(client, scope, { closed.incrementAndGet() }, { stayed.incrementAndGet() })
        compose.setContent { MaterialTheme { ConfigWindowContent(controller) } }
        await { !controller.editor.state.value.loading }
    }

    protected fun await(condition: () -> Boolean) = compose.waitUntil(Harness.TIMEOUT_MS, condition)

    protected fun awaitText(text: String) =
        await { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }

    protected fun awaitNoText(text: String) =
        await { compose.onAllNodesWithText(text).fetchSemanticsNodes().isEmpty() }

    protected fun awaitTag(tag: String) =
        await { compose.onAllNodesWithTag(tag).fetchSemanticsNodes().isNotEmpty() }

    /** Waits for the validation of the last edit. */
    protected fun settle() {
        await { !controller.editor.state.value.validationPending }
        compose.waitForIdle()
    }

    protected fun node(tag: String): SemanticsNodeInteraction = compose.onNodeWithTag(tag)

    /** Clicks a control of the scrolling form. */
    protected fun click(tag: String) {
        node(tag).performScrollTo().performClick()
        compose.waitForIdle()
    }

    protected fun type(tag: String, text: String) {
        node(tag).performScrollTo().performTextReplacement(text)
        compose.waitForIdle()
    }

    protected fun open(group: ConfigGroup) {
        node(ConfigTags.group(group)).performClick()
        compose.waitForIdle()
    }

    protected val config: Config get() = controller.editor.state.value.config!!

    protected fun configUpdates() = fake.requestsOf<UpdateConfigRequest>()

    protected fun codecUpdates() = fake.requestsOf<UpdateCodecsRequest>()
}
