// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Thumbnails of finished renders (§6.16) and the history retention that deletes them (§6.13).
//!
//! A [`Thumbnailer`] runs at most [`MAX_CONCURRENT_THUMBNAILS`] ffmpeg commands at once. The
//! command itself is injected as a [`Runner`], so tests can observe the concurrency and fail a
//! run without a real ffmpeg; [`Thumbnailer::with_tool`] runs the resolved ffmpeg.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use crate::store::{LogFields, LogLevel, RenderedVideo, Store, StoreResult, log_event};
use crate::tools::ToolCommand;

/// How many thumbnails are made at once (§6.16).
pub const MAX_CONCURRENT_THUMBNAILS: usize = 2;
/// The width of a poster frame; the height keeps the aspect ratio (§6.16).
pub const THUMBNAIL_WIDTH: u32 = 480;
/// The folder under the data directory that holds the thumbnails.
pub const THUMBNAILS_DIR: &str = "thumbnails";

/// Runs ffmpeg with the arguments after the program. `Err` carries the text for the log row.
pub type Runner = Box<dyn Fn(&[String]) -> Result<(), String> + Send + Sync>;

/// A counting semaphore: at most `slots` holders at once. A holder waits while the gate is full.
pub struct SlotGate {
    slots: usize,
    held: Mutex<usize>,
    freed: Condvar,
}

impl SlotGate {
    /// A gate with `slots` slots. Panics when `slots` is zero: nothing could ever run.
    pub fn new(slots: usize) -> SlotGate {
        assert!(slots > 0, "a slot gate needs at least one slot");
        SlotGate {
            slots,
            held: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// Waits until a slot is free, then takes it. The slot is returned when the guard drops.
    pub fn acquire(&self) -> SlotGuard<'_> {
        let mut held = self.lock();
        while *held >= self.slots {
            held = self
                .freed
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *held += 1;
        SlotGuard { gate: self }
    }

    /// How many slots are taken now.
    pub fn in_use(&self) -> usize {
        *self.lock()
    }

    fn lock(&self) -> MutexGuard<'_, usize> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A slot taken from a [`SlotGate`]; dropping it frees the slot.
pub struct SlotGuard<'a> {
    gate: &'a SlotGate,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        *self.gate.lock() -= 1;
        self.gate.freed.notify_one();
    }
}

/// The arguments of the §6.16 command after `ffmpeg`: a poster frame at the middle of a file of
/// `duration_seconds`, scaled to [`THUMBNAIL_WIDTH`] wide, written to `output` (last).
///
/// A deliberate deviation from §6.16: when the duration is unknown (0, as imported rows have, since
/// the Swift history carries none and no ffprobe is run for it) the midpoint is 0, so the poster
/// frame is the first frame. Recorded in §6.16 of the spec.
pub fn thumbnail_args(input: &Path, duration_seconds: f64, output: &Path) -> Vec<String> {
    let midpoint = if duration_seconds.is_finite() && duration_seconds > 0.0 {
        duration_seconds / 2.0
    } else {
        0.0
    };
    let mut args: Vec<String> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.push(format!("{midpoint:.3}"));
    args.push("-i".to_owned());
    args.push(input.to_string_lossy().into_owned());
    args.push("-frames:v".to_owned());
    args.push("1".to_owned());
    args.push("-vf".to_owned());
    args.push(format!("scale={THUMBNAIL_WIDTH}:-2"));
    args.push("-q:v".to_owned());
    args.push("3".to_owned());
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Runs `tool` with `args`. A non-zero exit carries the status and the stderr text.
pub fn run_ffmpeg(tool: &ToolCommand, args: &[String]) -> Result<(), String> {
    let output = tool
        .command()
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("cannot start ffmpeg: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(match output.status.code() {
        Some(code) => format!("ffmpeg exited {code}: {stderr}"),
        None => format!("ffmpeg was killed: {stderr}"),
    })
}

/// Makes poster frames into `<data>/thumbnails/<video id>.jpg`, two at a time.
pub struct Thumbnailer {
    dir: PathBuf,
    gate: SlotGate,
    runner: Runner,
}

impl Thumbnailer {
    /// A thumbnailer for the data directory `data_dir` that runs ffmpeg through `runner`.
    pub fn new(data_dir: &Path, runner: Runner) -> Thumbnailer {
        Thumbnailer {
            dir: data_dir.join(THUMBNAILS_DIR),
            gate: SlotGate::new(MAX_CONCURRENT_THUMBNAILS),
            runner,
        }
    }

    /// A thumbnailer that runs the resolved ffmpeg `ffmpeg`.
    pub fn with_tool(data_dir: &Path, ffmpeg: ToolCommand) -> Thumbnailer {
        Thumbnailer::new(data_dir, Box::new(move |args| run_ffmpeg(&ffmpeg, args)))
    }

    /// The folder the thumbnails are written to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the thumbnail of the video `video_id` is written.
    pub fn path_for(&self, video_id: &str) -> PathBuf {
        self.dir.join(format!("{video_id}.jpg"))
    }

    /// Makes the poster frame of `video`, a finished render whose file is at `output_path`, and
    /// stores its path in the row. Returns `Ok(true)` when the thumbnail was written.
    ///
    /// A failing ffmpeg, or one that writes no file, leaves the row without a thumbnail, removes
    /// any partial file, and logs a WARN row. Only store errors are returned as errors.
    pub fn make(&self, store: &Store, video: &RenderedVideo, now_ms: i64) -> StoreResult<bool> {
        let output = self.path_for(&video.id);
        let outcome = match fs::create_dir_all(&self.dir) {
            Err(err) => Err(format!("cannot create {}: {err}", self.dir.display())),
            Ok(()) => {
                let args = thumbnail_args(
                    Path::new(&video.output_path),
                    video.duration_seconds,
                    &output,
                );
                let _slot = self.gate.acquire();
                match (self.runner)(&args) {
                    Ok(()) if output.is_file() => Ok(()),
                    Ok(()) => Err(format!("ffmpeg wrote no file at {}", output.display())),
                    Err(message) => Err(message),
                }
            }
        };
        match outcome {
            Ok(()) => {
                let mut row = video.clone();
                row.thumbnail_path = Some(output.to_string_lossy().into_owned());
                store.put_rendered_video(&row)?;
                Ok(true)
            }
            Err(message) => {
                remove_if_exists(&output);
                let text = format!("thumbnail of {} failed: {message}", video.output_filename);
                let fields = LogFields {
                    event: Some("thumbnail_failed".to_owned()),
                    sequence_name: Some(video.sequence_name.clone()),
                    output_filename: Some(video.output_filename.clone()),
                    rendered_video_id: Some(video.id.clone()),
                    ..LogFields::default()
                };
                log_event(store, now_ms, LogLevel::Warn, "thumbnails", &text, &fields)?;
                Ok(false)
            }
        }
    }
}

/// Makes the thumbnail of the rendered video `video_id` on demand (§6.16): when its row has none
/// and its file exists. Returns the thumbnail's path when one exists afterwards, and `None` when
/// the row is missing or its file is not there.
pub fn ensure_thumbnail(
    store: &Store,
    thumbnailer: &Thumbnailer,
    video_id: &str,
    now_ms: i64,
) -> StoreResult<Option<PathBuf>> {
    let Some(video) = store.rendered_video(video_id)? else {
        return Ok(None);
    };
    if let Some(path) = video
        .thumbnail_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Ok(Some(path));
    }
    if !Path::new(&video.output_path).is_file() {
        return Ok(None);
    }
    if thumbnailer.make(store, &video, now_ms)? {
        Ok(Some(thumbnailer.path_for(video_id)))
    } else {
        Ok(None)
    }
}

/// Applies the history retention (§6.13) and deletes the thumbnail files of the rows it removes.
/// A thumbnail that is already gone is not an error; any other failure to delete one is logged
/// as a WARN row and the rest are still deleted. Returns how many thumbnail files were deleted.
pub fn prune_history(store: &Store, now_ms: i64, history_days: i64) -> StoreResult<usize> {
    let mut deleted = 0;
    for path in store.prune(now_ms, history_days)? {
        match fs::remove_file(&path) {
            Ok(()) => deleted += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                let text = format!("cannot delete thumbnail {path}: {err}");
                let fields = LogFields {
                    event: Some("thumbnail_delete_failed".to_owned()),
                    ..LogFields::default()
                };
                log_event(store, now_ms, LogLevel::Warn, "thumbnails", &text, &fields)?;
            }
        }
    }
    Ok(deleted)
}

/// Deletes `path` when it exists. Failing to delete a partial file is not worth more than this:
/// the thumbnail is already recorded as missing.
fn remove_if_exists(path: &Path) {
    let _ = fs::remove_file(path);
}
