// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! XMP embedding and corrupt-render detection (§6.6).
//!
//! The canonical XMP-EXIF fields of a sequence come from its EXIF histogram (§6.4): each field
//! gets its most frequent value, and `DateTimeOriginal` the earliest timestamp, so the embedded
//! date marks the start of the timelapse. [`embed`] writes them into the rendered file with
//! exiftool. A `Truncated mdat atom` or a similar container error means the video itself is
//! corrupt; any other exiftool failure leaves the video usable. This is a port of the Swift
//! `XMPEmbedder` and `XMPFieldMapper`, which the mapping tests hold to the same output.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use crate::exif::{ExifError, Histogram};
use crate::tools::ResolvedTool;

/// The outcome of an embed attempt. The caller decides what to do with a corrupt render (§6.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmpResult {
    /// exiftool exited 0 and wrote the XMP block.
    Succeeded,
    /// No XMP to write: the histogram has no tag that maps to one. Not a problem with the video.
    Skipped { reason: String },
    /// exiftool refused to write because the container is corrupt; `reason` is the matched
    /// message and `output` the combined exiftool output.
    StructuralCorruption {
        reason: &'static str,
        output: String,
    },
    /// exiftool failed for another reason (permissions, disk full, …). The video is presumed
    /// usable. `code` is the exit code, or -1 when a signal ended the process.
    ToolError { output: String, code: i32 },
}

/// The exiftool messages that mean the container is corrupt, in the order they are tried. These
/// are the strings the QuickTime writer prints when the moov sample table is inconsistent or the
/// mdat is shorter than stsz claims.
const STRUCTURAL_NEEDLES: [&str; 4] = [
    "Truncated mdat atom",
    "Bad atom size",
    "Invalid mdat",
    "Could not parse moov",
];

/// The `-XMP-…` assignments for a histogram, in the Swift order. A histogram with none of the
/// mapped columns gives an empty list.
pub fn assignments(histogram: &Histogram) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = mode(histogram.get("Make")) {
        out.push(format!("-XMP-tiff:Make={value}"));
    }
    if let Some(value) = mode(histogram.get("Model")) {
        out.push(format!("-XMP-tiff:Model={value}"));
    }
    if let Some(value) = mode(histogram.get("LensModel")) {
        out.push(format!("-XMP-aux:Lens={value}"));
    }
    // exiftool's `ISO` tag is the XMP exif:ISOSpeedRatings element.
    if let Some(value) = mode(histogram.get("ISO")) {
        out.push(format!("-XMP-exif:ISO={value}"));
    }
    if let Some(value) = mode(histogram.get("FNumber")) {
        out.push(format!("-XMP-exif:FNumber={value}"));
    }
    if let Some(value) = mode(histogram.get("ExposureTime")) {
        out.push(format!("-XMP-exif:ExposureTime={value}"));
    }
    if let Some(value) = mode(histogram.get("FocalLength")) {
        out.push(format!(
            "-XMP-exif:FocalLength={}",
            strip_focal_length_suffix(value)
        ));
    }
    // The three enum tags are integers in XMP. The `#` suffix makes exiftool write the raw value
    // and skip its text PrintConv.
    if let Some(value) = mode(histogram.get("ExposureProgram")) {
        out.push(format!(
            "-XMP-exif:ExposureProgram#={}",
            exposure_program_to_int(value)
        ));
    }
    if let Some(value) = mode(histogram.get("ExposureMode")) {
        out.push(format!(
            "-XMP-exif:ExposureMode#={}",
            exposure_mode_to_int(value)
        ));
    }
    if let Some(value) = mode(histogram.get("WhiteBalance")) {
        out.push(format!(
            "-XMP-exif:WhiteBalance#={}",
            white_balance_to_int(value)
        ));
    }
    if let Some(value) = earliest(histogram.get("DateTimeOriginal")) {
        out.push(format!("-XMP-exif:DateTimeOriginal={}", to_iso8601(value)));
    }
    out
}

/// The most frequent value. On equal counts the lexicographically smallest value wins, as the
/// Swift `max(by:)` comparator does.
fn mode(counts: Option<&BTreeMap<String, u64>>) -> Option<&str> {
    let counts = counts?;
    // The map iterates in ascending key order, so a strictly larger count replaces the best and an
    // equal count keeps the earlier, smaller key.
    let mut best: Option<(&str, u64)> = None;
    for (value, &count) in counts {
        match best {
            Some((_, best_count)) if count <= best_count => {}
            _ => best = Some((value.as_str(), count)),
        }
    }
    best.map(|(value, _)| value)
}

/// The smallest key. For EXIF dates (`YYYY:MM:DD HH:MM:SS`) byte order is chronological order.
fn earliest(counts: Option<&BTreeMap<String, u64>>) -> Option<&str> {
    counts?.keys().next().map(String::as_str)
}

/// `"14.0 mm"` → `"14.0"`, `"85mm"` → `"85"`, `"85"` → `"85"`.
fn strip_focal_length_suffix(value: &str) -> &str {
    let trimmed = value.trim_matches([' ', '\t']);
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[bytes.len() - 2..].eq_ignore_ascii_case(b"mm") {
        // The two bytes are ASCII, so the cut is on a character boundary.
        return trimmed[..trimmed.len() - 2].trim_matches([' ', '\t']);
    }
    trimmed
}

/// EXIF ExposureProgram text → its integer. Unknown values map to 0 (Not defined).
fn exposure_program_to_int(value: &str) -> u32 {
    let lower = value.to_lowercase();
    if lower == "manual" {
        1
    } else if lower == "program ae" || lower == "normal program" {
        2
    } else if lower.contains("aperture") {
        3
    } else if lower.contains("shutter") {
        4
    } else if lower.contains("creative") {
        5
    } else if lower.contains("action") {
        6
    } else if lower == "portrait" {
        7
    } else if lower == "landscape" {
        8
    } else {
        0
    }
}

/// EXIF ExposureMode: Auto = 0, Manual = 1, Auto-bracket = 2.
fn exposure_mode_to_int(value: &str) -> u32 {
    let lower = value.to_lowercase();
    if lower == "manual" {
        1
    } else if lower == "auto bracket" || lower == "auto-bracket" {
        2
    } else {
        0
    }
}

/// EXIF WhiteBalance is Auto = 0 or Manual / preset = 1.
fn white_balance_to_int(value: &str) -> u32 {
    if value.to_lowercase() == "auto" { 0 } else { 1 }
}

/// `YYYY:MM:DD HH:MM:SS` → `YYYY-MM-DDTHH:MM:SS`. Input of any other shape is returned as it is:
/// exiftool accepts both forms.
///
/// The split follows Swift's `split`: empty pieces are skipped, so leading and repeated spaces
/// do not make a second piece. The date and time must each have three non-empty parts.
fn to_iso8601(value: &str) -> String {
    let Some((date, time)) = split_at_first_space(value) else {
        return value.to_owned();
    };
    let date = date.replace(':', "-");
    let date_parts = non_empty_parts(&date, '-');
    let time_parts = non_empty_parts(time, ':');
    if date_parts == 3 && time_parts == 3 {
        format!("{date}T{time}")
    } else {
        value.to_owned()
    }
}

/// Splits `value` at its first space, as Swift's `split(separator:maxSplits: 1)` does with empty
/// pieces omitted. `None` when there are fewer than two pieces.
fn split_at_first_space(value: &str) -> Option<(&str, &str)> {
    let value = value.trim_start_matches(' ');
    let (first, rest) = value.split_once(' ')?;
    let rest = rest.trim_start_matches(' ');
    if rest.is_empty() {
        None
    } else {
        Some((first, rest))
    }
}

/// How many non-empty pieces `text` has when split on `separator`.
fn non_empty_parts(text: &str, separator: char) -> usize {
    text.split(separator)
        .filter(|part| !part.is_empty())
        .count()
}

/// The combined output: the lines of stdout, then the lines of stderr, joined with line breaks.
fn combined_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The matched message when `output` shows a structurally corrupt container; `None` otherwise.
pub fn detect_structural_corruption(output: &str) -> Option<&'static str> {
    STRUCTURAL_NEEDLES
        .into_iter()
        .find(|needle| output.contains(needle))
}

/// Writes the XMP assignments for `histogram` into `video` with
/// `exiftool -overwrite_original -q <assignments> <video>`.
///
/// Returns `Skipped` without running exiftool when no tag maps. `Err` when the tool has no command
/// (`ToolUnavailable`) or cannot be started (`Launch`). A non-zero exit is `StructuralCorruption`
/// when the output matches a corruption message, else `ToolError`.
pub fn embed(
    video: &Path,
    histogram: &Histogram,
    tool: &ResolvedTool,
) -> Result<XmpResult, ExifError> {
    let assignments = assignments(histogram);
    if assignments.is_empty() {
        return Ok(XmpResult::Skipped {
            reason: "no tags resolved from sidecar".to_owned(),
        });
    }
    let command = tool.command.as_ref().ok_or(ExifError::ToolUnavailable)?;
    let output = command
        .command()
        .args(["-overwrite_original", "-q"])
        .args(&assignments)
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ExifError::Launch {
            program: command.program.to_string_lossy().into_owned(),
            source,
        })?;
    let combined = combined_output(&output.stdout, &output.stderr);
    if output.status.success() {
        return Ok(XmpResult::Succeeded);
    }
    if let Some(reason) = detect_structural_corruption(&combined) {
        return Ok(XmpResult::StructuralCorruption {
            reason,
            output: combined,
        });
    }
    Ok(XmpResult::ToolError {
        output: combined,
        code: output.status.code().unwrap_or(-1),
    })
}
