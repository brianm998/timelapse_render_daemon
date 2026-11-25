// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The Activity snapshot served to clients (overview §6.1).
//!
//! The types here mirror the proto messages of §7 (`Snapshot`, `SequenceCard`, `VariantRow`,
//! `RenderedVideo`, `FailureCard`, `Counts`) in plain Rust, without a proto dependency. The engine
//! builds one [`Snapshot`] from its state; this module holds the rules that decide its contents:
//! the §6.10 `in_progress` order, the stage of a sequence, the phase, the count-mismatch note, and
//! the `file_exists` cache of the recently rendered rows.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub use crate::problems::{Problem, Severity};

/// How many rows `recently_rendered` holds (§6.10).
pub const RECENT_LIMIT: usize = 30;

/// How long a `file_exists` answer is kept before the file is checked again (§6.10).
pub const FILE_EXISTS_RECHECK: Duration = Duration::from_secs(60);

/// The daemon's phase (§6.10, proto `Phase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Starting,
    NoConfig,
    WaitingForDiskAccess,
    Idle,
    Exporting,
    Rendering,
    Paused,
}

/// Where a sequence is in its life (proto `Stage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Exporting,
    CountMismatch,
    CollectingMetadata,
    Queued,
    Rendering,
    Blocked,
    Paused,
}

/// Where a variant of a sequence is (proto `VariantStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantStatus {
    /// The sequence is not complete, or its histogram is not known yet.
    Waiting,
    /// The output existed before trd rendered it.
    AlreadyRendered,
    Queued,
    Running,
    Suspended,
    /// Rendered by trd since the sequence became complete.
    Done,
    Failed,
    /// The variant or its sequence is paused.
    Paused,
    UnknownCodec,
}

impl VariantStatus {
    /// Whether the variant needs no more work (§6.10: the sequence leaves `in_progress`).
    pub fn is_finished(self) -> bool {
        matches!(self, VariantStatus::Done | VariantStatus::AlreadyRendered)
    }
}

/// One variant of a sequence (proto `VariantRow`).
#[derive(Debug, Clone, PartialEq)]
pub struct VariantRow {
    pub variant_key: String,
    pub codec: String,
    pub resolution_label: String,
    pub frame_rate: u32,
    pub output_filename: String,
    pub output_path: String,
    pub status: VariantStatus,
    /// The render's id, for a queued, running, suspended, done or failed variant; else empty.
    pub render_id: String,
    pub frame: u64,
    pub total_frames: u64,
    pub fps: f64,
    /// Seconds until the render ends; -1 when unknown.
    pub eta_seconds: i64,
    pub progress_known: bool,
    pub adopted: bool,
    /// The output's size when it is done or already rendered; the partial's size when running.
    /// -1 otherwise.
    pub size_bytes: i64,
    pub message: String,
    pub failure_id: String,
}

/// One sequence that is not finished (proto `SequenceCard`).
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceCard {
    /// `<export_path>/<name>`.
    pub id: String,
    pub name: String,
    pub export_path: String,
    pub raw_sequence_name: String,
    pub raw_sequence_location: String,
    pub raw_from_backup: bool,
    pub expected_frames: u32,
    pub exported_frames: u32,
    pub stage: Stage,
    pub paused: bool,
    pub export_frames_per_minute: f64,
    /// Seconds until the export ends; -1 when unknown.
    pub export_eta_seconds: i64,
    /// Recursive size of the export directory; -1 until measured.
    pub export_size_bytes: i64,
    pub first_seen_ms: i64,
    pub last_frame_ms: i64,
    /// The variants, in the config's `rendering` order.
    pub variants: Vec<VariantRow>,
    pub note: String,
}

impl SequenceCard {
    /// The highest progress of a running or suspended variant, as a fraction of its frames.
    fn render_progress(&self) -> f64 {
        self.variants
            .iter()
            .filter(|v| matches!(v.status, VariantStatus::Running | VariantStatus::Suspended))
            .filter(|v| v.total_frames > 0)
            .map(|v| v.frame as f64 / v.total_frames as f64)
            .fold(0.0, f64::max)
    }

    /// The exported frames as a fraction of the expected ones.
    fn export_fraction(&self) -> f64 {
        if self.expected_frames == 0 {
            0.0
        } else {
            f64::from(self.exported_frames) / f64::from(self.expected_frames)
        }
    }
}

/// A rendered video (proto `RenderedVideo`), with its file state checked.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedVideo {
    pub id: String,
    pub rendered_at_ms: i64,
    pub sequence_name: String,
    pub export_path: String,
    pub output_filename: String,
    pub output_path: String,
    pub variant_key: String,
    pub codec: String,
    pub resolution_label: String,
    pub frame_rate: i64,
    pub frame_count: i64,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_seconds: f64,
    /// Seconds the render took; -1 when unknown.
    pub render_seconds: i64,
    pub has_thumbnail: bool,
    pub file_exists: bool,
    pub xmp_embedded: bool,
    /// `TRD`, `ADOPTED` or `IMPORTED`.
    pub source: String,
}

/// A failure card (proto `FailureCard`), the fields a client lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureCard {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub detail: String,
    pub sequence_name: String,
    pub output_filename: String,
    pub output_path: String,
    pub variant_key: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub occurrences: i64,
}

/// The counts the activity header shows (proto `Counts`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Running renders, adopted ones included.
    pub rendering: u32,
    pub suspended: u32,
    pub queued: u32,
    /// Sequences whose stage is exporting.
    pub exporting: u32,
    /// Sequences that are paused.
    pub paused_sequences: u32,
    pub failures: u32,
    pub rendered_last_24h: u32,
}

/// The state the clients see (§6.10, proto `Snapshot`). `revision` goes up by one on every change.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub revision: u64,
    pub generated_at_ms: i64,
    pub phase: Phase,
    pub global_pause: bool,
    /// Errors first, then by id.
    pub problems: Vec<Problem>,
    /// Newest first.
    pub failures: Vec<FailureCard>,
    /// In the §6.10 display order.
    pub in_progress: Vec<SequenceCard>,
    /// Newest first, at most [`RECENT_LIMIT`].
    pub recently_rendered: Vec<RenderedVideo>,
    /// Rows within the retention window.
    pub recently_rendered_total: u32,
    pub counts: Counts,
    /// 0 before the first scan.
    pub last_scan_completed_ms: i64,
    pub last_scan_duration_ms: u32,
    pub config_path: String,
}

impl Snapshot {
    /// Whether `other` shows the same state. The revision, the generation time and the last-scan
    /// timing are not compared: a scan that changes nothing is not a change, so an idle daemon
    /// publishes nothing. The timing reaches the clients with the next real change.
    pub fn same_content(&self, other: &Snapshot) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        for s in [&mut a, &mut b] {
            s.revision = 0;
            s.generated_at_ms = 0;
            s.last_scan_completed_ms = 0;
            s.last_scan_duration_ms = 0;
        }
        a == b
    }
}

/// The note of a sequence whose export has more frames than its raw sequence (§6.10).
pub fn mismatch_note(exported: u64, expected: u64) -> String {
    format!(
        "{exported} frames exported but the raw sequence has {expected}; trd renders only when the counts are equal"
    )
}

/// What decides a sequence's stage (§6.10). The flags are checked in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageInputs {
    pub paused: bool,
    /// Exported frames are more than the raw sequence has.
    pub count_mismatch: bool,
    /// The export is still growing (fewer frames than its raw sequence).
    pub exporting: bool,
    /// A variant is running or suspended.
    pub rendering: bool,
    /// A variant is queued.
    pub queued: bool,
    /// The sequence is complete but its histogram is not known yet.
    pub collecting: bool,
}

/// The stage of a sequence: paused, count mismatch, exporting, rendering, queued, collecting
/// metadata, else blocked (complete, but nothing left that can run).
pub fn stage_of(inputs: StageInputs) -> Stage {
    if inputs.paused {
        Stage::Paused
    } else if inputs.count_mismatch {
        Stage::CountMismatch
    } else if inputs.exporting {
        Stage::Exporting
    } else if inputs.rendering {
        Stage::Rendering
    } else if inputs.queued {
        Stage::Queued
    } else if inputs.collecting {
        Stage::CollectingMetadata
    } else {
        Stage::Blocked
    }
}

/// The rank of a stage in the §6.10 `in_progress` order. Rank 1 holds both queued and collecting.
pub fn stage_rank(stage: Stage) -> u8 {
    match stage {
        Stage::Rendering => 0,
        Stage::Queued | Stage::CollectingMetadata => 1,
        Stage::Exporting => 2,
        Stage::CountMismatch => 3,
        Stage::Blocked => 4,
        Stage::Paused => 5,
    }
}

/// Sorts the cards into the §6.10 order: rendering by highest variant progress then name; queued
/// and collecting by first sighting; exporting by exported over expected, descending, then name;
/// then count mismatch, blocked and paused by name.
pub fn order(cards: &mut [SequenceCard]) {
    cards.sort_by(|a, b| {
        stage_rank(a.stage)
            .cmp(&stage_rank(b.stage))
            .then_with(|| match stage_rank(a.stage) {
                0 => b.render_progress().total_cmp(&a.render_progress()),
                1 => a.first_seen_ms.cmp(&b.first_seen_ms),
                2 => b.export_fraction().total_cmp(&a.export_fraction()),
                _ => Ordering::Equal,
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// What decides the phase (§6.10). `started` is false until the first tick has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseInputs {
    pub started: bool,
    pub config_loaded: bool,
    pub fda_missing: bool,
    pub global_pause: bool,
    pub job_running: bool,
    pub sequence_exporting: bool,
}

/// The phase, by precedence: starting, waiting for disk access, no config, paused, rendering,
/// exporting, else idle. Waiting comes before no config: the config is not read while access is
/// missing (§6.12), so at launch without access it is not yet known whether one exists.
pub fn phase_of(inputs: PhaseInputs) -> Phase {
    if !inputs.started {
        Phase::Starting
    } else if inputs.fda_missing {
        Phase::WaitingForDiskAccess
    } else if !inputs.config_loaded {
        Phase::NoConfig
    } else if inputs.global_pause {
        Phase::Paused
    } else if inputs.job_running {
        Phase::Rendering
    } else if inputs.sequence_exporting {
        Phase::Exporting
    } else {
        Phase::Idle
    }
}

/// The `file_exists` answers of the recently rendered rows, each kept for
/// [`FILE_EXISTS_RECHECK`] by the injected instant.
#[derive(Debug, Default)]
pub struct FileExistsCache {
    checked: HashMap<String, (bool, Instant)>,
}

impl FileExistsCache {
    /// Whether `path` exists. The file is probed with `probe` only when no answer is held or the
    /// held answer is at least [`FILE_EXISTS_RECHECK`] old at `now`.
    pub fn check(&mut self, path: &str, now: Instant, probe: impl FnOnce(&str) -> bool) -> bool {
        if let Some((exists, at)) = self.checked.get(path)
            && now.saturating_duration_since(*at) < FILE_EXISTS_RECHECK
        {
            return *exists;
        }
        let exists = probe(path);
        self.checked.insert(path.to_owned(), (exists, now));
        exists
    }

    /// The answer held for `path`, without probing it. `None` when no answer is held.
    pub fn held(&self, path: &str) -> Option<bool> {
        self.checked.get(path).map(|(exists, _)| *exists)
    }

    /// Forgets the answers of paths not in `keep`.
    pub fn retain(&mut self, keep: &[String]) {
        self.checked.retain(|path, _| keep.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(
        name: &str,
        stage: Stage,
        first_seen_ms: i64,
        exported: u32,
        expected: u32,
    ) -> SequenceCard {
        SequenceCard {
            id: name.to_owned(),
            name: name.to_owned(),
            export_path: String::new(),
            raw_sequence_name: name.to_owned(),
            raw_sequence_location: String::new(),
            raw_from_backup: false,
            expected_frames: expected,
            exported_frames: exported,
            stage,
            paused: false,
            export_frames_per_minute: 0.0,
            export_eta_seconds: -1,
            export_size_bytes: -1,
            first_seen_ms,
            last_frame_ms: 0,
            variants: Vec::new(),
            note: String::new(),
        }
    }

    #[test]
    fn exporting_cards_sort_by_fraction_then_name() {
        let mut cards = vec![
            card("b", Stage::Exporting, 0, 50, 100),
            card("a", Stage::Exporting, 0, 50, 100),
            card("c", Stage::Exporting, 0, 90, 100),
        ];
        order(&mut cards);
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }

    #[test]
    fn mismatch_note_matches_the_spec_text() {
        assert_eq!(
            mismatch_note(12, 10),
            "12 frames exported but the raw sequence has 10; trd renders only when the counts are equal"
        );
    }

    #[test]
    fn cache_holds_an_answer_for_a_minute() {
        let mut cache = FileExistsCache::default();
        let start = Instant::now();
        assert!(cache.check("/x", start, |_| true));
        assert!(cache.check("/x", start + Duration::from_secs(59), |_| false));
        assert!(!cache.check("/x", start + Duration::from_secs(60), |_| false));
    }
}
