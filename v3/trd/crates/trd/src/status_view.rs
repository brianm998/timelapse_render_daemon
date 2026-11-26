// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The text Activity view printed by trd status (spec 1 §5, overview §6.1), and the line `trd log`
//! prints for one entry. Pure functions over the trd.v1 messages: no paths into the crate, so the
//! integration tests include this file by path and compare the golden text.
//!
//! Colours follow the Perl console: blue brackets, green `*` for done frames, yellow `-` for the
//! rest of a progress bar (`progress_bar` in the Perl daemon).

use trd_proto::trd::v1 as pb;

const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const CYAN: &str = "\x1b[36m";

/// How many recently rendered videos the view lists (§5: newest 10).
const RECENT_LINES: usize = 10;

/// How the view is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Cells in a progress bar (`progress_bar_width`).
    pub bar_width: usize,
    /// Whether to colour the output: `log_in_color`, a terminal, and not `--plain`.
    pub color: bool,
}

impl Style {
    fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("{code}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }
}

/// Renders a snapshot as the lines of the Activity view. `now_ms` sets the relative times and
/// `utc_offset_seconds` the local clock used for absolute times.
pub fn render(
    snapshot: &pb::Snapshot,
    style: Style,
    now_ms: i64,
    utc_offset_seconds: i32,
) -> Vec<String> {
    let mut lines = Vec::new();
    header(snapshot, style, &mut lines);
    problems(snapshot, style, &mut lines);
    failures(snapshot, style, &mut lines);
    in_progress(snapshot, style, &mut lines);
    recently_rendered(snapshot, style, now_ms, utc_offset_seconds, &mut lines);
    lines
}

/// The line `trd log` prints for one entry: local time, level, then the subject and message.
pub fn log_line(entry: &pb::LogEntry, utc_offset_seconds: i32) -> String {
    let level = match entry.level() {
        pb::LogLevel::LogDebug => "DEBUG",
        pb::LogLevel::LogWarn => "WARN",
        pb::LogLevel::LogError => "ERROR",
        _ => "INFO",
    };
    let text = if entry.subject.is_empty() {
        entry.message.clone()
    } else {
        format!("{}: {}", entry.subject, entry.message)
    };
    format!(
        "{} {level:<5} {text}",
        clock_time(entry.at_ms, utc_offset_seconds)
    )
}

/// `HH:MM:SS` on the local clock.
pub fn clock_time(ms: i64, utc_offset_seconds: i32) -> String {
    let (_, _, _, hour, minute, second) = local_parts(ms, utc_offset_seconds);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// `YYYY-MM-DD HH:MM` on the local clock.
pub fn date_time(ms: i64, utc_offset_seconds: i32) -> String {
    let (year, month, day, hour, minute, _) = local_parts(ms, utc_offset_seconds);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Calendar and clock fields of `ms` shifted by the offset: year, month, day, hour, minute, second.
fn local_parts(ms: i64, utc_offset_seconds: i32) -> (i64, u32, u32, u32, u32, u32) {
    let seconds = ms.div_euclid(1000) + i64::from(utc_offset_seconds);
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        (time_of_day / 3600) as u32,
        (time_of_day % 3600 / 60) as u32,
        (time_of_day % 60) as u32,
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = (if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    }) as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `1,234`.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A size in bytes as `512 B`, `12.3 KiB`, `4.5 MiB` or `1.2 GiB`; `?` when unknown (-1).
pub fn size_text(bytes: i64) -> String {
    if bytes < 0 {
        return "?".to_owned();
    }
    let bytes = bytes as f64;
    if bytes < 1024.0 {
        format!("{bytes} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else if bytes < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MiB", bytes / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes / (1024.0 * 1024.0 * 1024.0))
    }
}

/// `m:ss`, or `h:mm:ss` from an hour.
fn duration_text(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, secs) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// `12 min ago` style text for the time since `then_ms`.
fn relative_text(now_ms: i64, then_ms: i64) -> String {
    let seconds = (now_ms - then_ms).max(0) / 1000;
    if seconds < 60 {
        "just now".to_owned()
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h ago", seconds / 3600)
    } else {
        format!("{} d ago", seconds / 86_400)
    }
}

/// An ETA in seconds as `~6 min` or `~45 s`; `None` when unknown (-1).
fn eta_text(seconds: i64) -> Option<String> {
    if seconds < 0 {
        None
    } else if seconds < 60 {
        Some(format!("~{seconds} s"))
    } else {
        Some(format!("~{} min", (seconds + 30) / 60))
    }
}

/// A bar of `style.bar_width` cells filled to `fraction` (0 to 1), as the Perl `progress_bar`.
fn bar(fraction: f64, style: Style) -> String {
    let mut out = style.paint(BLUE, "[");
    for i in 0..style.bar_width {
        if (i as f64) / (style.bar_width as f64) < fraction {
            out.push_str(&style.paint(GREEN, "*"));
        } else {
            out.push_str(&style.paint(YELLOW, "-"));
        }
    }
    out.push_str(&style.paint(BLUE, "]"));
    out
}

fn fraction(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64).min(1.0)
    }
}

fn header(snapshot: &pb::Snapshot, style: Style, lines: &mut Vec<String>) {
    let config = if snapshot.config_path.is_empty() {
        "none".to_owned()
    } else {
        snapshot.config_path.clone()
    };
    lines.push(format!("Config: {config}"));
    let pause = if snapshot.global_pause {
        style.paint(YELLOW, "on")
    } else {
        "off".to_owned()
    };
    lines.push(format!(
        "Phase: {}    Pause all rendering: {pause}",
        phase_label(snapshot.phase())
    ));
    let counts = snapshot.counts.unwrap_or_default();
    let parts: Vec<String> = [
        (counts.rendering, "rendering".to_owned()),
        (counts.suspended, "suspended".to_owned()),
        (counts.queued, "queued".to_owned()),
        (counts.exporting, "exporting".to_owned()),
        (counts.paused_sequences, "paused".to_owned()),
        (counts.failures, "failed".to_owned()),
        (counts.rendered_last_24h, "rendered in 24 h".to_owned()),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, label)| format!("{n} {label}"))
    .collect();
    lines.push(if parts.is_empty() {
        "idle".to_owned()
    } else {
        parts.join(" · ")
    });
    lines.push(String::new());
}

fn problems(snapshot: &pb::Snapshot, style: Style, lines: &mut Vec<String>) {
    if snapshot.problems.is_empty() {
        return;
    }
    lines.push("Problems".to_owned());
    for problem in &snapshot.problems {
        let (code, label) = match problem.severity() {
            pb::Severity::Error => (RED, "ERROR"),
            _ => (YELLOW, "WARNING"),
        };
        lines.push(format!("  {}  {}", style.paint(code, label), problem.title));
        if !problem.detail.is_empty() {
            lines.push(format!("      {}", problem.detail));
        }
        if !problem.fix.is_empty() {
            lines.push(format!("      Fix: {}", problem.fix));
        }
    }
    lines.push(String::new());
}

fn failures(snapshot: &pb::Snapshot, style: Style, lines: &mut Vec<String>) {
    if snapshot.failures.is_empty() {
        return;
    }
    lines.push(format!("Needs attention ({})", snapshot.failures.len()));
    for failure in &snapshot.failures {
        let file = if failure.output_filename.is_empty() {
            failure.sequence_name.as_str()
        } else {
            failure.output_filename.as_str()
        };
        let times = if failure.occurrences > 1 {
            format!("  ×{}", failure.occurrences)
        } else {
            String::new()
        };
        lines.push(format!(
            "  {} {file}{times}",
            style.paint(RED, &format!("[{}]", failure.kind_label))
        ));
        if !failure.summary.is_empty() {
            lines.push(format!("      {}", failure.summary));
        }
        let mut note = format!("      id {}", failure.id);
        if failure.variant_auto_paused {
            note.push_str(" · variant paused after repeated corrupt renders");
        }
        lines.push(note);
    }
    lines.push(String::new());
}

fn in_progress(snapshot: &pb::Snapshot, style: Style, lines: &mut Vec<String>) {
    if snapshot.in_progress.is_empty() {
        lines.push("Nothing in progress".to_owned());
        lines.push(String::new());
        return;
    }
    lines.push(format!("In progress ({})", snapshot.in_progress.len()));
    for card in &snapshot.in_progress {
        let paused = if card.paused {
            style.paint(YELLOW, " (paused)")
        } else {
            String::new()
        };
        lines.push(format!(
            "  {}  [{}]{paused}",
            card.name,
            stage_label(card.stage())
        ));
        lines.push(export_line(card, style));
        lines.push(format!("      {}", card.export_path));
        if !card.note.is_empty() {
            lines.push(format!("      {}", card.note));
        }
        for variant in &card.variants {
            lines.push(variant_line(variant, style));
        }
    }
    lines.push(String::new());
}

/// The sequence card's progress line: the bar, the frames, then the rate, the ETA and the size
/// while they are known.
fn export_line(card: &pb::SequenceCard, style: Style) -> String {
    let mut line = format!(
        "      {} {} / {} frames",
        bar(
            fraction(
                u64::from(card.exported_frames),
                u64::from(card.expected_frames)
            ),
            style
        ),
        grouped(u64::from(card.exported_frames)),
        grouped(u64::from(card.expected_frames)),
    );
    if card.stage() == pb::Stage::Exporting {
        if card.export_frames_per_minute > 0.0 {
            line.push_str(&format!(
                " · {:.0} frames/min",
                card.export_frames_per_minute
            ));
        }
        if let Some(eta) = eta_text(card.export_eta_seconds) {
            line.push_str(&format!(" · {eta} left"));
        }
    }
    if card.export_size_bytes >= 0 {
        line.push_str(&format!(" · {}", size_text(card.export_size_bytes)));
    }
    line
}

/// One variant row under a sequence card: label, bar, frames, fps, ETA and status.
fn variant_line(variant: &pb::VariantRow, style: Style) -> String {
    use pb::VariantStatus as S;
    let label = format!(
        "{} · {} · {} fps",
        variant.codec, variant.resolution_label, variant.frame_rate
    );
    let (done_fraction, detail, status) = match variant.status() {
        S::VariantDone | S::VariantAlreadyRendered => {
            let size = if variant.size_bytes >= 0 {
                size_text(variant.size_bytes)
            } else {
                String::new()
            };
            let status = if variant.status() == S::VariantDone {
                "done"
            } else {
                "exists"
            };
            (1.0, size, status)
        }
        S::VariantRunning | S::VariantSuspended => {
            let mut detail = format!(
                "{} / {} frames",
                grouped(u64::from(variant.frame)),
                grouped(u64::from(variant.total_frames))
            );
            if variant.fps > 0.0 {
                detail.push_str(&format!(" · {:.1} fps", variant.fps));
            }
            if let Some(eta) = eta_text(variant.eta_seconds) {
                detail.push_str(&format!(" · {eta} left"));
            }
            let status = if variant.status() == S::VariantSuspended {
                "suspended"
            } else {
                "rendering"
            };
            (
                fraction(u64::from(variant.frame), u64::from(variant.total_frames)),
                detail,
                status,
            )
        }
        S::VariantFailed => (0.0, variant.message.clone(), "failed"),
        S::VariantPaused => (0.0, String::new(), "paused"),
        S::VariantUnknownCodec => (0.0, String::new(), "unknown codec"),
        S::VariantQueued => (0.0, String::new(), "queued"),
        _ => (0.0, String::new(), "waiting"),
    };
    let mut line = format!("      {label}  {}", bar(done_fraction, style));
    if !detail.is_empty() {
        line.push_str(&format!(" {detail}"));
    }
    line.push_str(&format!("  {status}"));
    line
}

fn recently_rendered(
    snapshot: &pb::Snapshot,
    style: Style,
    now_ms: i64,
    utc_offset_seconds: i32,
    lines: &mut Vec<String>,
) {
    if snapshot.recently_rendered.is_empty() {
        lines.push("Nothing rendered yet".to_owned());
        return;
    }
    let shown = snapshot.recently_rendered.len().min(RECENT_LINES);
    lines.push(format!(
        "Recently rendered (newest {shown} of {})",
        snapshot
            .recently_rendered_total
            .max(snapshot.recently_rendered.len() as u32)
    ));
    for video in snapshot.recently_rendered.iter().take(RECENT_LINES) {
        lines.push(format!(
            "  {} · {} · {}",
            date_time(video.rendered_at_ms, utc_offset_seconds),
            style.paint(CYAN, &relative_text(now_ms, video.rendered_at_ms)),
            video.output_filename
        ));
        let mut detail = format!(
            "{} · {} · {} fps · {} frames · {} · {}",
            video.codec,
            video.resolution_label,
            video.frame_rate,
            grouped(u64::from(video.frame_count)),
            duration_text(video.duration_seconds),
            size_text(video.size_bytes),
        );
        if !video.file_exists {
            detail.push_str(" · moved or deleted");
        }
        lines.push(format!("      {detail}"));
        lines.push(format!("      {}", video.export_path));
    }
}

fn phase_label(phase: pb::Phase) -> &'static str {
    match phase {
        pb::Phase::Starting => "starting",
        pb::Phase::NoConfig => "no config",
        pb::Phase::WaitingForDiskAccess => "waiting for disk access",
        pb::Phase::Exporting => "exporting",
        pb::Phase::Rendering => "rendering",
        pb::Phase::Paused => "paused",
        _ => "idle",
    }
}

fn stage_label(stage: pb::Stage) -> &'static str {
    match stage {
        pb::Stage::Exporting => "exporting",
        pb::Stage::CountMismatch => "count mismatch",
        pb::Stage::CollectingMetadata => "collecting metadata",
        pb::Stage::Queued => "queued",
        pb::Stage::Rendering => "rendering",
        pb::Stage::Blocked => "blocked",
        pb::Stage::Paused => "paused",
        pb::Stage::Unspecified => "unknown",
    }
}
