// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Failure diagnosis from render logs (§6.9).
//!
//! Ported from the Swift `RenderFailure.swift` and `RenderFailureDiagnosis.swift`. The message
//! texts are the Swift ones, verbatim: the owner reads them in the console, the log and the GUI.
//! ffmpeg's own output is mostly banner noise, so [`significant_lines`] strips it, [`diagnose`]
//! recognises the failure signatures, and for the common "no frames matched" case
//! [`analyze_input_sequence`] goes back to the input directory to say how the files on disk differ
//! from the pattern the daemon asked for. Storing failures is TRD-12's job, not this module's.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::pattern::{self, IMAGE_DEMUXER_PROBE_RANGE};
use crate::plan::{FAILURE_NOT_QUEUEABLE, Plan};

pub use crate::pattern::{Decomposed, decompose};

/// The kind of a failed render. The labels are the Swift `RenderFailure.Kind` labels; the codes
/// are the `FAILURE_*` names of `v3/proto/trd/v1/trd.proto`, which is what is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// ffmpeg's image2 demuxer matched none of the files in the input dir.
    InputFramesNotFound,
    /// The input directory could not be listed or opened.
    InputUnreadable,
    /// A source frame is present but not decodable.
    SourceFrameUnreadable,
    /// ffmpeg rejected the command line (unknown encoder, bad option…).
    FfmpegRejectedArguments,
    /// The output path could not be written.
    OutputNotWritable,
    DiskFull,
    /// ffmpeg was killed by a signal rather than exiting on its own.
    TerminatedBySignal,
    /// ffmpeg exited 0 but there is no output file on disk.
    ProducedNoOutput,
    /// The render completed but the container is structurally broken.
    StructuralCorruption,
    /// ffmpeg could not be launched at all.
    LaunchFailed,
    /// The variant could not even be queued (no images in the sequence…).
    NotQueueable,
    /// No signature matched.
    Unknown,
}

impl FailureKind {
    /// Short label for the GUI badge and the console header.
    pub fn label(self) -> &'static str {
        match self {
            FailureKind::InputFramesNotFound => "no input frames",
            FailureKind::InputUnreadable => "input unreadable",
            FailureKind::SourceFrameUnreadable => "bad source frame",
            FailureKind::FfmpegRejectedArguments => "ffmpeg rejected arguments",
            FailureKind::OutputNotWritable => "output not writable",
            FailureKind::DiskFull => "disk full",
            FailureKind::TerminatedBySignal => "killed",
            FailureKind::ProducedNoOutput => "no output produced",
            FailureKind::StructuralCorruption => "corrupt output",
            FailureKind::LaunchFailed => "ffmpeg would not start",
            FailureKind::NotQueueable => "cannot start render",
            FailureKind::Unknown => "render failed",
        }
    }

    /// The stored `FAILURE_*` code, as in `trd.proto`.
    pub fn code(self) -> &'static str {
        match self {
            FailureKind::InputFramesNotFound => "FAILURE_INPUT_FRAMES_NOT_FOUND",
            FailureKind::InputUnreadable => "FAILURE_INPUT_UNREADABLE",
            FailureKind::SourceFrameUnreadable => "FAILURE_SOURCE_FRAME_UNREADABLE",
            FailureKind::FfmpegRejectedArguments => "FAILURE_FFMPEG_REJECTED_ARGUMENTS",
            FailureKind::OutputNotWritable => "FAILURE_OUTPUT_NOT_WRITABLE",
            FailureKind::DiskFull => "FAILURE_DISK_FULL",
            FailureKind::TerminatedBySignal => "FAILURE_TERMINATED_BY_SIGNAL",
            FailureKind::ProducedNoOutput => "FAILURE_PRODUCED_NO_OUTPUT",
            FailureKind::StructuralCorruption => "FAILURE_STRUCTURAL_CORRUPTION",
            FailureKind::LaunchFailed => "FAILURE_LAUNCH_FAILED",
            FailureKind::NotQueueable => FAILURE_NOT_QUEUEABLE,
            FailureKind::Unknown => "FAILURE_UNKNOWN",
        }
    }
}

/// A render that did not produce a usable video, with a human-readable explanation of why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFailure {
    /// When this failure was last seen, in Unix seconds.
    pub timestamp: i64,
    pub sequence_name: String,
    pub export_path: String,
    pub output_filename: String,
    pub output_path: String,
    pub variant_key: String,
    pub kind: FailureKind,
    /// One line: what went wrong.
    pub summary: String,
    /// Optional multi-line elaboration: what was expected, what was found, and what to change.
    pub detail: Option<String>,
    pub exit_code: Option<i32>,
    /// The ffmpeg command line, so the render can be reproduced by hand.
    pub command_line: Option<String>,
    /// Tool output worth reading; ffmpeg's banner is already stripped by [`significant_lines`].
    pub tool_output: Vec<String>,
    /// How many times this same failure has been seen.
    pub occurrences: u32,
}

impl RenderFailure {
    /// Whether `other` is the same failure: same output path and same kind.
    pub fn same_failure(&self, other: &RenderFailure) -> bool {
        self.output_path == other.output_path && self.kind == other.kind
    }

    /// Compact one-liner for the render job's message and log lines.
    pub fn short_message(&self) -> String {
        let base = format!("{}: {}", self.kind.label(), self.summary);
        if self.occurrences > 1 {
            format!("{base} (x{})", self.occurrences)
        } else {
            base
        }
    }

    /// Multi-line block written to the console (and, indented, to the log file). Deliberately
    /// verbose: it has to answer "why did my render fail?" without the user going digging.
    pub fn console_report(&self) -> String {
        let mut header = format!(
            "RENDER FAILED  {}  —  {}",
            self.output_filename,
            self.kind.label()
        );
        if self.occurrences > 1 {
            header += &format!("  (occurrence {})", self.occurrences);
        }
        let mut out = vec![
            header,
            format!("  sequence: {}", self.sequence_name),
            format!("  output:   {}", self.output_path),
            format!("  variant:  {}", self.variant_key),
        ];
        if let Some(code) = self.exit_code {
            out.push(format!("  ffmpeg exit code: {code}"));
        }
        out.push(format!("  why: {}", self.summary));
        if let Some(detail) = self.detail.as_deref().filter(|d| !d.is_empty()) {
            for line in detail.split('\n') {
                out.push(format!("       {line}"));
            }
        }
        if !self.tool_output.is_empty() {
            out.push("  tool output:".to_owned());
            for line in &self.tool_output {
                out.push(format!("    | {line}"));
            }
        }
        if let Some(command) = &self.command_line {
            out.push(format!("  command: {command}"));
        }
        out.join("\n")
    }

    /// Folds a repeat of the same failure into this one: the counter goes up by one and the newer
    /// record (timestamp, output, detail) replaces this one, as in the Swift daemon.
    pub fn merging(&self, newer: RenderFailure) -> RenderFailure {
        RenderFailure {
            occurrences: self.occurrences.saturating_add(1),
            ..newer
        }
    }
}

// MARK: - Output filtering

/// Drops ffmpeg's startup banner, library-version block and progress lines, leaving the lines
/// that say something.
pub fn significant_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|raw| is_significant(raw))
        .cloned()
        .collect()
}

fn is_significant(raw: &str) -> bool {
    let line = raw.trim_matches(|c: char| c == ' ' || c == '\t');
    if line.is_empty()
        || line.starts_with("ffmpeg version ")
        || line.starts_with("built with ")
        || line.starts_with("configuration:")
        || line.starts_with("Press [q]")
    {
        return false;
    }
    // "libavcodec     62. 28.100 / 62. 28.100"
    if library_banner().is_match(line) {
        return false;
    }
    // Progress lines: how far it got is tracked separately.
    if parse_progress_line(line).is_some() {
        return false;
    }
    true
}

fn library_banner() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^lib[a-z]+\s+\d").expect("valid regex"))
}

/// `frame=   30 fps=0.0 ...`, as (frame, fps). Both captures must parse for the line to count.
/// This is local to diagnosis: progress.rs reads the `-progress` file, not stderr.
fn parse_progress_line(line: &str) -> Option<(i64, f64)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)").expect("valid regex")
    });
    let caps = re.captures(line)?;
    let frame = caps[1].parse::<i64>().ok()?;
    let fps = caps[2].parse::<f64>().ok()?;
    Some((frame, fps))
}

// MARK: - Input sequence analysis

/// What is wrong with the input directory, as found by [`analyze_input_sequence`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    DirectoryMissing,
    DirectoryUnreadable,
    DirectoryEmpty,
    NoImageFiles { sample: Vec<String> },
    FramesNotNumbered { sample: Vec<String> },
    PrefixMismatch { expected: String, found: String },
    DigitWidthMismatch { expected: usize, found: usize },
    SuffixMismatch { expected: String, found: String },
    FirstIndexOutOfProbeRange { first_index: i64 },
    Unexplained,
}

/// The findings for one input directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSequenceReport {
    pub problems: Vec<Problem>,
    pub example_filename: Option<String>,
    pub image_file_count: usize,
}

impl InputSequenceReport {
    fn with_problem(problem: Problem) -> Self {
        InputSequenceReport {
            problems: vec![problem],
            example_filename: None,
            image_file_count: 0,
        }
    }
}

/// Compares what ffmpeg was told to look for against what is in the input directory. The
/// detection is the one the render queue uses ([`pattern::detect`]), so a report cannot describe
/// the directory differently from the pattern that was built from it.
pub fn analyze_input_sequence(
    input_dir: &str,
    expected_prefix: &str,
    expected_digit_width: usize,
    expected_suffix: &str,
    image_extensions: &[String],
) -> InputSequenceReport {
    let dir = Path::new(input_dir);
    if !dir.is_dir() {
        return InputSequenceReport::with_problem(Problem::DirectoryMissing);
    }
    let Some(names) = list_names(dir) else {
        return InputSequenceReport::with_problem(Problem::DirectoryUnreadable);
    };
    let mut visible: Vec<String> = names.into_iter().filter(|n| !n.starts_with('.')).collect();
    visible.sort();
    if visible.is_empty() {
        return InputSequenceReport::with_problem(Problem::DirectoryEmpty);
    }

    let wanted: Vec<String> = image_extensions.iter().map(|e| e.to_lowercase()).collect();
    let images: Vec<String> = visible
        .iter()
        .filter(|name| match name.rfind('.') {
            None => false,
            Some(dot) => {
                let ext = name[dot + 1..].to_lowercase();
                wanted.is_empty() || wanted.contains(&ext)
            }
        })
        .cloned()
        .collect();
    if images.is_empty() {
        return InputSequenceReport::with_problem(Problem::NoImageFiles {
            sample: visible.iter().take(5).cloned().collect(),
        });
    }

    let Some(actual) = pattern::detect(dir, image_extensions, expected_prefix) else {
        return InputSequenceReport {
            problems: vec![Problem::FramesNotNumbered {
                sample: images.iter().take(5).cloned().collect(),
            }],
            example_filename: images.first().cloned(),
            image_file_count: images.len(),
        };
    };

    let mut problems = Vec::new();
    if actual.prefix != expected_prefix {
        problems.push(Problem::PrefixMismatch {
            expected: expected_prefix.to_owned(),
            found: actual.prefix.clone(),
        });
    }
    if actual.digits != expected_digit_width {
        problems.push(Problem::DigitWidthMismatch {
            expected: expected_digit_width,
            found: actual.digits,
        });
    }
    if actual.suffix.to_lowercase() != expected_suffix.to_lowercase() {
        problems.push(Problem::SuffixMismatch {
            expected: expected_suffix.to_owned(),
            found: actual.suffix.clone(),
        });
    }
    if problems.is_empty() {
        // The naming is right, so the only explanation left is a sequence that starts past the
        // few indices ffmpeg probes.
        if actual.requires_explicit_start_number() {
            problems.push(Problem::FirstIndexOutOfProbeRange {
                first_index: actual.first_index,
            });
        } else {
            problems.push(Problem::Unexplained);
        }
    }

    InputSequenceReport {
        problems,
        example_filename: Some(actual.example_filename.clone()),
        image_file_count: images.len(),
    }
}

/// The names in `dir`, or `None` when it cannot be listed (every entry must be readable).
fn list_names(dir: &Path) -> Option<Vec<String>> {
    let entries = fs::read_dir(dir).ok()?;
    entries
        .map(|entry| {
            entry
                .ok()
                .map(|e| e.file_name().to_string_lossy().into_owned())
        })
        .collect()
}

/// Renders an [`InputSequenceReport`] as the multi-line `detail` text of a [`RenderFailure`].
/// `pattern` is the input pattern as ffmpeg was given it, e.g. `LRT_%05d.tiff`.
pub fn describe(report: &InputSequenceReport, input_dir: &str, pattern: &str) -> String {
    let mut lines = vec![
        format!("ffmpeg was told to read {pattern}"),
        format!("in {input_dir}"),
    ];
    if let Some(example) = &report.example_filename {
        lines.push(format!(
            "The directory holds {} image file(s); the first is {example}",
            report.image_file_count
        ));
    }

    for problem in &report.problems {
        lines.push(match problem {
            Problem::DirectoryMissing => {
                "- that directory does not exist (deleted, renamed, or on an \
                 unmounted volume?)"
                    .to_owned()
            }
            Problem::DirectoryUnreadable => {
                "- that directory exists but could not be listed — check \
                 permissions and that the volume is still mounted"
                    .to_owned()
            }
            Problem::DirectoryEmpty => "- that directory is empty".to_owned(),
            Problem::NoImageFiles { sample } => {
                let mut s = "- it holds no files with a configured image extension".to_owned();
                if !sample.is_empty() {
                    s += &format!("; it does hold: {}", sample.join(", "));
                }
                s
            }
            Problem::FramesNotNumbered { sample } => format!(
                "- its image files carry no frame number, so there is no sequence to \
                 render: {}",
                sample.join(", ")
            ),
            Problem::PrefixMismatch { expected, found } => format!(
                "- FILENAME PREFIX: files are named \"{found}…\" but this render was \
                 queued for \"{expected}…\". The daemon detects the prefix from the \
                 directory when it queues a render, so the contents changed in \
                 between, or the directory holds more than one sequence"
            ),
            Problem::DigitWidthMismatch { expected, found } => format!(
                "- FRAME NUMBER WIDTH: files use {found}-digit frame numbers but this \
                 render was queued for {expected} (%0{expected}d). ffmpeg needs an \
                 exact width, and the directory appears to hold mixed numbering"
            ),
            Problem::SuffixMismatch { expected, found } => {
                format!("- SUFFIX: files end in \"{found}\" but the pattern expects \"{expected}\"")
            }
            Problem::FirstIndexOutOfProbeRange { first_index } => format!(
                "- FIRST FRAME NUMBER: the sequence starts at {first_index}, but ffmpeg's \
                 image2 demuxer only probes indices 0-{} when looking for the first \
                 frame, and no -start_number was passed",
                IMAGE_DEMUXER_PROBE_RANGE - 1
            ),
            Problem::Unexplained => {
                "- the daemon could not work out how the files on disk differ from \
                 the pattern; run the command below by hand to see ffmpeg's own view"
                    .to_owned()
            }
        });
    }
    lines.join("\n")
}

// MARK: - Top-level diagnosis

/// The kind, one-line summary and optional detail of a failed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub kind: FailureKind,
    pub summary: String,
    pub detail: Option<String>,
}

/// Works out why an ffmpeg run failed, consulting the input directory when the output points at
/// an input-pattern problem. `output_lines` is ffmpeg's combined output; `output_file_exists` is
/// whether the output file is on disk after the run.
pub fn diagnose(
    plan: &Plan,
    exit_code: Option<i32>,
    terminated_normally: bool,
    output_lines: &[String],
    image_extensions: &[String],
    output_file_exists: bool,
) -> Diagnosis {
    let interesting = significant_lines(output_lines);
    let haystack = interesting.join("\n").to_lowercase();
    let first_line = |needle: &str| -> Option<String> {
        interesting
            .iter()
            .find(|line| line.to_lowercase().contains(needle))
            .cloned()
    };

    // 1. The image2 demuxer found nothing: by far the most common failure.
    if haystack.contains("could find no file or sequence with path")
        || (haystack.contains("error opening input file")
            && haystack.contains("no such file or directory"))
    {
        let report = analyze_input_sequence(
            &plan.input_dir,
            &plan.image_name_prefix,
            plan.frame_number_digits,
            &plan.image_suffix,
            image_extensions,
        );
        let pattern = plan.input_pattern_description();
        let first = report.problems.first();
        let summary = match first {
            Some(Problem::DirectoryMissing) => {
                format!("the input directory is gone: {}", plan.input_dir)
            }
            Some(Problem::DirectoryUnreadable) => {
                format!("the input directory could not be read: {}", plan.input_dir)
            }
            Some(
                Problem::DirectoryEmpty
                | Problem::NoImageFiles { .. }
                | Problem::FramesNotNumbered { .. },
            ) => format!("there are no images to render in {}", plan.input_dir),
            _ => format!("no frames matched {pattern} — the files on disk are named differently"),
        };
        let kind = if matches!(first, Some(Problem::DirectoryUnreadable)) {
            FailureKind::InputUnreadable
        } else {
            FailureKind::InputFramesNotFound
        };
        return Diagnosis {
            kind,
            summary,
            detail: Some(describe(&report, &plan.input_dir, &pattern)),
        };
    }

    // 2. Everything else, most specific signature first.
    if haystack.contains("no space left on device") {
        return Diagnosis {
            kind: FailureKind::DiskFull,
            summary: format!("the volume holding {} is full", plan.output_path),
            detail: Some(
                "Free space on that volume and the variant will be re-queued on the next scan."
                    .to_owned(),
            ),
        };
    }
    if haystack.contains("permission denied") {
        let mentions_output = interesting.iter().any(|line| {
            line.to_lowercase().contains("permission denied") && line.contains(&plan.output_path)
        });
        let (kind, summary) = if mentions_output {
            (
                FailureKind::OutputNotWritable,
                format!("cannot write {} — permission denied", plan.output_path),
            )
        } else {
            (
                FailureKind::InputUnreadable,
                "permission denied reading the source frames".to_owned(),
            )
        };
        return Diagnosis {
            kind,
            summary,
            detail: first_line("permission denied"),
        };
    }
    if haystack.contains("unknown encoder")
        || haystack.contains("unrecognized option")
        || haystack.contains("error splitting the argument list")
        || haystack.contains("option not found")
        || haystack.contains("invalid argument")
    {
        let line = first_line("unknown encoder")
            .or_else(|| first_line("unrecognized option"))
            .or_else(|| first_line("option not found"))
            .or_else(|| first_line("invalid argument"))
            .or_else(|| first_line("error splitting the argument list"));
        return Diagnosis {
            kind: FailureKind::FfmpegRejectedArguments,
            summary: line.unwrap_or_else(|| "ffmpeg rejected the command line".to_owned()),
            detail: Some(
                "The codec entry for this variant produced arguments this build of \
                 ffmpeg does not accept. Check the codec's ffmpeg settings in \
                 video_codecs.json against `ffmpeg -encoders`."
                    .to_owned(),
            ),
        };
    }
    if haystack.contains("invalid data found when processing input")
        || haystack.contains("error while decoding")
        || haystack.contains("could not open file")
    {
        let summary = first_line("invalid data found when processing input")
            .or_else(|| first_line("error while decoding"))
            .unwrap_or_else(|| "a source frame could not be decoded".to_owned());
        return Diagnosis {
            kind: FailureKind::SourceFrameUnreadable,
            summary,
            detail: Some(format!(
                "One of the exported frames in {} is truncated or corrupt. Re-export the sequence.",
                plan.input_dir
            )),
        };
    }

    // 3. No signature matched: fall back to the exit status.
    if !terminated_normally {
        return Diagnosis {
            kind: FailureKind::TerminatedBySignal,
            summary: "ffmpeg was killed before it finished".to_owned()
                + &exit_code
                    .map(|c| format!(" (signal {c})"))
                    .unwrap_or_default(),
            detail: Some(
                "Something outside the daemon stopped the process — the OOM killer, a \
                 manual kill, or the machine going to sleep."
                    .to_owned(),
            ),
        };
    }
    if exit_code == Some(0) && !output_file_exists {
        return Diagnosis {
            kind: FailureKind::ProducedNoOutput,
            summary: format!(
                "ffmpeg reported success but {} does not exist",
                plan.output_path
            ),
            detail: Some(
                "The output volume may have gone away mid-render, or something deleted \
                 the file immediately after it was written."
                    .to_owned(),
            ),
        };
    }
    let code = exit_code.map_or_else(|| "abnormally".to_owned(), |c| c.to_string());
    Diagnosis {
        kind: FailureKind::Unknown,
        summary: interesting.last().cloned().unwrap_or_else(|| {
            format!("ffmpeg exited {code} without a recognisable error message")
        }),
        detail: if interesting.is_empty() {
            Some("ffmpeg produced no output beyond its startup banner.".to_owned())
        } else {
            None
        },
    }
}
