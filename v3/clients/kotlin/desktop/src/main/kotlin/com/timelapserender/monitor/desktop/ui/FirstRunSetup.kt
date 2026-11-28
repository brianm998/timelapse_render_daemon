// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.desktop.AppState
import com.timelapserender.monitor.desktop.AppUi
import com.timelapserender.trd.v1.ConfigPathSource
import com.timelapserender.trd.v1.Phase
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.awt.FileDialog
import java.awt.Frame

const val SETUP_TITLE = "Set up trd"
const val STEP_CONFIG = "Choose a config"
const val USE_EXISTING_CONFIG = "Use an existing config.json…"
const val CREATE_NEW_CONFIG = "Create a new config"
const val RESTART_HINT = "If trd still reports no config, restart it: trd ▸ Start trd."
const val STEP_DISK_ACCESS = "Allow Full Disk Access"
const val OPEN_PRIVACY_SETTINGS = "Open Privacy settings"
const val SHOW_TRD_IN_FINDER = "Show trd in Finder"
const val DISK_ACCESS_ALLOWED = "Full Disk Access is allowed."
const val SETUP_COMPLETE = "Nothing is left to set up."

/** The problem's detail and fix (spec 1 §6.12). */
const val DISK_ACCESS_DETAIL =
    "trd reads and writes your export, source, backup and video folders, which macOS protects. Until access " +
        "is granted trd scans nothing, so macOS never asks for permission while you are away."
const val DISK_ACCESS_FIX =
    "Open System Settings → Privacy & Security → Full Disk Access, add /Applications/Timelapse Render " +
        "Daemon.app (the + button), and turn it on. trd checks again every 30 seconds."

/** Where step 2 is: not asked for, waiting for the access, or granted. */
enum class DiskAccessStep { NONE, PENDING, DONE }

data class FirstRunUi(
    /** `SetConfigPath` is on its way. */
    val busy: Boolean = false,
    /** The config that was chosen or created. */
    val configPath: String? = null,
    /** Why the last attempt failed. */
    val error: String? = null,
    val diskAccess: DiskAccessStep = DiskAccessStep.NONE,
)

/**
 * One first-run setup against the local `trd` behind [client]. [defaultPath] is where this computer
 * keeps the config by default; [onChosen] runs after `trd` accepted a config.
 */
class FirstRunController(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val defaultPath: () -> String,
    private val onChosen: () -> Unit,
) {
    private val _ui = MutableStateFlow(FirstRunUi())
    val ui: StateFlow<FirstRunUi> = _ui.asStateFlow()

    fun useExisting(path: String) = choose(createIfMissing = false) { path }

    /** Creates the config at `trd`'s default path; a file already there is kept. */
    fun createNew() = choose(createIfMissing = true) {
        val state = client.getConfig()
        // trd's own answer where it is on its default path; else the default of this computer.
        state.path.takeIf { state.pathSource == ConfigPathSource.CONFIG_PATH_DEFAULT && it.isNotEmpty() }
            ?: defaultPath()
    }

    /** Takes in whether the latest snapshot lists the Full Disk Access problem. */
    fun observeDiskAccess(missing: Boolean) = _ui.update {
        val step = when {
            missing -> DiskAccessStep.PENDING
            it.diskAccess == DiskAccessStep.NONE -> DiskAccessStep.NONE
            else -> DiskAccessStep.DONE
        }
        it.copy(diskAccess = step)
    }

    private fun choose(createIfMissing: Boolean, path: suspend () -> String) {
        if (_ui.getAndUpdate { it.copy(busy = true, error = null) }.busy) return
        scope.launch {
            try {
                val chosen = path()
                client.setConfigPath(chosen, createIfMissing)
                _ui.update { it.copy(busy = false, configPath = chosen) }
                onChosen()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val status = Status.fromThrowable(e)
                _ui.update { it.copy(busy = false, error = status.description ?: status.code.name) }
            }
        }
    }
}

/** The system's file chooser for a `config.json` on this computer; null when it is cancelled. Blocks. */
fun chooseConfigFile(): String? {
    val dialog = FileDialog(null as Frame?, "Choose a config.json", FileDialog.LOAD)
    dialog.setFilenameFilter { _, name -> name.endsWith(".json", ignoreCase = true) }
    dialog.isVisible = true
    return dialog.files.firstOrNull()?.absolutePath
}

/**
 * First-run setup (spec 2 §6.7): choosing a config while `trd` has none, and on macOS allowing Full
 * Disk Access. The second step completes when the problem leaves the snapshot.
 */
@Composable
fun FirstRunSetup(ui: AppUi, controller: FirstRunController, app: AppState) {
    val setup by controller.ui.collectAsState()
    val choosing = ui.phase == Phase.PHASE_NO_CONFIG || setup.configPath != null
    val access = setup.diskAccess
    AlertDialog(
        onDismissRequest = app::dismissSetup,
        title = { Text(SETUP_TITLE) },
        text = {
            Column(
                Modifier.width(420.dp).heightIn(max = 480.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (choosing) ConfigStep(ui, setup, controller, app)
                if (choosing && access != DiskAccessStep.NONE) HorizontalDivider()
                if (access != DiskAccessStep.NONE) DiskAccessSection(access, app)
                if (!choosing && access == DiskAccessStep.NONE) Text(SETUP_COMPLETE)
            }
        },
        confirmButton = { TextButton(onClick = app::dismissSetup) { Text("Close") } },
    )
}

@Composable
private fun ConfigStep(ui: AppUi, setup: FirstRunUi, controller: FirstRunController, app: AppState) {
    Text(STEP_CONFIG, style = MaterialTheme.typography.titleMedium)
    val path = setup.configPath
    if (path != null) {
        Text("✓ Config: $path")
        if (ui.phase == Phase.PHASE_NO_CONFIG) Text(RESTART_HINT)
    } else {
        Text("trd has no config yet, so it scans nothing.")
    }
    // Still offered while trd has no config, to correct a wrong choice.
    if (ui.phase == Phase.PHASE_NO_CONFIG) {
        Button(onClick = app::chooseExistingConfig, enabled = !setup.busy) { Text(USE_EXISTING_CONFIG) }
        OutlinedButton(onClick = controller::createNew, enabled = !setup.busy) { Text(CREATE_NEW_CONFIG) }
    }
    setup.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
}

@Composable
private fun DiskAccessSection(step: DiskAccessStep, app: AppState) {
    Text(STEP_DISK_ACCESS, style = MaterialTheme.typography.titleMedium)
    if (step == DiskAccessStep.DONE) {
        Text("✓ $DISK_ACCESS_ALLOWED")
        return
    }
    Text(DISK_ACCESS_DETAIL)
    Text(DISK_ACCESS_FIX)
    Button(onClick = app::openPrivacySettings) { Text(OPEN_PRIVACY_SETTINGS) }
    OutlinedButton(onClick = app::showTrdInFinder) { Text(SHOW_TRD_IN_FINDER) }
}
