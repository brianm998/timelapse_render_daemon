// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigIssue
import com.timelapserender.trd.v1.ConfigState
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.RenderVariant
import com.timelapserender.trd.v1.VideoCodec
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

const val CONFIG_CONFLICT_MESSAGE = "The config changed on the server"

data class ConfigEditorUi(
    val loading: Boolean = true,
    /** The config file on the server, and its text as last read or saved. */
    val path: String = "",
    val rawJson: String = "",
    /** The editable copy; null until the first load succeeds. */
    val config: Config? = null,
    val codecs: List<VideoCodec> = emptyList(),
    /** Every issue of the latest validation (or of the load until the first edit). */
    val issues: List<ConfigIssue> = emptyList(),
    /** Issues by JSON pointer (`/rendering/2/codec`). */
    val issuesByField: Map<String, List<ConfigIssue>> = emptyMap(),
    /** The server's computed rendering rows (variant keys, example file names) from the latest validation. */
    val rendering: List<RenderVariant> = emptyList(),
    val dirty: Boolean = false,
    /** An edit was made and its validation has not answered yet. */
    val validationPending: Boolean = false,
    val saving: Boolean = false,
    /** FAILED_PRECONDITION: the file changed under the editor; offer [ConfigEditorModel.reload]. */
    val conflict: Boolean = false,
    val error: String? = null,
) {
    val hasErrors: Boolean get() = issues.any { it.severity == IssueSeverity.ISSUE_ERROR }

    /** Disabled while loading, saving, validating or while any ERROR issue exists. */
    val canSave: Boolean get() = config != null && !loading && !saving && !validationPending && !hasErrors
}

/** Spec 2 §5.4: `GetConfig`, an editable copy, `ValidateConfig` after [debounceMs] of quiet, save with revisions. */
class ConfigEditorModel(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val debounceMs: Long = 400,
) {
    private val _state = MutableStateFlow(ConfigEditorUi())
    val state: StateFlow<ConfigEditorUi> = _state.asStateFlow()

    private var loaded: ConfigState? = null
    private var config: Config = Config.getDefaultInstance()
    private var codecs: List<VideoCodec> = emptyList()
    private var validation: Job? = null
    private var loadJob: Job? = null
    private var saveJob: Job? = null
    private var saving = false

    /** Loads (or reloads) the config, discarding the edited copy and any conflict. */
    fun load() {
        validation?.cancel()
        loadJob?.cancel()
        saveJob?.cancel()
        saving = false
        _state.update { it.copy(loading = true, saving = false, error = null, validationPending = false) }
        loadJob = scope.launch {
            try {
                val s = client.getConfig()
                loaded = s
                config = s.config
                codecs = s.codecsList
                _state.value = ConfigEditorUi(loading = false)
                publish(s.issuesList, emptyList(), conflict = false, error = null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, error = e.statusMessage()) }
            }
        }
    }

    fun reload() = load()

    /** Replaces the edited config with `transform` of it and schedules validation. */
    fun edit(transform: (Config) -> Config) {
        if (loaded == null) return
        config = transform(config)
        changed()
    }

    fun editCodecs(transform: (List<VideoCodec>) -> List<VideoCodec>) {
        if (loaded == null) return
        codecs = transform(codecs)
        changed()
    }

    private fun changed() {
        publish(_state.value.issues, _state.value.rendering, pending = true)
        validation?.cancel()
        validation = scope.launch {
            delay(debounceMs)
            try {
                val r = client.validateConfig(config, if (codecsChanged()) codecs else emptyList())
                publish(r.issuesList, r.renderingList, error = null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                publish(_state.value.issues, _state.value.rendering, error = e.statusMessage())
            }
        }
    }

    /**
     * Sends `UpdateConfig` and, only if the codecs were edited, `UpdateCodecs`, each with the loaded
     * revision. The config goes first, unless it renders with a codec that this edit adds. Edits made
     * while the save is in flight are kept, and validated again.
     */
    fun save() {
        val base = loaded ?: return
        if (!_state.value.canSave) return
        validation?.cancel()
        val sentConfig = config
        val sentCodecs = codecs
        val configEdited = sentConfig != base.config
        val codecsEdited = sentCodecs != base.codecsList
        val added = sentCodecs.map { it.name }.toSet() - base.codecsList.map { it.name }.toSet()
        val codecsFirst = codecsEdited && sentConfig.renderingList.any { it.codec in added }
        saving = true
        _state.update { it.copy(saving = true, validationPending = false, error = null) }
        saveJob = scope.launch {
            var saved = base
            // After one of two updates, the other half keeps its loaded value and revision, so that a
            // retry still detects a change made on the server.
            suspend fun sendConfig() {
                saved = client.updateConfig(sentConfig, base.revision)
                val current = loaded ?: base
                loaded = if (codecsEdited && !codecsFirst) {
                    saved.toBuilder().clearCodecs().addAllCodecs(current.codecsList)
                        .setCodecsRevision(current.codecsRevision).build()
                } else {
                    saved
                }
            }
            suspend fun sendCodecs() {
                saved = client.updateCodecs(sentCodecs, base.codecsRevision)
                val current = loaded ?: base
                loaded = if (configEdited && codecsFirst) {
                    saved.toBuilder().setConfig(current.config).setRevision(current.revision).build()
                } else {
                    saved
                }
            }
            try {
                if (codecsEdited && codecsFirst) sendCodecs()
                if (configEdited) sendConfig()
                if (codecsEdited && !codecsFirst) sendCodecs()
                saving = false
                val editedMeanwhile = config != sentConfig || codecs != sentCodecs
                if (config == sentConfig) config = saved.config
                if (codecs == sentCodecs) codecs = saved.codecsList
                publish(saved.issuesList, emptyList(), conflict = false, error = null)
                if (editedMeanwhile) changed()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                saving = false
                val conflict = e.statusCode() == Status.Code.FAILED_PRECONDITION
                val editedMeanwhile = config != sentConfig || codecs != sentCodecs
                publish(
                    _state.value.issues, _state.value.rendering, conflict = conflict,
                    error = if (conflict) CONFIG_CONFLICT_MESSAGE else e.statusMessage(),
                )
                if (editedMeanwhile) changed()
            }
        }
    }

    private fun codecsChanged() = loaded?.let { codecs != it.codecsList } ?: false

    private fun publish(
        issues: List<ConfigIssue>,
        rendering: List<RenderVariant>,
        pending: Boolean = false,
        conflict: Boolean = _state.value.conflict,
        error: String? = _state.value.error,
    ) {
        val base = loaded
        _state.value = ConfigEditorUi(
            loading = false,
            path = base?.path.orEmpty(),
            rawJson = base?.rawJson.orEmpty(),
            config = config,
            codecs = codecs,
            issues = issues,
            issuesByField = issues.groupBy { it.field },
            rendering = rendering,
            dirty = base != null && (config != base.config || codecs != base.codecsList),
            validationPending = pending,
            saving = saving,
            conflict = conflict,
            error = error,
        )
    }
}
