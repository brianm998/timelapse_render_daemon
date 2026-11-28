// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! config.json: keys, defaults, reload on change and validation (§6.1).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::codecs::VideoCodecs;
use crate::paths::{Clock, expand_year_path, has_year_placeholder};

/// No config file exists; nothing is scanned (§6.1).
pub const PROBLEM_CONFIG_MISSING: &str = "PROBLEM_CONFIG_MISSING";
/// The file does not parse; the previous config is kept (§6.1).
pub const PROBLEM_CONFIG_INVALID: &str = "PROBLEM_CONFIG_INVALID";
/// The file loaded, but an entry with an ERROR was ignored (§6.1).
pub const PROBLEM_CONFIG_ISSUES: &str = "PROBLEM_CONFIG_ISSUES";
/// A `rendering` entry names a codec that is not in the codec table (§6.1).
pub const PROBLEM_UNKNOWN_CODEC: &str = "PROBLEM_UNKNOWN_CODEC";

/// Every key that §6.1 defines. Any other top-level key is kept in [`Config::extra`].
pub const KNOWN_KEYS: [&str; 21] = [
    "export_paths",
    "source_paths",
    "backup_paths",
    "master_videos",
    "max_processes",
    "max_cuncurrent_renders",
    "image_name_prefix",
    "image_sequence_dirname_prefix",
    "raw_types",
    "image_extensions",
    "rendering",
    "log_in_color",
    "sleep_time_seconds",
    "scan_interval_seconds",
    "progress_bar_width",
    "ignore_sequence_regexes",
    "delete_sequence_after_render_regexes",
    "paused_sequence_names",
    "paused_variants",
    "recent_render_history_days",
    "render_history_path",
];

/// How serious a [`ConfigIssue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The entry is ignored on load, and an edit that would leave it in place is refused.
    Error,
    /// The config is used as it is.
    Warning,
}

/// One validation finding. `field` is a JSON pointer into config.json (`""` is the whole file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub severity: Severity,
    pub field: String,
    pub message: String,
}

impl ConfigIssue {
    /// An ERROR issue.
    pub fn error(field: impl Into<String>, message: impl Into<String>) -> Self {
        ConfigIssue {
            severity: Severity::Error,
            field: field.into(),
            message: message.into(),
        }
    }

    /// A WARNING issue.
    pub fn warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        ConfigIssue {
            severity: Severity::Warning,
            field: field.into(),
            message: message.into(),
        }
    }

    /// Whether this issue is an ERROR.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// `max_processes`: each key defaults separately (§6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxProcesses {
    pub ffmpeg: i64,
    pub exiftool: i64,
}

impl Default for MaxProcesses {
    fn default() -> Self {
        MaxProcesses {
            ffmpeg: 1,
            exiftool: 10,
        }
    }
}

/// One `rendering` entry: a codec, a frame rate and an optional resolution (§6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderType {
    pub codec: String,
    pub frame_rate: i64,
    pub resolution: Option<Vec<i64>>,
}

impl RenderType {
    /// The width: the first resolution entry.
    pub fn width(&self) -> Option<i64> {
        self.resolution.as_deref().and_then(|r| r.first().copied())
    }

    /// The height: the second resolution entry.
    pub fn height(&self) -> Option<i64> {
        self.resolution.as_deref().and_then(|r| r.get(1).copied())
    }

    /// `<codec>_<res>_<frame_rate>`, with `<res>` `<W>x<H>`, `<W>w` or `ori` (§6.5).
    pub fn variant_key(&self) -> String {
        let res = match self.resolution.as_deref() {
            Some([w, h, ..]) => format!("{w}x{h}"),
            Some([w]) => format!("{w}w"),
            _ => "ori".to_owned(),
        };
        format!("{}_{}_{}", self.codec, res, self.frame_rate)
    }

    /// `<H>p`, `<W>w` or `OriRes` (§6.5).
    pub fn resolution_label(&self) -> String {
        match self.resolution.as_deref() {
            Some([_, h, ..]) => format!("{h}p"),
            Some([w]) => format!("{w}w"),
            _ => "OriRes".to_owned(),
        }
    }

    fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("codec".to_owned(), json!(self.codec));
        m.insert("frame_rate".to_owned(), json!(self.frame_rate));
        if let Some(resolution) = &self.resolution {
            m.insert("resolution".to_owned(), json!(resolution));
        }
        Value::Object(m)
    }
}

/// One `paused_variants` entry (§6.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PausedVariant {
    pub sequence_name: String,
    pub variant_key: String,
}

/// The typed view of config.json (§6.1). Keys this struct does not know are kept in `extra`.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub export_paths: Vec<String>,
    pub source_paths: Vec<String>,
    pub backup_paths: Vec<String>,
    pub master_videos: Vec<String>,
    pub max_processes: MaxProcesses,
    pub max_cuncurrent_renders: i64,
    pub image_name_prefix: String,
    pub image_sequence_dirname_prefix: String,
    pub raw_types: Vec<String>,
    pub image_extensions: Vec<String>,
    pub rendering: Vec<RenderType>,
    pub log_in_color: bool,
    pub sleep_time_seconds: f64,
    pub scan_interval_seconds: f64,
    pub progress_bar_width: i64,
    pub ignore_sequence_regexes: Vec<String>,
    pub delete_sequence_after_render_regexes: Vec<String>,
    pub paused_sequence_names: Vec<String>,
    pub paused_variants: Vec<PausedVariant>,
    pub recent_render_history_days: i64,
    pub render_history_path: Option<String>,
    /// Top-level keys this version does not know, written back unchanged.
    pub extra: BTreeMap<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            export_paths: Vec::new(),
            source_paths: Vec::new(),
            backup_paths: Vec::new(),
            master_videos: Vec::new(),
            max_processes: MaxProcesses::default(),
            max_cuncurrent_renders: 1,
            image_name_prefix: String::new(),
            image_sequence_dirname_prefix: String::new(),
            raw_types: vec!["ARW".to_owned(), "ORF".to_owned(), "DNG".to_owned()],
            image_extensions: vec!["tif".to_owned(), "tiff".to_owned()],
            rendering: Vec::new(),
            log_in_color: true,
            sleep_time_seconds: 12.0,
            scan_interval_seconds: 2.0,
            progress_bar_width: 40,
            ignore_sequence_regexes: Vec::new(),
            delete_sequence_after_render_regexes: Vec::new(),
            paused_sequence_names: Vec::new(),
            paused_variants: Vec::new(),
            recent_render_history_days: 30,
            render_history_path: None,
            extra: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Reads a parsed config.json. A known key with the wrong JSON type fails the whole file, as
    /// the Swift decoder did. A known key that is `null` takes its default. Numeric ranges are
    /// not checked here; that is [`validate`]'s job.
    pub fn from_value(value: &Value) -> Result<Config, String> {
        let obj = value
            .as_object()
            .ok_or_else(|| "config.json must be a JSON object".to_owned())?;
        let mut cfg = Config::default();
        for (key, v) in obj {
            if v.is_null() && KNOWN_KEYS.contains(&key.as_str()) {
                continue;
            }
            match key.as_str() {
                "export_paths" => cfg.export_paths = string_list(v, key)?,
                "source_paths" => cfg.source_paths = string_list(v, key)?,
                "backup_paths" => cfg.backup_paths = string_list(v, key)?,
                "master_videos" => cfg.master_videos = string_list(v, key)?,
                "max_processes" => cfg.max_processes = max_processes(v)?,
                "max_cuncurrent_renders" => cfg.max_cuncurrent_renders = int(v, key)?,
                "image_name_prefix" => cfg.image_name_prefix = string(v, key)?,
                "image_sequence_dirname_prefix" => {
                    cfg.image_sequence_dirname_prefix = string(v, key)?
                }
                "raw_types" => cfg.raw_types = string_list(v, key)?,
                "image_extensions" => cfg.image_extensions = string_list(v, key)?,
                "rendering" => cfg.rendering = rendering(v)?,
                "log_in_color" => cfg.log_in_color = boolean(v, key)?,
                "sleep_time_seconds" => cfg.sleep_time_seconds = float(v, key)?,
                "scan_interval_seconds" => cfg.scan_interval_seconds = float(v, key)?,
                "progress_bar_width" => cfg.progress_bar_width = int(v, key)?,
                "ignore_sequence_regexes" => cfg.ignore_sequence_regexes = string_list(v, key)?,
                "delete_sequence_after_render_regexes" => {
                    cfg.delete_sequence_after_render_regexes = string_list(v, key)?
                }
                "paused_sequence_names" => cfg.paused_sequence_names = string_list(v, key)?,
                "paused_variants" => cfg.paused_variants = paused_variants(v)?,
                "recent_render_history_days" => cfg.recent_render_history_days = int(v, key)?,
                "render_history_path" => cfg.render_history_path = Some(string(v, key)?),
                _ => {
                    cfg.extra.insert(key.clone(), v.clone());
                }
            }
        }
        Ok(cfg)
    }

    /// The config as JSON. Every known key is written (`render_history_path` only when set),
    /// then the unknown keys unchanged.
    pub fn to_value(&self) -> Value {
        let mut m: Map<String, Value> = self
            .extra
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let known: Vec<(&str, Value)> = vec![
            ("export_paths", json!(self.export_paths)),
            ("source_paths", json!(self.source_paths)),
            ("backup_paths", json!(self.backup_paths)),
            ("master_videos", json!(self.master_videos)),
            (
                "max_processes",
                json!({"ffmpeg": self.max_processes.ffmpeg, "exiftool": self.max_processes.exiftool}),
            ),
            ("max_cuncurrent_renders", json!(self.max_cuncurrent_renders)),
            ("image_name_prefix", json!(self.image_name_prefix)),
            ("image_sequence_dirname_prefix", json!(self.image_sequence_dirname_prefix)),
            ("raw_types", json!(self.raw_types)),
            ("image_extensions", json!(self.image_extensions)),
            (
                "rendering",
                Value::Array(self.rendering.iter().map(RenderType::to_value).collect()),
            ),
            ("log_in_color", json!(self.log_in_color)),
            ("sleep_time_seconds", number(self.sleep_time_seconds)),
            ("scan_interval_seconds", number(self.scan_interval_seconds)),
            ("progress_bar_width", json!(self.progress_bar_width)),
            ("ignore_sequence_regexes", json!(self.ignore_sequence_regexes)),
            (
                "delete_sequence_after_render_regexes",
                json!(self.delete_sequence_after_render_regexes),
            ),
            ("paused_sequence_names", json!(self.paused_sequence_names)),
            (
                "paused_variants",
                Value::Array(
                    self.paused_variants
                        .iter()
                        .map(|p| json!({"sequence_name": p.sequence_name, "variant_key": p.variant_key}))
                        .collect(),
                ),
            ),
            ("recent_render_history_days", json!(self.recent_render_history_days)),
        ];
        for (key, value) in known {
            m.insert(key.to_owned(), value);
        }
        if let Some(path) = &self.render_history_path {
            m.insert("render_history_path".to_owned(), json!(path));
        }
        Value::Object(m)
    }

    /// The pretty JSON written to disk: sorted keys, two-space indent, a final newline.
    pub fn to_json_pretty(&self) -> String {
        let mut text = pretty_sorted(&self.to_value());
        text.push('\n');
        text
    }

    /// The output file name of one render of `sequence_dir` (§6.5, `outputVideoFilename`).
    /// Both prefixes are stripped repeatedly from the directory name first.
    pub fn output_filename(
        &self,
        codecs: &VideoCodecs,
        rt: &RenderType,
        sequence_dir: &str,
    ) -> String {
        let mut name = sequence_dir.to_owned();
        strip_all(&mut name, &self.image_sequence_dirname_prefix);
        strip_all(&mut name, &self.image_name_prefix);
        let label = rt.resolution_label();
        match codecs.get(&rt.codec) {
            Some(codec) => format!(
                "{name}_{}-{}_Rec.709F_{label}_{}_{}.mov",
                codec.filename.codec,
                codec.filename.pixel_format,
                rt.frame_rate,
                codec.filename.quality
            ),
            None => format!("{name}_{}_{label}_{}.mov", rt.codec, rt.frame_rate),
        }
    }
}

fn strip_all(name: &mut String, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    while name.contains(prefix) {
        *name = name.replace(prefix, "");
    }
}

/// Whether a path entry is absolute on any platform: a leading `/` or `\`, or a drive letter.
pub fn is_absolute_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

fn is_unknown_codec(rt: &RenderType, codecs: &VideoCodecs) -> bool {
    !rt.codec.is_empty() && !codecs.contains_key(&rt.codec)
}

/// Validates a config against §6.1. `now_year` is the year `$YEAR` expands down from.
///
/// Every rule is one ERROR or WARNING issue; see the section for each rule's field.
pub fn validate(cfg: &Config, codecs: &VideoCodecs, now_year: i32) -> Vec<ConfigIssue> {
    let mut out = Vec::new();

    let path_lists: [(&str, &[String]); 4] = [
        ("export_paths", cfg.export_paths.as_slice()),
        ("source_paths", cfg.source_paths.as_slice()),
        ("backup_paths", cfg.backup_paths.as_slice()),
        ("master_videos", cfg.master_videos.as_slice()),
    ];
    for (key, list) in path_lists {
        for (i, path) in list.iter().enumerate() {
            let field = format!("/{key}/{i}");
            if !is_absolute_path(path) {
                out.push(ConfigIssue::error(
                    field,
                    format!("{path:?} is not an absolute path"),
                ));
            } else if has_year_placeholder(path) {
                if expand_year_path(path, now_year).is_empty() {
                    out.push(ConfigIssue::warning(
                        field,
                        format!("{path:?} expands to no existing directory"),
                    ));
                }
            } else if !Path::new(path).exists() {
                out.push(ConfigIssue::warning(
                    field,
                    format!("{path:?} does not exist"),
                ));
            }
        }
    }
    if cfg.export_paths.is_empty() {
        out.push(ConfigIssue::warning(
            "/export_paths",
            "no export paths are configured",
        ));
    }
    if cfg.source_paths.is_empty() {
        out.push(ConfigIssue::warning(
            "/source_paths",
            "no source paths are configured",
        ));
    }

    let regex_lists: [(&str, &[String]); 2] = [
        (
            "ignore_sequence_regexes",
            cfg.ignore_sequence_regexes.as_slice(),
        ),
        (
            "delete_sequence_after_render_regexes",
            cfg.delete_sequence_after_render_regexes.as_slice(),
        ),
    ];
    for (key, list) in regex_lists {
        for (i, pattern) in list.iter().enumerate() {
            if Regex::new(pattern).is_err() {
                out.push(ConfigIssue::error(
                    format!("/{key}/{i}"),
                    format!("{pattern:?} does not compile as a regular expression"),
                ));
            }
        }
    }

    if cfg.max_processes.ffmpeg < 1 {
        out.push(ConfigIssue::error(
            "/max_processes/ffmpeg",
            "must be at least 1",
        ));
    }
    if cfg.max_processes.exiftool < 1 {
        out.push(ConfigIssue::error(
            "/max_processes/exiftool",
            "must be at least 1",
        ));
    }
    if cfg.max_cuncurrent_renders < 1 {
        out.push(ConfigIssue::error(
            "/max_cuncurrent_renders",
            "must be at least 1",
        ));
    }
    if cfg.scan_interval_seconds < 0.5 {
        out.push(ConfigIssue::error(
            "/scan_interval_seconds",
            "must be at least 0.5",
        ));
    }
    if cfg.recent_render_history_days < 1 {
        out.push(ConfigIssue::error(
            "/recent_render_history_days",
            "must be at least 1",
        ));
    }

    let mut seen = BTreeSet::new();
    for (i, rt) in cfg.rendering.iter().enumerate() {
        let field = format!("/rendering/{i}");
        if rt.codec.is_empty() {
            out.push(ConfigIssue::error(
                format!("{field}/codec"),
                "codec is empty",
            ));
        } else if is_unknown_codec(rt, codecs) {
            out.push(ConfigIssue::error(
                format!("{field}/codec"),
                format!("codec {:?} is not in the codec table", rt.codec),
            ));
        }
        if !(1..=240).contains(&rt.frame_rate) {
            out.push(ConfigIssue::error(
                format!("{field}/frame_rate"),
                "frame_rate must be 1 to 240",
            ));
        }
        if let Some(resolution) = &rt.resolution {
            if resolution.len() > 2 {
                out.push(ConfigIssue::error(
                    format!("{field}/resolution"),
                    "resolution has more than 2 entries",
                ));
            }
            if resolution.iter().any(|n| *n < 1) {
                out.push(ConfigIssue::error(
                    format!("{field}/resolution"),
                    "resolution entries must be positive",
                ));
            }
        }
        let key = rt.variant_key();
        if !seen.insert(key.clone()) {
            out.push(ConfigIssue::error(
                field,
                format!("duplicate variant_key {key}"),
            ));
        }
    }
    if cfg.rendering.is_empty() {
        out.push(ConfigIssue::warning(
            "/rendering",
            "no renders are configured",
        ));
    }

    out
}

/// The result of reading a config from text that parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The config in use: entries with an ERROR removed, scalars with an ERROR reset to default.
    pub config: Config,
    /// Every validation issue of the file as written.
    pub issues: Vec<ConfigIssue>,
    /// The problem ids to raise for this load (`PROBLEM_CONFIG_ISSUES`, `PROBLEM_UNKNOWN_CODEC`).
    pub problems: Vec<&'static str>,
}

/// Loads config text (§6.1 "on load from disk"). `Err` means the text does not parse, which is
/// `PROBLEM_CONFIG_INVALID` for the caller.
///
/// An entry with an ERROR is ignored and the rest of the config is used. An unknown codec raises
/// `PROBLEM_UNKNOWN_CODEC` in place of `PROBLEM_CONFIG_ISSUES` for that entry.
pub fn load_text(text: &str, codecs: &VideoCodecs, now_year: i32) -> Result<Loaded, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let mut cfg = Config::from_value(&value)?;
    let issues = validate(&cfg, codecs, now_year);

    let unknown_codec_fields: BTreeSet<String> = cfg
        .rendering
        .iter()
        .enumerate()
        .filter(|(_, rt)| is_unknown_codec(rt, codecs))
        .map(|(i, _)| format!("/rendering/{i}/codec"))
        .collect();
    let other_error = issues
        .iter()
        .any(|i| i.is_error() && !unknown_codec_fields.contains(&i.field));
    let mut problems = Vec::new();
    if !unknown_codec_fields.is_empty() {
        problems.push(PROBLEM_UNKNOWN_CODEC);
    }
    if other_error {
        problems.push(PROBLEM_CONFIG_ISSUES);
    }

    drop_invalid_entries(&mut cfg, &issues);
    Ok(Loaded {
        config: cfg,
        issues,
        problems,
    })
}

/// Removes every list entry and resets every scalar that has an ERROR issue.
fn drop_invalid_entries(cfg: &mut Config, issues: &[ConfigIssue]) {
    let defaults = Config::default();
    let mut bad: BTreeSet<(String, usize)> = BTreeSet::new();
    for issue in issues.iter().filter(|i| i.is_error()) {
        let parts: Vec<&str> = issue.field.split('/').collect();
        if let (Some(list), Some(Ok(index))) =
            (parts.get(1), parts.get(2).map(|s| s.parse::<usize>()))
        {
            bad.insert(((*list).to_owned(), index));
        }
        match issue.field.as_str() {
            "/max_processes/ffmpeg" => cfg.max_processes.ffmpeg = defaults.max_processes.ffmpeg,
            "/max_processes/exiftool" => {
                cfg.max_processes.exiftool = defaults.max_processes.exiftool
            }
            "/max_cuncurrent_renders" => {
                cfg.max_cuncurrent_renders = defaults.max_cuncurrent_renders
            }
            "/scan_interval_seconds" => cfg.scan_interval_seconds = defaults.scan_interval_seconds,
            "/recent_render_history_days" => {
                cfg.recent_render_history_days = defaults.recent_render_history_days
            }
            _ => {}
        }
    }
    retain_indexed(&mut cfg.export_paths, "export_paths", &bad);
    retain_indexed(&mut cfg.source_paths, "source_paths", &bad);
    retain_indexed(&mut cfg.backup_paths, "backup_paths", &bad);
    retain_indexed(&mut cfg.master_videos, "master_videos", &bad);
    retain_indexed(
        &mut cfg.ignore_sequence_regexes,
        "ignore_sequence_regexes",
        &bad,
    );
    retain_indexed(
        &mut cfg.delete_sequence_after_render_regexes,
        "delete_sequence_after_render_regexes",
        &bad,
    );
    retain_indexed(&mut cfg.rendering, "rendering", &bad);
}

fn retain_indexed<T>(items: &mut Vec<T>, list: &str, bad: &BTreeSet<(String, usize)>) {
    let mut index = 0;
    items.retain(|_| {
        let keep = !bad.contains(&(list.to_owned(), index));
        index += 1;
        keep
    });
}

/// SHA-256 of the bytes, as lowercase hex. This is the revision an edit must quote.
pub fn revision_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Writes `bytes` to `path` atomically: a sibling temp file is created, written, fsynced and
/// renamed over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy()
        .into_owned();
    let mut attempt: u32 = 0;
    let (tmp, mut file) = loop {
        let tmp = dir.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::rename(&tmp, path)
        });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Pretty JSON with sorted keys at every level, two-space indent.
pub fn pretty_sorted(value: &Value) -> String {
    serde_json::to_string_pretty(&sorted(value)).expect("a JSON value always serializes")
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            let ordered: BTreeMap<&String, Value> = m.iter().map(|(k, v)| (k, sorted(v))).collect();
            Value::Object(ordered.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Modification time and size of a file: the cheap test for "changed since last load".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl Stamp {
    /// The stamp of the file at `path`.
    pub fn of(path: &Path) -> io::Result<Stamp> {
        let meta = fs::metadata(path)?;
        Ok(Stamp {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// Whether config.json was present and usable at the last check (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// There is no file.
    Missing,
    /// The file does not parse; `config()` still returns the last good config.
    Invalid,
    /// The file parsed and is in use.
    Loaded,
}

/// The result of a successful [`ConfigStore::apply`].
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyOutcome {
    /// The revision of the file as written.
    pub revision: String,
    /// The validation issues of the file as written (warnings only, since errors refuse the edit).
    pub issues: Vec<ConfigIssue>,
}

/// Why an edit to config.json was not made.
#[derive(Debug)]
pub enum ConfigError {
    /// The caller's revision is not the file's current revision; nothing was written.
    StaleRevision { current: String },
    /// The edited config has ERROR issues; nothing was written.
    Refused(Vec<ConfigIssue>),
    /// The file on disk does not parse, so it cannot be edited; nothing was written.
    Unparseable(String),
    /// Reading or writing the file failed.
    Io(io::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::StaleRevision { current } => {
                write!(
                    f,
                    "config.json changed since it was read (current revision {current})"
                )
            }
            ConfigError::Refused(issues) => {
                write!(
                    f,
                    "the edit would leave {} error(s) in config.json",
                    issues.len()
                )
            }
            ConfigError::Unparseable(msg) => write!(f, "config.json cannot be edited: {msg}"),
            ConfigError::Io(e) => write!(f, "config.json: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn parse_current(bytes: &[u8]) -> Result<Config, ConfigError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ConfigError::Unparseable(e.to_string()))?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| ConfigError::Unparseable(e.to_string()))?;
    Config::from_value(&value).map_err(ConfigError::Unparseable)
}

/// The config file, its last good config, and the revision an edit must quote (§6.1).
pub struct ConfigStore {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    status: Status,
    stamp: Option<Stamp>,
    revision: String,
    last_good: Option<Config>,
    issues: Vec<ConfigIssue>,
    loaded_problems: Vec<&'static str>,
}

impl ConfigStore {
    /// A store for `path`. Nothing is read until [`ConfigStore::reload_if_changed`].
    pub fn new(path: impl Into<PathBuf>, clock: Arc<dyn Clock>) -> Self {
        ConfigStore {
            path: path.into(),
            clock,
            status: Status::Missing,
            stamp: None,
            revision: revision_of(&[]),
            last_good: None,
            issues: Vec::new(),
            loaded_problems: Vec::new(),
        }
    }

    /// The config file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file is missing, invalid or loaded.
    pub fn status(&self) -> Status {
        self.status
    }

    /// The config in use: the last one that parsed. `None` while no file has ever parsed.
    pub fn config(&self) -> Option<&Config> {
        self.last_good.as_ref()
    }

    /// The revision of the file as last read or written: SHA-256 of its bytes.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// The validation issues of the last load.
    pub fn issues(&self) -> &[ConfigIssue] {
        &self.issues
    }

    /// The problem ids that apply now (§6.10 `config`, `config-issues`).
    pub fn problem_ids(&self) -> Vec<&'static str> {
        match self.status {
            Status::Missing => vec![PROBLEM_CONFIG_MISSING],
            Status::Invalid => vec![PROBLEM_CONFIG_INVALID],
            Status::Loaded => self.loaded_problems.clone(),
        }
    }

    /// Forgets the file's stamp, so the next [`ConfigStore::reload_if_changed`] re-reads and
    /// re-validates it (the codec table it is validated against changed).
    pub fn invalidate(&mut self) {
        self.stamp = None;
    }

    /// Re-reads the file if its mtime or size differs from the last load. Returns whether the
    /// state changed, so the caller logs a bad file once per change.
    pub fn reload_if_changed(&mut self, codecs: &VideoCodecs) -> bool {
        let year = self.clock.year();
        let stamp = match Stamp::of(&self.path) {
            Ok(stamp) => stamp,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return self.mark_missing(),
            Err(e) => return self.mark_unreadable(&e),
        };
        if self.stamp == Some(stamp) {
            return false;
        }
        self.stamp = Some(stamp);
        match fs::read(&self.path) {
            Ok(bytes) => {
                self.revision = revision_of(&bytes);
                self.load_bytes(&bytes, codecs, year);
                true
            }
            Err(e) => self.mark_unreadable(&e),
        }
    }

    /// Edits the file. `edit` changes the config as it is on disk, so keys this version does not
    /// know survive. The edit is refused when `expected_revision` is not the file's current
    /// revision, or when the result has an ERROR. A refused or stale edit writes nothing.
    /// The write is atomic, and the new mtime is recorded so the store does not reload it.
    pub fn apply<F>(
        &mut self,
        edit: F,
        expected_revision: &str,
        codecs: &VideoCodecs,
    ) -> Result<ApplyOutcome, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        let year = self.clock.year();
        let existing = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(ConfigError::Io(e)),
        };
        let current = revision_of(existing.as_deref().unwrap_or(&[]));
        if current != expected_revision {
            return Err(ConfigError::StaleRevision { current });
        }
        let mut cfg = match &existing {
            Some(bytes) => parse_current(bytes)?,
            None => Config::default(),
        };
        edit(&mut cfg);

        let errors: Vec<ConfigIssue> = validate(&cfg, codecs, year)
            .into_iter()
            .filter(ConfigIssue::is_error)
            .collect();
        if !errors.is_empty() {
            return Err(ConfigError::Refused(errors));
        }

        let text = cfg.to_json_pretty();
        write_atomic(&self.path, text.as_bytes()).map_err(ConfigError::Io)?;
        self.revision = revision_of(text.as_bytes());
        self.load_bytes(text.as_bytes(), codecs, year);
        self.stamp = Stamp::of(&self.path).ok();
        Ok(ApplyOutcome {
            revision: self.revision.clone(),
            issues: self.issues.clone(),
        })
    }

    fn load_bytes(&mut self, bytes: &[u8], codecs: &VideoCodecs, year: i32) {
        let loaded = std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())
            .and_then(|text| load_text(text, codecs, year));
        match loaded {
            Ok(loaded) => {
                self.status = Status::Loaded;
                self.last_good = Some(loaded.config);
                self.issues = loaded.issues;
                self.loaded_problems = loaded.problems;
            }
            Err(message) => {
                self.status = Status::Invalid;
                self.issues = vec![ConfigIssue::error("", message)];
                self.loaded_problems.clear();
            }
        }
    }

    fn mark_missing(&mut self) -> bool {
        let changed = self.status != Status::Missing || self.stamp.is_some();
        self.status = Status::Missing;
        self.stamp = None;
        self.last_good = None;
        self.issues.clear();
        self.loaded_problems.clear();
        self.revision = revision_of(&[]);
        changed
    }

    fn mark_unreadable(&mut self, error: &io::Error) -> bool {
        self.status = Status::Invalid;
        self.issues = vec![ConfigIssue::error(
            "",
            format!("cannot read the file: {error}"),
        )];
        self.loaded_problems.clear();
        true
    }
}

fn string_list(v: &Value, key: &str) -> Result<Vec<String>, String> {
    let items = v
        .as_array()
        .ok_or_else(|| format!("{key} must be an array of strings"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must be an array of strings"))
        })
        .collect()
}

fn int_list(v: &Value, key: &str) -> Result<Vec<i64>, String> {
    let items = v
        .as_array()
        .ok_or_else(|| format!("{key} must be an array of integers"))?;
    items
        .iter()
        .map(|item| {
            item.as_i64()
                .ok_or_else(|| format!("{key} must be an array of integers"))
        })
        .collect()
}

fn int(v: &Value, key: &str) -> Result<i64, String> {
    v.as_i64()
        .ok_or_else(|| format!("{key} must be an integer"))
}

fn float(v: &Value, key: &str) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("{key} must be a number"))
}

fn boolean(v: &Value, key: &str) -> Result<bool, String> {
    v.as_bool()
        .ok_or_else(|| format!("{key} must be true or false"))
}

fn string(v: &Value, key: &str) -> Result<String, String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} must be a string"))
}

fn max_processes(v: &Value) -> Result<MaxProcesses, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "max_processes must be an object".to_owned())?;
    let mut mp = MaxProcesses::default();
    if let Some(x) = obj.get("ffmpeg").filter(|x| !x.is_null()) {
        mp.ffmpeg = int(x, "max_processes.ffmpeg")?;
    }
    if let Some(x) = obj.get("exiftool").filter(|x| !x.is_null()) {
        mp.exiftool = int(x, "max_processes.exiftool")?;
    }
    Ok(mp)
}

fn rendering(v: &Value) -> Result<Vec<RenderType>, String> {
    let items = v
        .as_array()
        .ok_or_else(|| "rendering must be an array".to_owned())?;
    items
        .iter()
        .map(|item| -> Result<RenderType, String> {
            let obj = item
                .as_object()
                .ok_or_else(|| "rendering entries must be objects".to_owned())?;
            let codec = string(
                obj.get("codec")
                    .ok_or_else(|| "rendering entries need a codec".to_owned())?,
                "rendering.codec",
            )?;
            let frame_rate = int(
                obj.get("frame_rate")
                    .ok_or_else(|| "rendering entries need a frame_rate".to_owned())?,
                "rendering.frame_rate",
            )?;
            let resolution = match obj.get("resolution") {
                None | Some(Value::Null) => None,
                Some(r) => Some(int_list(r, "rendering.resolution")?),
            };
            Ok(RenderType {
                codec,
                frame_rate,
                resolution,
            })
        })
        .collect()
}

fn paused_variants(v: &Value) -> Result<Vec<PausedVariant>, String> {
    let items = v
        .as_array()
        .ok_or_else(|| "paused_variants must be an array".to_owned())?;
    items
        .iter()
        .map(|item| -> Result<PausedVariant, String> {
            let obj = item
                .as_object()
                .ok_or_else(|| "paused_variants entries must be objects".to_owned())?;
            let sequence_name = string(
                obj.get("sequence_name")
                    .ok_or_else(|| "paused_variants need sequence_name".to_owned())?,
                "paused_variants.sequence_name",
            )?;
            let variant_key = string(
                obj.get("variant_key")
                    .ok_or_else(|| "paused_variants need variant_key".to_owned())?,
                "paused_variants.variant_key",
            )?;
            Ok(PausedVariant {
                sequence_name,
                variant_key,
            })
        })
        .collect()
}

/// A number as JSON: a whole value is written as an integer, as the Perl and Swift files do.
fn number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        json!(f as i64)
    } else {
        json!(f)
    }
}
