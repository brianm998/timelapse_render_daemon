// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.use
import com.timelapserender.monitor.core.net.ChannelFactory
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.ActivityFilter
import com.timelapserender.monitor.core.state.ActivityPresenter
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.monitor.desktop.ui.ActivityList
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.trd.v1.FailureCard
import com.timelapserender.trd.v1.Phase
import com.timelapserender.trd.v1.RenderedVideo
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.Stage
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.jetbrains.skia.EncodedImageFormat
import java.io.File
import org.jetbrains.skia.Image as SkiaImage

const val SELF_TEST_FLAG = "--self-test"
const val SELF_TEST_OK = "SELF-TEST OK"
const val SELF_TEST_FAILED = "SELF-TEST FAILED"

private const val OUTPUT_FLAG = "--self-test-output"
private const val SAMPLE_POSTER = "selftest/poster.jpg"
private const val SERVER_TIMEOUT_MS = 30_000L
private const val RENDER_FRAMES = 4
private const val FRAME_NANOS = 16_000_000L

private const val UI = "com.timelapserender.monitor.desktop.ui"

/** The class of every source file under `ui`, `ui/rows` and `ui/config`. */
val SCREEN_CLASSES: List<String> = listOf(
    "ActivityListKt",
    "ConfirmDialogKt",
    "DetailsPaneKt",
    "DevicesDialogKt",
    "DialogsKt",
    "FirstRunSetupKt",
    "LogWindowKt",
    "MainWindowKt",
    "MenusKt",
    "NewItemsPillKt",
    "NotRunningScreenKt",
    "PairServerDialogKt",
    "PreferencesWindowKt",
    "RowActionsKt",
    "TopBarKt",
    "TrayIconKt",
    "config.CodecEditorKt",
    "config.ConfigWindowKt",
    "config.FolderBrowserDialogKt",
    "config.PathListEditorKt",
    "config.RenderingTableKt",
    "rows.CommonKt",
    "rows.FailureCardKt",
    "rows.ProblemBannerKt",
    "rows.RenderedRowKt",
    "rows.SequenceCardKt",
).map { "$UI.$it" }

private data class SelfTestArgs(val output: File?, val server: ServerEndpoint?)

private fun parseSelfTest(args: List<String>): SelfTestArgs {
    var output: File? = null
    val others = mutableListOf<String>()
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            SELF_TEST_FLAG -> i++
            OUTPUT_FLAG -> {
                output = File(requireNotNull(args.getOrNull(i + 1)) { "$OUTPUT_FLAG needs a file" })
                i += 2
            }
            else -> others += args[i++]
        }
    }
    // The rest follows the rules of the ordinary command line (--server is a loopback trd).
    return SelfTestArgs(output, parseOptions(others).server)
}

/** The file named by [OUTPUT_FLAG], so that even a bad command line is reported there. */
private fun requestedOutput(args: List<String>): File? =
    args.indexOf(OUTPUT_FLAG).takeIf { it >= 0 }?.let { args.getOrNull(it + 1) }?.let(::File)

/**
 * The self-test of an installed package (spec 2 §9): proves that the runtime has the JDK modules,
 * native libraries and resources the app needs. It opens no window and reads none of the user's
 * settings, servers or caches. Writes [SELF_TEST_OK], or [SELF_TEST_FAILED] with the reason, to the
 * output file or to [print], and returns the exit code: 0 or 1.
 */
fun runSelfTest(
    args: List<String>,
    classLoader: ClassLoader = SelfTestMarker::class.java.classLoader,
    tempDir: File = File(System.getProperty("java.io.tmpdir")),
    serverTimeoutMs: Long = SERVER_TIMEOUT_MS,
    print: (String) -> Unit = ::println,
): Int {
    System.setProperty("java.awt.headless", "true")
    var step = "reading the command line"
    var output = requestedOutput(args)
    // Throwable: a missing module or native library arrives as an Error.
    val failure: String? = try {
        val options = parseSelfTest(args)
        output = options.output
        step = "loading the screen classes"
        SCREEN_CLASSES.forEach { Class.forName(it, true, classLoader) }
        step = "decoding the sample poster"
        val poster = samplePoster(classLoader)
        step = "rendering the Activity screen"
        renderActivity(poster, tempDir)
        options.server?.let {
            step = "calling GetServerInfo on ${it.host}:${it.port}"
            callServer(it, serverTimeoutMs)
        }
        null
    } catch (e: Throwable) {
        "$step: $e".replace(Regex("\\s*[\\r\\n]+\\s*"), " ")
    }
    val line = if (failure == null) SELF_TEST_OK else "$SELF_TEST_FAILED: $failure"
    val file = output
    if (file == null) {
        print(line)
    } else {
        try {
            file.writeText(line + "\n")
        } catch (e: Exception) {
            print("$SELF_TEST_FAILED: writing $file: $e")
            return 1
        }
    }
    return if (failure == null) 0 else 1
}

/** Names this module's class loader. */
private object SelfTestMarker

private fun samplePoster(classLoader: ClassLoader): ImageBitmap {
    val stream = checkNotNull(classLoader.getResourceAsStream(SAMPLE_POSTER)) { "$SAMPLE_POSTER is missing" }
    return SkiaImage.makeFromEncoded(stream.use { it.readBytes() }).toComposeImageBitmap()
}

/** A snapshot with a row of every section of the Activity screen. */
internal fun sampleSnapshot(): Snapshot = Snapshot.newBuilder()
    .setRevision(1)
    .setPhase(Phase.PHASE_RENDERING)
    .addFailures(
        FailureCard.newBuilder().setId("failure").setKindLabel("Render failed")
            .setSequenceName("Sample").setOutputFilename("Sample_H264.mp4").setSummary("ffmpeg exited 1"),
    )
    .addInProgress(
        SequenceCard.newBuilder().setId("card").setName("Sample").setStage(Stage.STAGE_RENDERING)
            .setExportedFrames(300).setExpectedFrames(300)
            .addVariants(
                VariantRow.newBuilder().setVariantKey("h264").setCodec("H264").setResolutionLabel("2160p")
                    .setFrameRate(30).setStatus(VariantStatus.VARIANT_RUNNING)
                    .setProgressKnown(true).setFrame(150).setTotalFrames(300),
            ),
    )
    .addRecentlyRendered(
        RenderedVideo.newBuilder().setId("video").setSequenceName("Sample").setOutputFilename("Sample_ProRes.mov")
            .setCodec("ProRes").setFrameRate(30).setFrameCount(300).setHasThumbnail(true).setFileExists(true),
    )
    .build()

/** [sampleSnapshot] as the presenter shows it. Nothing is connected or started. */
internal fun sampleActivity(): ActivityUi {
    val endpoint = ServerEndpoint("127.0.0.1", 1, TlsMode.Plaintext)
    // Already cancelled: the presenter builds its first state at once and then does nothing.
    val scope = CoroutineScope(Job().apply { cancel() })
    val client = TrdClient(ChannelFactory.create(endpoint), null, scope)
    try {
        val repository = SnapshotRepository(client, scope, initialSnapshot = sampleSnapshot())
        return ActivityPresenter(repository, MutableStateFlow(ActivityFilter()), client, scope).ui.value
    } finally {
        client.close()
    }
}

private fun renderActivity(poster: ImageBitmap, tempDir: File) {
    val activity = sampleActivity()
    val listState = LazyListState()
    val keeper = ScrollKeeper()
    val png = ImageComposeScene(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT) {
        MaterialTheme {
            Surface {
                ActivityList(activity, listState, keeper, PosterLoader { poster }, true, null, {}, {})
            }
        }
    }.use { scene ->
        var image = scene.render(0)
        for (frame in 1..RENDER_FRAMES) image = scene.render(frame * FRAME_NANOS)
        checkNotNull(image.encodeToData(EncodedImageFormat.PNG)) { "the screen could not be encoded" }.bytes
    }
    val file = File.createTempFile("trm-selftest-", ".png", tempDir)
    try {
        file.writeBytes(png)
        check(file.length() > 0) { "$file is empty" }
    } finally {
        file.delete()
    }
}

private fun callServer(endpoint: ServerEndpoint, timeoutMs: Long) {
    runBlocking {
        val client = TrdClient(ChannelFactory.create(endpoint), null, this)
        try {
            withTimeout(timeoutMs) { client.serverInfo() }
        } finally {
            client.close()
        }
    }
}
