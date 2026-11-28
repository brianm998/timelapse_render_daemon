// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.contract

import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.servers.MiniJson
import com.timelapserender.trd.v1.SequenceCard
import com.timelapserender.trd.v1.Snapshot
import com.timelapserender.trd.v1.VariantRow
import com.timelapserender.trd.v1.VariantStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assume.assumeTrue
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.nio.file.StandardCopyOption
import java.util.Locale
import java.util.concurrent.CompletableFuture
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/** Stops a test that waits for a state that never comes. It is not a timing assertion. */
internal const val HANG_GUARD_MS = 120_000L
private const val POLL_MS = 50L
private const val STOP_GUARD_MS = 30_000L

/** The variant keys of the sandbox's rendering list, in its order. */
internal const val PRORES_MQ = "ProResMQ_1080x720_30"
internal const val H264 = "h264_720w_30"
internal const val PRORES_UHQ = "ProResUHQ_ori_30"

private const val LETTERS = "abcdefg"

/** The export directory name of a sandbox sequence: the `sequence_name` of the API. */
internal fun sequenceName(letter: Char) = "LRT_2026_01_01-sandbox-$letter"

internal fun Snapshot.card(letter: Char): SequenceCard? = inProgressList.firstOrNull { it.name == sequenceName(letter) }

internal fun Snapshot.variant(letter: Char, key: String): VariantRow? =
    card(letter)?.variantsList?.firstOrNull { it.variantKey == key }

internal fun Snapshot.statusOf(letter: Char, key: String): VariantStatus? = variant(letter, key)?.status

/** A short account of a snapshot for a failed wait. */
internal fun Snapshot.summary(): String = buildString {
    append("phase=$phase pause=$globalPause rendered=$recentlyRenderedTotal")
    for (card in inProgressList) {
        append("\n  ${card.name} ${card.stage} ${card.exportedFrames}/${card.expectedFrames}")
        for (v in card.variantsList) append("\n    ${v.variantKey} ${v.status} ${v.renderId} ${v.message}")
    }
    for (f in failuresList) append("\n  failure ${f.sequenceName} ${f.variantKey}: ${f.summary}")
    for (p in problemsList) append("\n  problem ${p.id}: ${p.detail}")
}

/** Polls [probe] until it answers, under the hang guard. */
internal suspend fun <T : Any> await(what: String, describe: () -> String = { "" }, probe: suspend () -> T?): T {
    var found: T? = null
    withTimeoutOrNull(HANG_GUARD_MS) {
        while (found == null) {
            found = probe()
            if (found == null) delay(POLL_MS)
        }
    }
    return found ?: throw AssertionError("$what was not reached: ${describe()}")
}

/** The first value of the flow that satisfies [predicate], under the hang guard. */
internal suspend fun <T> StateFlow<T>.reach(what: String, predicate: (T) -> Boolean): T {
    var found: Pair<T, Boolean>? = null
    withTimeoutOrNull(HANG_GUARD_MS) { found = first(predicate) to true }
    return found?.first ?: throw AssertionError("$what was not reached: $value")
}

/** Polls `GetSnapshot` until [predicate] accepts the answer, which it returns. */
internal suspend fun TrdClient.awaitSnapshot(what: String, predicate: (Snapshot) -> Boolean): Snapshot {
    var last = Snapshot.getDefaultInstance()
    return await(what, { last.summary() }) {
        last = getSnapshot()
        last.takeIf(predicate)
    }
}

/**
 * A real `trd` on its own sandbox tree in a temporary directory, with the fake media tools (spec 2
 * §5.7). Use it as: [create], then [keepSequences] and [script], then [start], then [test]; `use`
 * stops every render and the daemon and deletes the tree.
 *
 * [create] skips the test unless `TRM_E2E=1`, before it reads or writes anything else.
 *
 * Coroutines of [test] and of [scope] share one thread, as a client's models share the UI thread.
 */
internal class TrdProcess private constructor(
    /** Everything of this test: the sandbox [tree], the daemon's home, the fake script and caches. */
    val root: Path,
    private val binary: Path,
    private val tools: Path,
) : AutoCloseable {
    /** The sandbox tree of `sandbox.py init`. */
    val tree: Path = root.resolve("tree")
    val configFile: Path = tree.resolve("config.json")
    val home: Path = root.resolve("home")
    private val scriptFile: Path = root.resolve("fake-ffmpeg.json")
    private val stderrFile: Path = root.resolve("daemon.stderr")
    private val releases = LinkedHashSet<Path>()
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "trm-contract").apply { isDaemon = true } }
    private val dispatcher = executor.asCoroutineDispatcher()

    /** The scope the client classes under test run in. */
    val scope: CoroutineScope = CoroutineScope(SupervisorJob() + dispatcher)
    private val clients = ArrayList<TrdClient>()
    private var daemon: Process? = null

    var port: Int = 0
        private set
    var lanPort: Int = 0
        private set

    /** Runs a test body on the models' thread. */
    fun test(block: suspend CoroutineScope.() -> Unit) = runBlocking(dispatcher, block)

    // ── before the start ────────────────────────────────────────────────────

    /**
     * Ignores every sandbox sequence but [letters], and lets [renders] ffmpeg processes run, all of
     * one sequence if need be. [adjust] changes anything else of config.json.
     */
    fun keepSequences(letters: String, renders: Int = 3, adjust: (MutableMap<String, Any?>) -> Unit = {}) {
        val ignored = LETTERS.filter { it !in letters }
        editConfigFile { config ->
            config["ignore_sequence_regexes"] = listOf("sandbox-[$ignored]")
            config["max_processes"] = mapOf("ffmpeg" to renders, "exiftool" to 4)
            config["max_cuncurrent_renders"] = renders
            adjust(config)
        }
    }

    /** The file that ends every fake render waiting for [name], once [release] creates it. */
    fun releaseFile(name: String = "all"): Path =
        root.resolve("release").resolve(name).also { releases.add(it) }

    fun release(name: String = "all") {
        val file = releaseFile(name)
        Files.createDirectories(file.parent)
        Files.write(file, ByteArray(0))
    }

    /** A fake ffmpeg behaviour that reports one frame and then waits for [release] of [name]. */
    fun held(name: String = "all", rules: List<Map<String, Any?>> = emptyList()): Map<String, Any?> =
        mapOf("frames" to 2, "wait_for_file" to releaseFile(name).toString(), "rules" to rules)

    /** Sets the fake ffmpeg's behaviour (spec 1 TRD-05) for every run that starts from now on. */
    fun script(behaviour: Map<String, Any?>) = writeAtomically(scriptFile, MiniJson.write(behaviour))

    // ── the tree ────────────────────────────────────────────────────────────

    fun config(): Map<String, Any?> = readJson(configFile)

    fun editConfigFile(edit: (MutableMap<String, Any?>) -> Unit) {
        val config = LinkedHashMap(config())
        edit(config)
        writeAtomically(configFile, MiniJson.write(config))
    }

    /** `paused_sequence_names` of config.json. */
    fun pausedSequences(): List<String> =
        (config()["paused_sequence_names"] as? List<*>).orEmpty().map { it as String }

    /** `paused_variants` of config.json, as (sequence name, variant key). */
    fun pausedVariants(): List<Pair<String, String>> =
        (config()["paused_variants"] as? List<*>).orEmpty().map {
            val entry = it as Map<*, *>
            (entry["sequence_name"] as String) to (entry["variant_key"] as String)
        }

    fun readJson(file: Path): Map<String, Any?> {
        val parsed = MiniJson.parse(String(Files.readAllBytes(file), Charsets.UTF_8)) as Map<*, *>
        return parsed.entries.associate { (it.key as String) to it.value }
    }

    /** Exports [frames] of a sequence, as copies of its raw frames. */
    fun exportFrames(letter: Char, frames: IntRange) {
        val raw = tree.resolve("raw").resolve("2026_01_01-sandbox-$letter")
        val export = tree.resolve("export").resolve(sequenceName(letter))
        Files.createDirectories(export)
        for (frame in frames) {
            val number = String.format(Locale.ROOT, "%05d", frame)
            Files.copy(raw.resolve("IMG_$number.tif"), export.resolve("LRT_$number.tif"))
        }
    }

    // ── the daemon ──────────────────────────────────────────────────────────

    /** Starts the daemon and waits for its `TRD_LISTENING` line. */
    fun start() {
        check(daemon == null) { "the daemon is already started" }
        val builder = ProcessBuilder(
            binary.toString(), "daemon", "--sandbox",
            "--config", configFile.toString(),
            "--data-dir", tree.resolve("data").toString(),
            "--log-dir", tree.resolve("logs").toString(),
            "--port", "0",
            "--lan-bind", "127.0.0.1:0",
            "--no-mdns",
        ).redirectError(stderrFile.toFile())
        toolEnvironment(builder)
        builder.environment()["TRD_FAKE_FFMPEG"] = scriptFile.toString()
        if (!Files.exists(scriptFile)) script(emptyMap())
        val process = builder.start()
        daemon = process
        process.outputStream.close()
        val listening = CompletableFuture<String>()
        // Standard output is drained for the daemon's whole life, so it never blocks on a full pipe.
        thread(isDaemon = true, name = "trd-stdout") {
            process.inputStream.bufferedReader().forEachLine {
                if (it.startsWith("TRD_LISTENING ")) listening.complete(it)
            }
            listening.completeExceptionally(IllegalStateException("trd ended without TRD_LISTENING"))
        }
        val line = try {
            listening.get(HANG_GUARD_MS, TimeUnit.MILLISECONDS)
        } catch (e: Exception) {
            throw AssertionError("trd did not start: ${String(Files.readAllBytes(stderrFile))}", e)
        }
        port = portOf(line, "loopback")
        lanPort = portOf(line, "lan")
    }

    private fun portOf(line: String, name: String): Int =
        Regex("""\b$name=127\.0\.0\.1:(\d+)""").find(line)?.groupValues?.get(1)?.toInt()
            ?: throw AssertionError("no $name port in $line")

    /** A client of the loopback listener, not yet started. */
    fun client(): TrdClient = client(ServerEndpoint("127.0.0.1", port, TlsMode.Plaintext), null)

    /** A client of [endpoint], closed with this process. */
    fun client(endpoint: ServerEndpoint, token: String?): TrdClient =
        TrdClient.create(endpoint, token, scope).also { clients.add(it) }

    /** A started loopback client that has connected. */
    suspend fun connected(): TrdClient {
        val client = client()
        client.start()
        client.state.reach("the loopback connection") { it is ConnectionState.Connected }
        return client
    }

    /** Stops every render, so that no fake ffmpeg is left suspended or waiting. */
    private suspend fun stopRenders() {
        val client = client()
        while (true) {
            val ids = client.getSnapshot().inProgressList.flatMap { it.variantsList }
                .filter { it.status in IN_FLIGHT && it.renderId.isNotEmpty() }
                .map { it.renderId }
            if (ids.isEmpty()) return
            for (id in ids) runCatching { client.stopRender(id) }
            delay(POLL_MS)
        }
    }

    override fun close() {
        val process = daemon
        try {
            if (process != null && process.isAlive) {
                // The daemon ends its own renders; this test signals no ffmpeg. A daemon that
                // cannot answer is destroyed below, and the release files end its fakes.
                runCatching { runBlocking(dispatcher) { withTimeoutOrNull(STOP_GUARD_MS) { stopRenders() } } }
            }
            val children = process?.descendants()?.toList().orEmpty()
            for (name in releases.toList()) {
                Files.createDirectories(name.parent)
                if (!Files.exists(name)) Files.write(name, ByteArray(0))
            }
            scope.cancel()
            clients.forEach { it.close() }
            if (process != null) {
                process.destroy()
                if (!process.waitFor(STOP_GUARD_MS, TimeUnit.MILLISECONDS)) {
                    process.destroyForcibly().waitFor(STOP_GUARD_MS, TimeUnit.MILLISECONDS)
                }
            }
            for (child in children) child.onExit().get(HANG_GUARD_MS, TimeUnit.MILLISECONDS)
        } finally {
            dispatcher.close()
            root.toFile().deleteRecursively()
        }
    }

    /** The fake tools, a home inside [root], and none of the caller's fake behaviours. */
    private fun toolEnvironment(builder: ProcessBuilder) {
        val env = builder.environment()
        env.keys.removeAll { it.startsWith("TRD_FAKE_") }
        env["TRD_FFMPEG_DIR"] = tools.toString()
        env["TRD_EXIFTOOL_DIR"] = tools.toString()
        env["HOME"] = home.toString()
    }

    private fun initTree() {
        Files.createDirectories(home)
        val script = repositoryFile("trd/sandbox/sandbox.py")
        val log = root.resolve("sandbox.log")
        val builder = ProcessBuilder("python3", "-I", script.toString(), "init", "--dir", tree.toString())
            .redirectErrorStream(true)
            .redirectOutput(log.toFile())
        toolEnvironment(builder)
        val process = builder.start()
        process.outputStream.close()
        val ended = process.waitFor(HANG_GUARD_MS, TimeUnit.MILLISECONDS)
        if (!ended) process.destroyForcibly()
        check(ended && process.exitValue() == 0) { "sandbox.py init failed: ${String(Files.readAllBytes(log))}" }
    }

    companion object {
        private val IN_FLIGHT =
            setOf(VariantStatus.VARIANT_QUEUED, VariantStatus.VARIANT_RUNNING, VariantStatus.VARIANT_SUSPENDED)

        /** A new sandbox tree. Skips the calling test unless `TRM_E2E=1`. */
        fun create(): TrdProcess {
            assumeTrue("contract tests run only with TRM_E2E=1", System.getenv("TRM_E2E") == "1")
            val binary = requiredPath("TRM_TRD_BIN")
            val tools = requiredPath("TRM_FAKE_TOOLS_DIR")
            val root = Files.createTempDirectory("trm-contract-").toRealPath()
            val process = TrdProcess(root, binary, tools)
            try {
                process.initTree()
            } catch (e: Throwable) {
                process.close()
                throw e
            }
            return process
        }

        private fun requiredPath(variable: String): Path {
            val value = System.getenv(variable)
            check(!value.isNullOrEmpty()) { "$variable is not set; run scripts/contract-test.sh" }
            val path = Paths.get(value).toAbsolutePath()
            check(Files.exists(path)) { "$variable names $path, which does not exist" }
            return path
        }

        /** A file of `v3/`, found from the module directory the tests run in. */
        private fun repositoryFile(relative: String): Path =
            generateSequence(Paths.get("").toAbsolutePath()) { it.parent }
                .map { it.resolve(relative) }
                .firstOrNull { Files.isRegularFile(it) }
                ?: throw IllegalStateException("$relative is not above the working directory")

        private fun writeAtomically(file: Path, text: String) {
            val temp = file.resolveSibling(file.fileName.toString() + ".new")
            Files.write(temp, text.toByteArray(Charsets.UTF_8))
            Files.move(temp, file, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        }
    }
}
