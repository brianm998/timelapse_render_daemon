// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The render pipeline of one job, from start to finish (§6.6).
//!
//! [`Renders::poll_running`] reads the progress of each running job and, once a process has ended,
//! runs steps 4–9 of §6.6: diagnosis on a failure, the XMP step, the rename, the history row, the
//! thumbnail, the events, delete-after-render and the cleanup of the progress file and old logs.
//! [`sweep_stale_partials`] and [`retain_render_logs`] are the two sweeps of §6.6.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use uuid::Uuid;

use crate::config::{Config, PausedVariant, RenderType};
use crate::diagnosis::{FailureKind, diagnose};
use crate::events::{
    EVENT_RENDER_FAILED, EVENT_RENDER_FINISHED, EVENT_SEQUENCE_DELETED, EVENT_SEQUENCE_FINISHED,
};
use crate::exif::Histogram;
use crate::paths::expand_year_path;
use crate::plan::{self, Plan};
use crate::process::ExitInfo;
use crate::queue::{Active, Ctx, Renders, in_flight_jobs, is_paused_variant, note};
use crate::store::{
    Failure, JobStatus, LogFields, LogLevel, RenderJob, RenderedVideo, Store, StoreResult,
};
use crate::xmp::{self, XmpResult};

/// The newest render logs kept; older ones are deleted when a job ends (§6.6 step 9).
pub const RENDER_LOG_LIMIT: usize = 200;
/// How many lines of a render's log are kept as the failure's tool output (§6.6 step 4).
pub const LOG_TAIL_LINES: usize = 200;
/// Consecutive corrupt renders after which the variant is auto-paused (§6.6 step 5).
pub const CORRUPT_PAUSE_STRIKES: i64 = 3;
/// The first occurrences of a failure are logged at ERROR with the full report; after that only
/// every this many occurrences (§6.9).
pub const FAILURE_LOG_EVERY: i64 = 25;
/// The occurrences of a failure logged at ERROR with the full report from the first one.
pub const FULL_REPORT_OCCURRENCES: i64 = 3;

impl Renders {
    /// Reads the progress of every running job and reaps the ones whose process has ended.
    pub fn poll_running(&mut self, ctx: &mut Ctx<'_>) -> StoreResult<()> {
        for id in self.running_ids() {
            let polled = match self.running.get_mut(&id) {
                None => continue,
                Some(active) => {
                    // Progress is best effort: a missing or unreadable file leaves the last block.
                    let _ = active.progress.poll();
                    active.process.try_wait()
                }
            };
            let exit = match polled {
                Ok(Some(exit)) => exit,
                Ok(None) => continue,
                Err(err) => {
                    let message = format!("cannot read the exit status of render {id}: {err}");
                    let sequence = self.running.get(&id).map(|a| a.sequence_name.clone());
                    let subject = sequence.unwrap_or_else(|| "renders".to_owned());
                    note(
                        ctx,
                        LogLevel::Warn,
                        &subject,
                        &message,
                        LogFields::default(),
                    )?;
                    continue;
                }
            };
            if let Some(active) = self.running.remove(&id)
                && let Err(err) = self.finish(ctx, &id, active, exit)
            {
                // The process has ended and its entry is gone: end the row so it no longer owns its
                // output or a slot. The error still goes up.
                release_job(ctx, &id, &err.to_string());
                return Err(err);
            }
        }
        Ok(())
    }

    /// The end of a job (§6.6 steps 4–9).
    fn finish(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: &str,
        active: Active,
        exit: ExitInfo,
    ) -> StoreResult<()> {
        let Some(mut job) = ctx.store.render_job(id)? else {
            return Ok(());
        };
        let Active {
            plan,
            histogram,
            started_ms,
            ..
        } = active;
        let partial = PathBuf::from(&plan.partial_path);
        let partial_bytes = fs::metadata(&partial).ok().map(|meta| meta.len());
        if exit.code == Some(0) && partial_bytes.is_some_and(|bytes| bytes > 0) {
            return self.finish_success(ctx, job, &plan, &histogram, started_ms);
        }

        // Step 4: diagnose the failure from the log's last lines, then drop the partial file.
        let tail = read_tail(job.log_path.as_deref(), LOG_TAIL_LINES);
        let extensions = ctx
            .config
            .config()
            .map(|cfg| cfg.image_extensions.clone())
            .unwrap_or_default();
        let diagnosis = diagnose(
            &plan,
            exit.code,
            exit.code.is_some(),
            &tail,
            &extensions,
            partial_bytes.is_some(),
        );
        remove_file_quietly(&partial);
        let mut failure = job_failure(
            &job,
            ctx.now_ms,
            diagnosis.kind.code(),
            diagnosis.summary,
            diagnosis.detail.unwrap_or_default(),
        );
        failure.exit_code = exit.code.map(i64::from);
        failure.command_line = command_line(&plan);
        failure.tool_output_json = tool_output_json(&tail);
        record_failure(ctx, failure)?;
        finish_job(ctx, &mut job, JobStatus::Failed)
    }

    /// Steps 5–9 for a render whose process exited 0 and wrote its partial file.
    fn finish_success(
        &mut self,
        ctx: &mut Ctx<'_>,
        mut job: RenderJob,
        plan: &Plan,
        histogram: &Histogram,
        started_ms: i64,
    ) -> StoreResult<()> {
        let partial = PathBuf::from(&plan.partial_path);
        let output = PathBuf::from(&plan.output_path);

        // Step 5: XMP. Only a structurally corrupt container ends the render.
        let mut xmp_embedded = false;
        match xmp::embed(&partial, histogram, &ctx.tools.exiftool) {
            Ok(XmpResult::Succeeded) => xmp_embedded = true,
            Ok(XmpResult::Skipped { reason }) => {
                let message = format!("no XMP written into {}: {reason}", job.output_filename);
                note(
                    ctx,
                    LogLevel::Info,
                    &job.sequence_name,
                    &message,
                    LogFields::default(),
                )?;
            }
            Ok(XmpResult::StructuralCorruption {
                reason,
                output: text,
            }) => {
                return self.corrupt_render(ctx, job, plan, reason, &text);
            }
            Ok(XmpResult::ToolError { output: text, code }) => {
                let message = format!(
                    "exiftool exited {code} writing {}; keeping the render: {}",
                    job.output_filename,
                    first_line(&text)
                );
                note(
                    ctx,
                    LogLevel::Warn,
                    &job.sequence_name,
                    &message,
                    LogFields::default(),
                )?;
            }
            Err(err) => {
                let message = format!("exiftool could not run for {}: {err}", job.output_filename);
                note(
                    ctx,
                    LogLevel::Warn,
                    &job.sequence_name,
                    &message,
                    LogFields::default(),
                )?;
            }
        }

        // Step 6: rename. A final name that is taken means the variant is already rendered.
        if output.exists() {
            remove_file_quietly(&partial);
            let message = format!(
                "{} already exists; the partial file of the render is deleted",
                job.output_filename
            );
            note(
                ctx,
                LogLevel::Warn,
                &job.sequence_name,
                &message,
                LogFields::default(),
            )?;
            ctx.store
                .set_corrupt_strikes(&job.sequence_name, &job.variant_key, 0)?;
            ctx.store.delete_failures_for_output(&job.output_path)?;
            finish_job(ctx, &mut job, JobStatus::Done)?;
            return self.after_render(ctx, &job, false);
        }
        if let Err(err) = fs::rename(&partial, &output) {
            let summary = format!(
                "cannot rename {} to {}: {err}",
                partial.display(),
                output.display()
            );
            let failure = job_failure(
                &job,
                ctx.now_ms,
                FailureKind::OutputNotWritable.code(),
                summary,
                String::new(),
            );
            record_failure(ctx, failure)?;
            return finish_job(ctx, &mut job, JobStatus::Failed);
        }

        // Step 7: strikes and failures reset, the history row, the thumbnail and the events.
        let size_bytes = fs::metadata(&output)
            .map(|meta| meta.len() as i64)
            .unwrap_or(0);
        let (width, height) = match (plan.width, plan.height) {
            (Some(w), Some(h)) => (w, h),
            _ => plan::probe_size(ctx.tools, &output).unwrap_or((0, 0)),
        };
        let (codec, resolution_label) = render_identity(ctx.config.config(), &job.variant_key);
        let duration_seconds = if plan.frame_rate > 0 {
            plan.total_frames as f64 / plan.frame_rate as f64
        } else {
            0.0
        };
        ctx.store
            .set_corrupt_strikes(&job.sequence_name, &job.variant_key, 0)?;
        ctx.store.delete_failures_for_output(&job.output_path)?;
        let video = RenderedVideo {
            id: Uuid::new_v4().to_string(),
            rendered_at: ctx.now_ms,
            sequence_name: job.sequence_name.clone(),
            export_path: job.export_path.clone(),
            output_filename: job.output_filename.clone(),
            output_path: job.output_path.clone(),
            variant_key: job.variant_key.clone(),
            codec,
            resolution_label,
            frame_rate: plan.frame_rate,
            frame_count: plan.total_frames,
            size_bytes,
            width,
            height,
            duration_seconds,
            render_seconds: (ctx.now_ms - started_ms).max(0) as f64 / 1000.0,
            thumbnail_path: None,
            xmp_embedded,
            source: "TRD".to_owned(),
        };
        ctx.store.put_rendered_video(&video)?;
        self.thumbs.make(ctx.store, &video, ctx.now_ms)?;
        finish_job(ctx, &mut job, JobStatus::Done)?;
        let fields = LogFields {
            event: Some(EVENT_RENDER_FINISHED.to_owned()),
            sequence_name: Some(job.sequence_name.clone()),
            output_filename: Some(job.output_filename.clone()),
            rendered_video_id: Some(video.id.clone()),
            ..LogFields::default()
        };
        let message = format!("rendered {} ({size_bytes} bytes)", job.output_filename);
        note(ctx, LogLevel::Info, &job.sequence_name, &message, fields)?;
        self.after_render(ctx, &job, true)
    }

    /// Step 5's corrupt branch: the partial file goes, the strike is counted, and at the third
    /// strike the variant is auto-paused. The render is recorded as a failure.
    fn corrupt_render(
        &mut self,
        ctx: &mut Ctx<'_>,
        mut job: RenderJob,
        plan: &Plan,
        reason: &str,
        output: &str,
    ) -> StoreResult<()> {
        remove_file_quietly(Path::new(&plan.partial_path));
        let strikes = ctx
            .store
            .corrupt_strikes(&job.sequence_name, &job.variant_key)?
            + 1;
        ctx.store
            .set_corrupt_strikes(&job.sequence_name, &job.variant_key, strikes)?;
        let auto_paused = if strikes >= CORRUPT_PAUSE_STRIKES {
            self.auto_pause(ctx, &job, strikes, reason)?
        } else {
            false
        };
        let summary = format!("the render is corrupt ({reason})");
        let detail = format!(
            "exiftool refused to write the container, so the video is structurally broken. \
             {strikes} consecutive corrupt renders of this variant; at {CORRUPT_PAUSE_STRIKES} it is paused."
        );
        let mut failure = job_failure(
            &job,
            ctx.now_ms,
            FailureKind::StructuralCorruption.code(),
            summary,
            detail,
        );
        failure.command_line = command_line(plan);
        failure.tool_output_json =
            tool_output_json(&output.lines().map(str::to_owned).collect::<Vec<_>>());
        failure.variant_auto_paused = auto_paused;
        record_failure(ctx, failure)?;
        finish_job(ctx, &mut job, JobStatus::Failed)
    }

    /// Adds the variant to `paused_variants` in config.json (§6.6 step 5). Returns whether the
    /// config was written; a refused write is logged and the render's failure says so.
    fn auto_pause(
        &mut self,
        ctx: &mut Ctx<'_>,
        job: &RenderJob,
        strikes: i64,
        reason: &str,
    ) -> StoreResult<bool> {
        let revision = ctx.config.revision().to_owned();
        let sequence = job.sequence_name.clone();
        let variant = job.variant_key.clone();
        let (seq_for_edit, variant_for_edit) = (sequence.clone(), variant.clone());
        let edit = move |cfg: &mut Config| {
            if !is_paused_variant(cfg, &seq_for_edit, &variant_for_edit) {
                cfg.paused_variants.push(PausedVariant {
                    sequence_name: seq_for_edit,
                    variant_key: variant_for_edit,
                });
            }
        };
        match ctx.config.apply(edit, &revision, ctx.codecs) {
            Ok(_) => {
                let message = format!(
                    "auto-paused {sequence}/{variant} after {strikes} consecutive corrupt renders \
                     (last: {reason}); inspect the source sequence and resume manually"
                );
                note(
                    ctx,
                    LogLevel::Warn,
                    &sequence,
                    &message,
                    LogFields::default(),
                )?;
                Ok(true)
            }
            Err(err) => {
                let message = format!("could not auto-pause {sequence}/{variant}: {err}");
                note(
                    ctx,
                    LogLevel::Warn,
                    &sequence,
                    &message,
                    LogFields::default(),
                )?;
                Ok(false)
            }
        }
    }

    /// Step 7's sequence event and step 8's delete-after-render check, after a render of a sequence
    /// ended. `finished_render` is false when the output was already there (no event).
    fn after_render(
        &mut self,
        ctx: &mut Ctx<'_>,
        job: &RenderJob,
        finished_render: bool,
    ) -> StoreResult<()> {
        let Some(config) = ctx.config.config().cloned() else {
            return Ok(());
        };
        let outputs = variant_outputs(
            &config,
            ctx.codecs,
            &job.sequence_name,
            &job.export_path,
            ctx.year,
        );
        let missing_unpaused = outputs.iter().any(|o| !o.paused && !o.rendered);
        if finished_render && !missing_unpaused {
            // The videos trd rendered for the sequence since it became complete (§6.6 step 7): the
            // newest row of each configured variant that was rendered after that time. Older rows
            // (a render from before the sequence was complete, or from before a restart) are not
            // counted, and neither is a second row of the same variant.
            let since = self
                .complete_since
                .get(&job.sequence_name)
                .copied()
                .unwrap_or(i64::MIN);
            let configured: BTreeSet<String> = config
                .rendering
                .iter()
                .filter(|render| ctx.codecs.contains_key(&render.codec))
                .map(|render| render.variant_key())
                .collect();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut video_count = 0_i64;
            let mut total_bytes = 0_i64;
            // rendered_videos() is newest first, so the first row of a variant is its latest.
            for video in ctx.store.rendered_videos()? {
                if video.sequence_name == job.sequence_name
                    && video.source == "TRD"
                    && video.rendered_at >= since
                    && configured.contains(&video.variant_key)
                    && seen.insert(video.variant_key.clone())
                {
                    video_count += 1;
                    total_bytes += video.size_bytes;
                }
            }
            let fields = LogFields {
                event: Some(EVENT_SEQUENCE_FINISHED.to_owned()),
                sequence_name: Some(job.sequence_name.clone()),
                video_count: Some(video_count),
                total_bytes: Some(total_bytes),
                ..LogFields::default()
            };
            let message = format!(
                "sequence {} finished: {video_count} video(s), {total_bytes} bytes",
                job.sequence_name
            );
            note(ctx, LogLevel::Info, &job.sequence_name, &message, fields)?;
        }

        // Step 8: delete-after-render. Every variant with a known codec must have its output.
        let candidate = format!("{}/{}", job.export_path, job.sequence_name);
        let matches = config
            .delete_sequence_after_render_regexes
            .iter()
            .any(|pattern| Regex::new(pattern).is_ok_and(|re| re.is_match(&candidate)));
        let all_rendered = outputs.iter().all(|o| o.rendered);
        let busy = in_flight_jobs(ctx.store)?
            .iter()
            .any(|other| other.sequence_name == job.sequence_name);
        if !matches || !all_rendered || busy || !Path::new(&candidate).is_dir() {
            return Ok(());
        }
        match fs::remove_dir_all(&candidate) {
            Ok(()) => {
                let fields = LogFields {
                    event: Some(EVENT_SEQUENCE_DELETED.to_owned()),
                    sequence_name: Some(job.sequence_name.clone()),
                    ..LogFields::default()
                };
                let message = format!("removed {candidate} after every variant was rendered");
                note(ctx, LogLevel::Info, &job.sequence_name, &message, fields)
            }
            Err(err) => {
                let message = format!("could not remove {candidate}: {err}");
                note(
                    ctx,
                    LogLevel::Warn,
                    &job.sequence_name,
                    &message,
                    LogFields::default(),
                )
            }
        }
    }
}

/// Ends a job (§6.6 step 9): its status and finish time are recorded, its progress file is deleted
/// (the log is kept), and the logs of old jobs are pruned.
pub(crate) fn finish_job(ctx: &Ctx<'_>, job: &mut RenderJob, status: JobStatus) -> StoreResult<()> {
    job.status = status;
    job.finished_at = Some(ctx.now_ms);
    job.suspended_by = None;
    ctx.store.put_render_job(job)?;
    if let Some(progress) = &job.progress_path {
        remove_file_quietly(Path::new(progress));
    }
    retain_render_logs(ctx.store, RENDER_LOG_LIMIT)?;
    Ok(())
}

/// Ends the row of a job whose process has ended but whose completion failed part way (a store or
/// file error). The row is marked failed and a WARN row says why, so it no longer counts as in
/// flight. A row that already ended is left alone. Errors here are dropped: the caller returns the
/// error that started this.
pub(crate) fn release_job(ctx: &Ctx<'_>, id: &str, reason: &str) {
    let Ok(Some(mut job)) = ctx.store.render_job(id) else {
        return;
    };
    if !matches!(
        job.status,
        JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
    ) {
        return;
    }
    let message = format!(
        "render of {} ended without completing: {reason}",
        job.output_filename
    );
    let fields = LogFields {
        sequence_name: Some(job.sequence_name.clone()),
        output_filename: Some(job.output_filename.clone()),
        ..LogFields::default()
    };
    let _ = note(ctx, LogLevel::Warn, &job.sequence_name, &message, fields);
    let _ = finish_job(ctx, &mut job, JobStatus::Failed);
}

/// Ends a job that could not start or whose plan is unreadable, with a failure of `kind`.
pub(crate) fn fail_job(
    ctx: &Ctx<'_>,
    job: &mut RenderJob,
    kind: FailureKind,
    summary: String,
) -> StoreResult<()> {
    let failure = job_failure(job, ctx.now_ms, kind.code(), summary, String::new());
    record_failure(ctx, failure)?;
    finish_job(ctx, job, JobStatus::Failed)
}

/// A failure card of a job, with the fields the job already knows; the caller sets the rest.
pub(crate) fn job_failure(
    job: &RenderJob,
    now_ms: i64,
    kind: &str,
    summary: String,
    detail: String,
) -> Failure {
    Failure {
        id: Uuid::new_v4().to_string(),
        output_path: job.output_path.clone(),
        kind: kind.to_owned(),
        first_seen: now_ms,
        last_seen: now_ms,
        occurrences: 1,
        sequence_name: job.sequence_name.clone(),
        export_path: job.export_path.clone(),
        output_filename: job.output_filename.clone(),
        variant_key: job.variant_key.clone(),
        summary,
        detail,
        exit_code: None,
        command_line: String::new(),
        tool_output_json: "[]".to_owned(),
        variant_auto_paused: false,
        render_id: Some(job.id.clone()),
    }
}

/// Stores a failure (§6.9), merging it into the card of the same output and kind, and logs it: the
/// first occurrences and every [`FAILURE_LOG_EVERY`]th at ERROR with the full report, the rest at
/// WARN. Returns the card as stored.
pub(crate) fn record_failure(ctx: &Ctx<'_>, failure: Failure) -> StoreResult<Failure> {
    let stored = ctx.store.record_failure(&failure)?;
    let full_report = stored.occurrences <= FULL_REPORT_OCCURRENCES
        || stored.occurrences % FAILURE_LOG_EVERY == 0;
    let (level, message) = if full_report {
        let detail = if stored.detail.is_empty() {
            String::new()
        } else {
            format!("\n{}", stored.detail)
        };
        (
            LogLevel::Error,
            format!("{}: {}{detail}", stored.kind, stored.summary),
        )
    } else {
        (
            LogLevel::Warn,
            format!(
                "{}: {} (x{})",
                stored.kind, stored.summary, stored.occurrences
            ),
        )
    };
    let fields = LogFields {
        event: Some(EVENT_RENDER_FAILED.to_owned()),
        sequence_name: Some(stored.sequence_name.clone()),
        output_filename: Some(stored.output_filename.clone()),
        failure_id: Some(stored.id.clone()),
        ..LogFields::default()
    };
    note(ctx, level, &stored.sequence_name, &message, fields)?;
    Ok(stored)
}

/// Keeps the logs of the newest `keep` render jobs and deletes the rest (§6.6 step 9). Only jobs
/// that have a log count, and a log of a job still in flight is never deleted. Returns how many log
/// files were deleted.
pub fn retain_render_logs(store: &Store, keep: usize) -> StoreResult<usize> {
    let mut jobs = store.render_jobs(None)?;
    jobs.sort_by(|a, b| b.queued_at.cmp(&a.queued_at).then_with(|| b.id.cmp(&a.id)));
    let mut deleted = 0;
    for job in jobs.iter().filter(|job| job.log_path.is_some()).skip(keep) {
        let in_flight = matches!(
            job.status,
            JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
        );
        let log = job.log_path.as_deref().filter(|_| !in_flight);
        if log.is_some_and(|log| fs::remove_file(log).is_ok()) {
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Deletes the orphaned `.*.partial.mov` files in the export directories (§6.6): a file no queued,
/// running or suspended job owns. Returns how many were deleted.
pub fn sweep_stale_partials(store: &Store, config: &Config, year: i32) -> StoreResult<usize> {
    let owned: Vec<PathBuf> = in_flight_jobs(store)?
        .iter()
        .map(|job| PathBuf::from(&job.partial_path))
        .collect();
    let mut deleted = 0;
    for entry in &config.export_paths {
        for dir in expand_year_path(entry, year) {
            let Ok(listing) = fs::read_dir(&dir) else {
                continue;
            };
            for item in listing.flatten() {
                let name = item.file_name();
                let name = name.to_string_lossy();
                if !(name.starts_with('.') && name.ends_with(".partial.mov")) {
                    continue;
                }
                let path = item.path();
                if owned.contains(&path) {
                    continue;
                }
                if fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
        }
    }
    Ok(deleted)
}

/// The codec name and resolution label stored with a video: from the config's variant when it is
/// still there, else from the variant key (`<codec>_<res>_<fps>`).
fn render_identity(config: Option<&Config>, variant_key: &str) -> (String, String) {
    let found: Option<&RenderType> = config.and_then(|cfg| {
        cfg.rendering
            .iter()
            .find(|render| render.variant_key() == variant_key)
    });
    match found {
        Some(render) => (render.codec.clone(), render.resolution_label()),
        None => (
            variant_key.split('_').next().unwrap_or_default().to_owned(),
            String::new(),
        ),
    }
}

/// One variant of a sequence with a known codec: whether it is paused and whether its output exists.
pub(crate) struct VariantOutput {
    pub(crate) paused: bool,
    pub(crate) rendered: bool,
}

/// The variants of a sequence with a known codec (§6.6 steps 7–8).
fn variant_outputs(
    config: &Config,
    codecs: &crate::codecs::VideoCodecs,
    sequence_name: &str,
    export_path: &str,
    year: i32,
) -> Vec<VariantOutput> {
    config
        .rendering
        .iter()
        .filter(|render| codecs.contains_key(&render.codec))
        .map(|render| variant_output(config, codecs, render, sequence_name, export_path, year))
        .collect()
}

fn variant_output(
    config: &Config,
    codecs: &crate::codecs::VideoCodecs,
    render: &RenderType,
    sequence_name: &str,
    export_path: &str,
    year: i32,
) -> VariantOutput {
    let filename = plan::output_filename(config, codecs, render, sequence_name);
    VariantOutput {
        paused: is_paused_variant(config, sequence_name, &render.variant_key()),
        rendered: plan::already_rendered(config, export_path, &filename, &[], year),
    }
}

/// The last `lines` lines of a render's log, or none when the log is missing.
pub(crate) fn read_tail(log: Option<&str>, lines: usize) -> Vec<String> {
    let Some(path) = log else {
        return Vec::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].iter().map(|line| (*line).to_owned()).collect()
}

fn tool_output_json(lines: &[String]) -> String {
    serde_json::to_string(lines).unwrap_or_else(|_| "[]".to_owned())
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// The ffmpeg command line of a plan, as it would be typed by hand.
fn command_line(plan: &Plan) -> String {
    format!("ffmpeg {}", plan.argv().join(" "))
}

/// Deletes a file, ignoring one that is already gone or cannot be removed.
pub(crate) fn remove_file_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}
