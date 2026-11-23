// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Frame pattern detection for image sequences (§6.5).
//!
//! Ported from the Swift `ImageSequencePattern.detect` and `RenderFailureDiagnosis.decompose`.

use std::collections::HashMap;
use std::path::Path;

/// ffmpeg's image2 demuxer probes only indices below this when hunting for the first frame, so a
/// sequence starting at or past it needs an explicit `-start_number` (§6.5).
pub const IMAGE_DEMUXER_PROBE_RANGE: i64 = 5;

/// The digit width the daemon assumed before detection existed (§6.5).
pub const LEGACY_FRAME_NUMBER_DIGITS: usize = 5;

/// A file name split at its first run of ASCII digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decomposed {
    /// Everything before the first digit, e.g. `image_`.
    pub prefix: String,
    /// The first run of ASCII digits, e.g. `0001`.
    pub digits: String,
    /// Everything after that run, e.g. `.tiff` or `-severe-noise.tiff`.
    pub suffix: String,
}

/// Splits `filename` at its first run of ASCII digits. `None` when it has no digits.
pub fn decompose(filename: &str) -> Option<Decomposed> {
    let start = filename.find(|c: char| c.is_ascii_digit())?;
    let rest = &filename[start..];
    let run = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let end = start + run;
    Some(Decomposed {
        prefix: filename[..start].to_owned(),
        digits: filename[start..end].to_owned(),
        suffix: filename[end..].to_owned(),
    })
}

/// The `prefix%0Nd suffix` shape of an exported image sequence, worked out from the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSequencePattern {
    /// Everything before the frame number.
    pub prefix: String,
    /// Width of the zero-padded frame number.
    pub digits: usize,
    /// Everything after the frame number.
    pub suffix: String,
    /// Lowest frame number present.
    pub first_index: i64,
    /// How many files matched this pattern.
    pub frame_count: usize,
    /// The lowest-numbered matching file name; used to probe the source size.
    pub example_filename: String,
    /// True when the detected prefix is the configured `image_name_prefix`.
    pub matches_configured_prefix: bool,
}

impl ImageSequencePattern {
    /// The filename part of ffmpeg's `-i` argument, e.g. `image_%04d.tiff`.
    pub fn pattern_description(&self) -> String {
        format!("{}%0{}d{}", self.prefix, self.digits, self.suffix)
    }

    /// Whether `-start_number` is needed: the sequence starts past the demuxer's probe range.
    pub fn requires_explicit_start_number(&self) -> bool {
        self.first_index >= IMAGE_DEMUXER_PROBE_RANGE
    }
}

/// Works out the pattern from the files in `dir`. `None` when the directory cannot be read or
/// holds no numbered file with one of `image_extensions` (all extensions when that list is empty).
///
/// The group whose prefix equals `preferred_prefix` wins when there is one; otherwise the largest
/// group does. Ties go to the smallest example file name.
pub fn detect(
    dir: &Path,
    image_extensions: &[String],
    preferred_prefix: &str,
) -> Option<ImageSequencePattern> {
    let entries = std::fs::read_dir(dir).ok()?;
    let wanted: Vec<String> = image_extensions.iter().map(|e| e.to_lowercase()).collect();

    #[derive(PartialEq, Eq, Hash)]
    struct Key {
        prefix: String,
        digits: usize,
        suffix: String,
    }
    struct Group {
        count: usize,
        first_index: i64,
        example: String,
    }
    let mut groups: HashMap<Key, Group> = HashMap::new();

    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Some(dot) = name.rfind('.') else {
            continue;
        };
        let ext = name[dot + 1..].to_lowercase();
        if !wanted.is_empty() && !wanted.contains(&ext) {
            continue;
        }
        let Some(parts) = decompose(&name) else {
            continue;
        };
        let Ok(index) = parts.digits.parse::<i64>() else {
            continue;
        };
        let key = Key {
            digits: parts.digits.len(),
            prefix: parts.prefix,
            suffix: parts.suffix,
        };
        let group = groups.entry(key).or_insert_with(|| Group {
            count: 0,
            first_index: i64::MAX,
            example: String::new(),
        });
        group.count += 1;
        if index < group.first_index {
            group.first_index = index;
            group.example = name;
        }
    }
    if groups.is_empty() {
        return None;
    }

    let has_preferred = groups.keys().any(|k| k.prefix == preferred_prefix);
    let winner = groups
        .iter()
        .filter(|(k, _)| !has_preferred || k.prefix == preferred_prefix)
        .min_by(|(_, a), (_, b)| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.example.cmp(&b.example))
        })?;
    let (key, group) = winner;
    Some(ImageSequencePattern {
        prefix: key.prefix.clone(),
        digits: key.digits,
        suffix: key.suffix.clone(),
        first_index: group.first_index,
        frame_count: group.count,
        example_filename: group.example.clone(),
        matches_configured_prefix: key.prefix == preferred_prefix,
    })
}
