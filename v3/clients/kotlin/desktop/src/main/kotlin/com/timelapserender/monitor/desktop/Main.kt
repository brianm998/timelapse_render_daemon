// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.type
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.isSpecified
import androidx.compose.ui.window.Notification
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.WindowPlacement
import androidx.compose.ui.window.WindowPosition
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberTrayState
import androidx.compose.ui.window.rememberWindowState
import com.timelapserender.monitor.core.discovery.JmdnsDiscovery
import com.timelapserender.monitor.core.net.ServerEndpoint
import com.timelapserender.monitor.core.net.TlsMode
import com.timelapserender.monitor.core.servers.JsonFileServerStore
import com.timelapserender.monitor.core.servers.ServerRegistry
import com.timelapserender.monitor.core.servers.defaultDataDir
import com.timelapserender.monitor.desktop.platform.HostOs
import com.timelapserender.monitor.desktop.platform.PreferencesSettings
import com.timelapserender.monitor.desktop.platform.SystemCommandRunner
import com.timelapserender.monitor.desktop.platform.TrdService
import com.timelapserender.monitor.desktop.platform.WindowGeometry
import com.timelapserender.monitor.desktop.platform.defaultCacheDir
import com.timelapserender.monitor.desktop.ui.AppMenuBar
import com.timelapserender.monitor.desktop.ui.AppTray
import com.timelapserender.monitor.desktop.ui.LogWindow
import com.timelapserender.monitor.desktop.ui.MainWindow
import com.timelapserender.monitor.desktop.ui.PreferencesWindow
import com.timelapserender.monitor.desktop.ui.isSearchShortcut
import com.timelapserender.monitor.desktop.ui.perform
import com.timelapserender.monitor.desktop.ui.performTray
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.Dispatchers
import java.awt.Dimension
import java.awt.Frame
import java.awt.SystemTray
import java.net.InetAddress
import java.util.prefs.Preferences
import kotlin.system.exitProcess

const val WINDOW_TITLE = "Timelapse Render Monitor"

/** The command line (spec 2 §6.1). [server] is the `--server` plaintext loopback override. */
data class Options(val server: ServerEndpoint? = null)

private val LOOPBACK_HOSTS = setOf("127.0.0.1", "localhost", "::1")

/** @throws IllegalArgumentException with a message for the user. */
fun parseOptions(args: List<String>): Options {
    var server: ServerEndpoint? = null
    var i = 0
    while (i < args.size) {
        val arg = args[i]
        // Old macOS launchers add a process serial number.
        if (arg.startsWith("-psn_")) {
            i++
            continue
        }
        require(arg == "--server") { "Unknown option: $arg" }
        val value = requireNotNull(args.getOrNull(i + 1)) { "--server needs host:port" }
        val host = value.substringBeforeLast(':', "").removePrefix("[").removeSuffix("]")
        val port = value.substringAfterLast(':').toIntOrNull()
        require(host.isNotEmpty() && port != null && port in 1..65535) { "--server needs host:port, not $value" }
        require(host in LOOPBACK_HOSTS) { "--server is for a trd on this computer (127.0.0.1), not $host" }
        server = ServerEndpoint(host, port, TlsMode.Plaintext)
        i += 2
    }
    return Options(server)
}

/** The geometry worth remembering, or null while the window is maximized, full screen or unplaced. */
fun geometryOf(size: DpSize, position: WindowPosition, placement: WindowPlacement): WindowGeometry? {
    if (placement != WindowPlacement.Floating || !size.isSpecified) return null
    val absolute = position as? WindowPosition.Absolute
    return WindowGeometry(absolute?.x?.value, absolute?.y?.value, size.width.value, size.height.value)
}

fun positionOf(geometry: WindowGeometry): WindowPosition {
    val x = geometry.x
    val y = geometry.y
    return if (x == null || y == null) WindowPosition.PlatformDefault else WindowPosition(x.dp, y.dp)
}

/** Shown until the first attempt to reach the local server is answered. */
@Composable
fun StartScreen() {
    MaterialTheme {
        Text("Not connected")
    }
}

private fun createAppState(options: Options, notify: (Notification) -> Unit, traySupported: Boolean): AppState {
    val home = System.getProperty("user.home")
    val dataDir = defaultDataDir(System.getProperty("os.name"), System::getenv, home)
    val node = Preferences.userRoot().node(PreferencesSettings.NODE)
    return AppState(
        registry = ServerRegistry(JsonFileServerStore(dataDir), ServerRegistry.localEndpoint(options.server)),
        discovery = JmdnsDiscovery(),
        settings = PreferencesSettings(node),
        service = TrdService(HostOs.current, SystemCommandRunner(), home),
        scope = CoroutineScope(SupervisorJob() + Dispatchers.Swing),
        localOverride = options.server != null,
        deviceName = { runCatching { InetAddress.getLocalHost().hostName }.getOrDefault("Desktop") },
        posterCacheDir = defaultCacheDir(System.getProperty("os.name"), System::getenv, home),
        preferences = JavaAppPreferences(node),
        notify = notify,
        traySupported = traySupported,
    )
}

fun main(args: Array<String>) {
    val options = try {
        parseOptions(args.toList())
    } catch (e: IllegalArgumentException) {
        System.err.println(e.message)
        exitProcess(2)
    }
    val mac = HostOs.current == HostOs.MAC
    if (mac) {
        System.setProperty("apple.laf.useScreenMenuBar", "true")
        System.setProperty("apple.awt.application.name", WINDOW_TITLE)
    }
    application {
        val trayState = rememberTrayState()
        val traySupported = remember { SystemTray.isSupported() }
        val app = remember {
            createAppState(options, trayState::sendNotification, traySupported).also { it.start() }
        }
        val ui by app.ui.collectAsState()
        val geometry = remember { app.windowGeometry }
        val windowState = rememberWindowState(
            size = DpSize(geometry.width.dp, geometry.height.dp),
            position = positionOf(geometry),
        )
        val quit = {
            app.close()
            exitApplication()
        }
        val close = { app.closeWindow(quit) }
        if (traySupported) {
            AppTray(ui, trayState) { app.performTray(it, quit) }
        }
        if (ui.preferencesOpen) PreferencesWindow(app)
        Window(
            onCloseRequest = close,
            visible = ui.windowVisible,
            state = windowState,
            title = ui.windowTitle,
            onPreviewKeyEvent = { event ->
                val search = event.type == KeyEventType.KeyDown &&
                    isSearchShortcut(event.key, event.isMetaPressed, event.isCtrlPressed, mac)
                if (search) app.requestSearchFocus()
                search
            },
        ) {
            LaunchedEffect(Unit) { window.minimumSize = Dimension(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT) }
            LaunchedEffect(windowState) {
                snapshotFlow { geometryOf(windowState.size, windowState.position, windowState.placement) }
                    .collect { it?.let(app::saveWindowGeometry) }
            }
            LaunchedEffect(ui.openWindowRequests) {
                if (ui.openWindowRequests > 0) {
                    window.extendedState = window.extendedState and Frame.ICONIFIED.inv()
                    window.toFront()
                    window.requestFocus()
                }
            }
            AppMenuBar(ui, mac) { action -> app.perform(action, close) }
            MainWindow(app)
        }
        val logs by app.logs.collectAsState()
        for (log in logs) {
            key(log) { LogFrame(log) { app.closeLog(log.server.serverId) } }
        }
    }
}

/** The window of one server's log; a new focus request brings it to the front. */
@Composable
private fun LogFrame(log: AppState.LogSession, onClose: () -> Unit) {
    val state = rememberWindowState(size = DpSize(MIN_WINDOW_WIDTH.dp, MIN_WINDOW_HEIGHT.dp))
    Window(onCloseRequest = onClose, state = state, title = log.title) {
        val focus by log.focusRequests.collectAsState()
        LaunchedEffect(focus) {
            state.isMinimized = false
            window.toFront()
            window.requestFocus()
        }
        LogWindow(log.model, log.scroll)
    }
}
