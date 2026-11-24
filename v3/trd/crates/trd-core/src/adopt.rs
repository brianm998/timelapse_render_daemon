// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Adoption of renders left running by an earlier daemon (§6.11).
//!
//! [`readopt`] takes back the renders the database records as running or suspended. One whose
//! process is still there, with the recorded start time, is adopted with its progress; one that has
//! ended is finished as a success when its progress file ends and its partial file is not empty, and
//! recorded as `FAILURE_PRODUCED_NO_OUTPUT` otherwise. [`adopt_foreign`] watches the ffmpeg processes
//! this daemon did not start, found by their command line (the Swift `OrphanScanner`).
//!
//! Nothing here signals a process: only an explicit control does that (§6.11 step 3). The process
//! list comes from a [`ProcessTable`], so tests pass a [`FakeProcessTable`](crate::process::FakeProcessTable).

use std::fs;
use std::path::Path;

use uuid::Uuid;

use crate::diagnosis::FailureKind;
use crate::exif::Histogram;
use crate::pipeline::{self, job_failure};
use crate::plan::Plan;
use crate::process::{ProcessEntry, ProcessTable, RunningProcess};
use crate::progress::ProgressReader;
use crate::queue::{Active, Ctx, Renders, in_flight_jobs, note};
use crate::store::{JobStatus, LogFields, LogLevel, RenderJob, StoreResult};

/// How far the recorded start time of a process may differ from the table's (§6.11 step 1).
pub const START_TOLERANCE_SECS: i64 = 2;

/// The extensions an ffmpeg output may have to be a foreign render (§6.11 step 2).
const VIDEO_EXTENSIONS: [&str; 5] = [".mov", ".mp4", ".mkv", ".avi", ".m4v"];

/// What [`readopt`] did with the renders the database records as running or suspended.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReadoptReport {
    /// Renders whose process is still running, now held by the queue or the foreign watch.
    pub adopted: usize,
    /// Renders whose process ended while the daemon was down and that succeeded.
    pub finished: usize,
    /// Renders that ended while the daemon was down and failed, or whose plan is unreadable.
    pub failed: usize,
}

/// The outcome of one re-adopted or finished row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readopted {
    Adopted,
    Finished,
    Failed,
}

/// An ffmpeg command line that matches the shape of a render (§6.11 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignCommand {
    /// The `-i` argument, a pattern with `%` (image2 demuxer).
    pub input_pattern: String,
    /// The output, the trailing argument.
    pub output_path: String,
}

/// Reads an ffmpeg process's arguments: the executable is `ffmpeg`, the argument after `-i` is a
/// pattern with `%`, and the last argument is a video output that is not a `.partial.mov` file.
/// Returns `None` for any other process (`OrphanScanner.parse`, Swift).
pub fn parse_foreign(entry: &ProcessEntry) -> Option<ForeignCommand> {
    if entry.name != "ffmpeg" {
        return None;
    }
    let at = entry.args.iter().position(|arg| arg == "-i")?;
    let pattern = entry.args.get(at + 1)?;
    if !pattern.contains('%') {
        return None;
    }
    let output = entry.args.last()?;
    let is_video = VIDEO_EXTENSIONS.iter().any(|ext| output.ends_with(ext));
    if !is_video || output.ends_with(".partial.mov") {
        return None;
    }
    Some(ForeignCommand {
        input_pattern: pattern.clone(),
        output_path: output.clone(),
    })
}

/// Re-adopts the renders the database records as running or suspended (§6.11 step 1). Each row is
/// one of: adopted (its process is alive with the recorded start time and its command line contains
/// the partial path), finished (its process ended, the progress file ends with `progress=end` and the
/// partial file is not empty), or failed with `FAILURE_PRODUCED_NO_OUTPUT`. A row of a foreign render
/// that is still alive is watched; one that has ended is decided by its output (§6.11 step 2).
pub fn readopt(
    ctx: &mut Ctx<'_>,
    renders: &mut Renders,
    table: &mut dyn ProcessTable,
) -> StoreResult<ReadoptReport> {
    let entries = table.list();
    let mut jobs = ctx.store.render_jobs(Some(JobStatus::Running))?;
    jobs.extend(ctx.store.render_jobs(Some(JobStatus::Suspended))?);
    let mut report = ReadoptReport::default();
    for job in jobs {
        let live = find_live(&job, &entries).cloned();
        let outcome = match (job.foreign, live) {
            (true, Some(entry)) => {
                renders.foreign.insert(
                    job.id.clone(),
                    RunningProcess::adopted(entry.pid, entry.start_time),
                );
                Readopted::Adopted
            }
            (true, None) => {
                if renders.finish_foreign(ctx, job)? {
                    Readopted::Finished
                } else {
                    Readopted::Failed
                }
            }
            (false, Some(entry)) => adopt_own(ctx, renders, job, &entry)?,
            (false, None) => finish_gone(ctx, renders, job)?,
        };
        match outcome {
            Readopted::Adopted => report.adopted += 1,
            Readopted::Finished => report.finished += 1,
            Readopted::Failed => report.failed += 1,
        }
    }
    Ok(report)
}

/// Watches the ffmpeg processes of the table this daemon did not start (§6.11 step 2). Each one that
/// matches [`parse_foreign`], is not a `.partial.mov` and whose output no queued, running or suspended
/// job owns becomes a job with `adopted` and `foreign` set. It takes no slot of the render caps and
/// is never signalled. Returns how many were adopted.
pub fn adopt_foreign(
    ctx: &mut Ctx<'_>,
    renders: &mut Renders,
    table: &mut dyn ProcessTable,
) -> StoreResult<usize> {
    let entries = table.list();
    let mut owned: Vec<String> = in_flight_jobs(ctx.store)?
        .into_iter()
        .map(|job| job.output_path)
        .collect();
    let mut adopted = 0;
    for entry in &entries {
        let Some(command) = parse_foreign(entry) else {
            continue;
        };
        let already_held = renders.running.values().any(|a| a.process.pid == entry.pid)
            || renders.foreign.values().any(|p| p.pid == entry.pid);
        if already_held || owned.contains(&command.output_path) {
            continue;
        }
        let Some(input_dir) = Path::new(&command.input_pattern).parent() else {
            continue;
        };
        let Some(sequence_name) = input_dir.file_name() else {
            continue;
        };
        let sequence_name = sequence_name.to_string_lossy().into_owned();
        let export_path = input_dir
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let output_filename = Path::new(&command.output_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let job = RenderJob {
            id: Uuid::new_v4().to_string(),
            sequence_name: sequence_name.clone(),
            export_path,
            variant_key: String::new(),
            output_filename: output_filename.clone(),
            output_path: command.output_path.clone(),
            // A foreign render has no partial file of trd's; the output stands in for the column.
            partial_path: command.output_path.clone(),
            plan_json: "{}".to_owned(),
            status: JobStatus::Running,
            suspended_by: None,
            pid: Some(i64::from(entry.pid)),
            pid_start_time: Some(entry.start_time as i64),
            queued_at: ctx.now_ms,
            started_at: Some(ctx.now_ms),
            finished_at: None,
            progress_path: None,
            log_path: None,
            adopted: true,
            foreign: true,
        };
        ctx.store.put_render_job(&job)?;
        renders.foreign.insert(
            job.id.clone(),
            RunningProcess::adopted(entry.pid, entry.start_time),
        );
        owned.push(command.output_path);
        let fields = LogFields {
            sequence_name: Some(sequence_name.clone()),
            output_filename: Some(output_filename.clone()),
            ..LogFields::default()
        };
        let message = format!(
            "adopted foreign ffmpeg render of {output_filename} (pid {})",
            entry.pid
        );
        note(ctx, LogLevel::Info, &sequence_name, &message, fields)?;
        adopted += 1;
    }
    Ok(adopted)
}

/// The table entry that is the process of `job`: same pid, a start time within
/// [`START_TOLERANCE_SECS`] of the recorded one, and a command line that is this job's render (its
/// partial path) or, for a foreign row, a foreign ffmpeg. A recycled pid has another start time.
fn find_live<'a>(job: &RenderJob, entries: &'a [ProcessEntry]) -> Option<&'a ProcessEntry> {
    let pid = u32::try_from(job.pid?).ok()?;
    let recorded = job.pid_start_time?;
    entries.iter().find(|entry| {
        entry.pid == pid
            && (entry.start_time as i64 - recorded).abs() <= START_TOLERANCE_SECS
            && if job.foreign {
                parse_foreign(entry).is_some()
            } else {
                entry.args.iter().any(|arg| arg.contains(&job.partial_path))
            }
    })
}

/// Adopts a render whose process is still running: the queue holds it with its progress, its
/// status unchanged (§6.11 step 1).
fn adopt_own(
    ctx: &mut Ctx<'_>,
    renders: &mut Renders,
    mut job: RenderJob,
    entry: &ProcessEntry,
) -> StoreResult<Readopted> {
    let plan: Plan = match serde_json::from_str(&job.plan_json) {
        Ok(plan) => plan,
        Err(err) => {
            let summary = format!("the stored plan of the render is unreadable: {err}");
            pipeline::fail_job(ctx, &mut job, FailureKind::LaunchFailed, summary)?;
            return Ok(Readopted::Failed);
        }
    };
    let mut progress = ProgressReader::new(plan.progress_path.clone());
    // Progress is best effort, as in the poll: a missing file leaves no block.
    let _ = progress.poll();
    let fields = LogFields {
        sequence_name: Some(job.sequence_name.clone()),
        output_filename: Some(job.output_filename.clone()),
        ..LogFields::default()
    };
    let message = format!("re-adopted {} (pid {})", job.output_filename, entry.pid);
    note(ctx, LogLevel::Info, &job.sequence_name, &message, fields)?;
    let histogram = renders
        .histograms
        .get(&job.sequence_name)
        .cloned()
        .unwrap_or_default();
    renders.running.insert(
        job.id.clone(),
        Active {
            sequence_name: job.sequence_name.clone(),
            plan,
            process: RunningProcess::adopted(entry.pid, entry.start_time),
            progress,
            started_ms: job.started_at.unwrap_or(ctx.now_ms),
            histogram,
        },
    );
    Ok(Readopted::Adopted)
}

/// Ends a row whose process is gone (§6.11 step 1): finished as a success when the progress file
/// ends with `progress=end` and the partial file is not empty, else failed with
/// `FAILURE_PRODUCED_NO_OUTPUT` and the log's tail.
fn finish_gone(
    ctx: &mut Ctx<'_>,
    renders: &mut Renders,
    mut job: RenderJob,
) -> StoreResult<Readopted> {
    let plan = serde_json::from_str::<Plan>(&job.plan_json).ok();
    let partial_bytes = fs::metadata(&job.partial_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let ended = job.progress_path.as_deref().is_some_and(progress_ended);
    if let Some(plan) = plan.filter(|_| ended && partial_bytes > 0) {
        let started = job.started_at.unwrap_or(ctx.now_ms);
        renders.finish_success(ctx, job, &plan, &Histogram::default(), started)?;
        return Ok(Readopted::Finished);
    }
    let tail = pipeline::read_tail(job.log_path.as_deref(), pipeline::LOG_TAIL_LINES);
    let summary = format!(
        "the render of {} ended while trd was not running and produced no output",
        job.output_filename
    );
    let mut failure = job_failure(
        &job,
        ctx.now_ms,
        FailureKind::ProducedNoOutput.code(),
        summary,
        String::new(),
    );
    failure.tool_output_json = pipeline::tool_output_json(&tail);
    pipeline::record_failure(ctx, failure)?;
    pipeline::remove_file_quietly(Path::new(&job.partial_path));
    pipeline::finish_job(ctx, &mut job, JobStatus::Failed)?;
    Ok(Readopted::Failed)
}

/// Whether the last complete block of a progress file says `progress=end`.
fn progress_ended(path: &str) -> bool {
    let mut reader = ProgressReader::new(path);
    let _ = reader.poll();
    reader.latest().is_some_and(|block| block.ended)
}
