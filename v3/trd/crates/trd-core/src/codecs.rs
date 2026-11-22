// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! video_codecs.json and the built-in codec table (§6.2).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, ConfigIssue, Stamp, pretty_sorted, write_atomic};

/// The codec table's file name, read beside config.json (§6.2).
pub const CODECS_FILE_NAME: &str = "video_codecs.json";

/// Codec name to definition, as in video_codecs.json.
pub type VideoCodecs = BTreeMap<String, VideoCodec>;

/// One codec: how ffmpeg encodes it and how its output file is named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoCodec {
    pub ffmpeg: FfmpegCodec,
    pub filename: FilenameCodec,
}

/// The `ffmpeg` object of a codec. `profile` and `extra_args` are omitted when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfmpegCodec {
    pub codec: String,
    pub pixel_format: String,
    pub color_range: String,
    /// Value for `-profile:v`; `None` omits the flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Encoder arguments appended after the profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_args: Option<Vec<String>>,
}

/// The `filename` object of a codec: the parts of the output file name (§6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilenameCodec {
    pub pixel_format: String,
    pub codec: String,
    pub quality: String,
}

/// The built-in table: a verbatim copy of `ConfigLoader.defaultCodecs` in the Swift
/// `Config.swift` (`ProResUHQ`, `ProResMQ`, `h264`).
pub fn builtin() -> VideoCodecs {
    let mut table = BTreeMap::new();
    table.insert(
        "ProResUHQ".to_owned(),
        VideoCodec {
            ffmpeg: FfmpegCodec {
                codec: "prores_ks".to_owned(),
                pixel_format: "yuv422p10le".to_owned(),
                color_range: "pc".to_owned(),
                profile: Some("1".to_owned()),
                extra_args: None,
            },
            filename: FilenameCodec {
                pixel_format: "444".to_owned(),
                codec: "ProRes".to_owned(),
                quality: "UHQ".to_owned(),
            },
        },
    );
    table.insert(
        "ProResMQ".to_owned(),
        VideoCodec {
            ffmpeg: FfmpegCodec {
                codec: "prores_ks".to_owned(),
                pixel_format: "yuv444p10le".to_owned(),
                color_range: "pc".to_owned(),
                profile: Some("1".to_owned()),
                extra_args: None,
            },
            filename: FilenameCodec {
                pixel_format: "422".to_owned(),
                codec: "ProRes".to_owned(),
                quality: "MQ".to_owned(),
            },
        },
    );
    // Full-range h264 to match the rest of the (Rec.709F) pipeline.
    // crf 18 ~ visually lossless for delivery renders.
    table.insert(
        "h264".to_owned(),
        VideoCodec {
            ffmpeg: FfmpegCodec {
                codec: "libx264".to_owned(),
                pixel_format: "yuv420p".to_owned(),
                color_range: "pc".to_owned(),
                profile: Some("high".to_owned()),
                extra_args: Some(vec!["-crf".to_owned(), "18".to_owned()]),
            },
            filename: FilenameCodec {
                pixel_format: "420".to_owned(),
                codec: "H264".to_owned(),
                quality: "HQ".to_owned(),
            },
        },
    );
    table
}

/// The video_codecs.json beside a config file. It need not exist.
pub fn sibling_codecs_path(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(CODECS_FILE_NAME),
        _ => PathBuf::from(CODECS_FILE_NAME),
    }
}

/// The video_codecs.json beside a config file, if one exists (Swift `codecsPath(alongsideConfig:)`).
pub fn codecs_path_alongside_config(config_path: &Path) -> Option<PathBuf> {
    let candidate = sibling_codecs_path(config_path);
    candidate.is_file().then_some(candidate)
}

/// Parses a codec table.
pub fn parse_codecs(text: &str) -> Result<VideoCodecs, serde_json::Error> {
    serde_json::from_str(text)
}

fn read_codecs(path: &Path) -> Result<VideoCodecs, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_codecs(&text).map_err(|e| format!("{} is not a valid codec table: {e}", path.display()))
}

/// Where the codec table in use came from (§6.2 resolution order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecSource {
    /// The `--codecs` path.
    Explicit,
    /// video_codecs.json beside the config file.
    Sibling,
    /// The built-in table.
    Builtin,
}

/// Why a codec edit was refused or failed.
#[derive(Debug)]
pub enum CodecError {
    /// No codec of that name.
    NotFound(String),
    /// Renaming onto a name that already exists.
    Exists(String),
    /// The new name is empty.
    InvalidName,
    /// A `rendering` entry uses the codec, so it cannot be removed or renamed. `entries` are the
    /// indexes of those entries.
    InUse { codec: String, entries: Vec<usize> },
    /// Writing video_codecs.json failed.
    Io(io::Error),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::NotFound(name) => write!(f, "no codec named {name:?}"),
            CodecError::Exists(name) => write!(f, "a codec named {name:?} already exists"),
            CodecError::InvalidName => write!(f, "a codec needs a name"),
            CodecError::InUse { codec, entries } => write!(
                f,
                "codec {codec:?} is used by {} rendering entr{}",
                entries.len(),
                if entries.len() == 1 { "y" } else { "ies" }
            ),
            CodecError::Io(e) => write!(f, "video_codecs.json: {e}"),
        }
    }
}

impl std::error::Error for CodecError {}

fn refuse_if_in_use(name: &str, config: &Config) -> Result<(), CodecError> {
    let entries: Vec<usize> = config
        .rendering
        .iter()
        .enumerate()
        .filter(|(_, rt)| rt.codec == name)
        .map(|(i, _)| i)
        .collect();
    if entries.is_empty() {
        Ok(())
    } else {
        Err(CodecError::InUse {
            codec: name.to_owned(),
            entries,
        })
    }
}

/// The codec table in use, with its file and mtime reload (§6.2).
pub struct CodecStore {
    explicit: Option<PathBuf>,
    config_path: PathBuf,
    path: Option<PathBuf>,
    source: CodecSource,
    stamp: Option<Stamp>,
    codecs: VideoCodecs,
    issues: Vec<ConfigIssue>,
}

impl CodecStore {
    /// Resolves and loads the table: the `explicit` path, else video_codecs.json beside
    /// `config_path`, else the built-in table. A path that is missing or does not parse is
    /// reported in [`CodecStore::issues`] and resolution goes on to the next source.
    pub fn open(explicit: Option<PathBuf>, config_path: PathBuf) -> Self {
        let mut store = CodecStore {
            explicit,
            config_path,
            path: None,
            source: CodecSource::Builtin,
            stamp: None,
            codecs: builtin(),
            issues: Vec::new(),
        };
        store.resolve();
        store
    }

    fn resolve(&mut self) {
        let mut issues = Vec::new();
        let mut candidates: Vec<(PathBuf, CodecSource)> = Vec::new();
        if let Some(explicit) = self.explicit.clone() {
            candidates.push((explicit, CodecSource::Explicit));
        }
        candidates.push((sibling_codecs_path(&self.config_path), CodecSource::Sibling));

        for (path, source) in candidates {
            if !path.is_file() {
                if source == CodecSource::Explicit {
                    issues.push(ConfigIssue::error(
                        "",
                        format!("{} does not exist", path.display()),
                    ));
                }
                continue;
            }
            match read_codecs(&path) {
                Ok(codecs) => {
                    self.stamp = Stamp::of(&path).ok();
                    self.codecs = codecs;
                    self.path = Some(path);
                    self.source = source;
                    self.issues = issues;
                    return;
                }
                Err(message) => issues.push(ConfigIssue::error("", message)),
            }
        }
        self.issues = issues;
    }

    /// The codec table in use.
    pub fn codecs(&self) -> &VideoCodecs {
        &self.codecs
    }

    /// Where the table in use came from.
    pub fn source(&self) -> CodecSource {
        self.source
    }

    /// The file the table was read from, or `None` for the built-in table.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Problems with the codec files read so far.
    pub fn issues(&self) -> &[ConfigIssue] {
        &self.issues
    }

    /// Re-reads the table when its file's mtime or size changed. A file that does not parse
    /// keeps the last good table and records an issue. Returns whether anything was re-read.
    pub fn reload_if_changed(&mut self) -> bool {
        let target = match &self.path {
            Some(path) => path.clone(),
            None => {
                let sibling = sibling_codecs_path(&self.config_path);
                if !sibling.is_file() {
                    return false;
                }
                sibling
            }
        };
        let stamp = match Stamp::of(&target) {
            Ok(stamp) => stamp,
            Err(e) => {
                let issue =
                    ConfigIssue::error("", format!("cannot read {}: {e}", target.display()));
                let changed = self.issues != [issue.clone()];
                self.issues = vec![issue];
                return changed;
            }
        };
        if self.stamp == Some(stamp) && self.path.as_deref() == Some(target.as_path()) {
            return false;
        }
        self.stamp = Some(stamp);
        self.source = if self.explicit.as_deref() == Some(target.as_path()) {
            CodecSource::Explicit
        } else {
            CodecSource::Sibling
        };
        self.path = Some(target.clone());
        match read_codecs(&target) {
            Ok(codecs) => {
                self.codecs = codecs;
                self.issues.clear();
            }
            Err(message) => self.issues = vec![ConfigIssue::error("", message)],
        }
        true
    }

    /// Adds a codec or replaces its definition. Editing a codec in use is allowed.
    pub fn set(&mut self, name: &str, codec: VideoCodec) -> Result<(), CodecError> {
        if name.is_empty() {
            return Err(CodecError::InvalidName);
        }
        let mut next = self.codecs.clone();
        next.insert(name.to_owned(), codec);
        self.write_table(next)
    }

    /// Removes a codec. Refused while a `rendering` entry of `config` uses it.
    pub fn remove(&mut self, name: &str, config: &Config) -> Result<(), CodecError> {
        if !self.codecs.contains_key(name) {
            return Err(CodecError::NotFound(name.to_owned()));
        }
        refuse_if_in_use(name, config)?;
        let mut next = self.codecs.clone();
        next.remove(name);
        self.write_table(next)
    }

    /// Renames a codec. Refused while a `rendering` entry of `config` uses the old name, and
    /// when the new name already exists.
    pub fn rename(&mut self, from: &str, to: &str, config: &Config) -> Result<(), CodecError> {
        let codec = self
            .codecs
            .get(from)
            .cloned()
            .ok_or_else(|| CodecError::NotFound(from.to_owned()))?;
        refuse_if_in_use(from, config)?;
        if to.is_empty() {
            return Err(CodecError::InvalidName);
        }
        if self.codecs.contains_key(to) {
            return Err(CodecError::Exists(to.to_owned()));
        }
        let mut next = self.codecs.clone();
        next.remove(from);
        next.insert(to.to_owned(), codec);
        self.write_table(next)
    }

    /// Writes the table beside the config file (created when the built-in table was in use),
    /// atomically, then makes it the table in use.
    fn write_table(&mut self, next: VideoCodecs) -> Result<(), CodecError> {
        let target = sibling_codecs_path(&self.config_path);
        let value = serde_json::to_value(&next)
            .map_err(io::Error::other)
            .map_err(CodecError::Io)?;
        let mut text = pretty_sorted(&value);
        text.push('\n');
        write_atomic(&target, text.as_bytes()).map_err(CodecError::Io)?;
        self.stamp = Stamp::of(&target).ok();
        self.codecs = next;
        self.issues.clear();
        self.source = CodecSource::Sibling;
        self.path = Some(target);
        Ok(())
    }
}
