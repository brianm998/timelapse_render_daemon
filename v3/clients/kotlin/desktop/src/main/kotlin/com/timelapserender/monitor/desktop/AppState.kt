// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.ui.window.Notification
import com.timelapserender.monitor.core.discovery.ServerDiscovery
import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.net.ChannelFactory
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.pairing.PairingClient
import com.timelapserender.monitor.core.pairing.PairingException
import com.timelapserender.monitor.core.servers.RegisteredServer
import com.timelapserender.monitor.core.servers.ServerRecord
import com.timelapserender.monitor.core.servers.ServerRegistry
import com.timelapserender.monitor.core.servers.ServerRegistry.Companion.LOCAL_SERVER_ID
import com.timelapserender.monitor.core.servers.ServerSource
import com.timelapserender.monitor.core.state.ActivityFilter
import com.timelapserender.monitor.core.state.ActivityPresenter
import com.timelapserender.monitor.core.state.ActivityUi
import com.timelapserender.monitor.core.state.Commands
import com.timelapserender.monitor.core.state.EventWatcher
import com.timelapserender.monitor.core.state.LogModel
import com.timelapserender.monitor.core.state.ScrollKeeper
import com.timelapserender.monitor.core.state.SnapshotRepository
import com.timelapserender.monitor.desktop.platform.DesktopSettings
import com.timelapserender.monitor.desktop.platform.TrdService
import com.timelapserender.monitor.desktop.platform.WindowGeometry
import com.timelapserender.monitor.desktop.ui.LogScroll
import com.timelapserender.monitor.desktop.ui.rows.PosterLoader
import com.timelapserender.monitor.desktop.ui.rows.ThumbnailPosters
import com.timelapserender.monitor.desktop.ui.TrayStatus
import com.timelapserender.monitor.desktop.ui.config.ConfigController
import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.Snapshot
import io.grpc.ManagedChannel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.nio.file.Path
import java.time.ZoneId
import java.util.Locale
import kotlin.coroutines.CoroutineContext

const val MIN_WINDOW_WIDTH = 900
const val MIN_WINDOW_HEIGHT = 600
const val DEFAULT_WINDOW_WIDTH = 1200f
const val DEFAULT_WINDOW_HEIGHT = 800f

/** How often the app asks the local server again while it does not answer (spec 2 §6.1). */
const val LOCAL_PROBE_MS = 2000L

/** Which of the three main views the window shows. */
enum class Screen {
    /** The first attempt to reach the local server has not been answered yet. */
    STARTING,
    NOT_RUNNING,
    MAIN,
}

sealed interface StartState {
    data object Idle : StartState

    data object Running : StartState

    data class Failed(val output: String) : StartState
}

sealed interface AppDialog {
    /** "Connect to another computer…": the server list. */
    data object Servers : AppDialog

    data class Pair(val server: RegisteredServer, val busy: Boolean = false, val error: String? = null) : AppDialog

    data object About : AppDialog
}

/** An open Config window and the server it edits. */
class ConfigWindowHandle internal constructor(
    val serverId: String,
    val serverName: String,
    val controller: ConfigController,
    private val client: TrdClient,
    private val scope: CoroutineScope,
) {
    internal fun close() {
        scope.cancel()
        client.close()
    }
}

/** The preferences as the preferences window shows them. */
data class PreferencesUi(
    val notifyFinished: Boolean = true,
    val notifyFailed: Boolean = true,
    val keepRunningInTray: Boolean = true,
    val snapshotRate: Int = DEFAULT_SNAPSHOT_RATE,
    /** Whether any "Don't ask again" was chosen. */
    val confirmationsSkipped: Boolean = false,
)

private fun AppPreferences.toUi() = PreferencesUi(
    notifyFinished,
    notifyFailed,
    keepRunningInTray,
    snapshotRate,
    skipStopConfirmation || skipPauseAllConfirmation,
)

/** Everything the app draws: the main window, the tray and the preferences window. */
data class AppUi(
    val servers: List<RegisteredServer> = emptyList(),
    val activeServerId: String = LOCAL_SERVER_ID,
    val connection: ConnectionState = ConnectionState.Connecting,
    /** Whether the active server has answered since it was selected. */
    val everConnected: Boolean = false,
    val activity: ActivityUi? = null,
    /** The choices of the export-path filter. */
    val exportPaths: List<String> = emptyList(),
    /** The Details pane's text while nothing is selected. */
    val lastScanText: String = NOT_SCANNED,
    val filter: ActivityFilter = ActivityFilter(),
    val detailsVisible: Boolean = true,
    val start: StartState = StartState.Idle,
    val dialog: AppDialog? = null,
    /** A message about a failed action, until dismissed. */
    val notice: String? = null,
    /** Counts the requests to focus the search field (Cmd/Ctrl+F). */
    val searchFocusRequests: Int = 0,
    /** The Activity row the user selected, by its list key; null with nothing selected. */
    val selectedKey: String? = null,
    /** False while the app is only in the tray. */
    val windowVisible: Boolean = true,
    /** Counts the requests to bring the window to the front. */
    val openWindowRequests: Int = 0,
    val trayStatus: TrayStatus = TrayStatus.IDLE,
    val preferencesOpen: Boolean = false,
    val preferences: PreferencesUi = PreferencesUi(),
) {
    val activeServer: RegisteredServer? get() = servers.firstOrNull { it.serverId == activeServerId }

    val screen: Screen
        get() = when {
            activeServerId != LOCAL_SERVER_ID || everConnected -> Screen.MAIN
            connection is ConnectionState.Connecting -> Screen.STARTING
            connection is ConnectionState.Reconnecting || connection is ConnectionState.Offline -> Screen.NOT_RUNNING
            else -> Screen.MAIN
        }

    val connected: Boolean get() = connection is ConnectionState.Connected

    /** Connected to the `trd` of this computer through its loopback listener. */
    val loopback: Boolean
        get() = activeServerId == LOCAL_SERVER_ID &&
            (connection as? ConnectionState.Connected)?.info?.callerIsLocal == true

    val windowTitle: String
        get() {
            val name = activity?.header?.serverName.orEmpty()
            return if (screen == Screen.MAIN && name.isNotEmpty()) "$WINDOW_TITLE — $name" else WINDOW_TITLE
        }

    companion object {
        const val NOT_SCANNED = "Not scanned yet"
    }
}

/**
 * The client for [server]. The local server is probed every [LOCAL_PROBE_MS]; others follow the
 * client's own back-off. The channel's back-off is reset after each wait so every attempt really
 * dials.
 */
fun createClient(
    server: RegisteredServer,
    scope: CoroutineScope,
    channelFor: (ServerEndpoint) -> ManagedChannel = ChannelFactory::create,
    delayer: suspend (Long) -> Unit = { delay(it) },
): TrdClient {
    val channel = channelFor(server.endpoint)
    val local = server.source == ServerSource.LOCAL
    return TrdClient(
        channel,
        server.token,
        scope,
        lan = server.endpoint.tls is TlsMode.Pinned,
        delayer = { backoff ->
            delayer(if (local) LOCAL_PROBE_MS else backoff)
            channel.resetConnectBackoff()
        },
    )
}

/**
 * The desktop app's state: the server registry fed by discovery, the connection to the active
 * server with its presenters, and what is remembered between runs. One server is active at a time;
 * selecting another closes the old connection and puts the list back at the top.
 */
class AppState(
    private val registry: ServerRegistry,
    private val discovery: ServerDiscovery,
    private val settings: DesktopSettings,
    private val service: TrdService,
    private val scope: CoroutineScope,
    /** True with `--server`: start on the local entry whatever was used last. */
    private val localOverride: Boolean = false,
    private val clientFactory: (RegisteredServer, CoroutineScope) -> TrdClient = { s, c -> createClient(s, c) },
    private val pairer: suspend (ServerEndpoint, String, String) -> ServerRecord = { endpoint, code, device ->
        PairingClient().pair(endpoint, code, device, DevicePlatform.DEVICE_PLATFORM_DESKTOP)
    },
    private val deviceName: () -> String = { "Desktop" },
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val io: CoroutineContext = Dispatchers.IO,
    /** Where posters are cached; with null the rows keep their placeholder. */
    private val posterCacheDir: Path? = null,
    private val preferences: AppPreferences = InMemoryAppPreferences(),
    /** Shows a notification (the tray's `sendNotification`). */
    private val notify: (Notification) -> Unit = {},
    /** Whether the system has a tray; without one a hidden window could not be reached. */
    private val traySupported: Boolean = false,
    /** The wait before the event stream is asked for again. */
    private val eventDelayer: suspend (Long) -> Unit = { delay(it) },
) : AutoCloseable {
    /** The connection to one server and the models over it. */
    class Session internal constructor(
        val server: RegisteredServer,
        val client: TrdClient,
        val repository: SnapshotRepository,
        val presenter: ActivityPresenter,
        val commands: Commands,
        val posters: PosterLoader,
        internal val scope: CoroutineScope,
    ) {
        internal fun close() {
            scope.cancel()
            client.close()
        }
    }

    /**
     * One Log window: its own connection to its server, so it stays live when the main window
     * switches to another one.
     */
    class LogSession internal constructor(
        val server: RegisteredServer,
        val title: String,
        val client: TrdClient,
        val model: LogModel,
        private val scope: CoroutineScope,
    ) {
        val scroll = LogScroll()
        private val focus = MutableStateFlow(0)

        /** Counts the requests to bring the window to the front. */
        val focusRequests: StateFlow<Int> = focus.asStateFlow()

        internal fun requestFocus() = focus.update { it + 1 }

        internal fun close() {
            model.stop()
            scope.cancel()
            client.close()
        }
    }

    private val lock = Any()
    private val _logs = MutableStateFlow<List<LogSession>>(emptyList())

    /** The open Log windows, one per server at most. */
    val logs: StateFlow<List<LogSession>> = _logs.asStateFlow()
    private val filter = MutableStateFlow(ActivityFilter(settings.search, settings.exportPath))
    private val snapshotRate = MutableStateFlow(preferences.snapshotRate)
    private val _ui = MutableStateFlow(
        AppUi(
            servers = registry.servers.value,
            filter = filter.value,
            detailsVisible = settings.detailsPaneVisible,
            preferences = preferences.toUi(),
        ),
    )
    private var pairing: Job? = null
    private val _configWindows = MutableStateFlow(emptyList<ConfigWindowHandle>())

    /** Set while a quit waits for the answers about unsaved config changes. */
    private var pendingQuit: (() -> Unit)? = null
    private var started = false

    @Volatile
    var session: Session? = null
        private set

    val ui: StateFlow<AppUi> = _ui.asStateFlow()

    /** The open Config windows, at most one per server. */
    val configWindows: StateFlow<List<ConfigWindowHandle>> = _configWindows.asStateFlow()

    /** The Activity list's scroll position and anchoring, shared with the list's UI. */
    val listState = LazyListState()
    val scrollKeeper = ScrollKeeper()

    val windowGeometry: WindowGeometry
        get() = clamp(settings.windowGeometry ?: WindowGeometry(null, null, DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT))

    fun saveWindowGeometry(geometry: WindowGeometry) {
        settings.windowGeometry = clamp(geometry)
    }

    /** Starts discovery and connects to the last active server, or the local one. */
    fun start() {
        synchronized(lock) {
            if (started) return
            started = true
        }
        scope.launch { discovery.servers.collect { registry.updateDiscovered(it) } }
        scope.launch { registry.servers.collect { servers -> _ui.update { it.copy(servers = servers) } } }
        scope.launch(io) { discovery.start() }
        val servers = registry.servers.value
        val last = settings.lastServerId.takeUnless { localOverride }
        val initial = servers.firstOrNull { it.serverId == last && it.source != ServerSource.DISCOVERED }
            ?: servers.firstOrNull { it.serverId == LOCAL_SERVER_ID }
        if (initial != null) connect(initial)
    }

    /** The server switcher: a discovered server is paired first, another one is connected to. */
    fun selectServer(serverId: String) {
        val server = registry.servers.value.firstOrNull { it.serverId == serverId } ?: return
        if (server.source == ServerSource.DISCOVERED) {
            openPairing(serverId)
            return
        }
        dismissDialog()
        if (session?.server?.serverId == serverId) return
        settings.lastServerId = serverId
        connect(server)
    }

    /** Removes a paired server and its token; the local server takes over when it was active. */
    fun forgetServer(serverId: String) {
        if (serverId == LOCAL_SERVER_ID) return
        val wasActive = session?.server?.serverId == serverId
        _configWindows.value.firstOrNull { it.serverId == serverId }?.let(::closeConfig)
        closeLog(serverId)
        registry.remove(serverId)
        _ui.update { it.copy(servers = registry.servers.value) }
        if (settings.lastServerId == serverId) settings.lastServerId = null
        if (wasActive) {
            registry.servers.value.firstOrNull { it.serverId == LOCAL_SERVER_ID }?.let { connect(it) }
        }
    }

    private fun connect(server: RegisteredServer) {
        synchronized(lock) {
            session?.close()
            resetList()
            registry.select(server.serverId)

            val sessionScope = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext.job))
            val client = clientFactory(server, sessionScope)
            val repository = SnapshotRepository(client, sessionScope, clock, maxPerSecond = snapshotRate)
            val presenter = ActivityPresenter(
                repository, filter, client, sessionScope, clock, zone, serverName = server.name,
            )
            val posters = posterCacheDir?.let { ThumbnailPosters(client, it) } ?: PosterLoader { null }
            val next = Session(server, client, repository, presenter, Commands(client), posters, sessionScope)
            session = next
            _ui.update {
                it.copy(
                    servers = registry.servers.value,
                    activeServerId = server.serverId,
                    connection = ConnectionState.Connecting,
                    everConnected = false,
                    trayStatus = TrayStatus.IDLE,
                    activity = presenter.ui.value,
                    exportPaths = exportPaths(null, it.filter.exportPath),
                    lastScanText = AppUi.NOT_SCANNED,
                    notice = null,
                    selectedKey = null,
                )
            }
            // Unconfined, so that no connection state is conflated away before a reconnect is seen.
            sessionScope.launch(Dispatchers.Unconfined, CoroutineStart.UNDISPATCHED) {
                var connectedBefore = false
                var lost = false
                client.state.collect { state ->
                    if (state is ConnectionState.Connected) {
                        if (lost) resetList()
                        connectedBefore = true
                        lost = false
                    } else if (connectedBefore) {
                        lost = true
                    }
                }
            }
            sessionScope.launch {
                combine(repository.state, presenter.ui) { _, _ -> }
                    .collect {
                        synchronized(lock) {
                            if (session !== next) return@synchronized
                            // Read in this order, the repository is never older than what was presented.
                            val activity = presenter.ui.value
                            val repo = repository.state.value
                            _ui.update {
                                val up = repo.connection is ConnectionState.Connected
                                it.copy(
                                    connection = repo.connection,
                                    everConnected = it.everConnected || up,
                                    activity = activity,
                                    trayStatus = trayStatus(up, repo.snapshot),
                                    exportPaths = exportPaths(repo.snapshot, it.filter.exportPath),
                                    lastScanText = lastScanText(repo.snapshot),
                                    // A start failure is stale once the local server answers.
                                    start = if (up && server.serverId == LOCAL_SERVER_ID &&
                                        it.start is StartState.Failed
                                    ) {
                                        StartState.Idle
                                    } else {
                                        it.start
                                    },
                                )
                            }
                        }
                    }
            }
            sessionScope.launch { watchEvents(client, repository) }
            client.start()
            repository.start()
        }
    }

    /**
     * Notifies the events of the connected server. The last seen event is kept under the id the
     * server reports, so a `--server` sandbox and the installed trd do not share it.
     */
    private suspend fun watchEvents(client: TrdClient, repository: SnapshotRepository) {
        val notifier = Notifier(preferences, notify) { failureKindLabel(it, repository, client) }
        client.state
            .mapNotNull { (it as? ConnectionState.Connected)?.info?.serverId }
            .distinctUntilChanged()
            .collectLatest { serverId ->
                try {
                    EventWatcher(client, preferences, serverId, delayer = eventDelayer)
                        .events()
                        .collect { notifier.handle(it) }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    System.err.println("Notifications stopped: ${e.message ?: e}")
                }
            }
    }

    /** From the whole snapshot, so the search and the export-path filter do not change the dot. */
    private fun trayStatus(connected: Boolean, snapshot: Snapshot?): TrayStatus = when {
        !connected || snapshot == null -> TrayStatus.IDLE
        snapshot.failuresCount > 0 || snapshot.counts.failures > 0 -> TrayStatus.FAILURES
        snapshot.counts.rendering > 0 -> TrayStatus.RENDERING
        else -> TrayStatus.IDLE
    }

    /** The window's close button: stays in the tray if that is wanted and possible, else [quit]. */
    fun closeWindow(quit: () -> Unit) {
        if (preferences.keepRunningInTray && traySupported) {
            _ui.update { it.copy(windowVisible = false) }
        } else {
            quit()
        }
    }

    fun openWindow() = _ui.update { it.copy(windowVisible = true, openWindowRequests = it.openWindowRequests + 1) }

    fun showPreferences() = _ui.update { it.copy(preferencesOpen = true) }

    fun closePreferences() = _ui.update { it.copy(preferencesOpen = false) }

    fun setNotifyFinished(on: Boolean) = changePreferences { notifyFinished = on }

    fun setNotifyFailed(on: Boolean) = changePreferences { notifyFailed = on }

    fun setKeepRunningInTray(on: Boolean) = changePreferences { keepRunningInTray = on }

    fun setSnapshotRate(perSecond: Int) = changePreferences {
        snapshotRate = perSecond
        this@AppState.snapshotRate.value = snapshotRate
    }

    fun resetConfirmations() {
        changePreferences { resetConfirmations() }
    }

    private fun changePreferences(change: AppPreferences.() -> Unit) {
        synchronized(lock) {
            preferences.change()
            _ui.update { it.copy(preferences = preferences.toUi()) }
        }
    }

    /** Opens the Log window of the active server, or brings the one it has to the front. */
    fun openLog() {
        synchronized(lock) {
            val current = session ?: return
            val open = _logs.value.firstOrNull { it.server.serverId == current.server.serverId }
            if (open != null) {
                open.requestFocus()
                return
            }
            val name = _ui.value.activity?.header?.serverName?.takeIf { it.isNotEmpty() } ?: current.server.name
            _logs.update { it + logSession(current.server, "Log — $name") }
        }
    }

    fun closeLog(serverId: String) {
        synchronized(lock) {
            val open = _logs.value.firstOrNull { it.server.serverId == serverId } ?: return
            _logs.update { it - open }
            open.close()
        }
    }

    private fun logSession(server: RegisteredServer, title: String): LogSession {
        val logScope = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext.job))
        val client = clientFactory(server, logScope)
        val model = LogModel(client, logScope, clock, zone)
        val log = LogSession(server, title, client, model, logScope)
        logScope.launch(Dispatchers.Unconfined, CoroutineStart.UNDISPATCHED) {
            var connectedBefore = false
            var lost = false
            client.state.collect { state ->
                if (state is ConnectionState.Connected) {
                    // Overview §6.2 S1: after a reconnect the list is at the top.
                    if (lost) log.scroll.reset()
                    connectedBefore = true
                    lost = false
                } else if (connectedBefore) {
                    lost = true
                }
            }
        }
        client.start()
        model.start()
        return log
    }

    /** After pairing again, an open Log window moves to a connection with the new token. */
    private fun renewLog(server: RegisteredServer) {
        val open = _logs.value.firstOrNull { it.server.serverId == server.serverId } ?: return
        open.close()
        val next = logSession(server, open.title)
        _logs.update { logs -> logs.map { if (it === open) next else it } }
    }

    /** Overview §6.2 S1: on first display, reconnect and server switch the list is at the top. */
    private fun resetList() {
        scrollKeeper.reset()
        listState.requestScrollToItem(0)
    }

    private fun exportPaths(snapshot: Snapshot?, selected: String?): List<String> {
        val paths = sortedSetOf<String>()
        if (snapshot != null) {
            snapshot.failuresList.forEach { paths.add(it.exportPath) }
            snapshot.inProgressList.forEach { paths.add(it.exportPath) }
            snapshot.recentlyRenderedList.forEach { paths.add(it.exportPath) }
        }
        // The selected path stays listed so the filter shows what it is set to.
        if (selected != null) paths.add(selected)
        return paths.filter { it.isNotEmpty() }
    }

    private fun lastScanText(snapshot: Snapshot?): String {
        if (snapshot == null || snapshot.lastScanCompletedMs == 0L) return AppUi.NOT_SCANNED
        val at = Format.absoluteTime(snapshot.lastScanCompletedMs, clock(), zone)
        val seconds = String.format(Locale.ENGLISH, "%.1f s", snapshot.lastScanDurationMs / 1000.0)
        return "Last scan $at · took $seconds"
    }

    fun setSearch(text: String) {
        settings.search = text
        filter.update { it.copy(search = text) }
        _ui.update { it.copy(filter = filter.value) }
    }

    fun setExportPath(path: String?) {
        settings.exportPath = path
        filter.update { it.copy(exportPath = path) }
        _ui.update { it.copy(filter = filter.value) }
    }

    /** A single click on an Activity row (spec 2 §6.3). */
    fun select(key: String) = _ui.update { it.copy(selectedKey = key) }

    /**
     * Opens the Config window of the active server, or brings it to the front. It has its own
     * connection, so it stays on its server when the main window switches to another.
     */
    fun openConfig() {
        synchronized(lock) {
            val server = session?.server ?: return
            val open = _configWindows.value.firstOrNull { it.serverId == server.serverId }
            if (open != null) {
                open.controller.requestFocus()
                return
            }
            val windowScope = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext.job))
            val client = clientFactory(server, windowScope)
            lateinit var handle: ConfigWindowHandle
            val controller = ConfigController(
                client,
                windowScope,
                onClose = { closeConfig(handle) },
                onStay = { synchronized(lock) { pendingQuit = null } },
            )
            val name = _ui.value.activity?.header?.serverName?.takeIf { it.isNotEmpty() } ?: server.name
            handle = ConfigWindowHandle(server.serverId, name, controller, client, windowScope)
            _configWindows.update { it + handle }
        }
    }

    private fun closeConfig(window: ConfigWindowHandle) {
        val quit = synchronized(lock) {
            _configWindows.update { it - window }
            window.close()
            pendingQuit?.takeIf { _configWindows.value.none { it.controller.dirty } }?.also { pendingQuit = null }
        }
        quit?.invoke()
    }

    /** Calls [quit] at once, or after every Config window with unsaved changes has been answered. */
    fun requestQuit(quit: () -> Unit) {
        val unsaved = synchronized(lock) {
            val unsaved = _configWindows.value.filter { it.controller.dirty }
            pendingQuit = quit.takeIf { unsaved.isNotEmpty() }
            unsaved
        }
        if (unsaved.isEmpty()) {
            quit()
            return
        }
        for (window in unsaved) {
            window.controller.requestFocus()
            window.controller.requestClose()
        }
    }

    fun requestSearchFocus() = _ui.update { it.copy(searchFocusRequests = it.searchFocusRequests + 1) }

    fun toggleDetails() {
        val visible = !_ui.value.detailsVisible
        settings.detailsPaneVisible = visible
        _ui.update { it.copy(detailsVisible = visible) }
    }

    fun setGlobalPause(paused: Boolean) = command { it.setGlobalPause(paused) }

    fun rescanNow() = command { it.rescanNow() }

    private fun command(block: suspend (Commands) -> Result<Unit>) {
        val current = session ?: return
        current.scope.launch {
            block(current.commands).onFailure { e -> _ui.update { it.copy(notice = e.message) } }
        }
    }

    /** Runs the start command; the probe of the local server notices when it comes up. */
    fun startTrd() {
        synchronized(lock) {
            if (_ui.value.start is StartState.Running) return
            _ui.update { it.copy(start = StartState.Running) }
        }
        scope.launch {
            val error = try {
                withContext(io) { service.start() }.error
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                e.message ?: e.toString()
            }
            _ui.update {
                if (error == null) {
                    it.copy(start = StartState.Idle)
                } else {
                    // The not-running screen shows the output itself; elsewhere it is a notice.
                    it.copy(
                        start = StartState.Failed(error),
                        notice = if (it.screen == Screen.MAIN) "Start trd failed: $error" else it.notice,
                    )
                }
            }
        }
    }

    fun openReleasesPage() = localAction { service.openReleasesPage().error }

    fun openLogFolder() {
        if (!_ui.value.loopback) return
        localAction { service.openLogFolder().error }
    }

    private fun localAction(block: () -> String?) {
        scope.launch {
            val error = try {
                withContext(io) { block() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                e.message ?: e.toString()
            }
            if (error != null) _ui.update { it.copy(notice = error) }
        }
    }

    fun dismissNotice() = _ui.update { it.copy(notice = null) }

    fun showServers() = showDialog(AppDialog.Servers)

    fun showAbout() = showDialog(AppDialog.About)

    /** Opens the code entry for a discovered server, or for a paired one whose token no longer works. */
    fun openPairing(serverId: String) {
        val server = registry.servers.value.firstOrNull { it.serverId == serverId } ?: return
        if (server.endpoint.tls !is TlsMode.Pinned) return
        showDialog(AppDialog.Pair(server))
    }

    private fun showDialog(dialog: AppDialog?) {
        synchronized(lock) {
            pairing?.cancel()
            _ui.update { it.copy(dialog = dialog) }
        }
    }

    fun dismissDialog() = showDialog(null)

    /** Pairs with the dialog's server using [code], stores the record and connects to it. */
    fun submitPairing(code: String) {
        val dialog = _ui.value.dialog as? AppDialog.Pair ?: return
        if (dialog.busy) return
        val digits = code.filterNot { it.isWhitespace() }
        if (digits.isEmpty()) return
        _ui.update { it.copy(dialog = dialog.copy(busy = true, error = null)) }
        pairing = scope.launch {
            try {
                val record = pairer(dialog.server.endpoint, digits, withContext(io) { deviceName() })
                synchronized(lock) {
                    // Dismissing takes the same lock, so a dismissed pairing stores nothing.
                    ensureActive()
                    registry.save(record)
                    settings.lastServerId = record.serverId
                    _ui.update { it.copy(dialog = null, servers = registry.servers.value) }
                    registry.servers.value.firstOrNull { it.serverId == record.serverId }?.let {
                        connect(it)
                        renewLog(it)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val message = (e as? PairingException)?.message ?: e.message ?: PairingClient.CONNECTION_FAILED
                synchronized(lock) {
                    ensureActive()
                    _ui.update { it.copy(dialog = dialog.copy(busy = false, error = message)) }
                }
            }
        }
    }

    override fun close() {
        synchronized(lock) {
            pairing?.cancel()
            session?.close()
            session = null
            _configWindows.value.forEach { it.close() }
            _configWindows.value = emptyList()
            _logs.value.forEach { it.close() }
            _logs.value = emptyList()
        }
        scope.cancel()
        discovery.stop()
    }

    private fun clamp(g: WindowGeometry) = g.copy(
        width = maxOf(g.width, MIN_WINDOW_WIDTH.toFloat()),
        height = maxOf(g.height, MIN_WINDOW_HEIGHT.toFloat()),
    )
}
