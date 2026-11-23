// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Import of the Swift render history (§6.17).
//!
//! The Swift daemon kept its recent renders in `render_history.json`: an array of
//! `CompletedRenderResult` objects with ISO 8601 dates. The import runs once per database (the
//! `swift_history_imported` setting), and only after the file has been read: a missing file is
//! not an import, so a later start can still import it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::DateTime;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::store::{DAY_MS, LogFields, LogLevel, RenderedVideo, Store, StoreResult, log_event};

/// The `source` of a row imported from the Swift history.
pub const SOURCE_IMPORTED: &str = "IMPORTED";

/// The file the Swift daemon wrote on macOS, under `home`. Other platforms have none.
pub fn default_history_path(home: &Path, os: &str) -> Option<PathBuf> {
    (os == "macos").then(|| {
        home.join("Library")
            .join("Application Support")
            .join("TimelapseRenderDaemon")
            .join("render_history.json")
    })
}

/// One entry of the Swift history, as `CompletedRenderResult` encodes it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftRender {
    timestamp: String,
    sequence_name: String,
    export_path: String,
    output_filename: String,
    output_path: String,
    frame_count: i64,
    file_size: i64,
}

/// The variant fields that an output file name carries (§6.5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameFields {
    /// The codec: the file name's codec, or the codec of an unknown codec's name.
    pub codec: String,
    /// `OriRes`, `<W>w` or `<H>p`.
    pub resolution_label: String,
    pub frame_rate: i64,
}

// `<name>_<codec>-<pixel format>_Rec.709F_<resolution_label>_<frame_rate>_<quality>.mov`
static KNOWN_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^.+_([^_-]+)-[^_]+_Rec\.709F_(OriRes|\d+w|\d+p)_(\d+)_[^_]*\.mov$")
        .expect("the known-codec name pattern is valid")
});
// `<name>_<codec>_<resolution_label>_<frame_rate>.mov`, for a codec with no file name entry.
static UNKNOWN_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^.+_([^_]+)_(OriRes|\d+w|\d+p)_(\d+)\.mov$")
        .expect("the unknown-codec name pattern is valid")
});

/// The variant fields of an output file name that matches §6.5, or `None` when it does not.
pub fn parse_output_name(name: &str) -> Option<NameFields> {
    let captures = KNOWN_NAME
        .captures(name)
        .or_else(|| UNKNOWN_NAME.captures(name))?;
    Some(NameFields {
        codec: captures[1].to_owned(),
        resolution_label: captures[2].to_owned(),
        frame_rate: captures[3].parse().ok()?,
    })
}

/// What an import did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// Rows inserted.
    pub imported: usize,
    /// Entries older than the retention window, skipped.
    pub too_old: usize,
    /// Entries whose fields do not decode, or whose date is not ISO 8601, skipped.
    pub invalid: usize,
}

/// The outcome of [`import_swift_history`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStatus {
    /// The history was imported by an earlier start; nothing was read.
    AlreadyImported,
    /// The file does not exist. Nothing is imported and the setting stays unset.
    Missing,
    /// The file could not be read or is not a JSON array. A WARN row is logged and the setting
    /// stays unset, so a later start tries again.
    Unreadable,
    /// The file was read and its entries were handled; the setting is now set.
    Imported(ImportReport),
}

/// Imports the Swift history at `path` into `store`, once per database. Entries dated at or before
/// `now_ms - retention_days` are skipped, as the Swift daemon pruned them.
///
/// Each row's id is derived from its output path, so a repeated import would replace rather than
/// duplicate the rows.
pub fn import_swift_history(
    path: &Path,
    store: &Store,
    retention_days: i64,
    now_ms: i64,
) -> StoreResult<ImportStatus> {
    if store.swift_history_imported()? {
        return Ok(ImportStatus::AlreadyImported);
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(ImportStatus::Missing),
        Err(err) => {
            warn_unreadable(store, now_ms, path, &err.to_string())?;
            return Ok(ImportStatus::Unreadable);
        }
    };
    let values: Vec<serde_json::Value> = match serde_json::from_str(&text) {
        Ok(values) => values,
        Err(err) => {
            warn_unreadable(store, now_ms, path, &err.to_string())?;
            return Ok(ImportStatus::Unreadable);
        }
    };

    let cutoff = now_ms - retention_days * DAY_MS;
    let mut report = ImportReport::default();
    for value in values {
        let Ok(entry) = serde_json::from_value::<SwiftRender>(value) else {
            report.invalid += 1;
            continue;
        };
        let Some(rendered_at) = parse_timestamp(&entry.timestamp) else {
            report.invalid += 1;
            continue;
        };
        if rendered_at <= cutoff {
            report.too_old += 1;
            continue;
        }
        store.put_rendered_video(&row_for(&entry, rendered_at))?;
        report.imported += 1;
    }
    store.set_swift_history_imported(true)?;

    let text = format!(
        "imported {} renders from the Swift history ({} too old, {} invalid)",
        report.imported, report.too_old, report.invalid
    );
    let fields = LogFields {
        event: Some("history_imported".to_owned()),
        video_count: i64::try_from(report.imported).ok(),
        ..LogFields::default()
    };
    log_event(store, now_ms, LogLevel::Info, "history", &text, &fields)?;
    Ok(ImportStatus::Imported(report))
}

fn warn_unreadable(store: &Store, now_ms: i64, path: &Path, reason: &str) -> StoreResult<()> {
    let text = format!(
        "cannot import the Swift history {}: {reason}",
        path.display()
    );
    let fields = LogFields {
        event: Some("history_import_failed".to_owned()),
        ..LogFields::default()
    };
    log_event(store, now_ms, LogLevel::Warn, "history", &text, &fields)?;
    Ok(())
}

/// The milliseconds since the epoch of an RFC 3339 timestamp, as Swift's ISO 8601 encoder writes
/// it (with or without fractional seconds).
fn parse_timestamp(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|date| date.timestamp_millis())
}

/// The row for one Swift entry. Its variant fields come from the output file name when it matches
/// §6.5; otherwise they stay empty. `variant_key` is never derived: the codec in it is the codec
/// table's name, which the file name does not carry, so it stays empty.
fn row_for(entry: &SwiftRender, rendered_at: i64) -> RenderedVideo {
    let names = parse_output_name(&entry.output_filename).unwrap_or_default();
    RenderedVideo {
        id: imported_id(&entry.output_path),
        rendered_at,
        sequence_name: entry.sequence_name.clone(),
        export_path: entry.export_path.clone(),
        output_filename: entry.output_filename.clone(),
        output_path: entry.output_path.clone(),
        variant_key: String::new(),
        codec: names.codec,
        resolution_label: names.resolution_label,
        frame_rate: names.frame_rate,
        frame_count: entry.frame_count,
        size_bytes: entry.file_size,
        width: 0,
        height: 0,
        duration_seconds: 0.0,
        render_seconds: -1.0,
        thumbnail_path: None,
        xmp_embedded: false,
        source: SOURCE_IMPORTED.to_owned(),
    }
}

/// The row id of an imported output: `imported-` and the SHA-256 of its output path. The Swift
/// history keys its entries by output path too.
fn imported_id(output_path: &str) -> String {
    format!(
        "imported-{}",
        hex::encode(Sha256::digest(output_path.as_bytes()))
    )
}
