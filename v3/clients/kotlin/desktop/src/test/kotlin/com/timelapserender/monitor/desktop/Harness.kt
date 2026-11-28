// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import com.timelapserender.monitor.core.discovery.DiscoveredServer
import com.timelapserender.monitor.core.discovery.ServerDiscovery
import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.core.servers.ServerRegistry
import com.timelapserender.monitor.core.servers.ServerStore
import com.timelapserender.monitor.desktop.platform.CommandResult
import com.timelapserender.monitor.desktop.platform.CommandRunner
import com.timelapserender.monitor.desktop.platform.DesktopSettings
import com.timelapserender.monitor.desktop.platform.HostOs
import com.timelapserender.monitor.desktop.platform.TrdService
import com.timelapserender.monitor.desktop.platform.WindowGeometry
import com.timelapserender.trd.v1.ServerInfo
import io.grpc.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Surface
import java.nio.file.Files
import java.nio.file.Path
import java.time.ZoneOffset
import java.util.concurrent.CopyOnWriteArrayList

class FakeSettings(
    override var lastServerId: String? = null,
    override var windowGeometry: WindowGeometry? = null,
    override var detailsPaneVisible: Boolean = true,
    override var search: String = "",
    override var exportPath: String? = null,
) : DesktopSettings

class FakeDiscovery : ServerDiscovery {
    override val servers = MutableStateFlow<List<DiscoveredServer>>(emptyList())

    @Volatile var started = false

    @Volatile var stopped = false

    override fun start() {
        started = true
    }

    override fun stop() {
        stopped = true
    }
}

class MemoryStore : ServerStore {
    private val records = LinkedHashMap<String, ServerRecord>()

    @Synchronized
    override fun list() = records.values.toList()

    @Synchronized
    override fun save(record: ServerRecord) {
        records[record.serverId] = record
    }

    @Synchronized
    override fun remove(serverId: String) {
        records.remove(serverId)
    }
}

/** Records every command and runs none; [results] answers by program name. */
class RecordingRunner : CommandRunner {
    val calls = CopyOnWriteArrayList<List<String>>()

    @Volatile var results: Map<String, CommandResult> = mapOf("/usr/bin/id" to CommandResult(0, "501\n"))

    /** When set, every command throws it after being recorded. */
    @Volatile var failure: Exception? = null

    override fun run(argv: List<String>): CommandResult {
        calls += argv
        failure?.let { throw it }
        return results[argv.first()] ?: CommandResult(0, "")
    }
}

/** A small JPEG, as a server's poster. */
fun jpeg(): ByteArray = Surface.makeRasterN32Premul(32, 18).use { surface ->
    surface.canvas.clear(0xFF336699.toInt())
    checkNotNull(surface.makeImageSnapshot().encodeToData(EncodedImageFormat.JPEG)).bytes
}

fun record(id: String, port: Int, name: String = "stored-$id") =
    ServerRecord(id, name, "10.0.0.2", port, "ab", "token-$id", "linux-x64", 1L)

fun discovered(id: String, port: Int, name: String = "found-$id") =
    DiscoveredServer(id, name, "10.0.0.3", port, "cd", "3.0.0", 1)

/**
 * An [AppState] over in-process fakes: [local] answers at the local entry, [other] at port
 * [OTHER_PORT]. Nothing is dialled on the network or run on the computer; only posters are written,
 * to a temporary directory.
 */
class Harness(
    val settings: FakeSettings = FakeSettings(),
    val store: MemoryStore = MemoryStore(),
    localOverride: Boolean = false,
    os: HostOs = HostOs.MAC,
    pairer: suspend (ServerEndpoint, String, String) -> ServerRecord = { _, _, _ -> error("no pairing expected") },
) : AutoCloseable {
    val local = FakeTrdService()
    val other = FakeTrdService().also { it.serverInfo = info("other", "Other trd", local = false) }
    private val runningLocal = local.startInProcess()
    private val runningOther = other.startInProcess()
    val discovery = FakeDiscovery()
    val runner = RecordingRunner()
    val posterDir: Path = Files.createTempDirectory("trm-posters")

    /** The waits between connect attempts, as the app asked for them. */
    val delays = CopyOnWriteArrayList<Long>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val registry = ServerRegistry(store, ServerEndpoint("127.0.0.1", LOCAL_PORT, TlsMode.Plaintext))
    val app = AppState(
        registry = registry,
        discovery = discovery,
        settings = settings,
        service = TrdService(os, runner, "/home/tester") { null },
        scope = scope,
        localOverride = localOverride,
        clientFactory = { server, sessionScope ->
            createClient(
                server,
                sessionScope,
                channelFor = { if (it.port == LOCAL_PORT) runningLocal.newChannel() else runningOther.newChannel() },
                delayer = { ms ->
                    delays += ms
                    delay(10)
                },
            )
        },
        pairer = pairer,
        deviceName = { "test-desk" },
        zone = ZoneOffset.UTC,
        posterCacheDir = posterDir,
    )

    fun localDown() {
        local.failWith["GetServerInfo"] = Status.UNAVAILABLE
    }

    fun localUp() {
        local.failWith.clear()
    }

    fun await(predicate: (AppUi) -> Boolean): AppUi =
        runBlocking { withTimeout(TIMEOUT_MS) { app.ui.first(predicate) } }

    fun awaitTrue(condition: () -> Boolean) = runBlocking {
        withTimeout(TIMEOUT_MS) {
            while (!condition()) delay(10)
        }
    }

    override fun close() {
        app.close()
        runningLocal.close()
        runningOther.close()
        posterDir.toFile().deleteRecursively()
    }

    companion object {
        const val LOCAL_PORT = 50081
        const val OTHER_PORT = 7
        const val TIMEOUT_MS = 60_000L

        fun info(id: String, name: String, local: Boolean = true, sandbox: Boolean = false): ServerInfo =
            ServerInfo.newBuilder().setServerId(id).setName(name).setApiVersion(1)
                .setCallerIsLocal(local).setCallerIsPaired(!local).setSandbox(sandbox).build()
    }
}
