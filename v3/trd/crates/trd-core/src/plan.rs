// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Render planning: variants, output names and ffmpeg arguments (§6.5).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::aspect::aspect_string;
use crate::codecs::{VideoCodec, VideoCodecs};
use crate::config::{Config, RenderType};
use crate::paths::expand_year_path;
use crate::pattern;
use crate::tools::Tools;

/// The failure kind for a sequence that cannot be queued (§6.5, §6.9).
pub const FAILURE_NOT_QUEUEABLE: &str = "FAILURE_NOT_QUEUEABLE";

/// The `-stats_period` ffmpeg is given so the `-progress` file is refreshed every half second.
pub const PROGRESS_STATS_PERIOD: &str = "0.5";

/// The state of a render job, with the `render_jobs.status` labels of §6.13. Defined here so the
/// queue (TRD-12) can reuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobStatus {
    Queued,
    Running,
    Suspended,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// The stored label, as in `render_jobs.status`.
    pub fn label(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Suspended => "suspended",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    /// Whether a job in this state still owns its output: queued, running or suspended.
    pub fn is_in_flight(self) -> bool {
        matches!(
            self,
            JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
        )
    }
}

/// A render job as [`already_rendered`] needs it: its state and its output path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub status: JobStatus,
    pub output_path: String,
}

/// Why a render cannot be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The sequence cannot be rendered. `summary` is the one-line reason; `detail` the longer text.
    NotQueueable { summary: String, detail: String },
    /// The render names a codec that is not in the codec table.
    UnknownCodec { codec: String },
}

impl PlanError {
    /// The failure kind (§6.9) this error is recorded under.
    pub fn failure_kind(&self) -> &'static str {
        FAILURE_NOT_QUEUEABLE
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::NotQueueable { summary, .. } => f.write_str(summary),
            PlanError::UnknownCodec { codec } => write!(f, "unknown codec {codec}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// The output file name of one render of `sequence_dir` (§6.5). Delegates to
/// [`Config::output_filename`].
pub fn output_filename(
    config: &Config,
    codecs: &VideoCodecs,
    render: &RenderType,
    sequence_dir: &str,
) -> String {
    config.output_filename(codecs, render, sequence_dir)
}

/// Whether the output `filename` already exists in an expanded `master_videos` or `export_paths`
/// directory (`year` expands `$YEAR` entries), or a queued, running or suspended job owns
/// `export_path/filename`.
pub fn already_rendered(
    config: &Config,
    export_path: &str,
    filename: &str,
    in_flight: &[Job],
    year: i32,
) -> bool {
    let entries = config
        .master_videos
        .iter()
        .chain(config.export_paths.iter());
    for entry in entries {
        for dir in expand_year_path(entry, year) {
            if Path::new(&dir).join(filename).exists() {
                return true;
            }
        }
    }
    let target = format!("{export_path}/{filename}");
    in_flight
        .iter()
        .any(|job| job.status.is_in_flight() && job.output_path == target)
}

/// The width and height of the first video stream of `example_frame`, read with ffprobe through
/// `tools`. Any failure, or a missing or non-positive size, is `NotQueueable`.
pub fn probe_size(tools: &Tools, example_frame: &Path) -> Result<(i64, i64), PlanError> {
    let not_read = |detail: String| PlanError::NotQueueable {
        summary: format!(
            "could not read the frame size of {}",
            example_frame.display()
        ),
        detail,
    };
    let Some(command) = &tools.ffprobe.command else {
        return Err(not_read("ffprobe is not available".to_owned()));
    };
    let output = command
        .command()
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(example_frame)
        .output()
        .map_err(|err| not_read(format!("cannot run ffprobe: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(not_read(format!(
            "ffprobe exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse_probe_output(&stdout) {
        Some(size) => Ok(size),
        None => Err(not_read(
            "ffprobe printed no positive width and height".to_owned(),
        )),
    }
}

/// Reads the `width=` and `height=` lines of ffprobe's `default=noprint_wrappers=1` output.
pub fn parse_probe_output(text: &str) -> Option<(i64, i64)> {
    let mut width = None;
    let mut height = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("width=") {
            width = value.trim().parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("height=") {
            height = value.trim().parse::<i64>().ok();
        }
    }
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    }
}

/// The size a render is scaled to (§6.5). `[W, H]` is used as given; `[W]` keeps the source's
/// aspect ratio (`W × round(W·srcH/srcW)`, one more when that is odd); no width gives the source.
pub fn final_size(width: Option<i64>, height: Option<i64>, source: (i64, i64)) -> (i64, i64) {
    match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let (src_w, src_h) = source;
            let h = (w as f64 * src_h as f64 / src_w as f64).round() as i64;
            (w, if h % 2 == 0 { h } else { h + 1 })
        }
        _ => source,
    }
}

/// Everything ffmpeg is given for one render (§6.5). Serialised as the `plan_json` of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The sequence directory, `<export path>/<dir>`.
    pub input_dir: String,
    /// The detected prefix before the frame number.
    pub image_name_prefix: String,
    /// The detected suffix after the frame number, e.g. `.tiff`.
    pub image_suffix: String,
    /// The zero-padded width of the frame number.
    pub frame_number_digits: usize,
    /// `-start_number`, given only when the sequence starts past the demuxer's probe range.
    pub start_number: Option<i64>,
    /// `<export path>/<output file name>`.
    pub output_path: String,
    /// The `-progress` file, `<data>/renders/<render-id>.progress`.
    pub progress_path: String,
    /// `<export path>/.<output stem>.partial.mov`, which ffmpeg writes before the rename.
    pub partial_path: String,
    pub frame_rate: i64,
    /// The `-aspect` value.
    pub aspect: String,
    /// The `-filter_complex` scale, when a resolution is configured.
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub codec: VideoCodec,
    /// `-metadata` tags, sorted by tag.
    pub metadata: BTreeMap<String, String>,
    pub total_frames: i64,
}

impl Plan {
    /// `<prefix>%0<digits>d<suffix>`, the file-name part of the `-i` argument.
    pub fn input_pattern_description(&self) -> String {
        format!(
            "{}%0{}d{}",
            self.image_name_prefix, self.frame_number_digits, self.image_suffix
        )
    }

    /// The `-i` argument: `<input dir>/<prefix>%0<digits>d<suffix>`.
    pub fn input_pattern(&self) -> String {
        format!("{}/{}", self.input_dir, self.input_pattern_description())
    }

    /// The ffmpeg argument vector, in the order of §6.5. Never run through a shell.
    pub fn argv(&self) -> Vec<String> {
        let progress = self.progress_path.as_str();
        let frame_rate = self.frame_rate.to_string();
        let input = self.input_pattern();
        let codec = &self.codec.ffmpeg;
        let mut args = words(&[
            "-nostdin",
            "-hide_banner",
            "-nostats",
            "-y",
            "-progress",
            progress,
            "-stats_period",
            PROGRESS_STATS_PERIOD,
            "-r",
            frame_rate.as_str(),
        ]);
        if let Some(start) = self.start_number {
            args.extend(words(&["-start_number", start.to_string().as_str()]));
        }
        args.extend(words(&[
            "-i",
            input.as_str(),
            "-aspect",
            self.aspect.as_str(),
        ]));
        if let (Some(w), Some(h)) = (self.width, self.height) {
            let filter = format!(
                "crop=floor(iw/2)*2:floor(ih/2)*2,scale={w}:{h}:in_range=full:out_range=full,setsar=1/1"
            );
            args.extend(words(&["-filter_complex", filter.as_str()]));
        }
        args.extend(words(&[
            "-c:v",
            codec.codec.as_str(),
            "-pix_fmt",
            codec.pixel_format.as_str(),
            "-threads",
            "0",
        ]));
        if let Some(profile) = &codec.profile {
            args.extend(words(&["-profile:v", profile.as_str()]));
        }
        args.extend(codec.extra_args.iter().flatten().cloned());
        args.extend(words(&[
            "-movflags",
            "+write_colr",
            "-an",
            "-color_range",
            codec.color_range.as_str(),
            "-color_primaries",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_trc",
            "bt709",
            "-timecode",
            "00:00:00:00",
        ]));
        for (key, value) in &self.metadata {
            args.push("-metadata".to_owned());
            args.push(format!("{key}={value}"));
        }
        args.push(self.partial_path.clone());
        args
    }
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// What [`plan_render`] needs: the config and tables, the export directory and sequence directory
/// names, the render, where progress goes, and the metadata and frame count of the sequence.
pub struct PlanRequest<'a> {
    pub config: &'a Config,
    pub codecs: &'a VideoCodecs,
    pub tools: &'a Tools,
    /// The export directory the sequence sits in.
    pub export_path: &'a str,
    /// The sequence's directory name inside `export_path`.
    pub sequence_dir: &'a str,
    pub render: &'a RenderType,
    /// The daemon data directory; progress goes to `<data_dir>/renders/<render_id>.progress`.
    pub data_dir: &'a Path,
    pub render_id: &'a str,
    pub metadata: BTreeMap<String, String>,
    pub total_frames: i64,
}

/// Plans one render of a sequence (§6.5): the frame pattern, the size and aspect, and the output
/// paths. Probes the first frame when the render needs the source size.
pub fn plan_render(request: &PlanRequest<'_>) -> Result<Plan, PlanError> {
    let render = request.render;
    let codec = request
        .codecs
        .get(&render.codec)
        .ok_or_else(|| PlanError::UnknownCodec {
            codec: render.codec.clone(),
        })?;
    let filename = output_filename(request.config, request.codecs, render, request.sequence_dir);
    let dir = Path::new(request.export_path).join(request.sequence_dir);
    let pattern = pattern::detect(
        &dir,
        &request.config.image_extensions,
        &request.config.image_name_prefix,
    )
    .ok_or_else(|| PlanError::NotQueueable {
        summary: format!("no numbered image sequence in {}", dir.display()),
        detail: format!(
            "input analysis (TRD-11): directory {}, expected prefix {:?}, digit width {}, \
             extensions {:?}",
            dir.display(),
            request.config.image_name_prefix,
            pattern::LEGACY_FRAME_NUMBER_DIGITS,
            request.config.image_extensions
        ),
    })?;

    let (size, resize) = match (render.width(), render.height()) {
        (Some(w), Some(h)) => ((w, h), Some((w, h))),
        (Some(_), None) => {
            let source = probe_size(request.tools, &dir.join(&pattern.example_filename))?;
            let fitted = final_size(render.width(), None, source);
            (fitted, Some(fitted))
        }
        _ => {
            let source = probe_size(request.tools, &dir.join(&pattern.example_filename))?;
            (source, None)
        }
    };
    let (width, height) = match resize {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };

    let stem = filename.strip_suffix(".mov").unwrap_or(&filename);
    let progress = request
        .data_dir
        .join("renders")
        .join(format!("{}.progress", request.render_id));
    Ok(Plan {
        input_dir: format!("{}/{}", request.export_path, request.sequence_dir),
        image_name_prefix: pattern.prefix.clone(),
        image_suffix: pattern.suffix.clone(),
        frame_number_digits: pattern.digits,
        start_number: pattern
            .requires_explicit_start_number()
            .then_some(pattern.first_index),
        output_path: format!("{}/{}", request.export_path, filename),
        progress_path: progress.to_string_lossy().into_owned(),
        partial_path: format!("{}/.{stem}.partial.mov", request.export_path),
        frame_rate: render.frame_rate,
        aspect: aspect_string(size.0, size.1),
        width,
        height,
        codec: codec.clone(),
        metadata: request.metadata.clone(),
        total_frames: request.total_frames,
    })
}
