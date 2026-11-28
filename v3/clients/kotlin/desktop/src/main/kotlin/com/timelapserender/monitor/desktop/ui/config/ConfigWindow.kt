// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.CONFIG_CONFLICT_MESSAGE
import com.timelapserender.monitor.core.state.ConfigEditorModel
import com.timelapserender.monitor.core.state.ConfigEditorUi
import com.timelapserender.monitor.core.state.DirectoryBrowserModel
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.ConfigIssue
import com.timelapserender.trd.v1.IssueSeverity
import com.timelapserender.trd.v1.RenderVariant
import com.timelapserender.trd.v1.VideoCodec
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The groups of overview §6.4, in the sidebar's order, with the top-level keys each one edits. */
enum class ConfigGroup(val title: String, private val keys: Set<String>) {
    LOCATIONS("Locations", setOf("source_paths", "backup_paths", "export_paths", "master_videos")),
    MATCHING(
        "Matching",
        setOf(
            "image_name_prefix", "image_sequence_dirname_prefix", "raw_types", "image_extensions",
            "ignore_sequence_regexes",
        ),
    ),
    RENDERING("Rendering", setOf("rendering")),
    LIMITS("Limits", setOf("max_processes", "max_cuncurrent_renders", "max_concurrent_renders")),
    BEHAVIOUR(
        "Behaviour",
        setOf("scan_interval_seconds", "delete_sequence_after_render_regexes", "recent_render_history_days"),
    ),
    PAUSES("Pauses", setOf("paused_sequence_names", "paused_variants")),
    CODECS("Codecs", setOf("video_codecs")),
    ADVANCED("Advanced", emptySet()),
    ;

    companion object {
        /** The group of an issue's JSON pointer; one that no other group edits belongs to Advanced. */
        fun of(field: String): ConfigGroup {
            val key = field.removePrefix("/").substringBefore('/')
            return entries.firstOrNull { key in it.keys } ?: ADVANCED
        }
    }
}

/** What the Config window holds besides the editor's own state. */
data class ConfigWindowUi(
    val group: ConfigGroup = ConfigGroup.LOCATIONS,
    /** The save / discard / cancel question of a close with unsaved changes. */
    val closePrompt: Boolean = false,
    /** Why the last codec removal, rename or addition was refused. */
    val codecMessage: String? = null,
    /** A codec removal or rename is being checked with the server. */
    val checkingCodec: Boolean = false,
    /** The computed rendering rows of the loaded config, until an edit's validation brings newer ones. */
    val preview: List<RenderVariant> = emptyList(),
    /** Why `SetConfigPath` failed. */
    val pathMessage: String? = null,
    /** Counts the requests to bring the window to the front. */
    val focusRequests: Int = 0,
)

/**
 * One Config window: the editor over [client], the selected group and the close question.
 * [onClose] closes the window; [onStay] reports that a requested close did not happen.
 */
class ConfigController(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val onClose: () -> Unit = {},
    private val onStay: () -> Unit = {},
) {
    val editor = ConfigEditorModel(client, scope)
    private val _ui = MutableStateFlow(ConfigWindowUi())
    val ui: StateFlow<ConfigWindowUi> = _ui.asStateFlow()

    val dirty: Boolean get() = editor.state.value.dirty

    init {
        // GetConfig carries no example file names, so ask for them whenever there are none.
        scope.launch {
            editor.state
                .map { s -> s.config.takeIf { !s.loading && !s.dirty && s.rendering.isEmpty() } }
                .distinctUntilChanged()
                .collectLatest { config ->
                    if (config == null) return@collectLatest
                    val rows = try {
                        client.validateConfig(config).renderingList
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        emptyList()
                    }
                    _ui.update { it.copy(preview = rows) }
                }
        }
        editor.load()
    }

    fun selectGroup(group: ConfigGroup) = _ui.update { it.copy(group = group) }

    fun requestFocus() = _ui.update { it.copy(focusRequests = it.focusRequests + 1) }

    fun edit(transform: (Config.Builder) -> Unit) =
        editor.edit { config -> config.toBuilder().also(transform).build() }

    fun reload() {
        _ui.update { it.copy(codecMessage = null, pathMessage = null) }
        editor.reload()
    }

    /** Does nothing while Save is disabled. */
    fun save() {
        if (dirty) editor.save()
    }

    fun requestClose() {
        if (dirty) _ui.update { it.copy(closePrompt = true) } else onClose()
    }

    fun cancelClose() {
        _ui.update { it.copy(closePrompt = false) }
        onStay()
    }

    fun discardAndClose() {
        _ui.update { it.copy(closePrompt = false) }
        onClose()
    }

    /** Saves, then closes; the window stays open when the save does not leave it clean. */
    fun saveAndClose() {
        _ui.update { it.copy(closePrompt = false) }
        if (!editor.state.value.canSave) {
            onStay()
            return
        }
        editor.save()
        scope.launch {
            val s = editor.state.first { !it.saving }
            if (!s.dirty && s.error == null) onClose() else onStay()
        }
    }

    /** `SetConfigPath`, then loads that file. Refused here while there are unsaved changes. */
    fun setConfigPath(path: String) {
        if (dirty || path.isBlank()) return
        _ui.update { it.copy(pathMessage = null) }
        scope.launch {
            try {
                client.setConfigPath(path.trim(), createIfMissing = false)
                reload()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(pathMessage = rpcMessage(e)) }
            }
        }
    }

    fun addCodec(name: String) {
        val trimmed = name.trim()
        val codecs = editor.state.value.codecs
        val refusal = nameRefusal(trimmed, codecs)
        _ui.update { it.copy(codecMessage = refusal) }
        if (refusal == null) editor.editCodecs { it + VideoCodec.newBuilder().setName(trimmed).build() }
    }

    fun editCodec(name: String, transform: (VideoCodec.Builder) -> Unit) = editor.editCodecs { list ->
        list.map { if (it.name == name) it.toBuilder().also(transform).build() else it }
    }

    fun removeCodec(name: String) =
        changeUnlessUsed(name) { list -> list.filterNot { it.name == name } }

    fun renameCodec(from: String, to: String) {
        val trimmed = to.trim()
        if (trimmed == from) return
        val refusal = nameRefusal(trimmed, editor.state.value.codecs)
        if (refusal != null) {
            _ui.update { it.copy(codecMessage = refusal) }
            return
        }
        changeUnlessUsed(from) { list ->
            list.map { if (it.name == from) it.toBuilder().setName(trimmed).build() else it }
        }
    }

    private fun nameRefusal(name: String, codecs: List<VideoCodec>): String? = when {
        name.isEmpty() -> "A codec needs a name"
        codecs.any { it.name == name } -> "A codec named \"$name\" already exists"
        else -> null
    }

    /**
     * Applies [change], which takes the codec [name] away, unless the server finds that a rendering
     * variant needs it: then the table stays as it is and the server's message is shown.
     */
    private fun changeUnlessUsed(name: String, change: (List<VideoCodec>) -> List<VideoCodec>) {
        val state = editor.state.value
        val config = state.config ?: return
        if (_ui.value.checkingCodec) return
        val next = change(state.codecs)
        // An empty list would ask the server to validate against its current table.
        if (next.isEmpty()) return
        _ui.update { it.copy(checkingCodec = true, codecMessage = null) }
        scope.launch {
            val refusal = try {
                client.validateConfig(config, next).issuesList
                    .filter { it.severity == IssueSeverity.ISSUE_ERROR && usesCodec(config, it.field, name) }
                    .joinToString("\n") { "${it.field}: ${it.message}" }
                    .ifEmpty { null }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                rpcMessage(e)
            }
            if (refusal == null) editor.editCodecs(change)
            _ui.update { it.copy(checkingCodec = false, codecMessage = refusal) }
        }
    }

    private fun usesCodec(config: Config, field: String, codec: String): Boolean =
        config.renderingList.withIndex().any { (i, row) -> row.codec == codec && field == "/rendering/$i/codec" }

    fun directoryBrowser(scope: CoroutineScope, includeFiles: Boolean) =
        DirectoryBrowserModel(client, scope, includeFiles)

    private fun rpcMessage(e: Exception): String {
        val status = Status.fromThrowable(e)
        return status.description?.takeIf { it.isNotBlank() } ?: status.code.name
    }
}

/** Cmd+S on macOS, Ctrl+S elsewhere: save. */
fun isSaveShortcut(key: Key, meta: Boolean, ctrl: Boolean, mac: Boolean): Boolean =
    key == Key.S && (if (mac) meta else ctrl)

object ConfigTags {
    const val SAVE = "config-save"
    const val RELOAD = "config-reload"
    const val CONFIG_PATH = "config-path"
    const val USE_PATH = "config-use-path"
    const val BROWSE_PATH = "config-browse-path"
    const val PROMPT_SAVE = "config-prompt-save"
    const val PROMPT_DISCARD = "config-prompt-discard"
    const val PROMPT_CANCEL = "config-prompt-cancel"

    fun group(group: ConfigGroup) = "config-group-${group.name}"

    fun issueCount(group: ConfigGroup) = "config-count-${group.name}"

    /** The issues shown under the field with JSON pointer [field]. */
    fun issues(field: String) = "config-issues-$field"

    fun field(field: String) = "config-field-$field"
}

/** The Config window's content (spec 2 §6.5): the sidebar, the selected group's form and Save. */
@Composable
fun ConfigWindowContent(controller: ConfigController) {
    // Unconfined: a text field has to see its own edit at once, or typing is lost.
    val editor by controller.editor.state.collectAsState(Dispatchers.Unconfined)
    val window by controller.ui.collectAsState()
    var browse by remember { mutableStateOf<BrowseRequest?>(null) }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.weight(1f)) {
                Sidebar(editor, window.group, controller::selectGroup)
                VerticalDivider()
                val config = editor.config
                if (config == null) {
                    Column(Modifier.weight(1f).padding(16.dp)) {
                        Text(if (editor.loading) "Loading the config…" else "The config could not be loaded.")
                    }
                } else {
                    key(window.group) {
                        Column(
                            Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Text(window.group.title, style = MaterialTheme.typography.headlineSmall)
                            OtherIssues(editor, config, window.group)
                            val onBrowse: (BrowseRequest) -> Unit = { browse = it }
                            when (window.group) {
                                ConfigGroup.LOCATIONS -> LocationsGroup(controller, editor, config, onBrowse)
                                ConfigGroup.MATCHING -> MatchingGroup(controller, editor, config)
                                ConfigGroup.RENDERING -> RenderingGroup(controller, editor, config, window.preview)
                                ConfigGroup.LIMITS -> LimitsGroup(controller, editor, config)
                                ConfigGroup.BEHAVIOUR -> BehaviourGroup(controller, editor, config)
                                ConfigGroup.PAUSES -> PausesGroup(controller, editor, config)
                                ConfigGroup.CODECS -> CodecsGroup(controller, editor, window)
                                ConfigGroup.ADVANCED -> AdvancedGroup(controller, editor, config, window, onBrowse)
                            }
                        }
                    }
                }
            }
            HorizontalDivider()
            BottomBar(controller, editor)
        }
    }
    browse?.let { request ->
        FolderBrowserDialog(
            controller = controller,
            request = request,
            onDismiss = { browse = null },
        )
    }
    if (window.closePrompt) ClosePrompt(controller, editor)
}

@Composable
private fun Sidebar(editor: ConfigEditorUi, selected: ConfigGroup, select: (ConfigGroup) -> Unit) {
    val byGroup = editor.issues.groupBy { ConfigGroup.of(it.field) }
    Column(Modifier.width(180.dp).fillMaxHeight().verticalScroll(rememberScrollState())) {
        for (group in ConfigGroup.entries) {
            val issues = byGroup[group].orEmpty()
            val background = if (group == selected) {
                MaterialTheme.colorScheme.secondaryContainer
            } else {
                MaterialTheme.colorScheme.surface
            }
            Row(
                Modifier.fillMaxWidth()
                    .background(background)
                    .selectable(selected = group == selected, onClick = { select(group) })
                    .testTag(ConfigTags.group(group))
                    .padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(group.title, Modifier.weight(1f))
                if (issues.isNotEmpty()) {
                    Text(
                        issues.size.toString(),
                        modifier = Modifier.testTag(ConfigTags.issueCount(group)),
                        color = issueColor(issues.any { it.severity == IssueSeverity.ISSUE_ERROR }),
                    )
                }
            }
        }
    }
}

@Composable
private fun BottomBar(controller: ConfigController, editor: ConfigEditorUi) {
    Row(
        Modifier.fillMaxWidth().padding(12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            val message = if (editor.conflict) CONFIG_CONFLICT_MESSAGE else editor.error
            if (message != null) {
                Text(message, color = MaterialTheme.colorScheme.error)
            } else if (editor.dirty) {
                Text(if (editor.saving) "Saving…" else "Unsaved changes")
            }
        }
        if (editor.conflict || (editor.config == null && !editor.loading)) {
            TextButton(onClick = controller::reload, modifier = Modifier.testTag(ConfigTags.RELOAD)) { Text("Reload") }
        }
        Button(
            onClick = controller::save,
            enabled = editor.canSave && editor.dirty,
            modifier = Modifier.testTag(ConfigTags.SAVE),
        ) { Text("Save") }
    }
}

@Composable
private fun ClosePrompt(controller: ConfigController, editor: ConfigEditorUi) {
    AlertDialog(
        onDismissRequest = controller::cancelClose,
        title = { Text("Save the changes to the config?") },
        text = {
            Text(
                if (editor.canSave) {
                    "The changes are lost if you don't save them."
                } else {
                    "The changes cannot be saved right now. Discard them, or cancel and fix the config."
                },
            )
        },
        confirmButton = {
            TextButton(
                onClick = controller::saveAndClose,
                enabled = editor.canSave,
                modifier = Modifier.testTag(ConfigTags.PROMPT_SAVE),
            ) { Text("Save") }
        },
        dismissButton = {
            Row {
                TextButton(
                    onClick = controller::discardAndClose,
                    modifier = Modifier.testTag(ConfigTags.PROMPT_DISCARD),
                ) { Text("Discard") }
                TextButton(
                    onClick = controller::cancelClose,
                    modifier = Modifier.testTag(ConfigTags.PROMPT_CANCEL),
                ) { Text("Cancel") }
            }
        },
    )
}

@Composable
internal fun issueColor(error: Boolean) =
    if (error) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.tertiary

/** The server's issues for the field with JSON pointer [field], shown under it. */
@Composable
internal fun IssueLines(editor: ConfigEditorUi, field: String) {
    val issues = editor.issuesByField[field] ?: return
    Column(Modifier.testTag(ConfigTags.issues(field))) {
        for (issue in issues) IssueLine(issue, issue.message)
    }
}

@Composable
private fun IssueLine(issue: ConfigIssue, text: String) {
    val error = issue.severity == IssueSeverity.ISSUE_ERROR
    Text(
        (if (error) "Error: " else "Warning: ") + text,
        color = issueColor(error),
        style = MaterialTheme.typography.bodySmall,
    )
}

/** The JSON pointers that [group]'s form has a place for. */
private fun shownFields(config: Config, group: ConfigGroup): Set<String> {
    fun list(key: String, size: Int) = listOf("/$key") + List(size) { "/$key/$it" }
    return when (group) {
        ConfigGroup.LOCATIONS -> LOCATION_LISTS.flatMap { list(it.key, it.get(config).size) }
        ConfigGroup.MATCHING -> listOf("/image_name_prefix", "/image_sequence_dirname_prefix") +
            MATCHING_LISTS.flatMap { list(it.key, it.get(config).size) }
        ConfigGroup.RENDERING -> list("rendering", config.renderingCount) +
            List(config.renderingCount) { i -> RENDERING_FIELDS.map { "/rendering/$i/$it" } }.flatten()
        ConfigGroup.LIMITS ->
            listOf("/max_processes/ffmpeg", "/max_processes/exiftool", "/max_cuncurrent_renders")
        ConfigGroup.BEHAVIOUR -> listOf("/scan_interval_seconds", "/recent_render_history_days") +
            list(DELETE_REGEXES.key, DELETE_REGEXES.get(config).size)
        ConfigGroup.PAUSES -> list("paused_sequence_names", config.pausedSequenceNamesCount) +
            list("paused_variants", config.pausedVariantsCount)
        ConfigGroup.CODECS -> emptyList()
        ConfigGroup.ADVANCED -> listOf("/log_in_color", "/progress_bar_width", "/sleep_time_seconds")
    }.toSet()
}

/** The group's issues that no field shows, such as one about an entry `trd` left out of the config. */
@Composable
private fun OtherIssues(editor: ConfigEditorUi, config: Config, group: ConfigGroup) {
    val shown = shownFields(config, group)
    for (issue in editor.issues) {
        if (ConfigGroup.of(issue.field) == group && issue.field !in shown) {
            IssueLine(issue, if (issue.field.isEmpty()) issue.message else "${issue.field}: ${issue.message}")
        }
    }
}

@Composable
private fun LocationsGroup(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    browse: (BrowseRequest) -> Unit,
) {
    Text(
        "Folders on the trd machine. \$YEAR in a path stands for every year from this one back to 1970; " +
            "the folders that exist are used, newest first.",
        style = MaterialTheme.typography.bodyMedium,
    )
    for (list in LOCATION_LISTS) PathListEditor(controller, editor, config, list, browse)
}

@Composable
private fun MatchingGroup(controller: ConfigController, editor: ConfigEditorUi, config: Config) {
    TextSetting(editor, "Image name prefix", "/image_name_prefix", config.imageNamePrefix) { v ->
        controller.edit { it.imageNamePrefix = v }
    }
    TextSetting(
        editor, "Image sequence folder name prefix", "/image_sequence_dirname_prefix",
        config.imageSequenceDirnamePrefix,
    ) { v -> controller.edit { it.imageSequenceDirnamePrefix = v } }
    for (list in MATCHING_LISTS) StringListEditor(controller, editor, config, list)
}

@Composable
private fun LimitsGroup(controller: ConfigController, editor: ConfigEditorUi, config: Config) {
    CountSetting(editor, "ffmpeg processes", "/max_processes/ffmpeg", config.maxProcesses.ffmpeg) { v ->
        controller.edit { it.maxProcesses = it.maxProcesses.toBuilder().setFfmpeg(v).build() }
    }
    CountSetting(editor, "exiftool processes", "/max_processes/exiftool", config.maxProcesses.exiftool) { v ->
        controller.edit { it.maxProcesses = it.maxProcesses.toBuilder().setExiftool(v).build() }
    }
    CountSetting(editor, "Concurrent renders", "/max_cuncurrent_renders", config.maxConcurrentRenders) { v ->
        controller.edit { it.maxConcurrentRenders = v }
    }
}

@Composable
private fun BehaviourGroup(controller: ConfigController, editor: ConfigEditorUi, config: Config) {
    DecimalSetting(editor, "Scan interval (seconds)", "/scan_interval_seconds", config.scanIntervalSeconds) { v ->
        controller.edit { it.scanIntervalSeconds = v }
    }
    StringListEditor(controller, editor, config, DELETE_REGEXES)
    CountSetting(
        editor, "Days of recently rendered history", "/recent_render_history_days", config.recentRenderHistoryDays,
    ) { v -> controller.edit { it.recentRenderHistoryDays = v } }
}

@Composable
private fun PausesGroup(controller: ConfigController, editor: ConfigEditorUi, config: Config) {
    Text("Paused sequences", style = MaterialTheme.typography.titleMedium)
    IssueLines(editor, "/paused_sequence_names")
    if (config.pausedSequenceNamesCount == 0) Text("None")
    config.pausedSequenceNamesList.forEachIndexed { i, name ->
        RemovableRow(editor, "/paused_sequence_names/$i", name) {
            controller.edit { b ->
                val next = b.pausedSequenceNamesList.filterIndexed { j, _ -> j != i }
                b.clearPausedSequenceNames().addAllPausedSequenceNames(next)
            }
        }
    }
    Text("Paused variants", style = MaterialTheme.typography.titleMedium)
    IssueLines(editor, "/paused_variants")
    if (config.pausedVariantsCount == 0) Text("None")
    config.pausedVariantsList.forEachIndexed { i, variant ->
        RemovableRow(editor, "/paused_variants/$i", "${variant.sequenceName} · ${variant.variantKey}") {
            controller.edit { it.removePausedVariants(i) }
        }
    }
}

@Composable
private fun RemovableRow(editor: ConfigEditorUi, field: String, text: String, remove: () -> Unit) {
    Column {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(text, Modifier.weight(1f))
            TextButton(onClick = remove, modifier = Modifier.testTag(ListTags.remove(field))) { Text("Remove") }
        }
        IssueLines(editor, field)
    }
}

@Composable
private fun AdvancedGroup(
    controller: ConfigController,
    editor: ConfigEditorUi,
    config: Config,
    window: ConfigWindowUi,
    browse: (BrowseRequest) -> Unit,
) {
    var path by remember(editor.path) { mutableStateOf(editor.path) }
    OutlinedTextField(
        value = path,
        onValueChange = { path = it },
        label = { Text("Config file on the trd machine") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth().testTag(ConfigTags.CONFIG_PATH),
    )
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(
            onClick = { browse(BrowseRequest(path, pickFile = true) { path = it }) },
            modifier = Modifier.testTag(ConfigTags.BROWSE_PATH),
        ) { Text("Choose file…") }
        Button(
            onClick = { controller.setConfigPath(path) },
            enabled = !editor.dirty && path.isNotBlank() && path != editor.path,
            modifier = Modifier.testTag(ConfigTags.USE_PATH),
        ) { Text("Use this config") }
    }
    if (editor.dirty) Text("Save or discard the changes before using another config file.")
    window.pathMessage?.let { Text(it, color = MaterialTheme.colorScheme.error) }

    Text("Legacy settings", style = MaterialTheme.typography.titleMedium)
    Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
            checked = config.logInColor,
            onCheckedChange = { v -> controller.edit { it.logInColor = v } },
            modifier = Modifier.testTag(ConfigTags.field("/log_in_color")),
        )
        Text("log_in_color (used by trd status)")
    }
    IssueLines(editor, "/log_in_color")
    CountSetting(editor, "progress_bar_width (used by trd status)", "/progress_bar_width", config.progressBarWidth) { v ->
        controller.edit { it.progressBarWidth = v }
    }
    DecimalSetting(
        editor, "sleep_time_seconds (kept for the older daemons, ignored by trd)", "/sleep_time_seconds",
        config.sleepTimeSeconds,
    ) { v -> controller.edit { it.sleepTimeSeconds = v } }

    Text("config.json (read-only)", style = MaterialTheme.typography.titleMedium)
    SelectionContainer {
        Text(editor.rawJson, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun TextSetting(editor: ConfigEditorUi, label: String, field: String, value: String, set: (String) -> Unit) {
    Column {
        OutlinedTextField(
            value = value,
            onValueChange = set,
            label = { Text(label) },
            singleLine = true,
            isError = hasError(editor, field),
            modifier = Modifier.fillMaxWidth().testTag(ConfigTags.field(field)),
        )
        IssueLines(editor, field)
    }
}

internal fun hasError(editor: ConfigEditorUi, field: String) =
    editor.issuesByField[field].orEmpty().any { it.severity == IssueSeverity.ISSUE_ERROR }

/** A whole number. An empty field counts as 0, which `trd` refuses where 0 makes no sense. */
@Composable
internal fun CountSetting(
    editor: ConfigEditorUi,
    label: String,
    field: String,
    value: Int,
    modifier: Modifier = Modifier.fillMaxWidth(),
    set: (Int) -> Unit,
) {
    NumberSetting(
        editor, label, field, value, modifier,
        accepts = { text -> text.all { it in '0'..'9' } && (text.isEmpty() || text.toIntOrNull() != null) },
        parse = { it.toIntOrNull() ?: 0 },
        format = { it.toString() },
        set = set,
    )
}

@Composable
private fun DecimalSetting(editor: ConfigEditorUi, label: String, field: String, value: Double, set: (Double) -> Unit) {
    NumberSetting(
        editor, label, field, value, Modifier.fillMaxWidth(),
        accepts = { text -> text.all { it in '0'..'9' || it == '.' } && text.count { it == '.' } <= 1 },
        parse = { it.toDoubleOrNull() ?: 0.0 },
        format = { it.toString().removeSuffix(".0") },
        set = set,
    )
}

/** Keeps what was typed (such as `1.`) for as long as it means the config's value. */
@Composable
private fun <T> NumberSetting(
    editor: ConfigEditorUi,
    label: String,
    field: String,
    value: T,
    modifier: Modifier,
    accepts: (String) -> Boolean,
    parse: (String) -> T,
    format: (T) -> String,
    set: (T) -> Unit,
) {
    var text by remember { mutableStateOf(format(value)) }
    LaunchedEffect(value) { if (parse(text) != value) text = format(value) }
    Column(modifier) {
        OutlinedTextField(
            value = text,
            onValueChange = { typed ->
                if (accepts(typed)) {
                    text = typed
                    if (parse(typed) != value) set(parse(typed))
                }
            },
            label = { Text(label) },
            singleLine = true,
            isError = hasError(editor, field),
            modifier = Modifier.fillMaxWidth().testTag(ConfigTags.field(field)),
        )
        IssueLines(editor, field)
    }
}
