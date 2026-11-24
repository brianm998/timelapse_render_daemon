// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Render queue and concurrency limits (§6.7).
//!
//! [`Renders`] owns the running ffmpeg processes; the queue itself is the `render_jobs` table. A
//! scan calls [`Renders::plan_jobs`] (queue what is missing), then [`Renders::remove_unwanted`]
//! (drop what is no longer wanted) and [`Renders::promote`] (start what the caps allow). Every
//! [`Renders::tick`] reaps the processes that ended (§6.6, in `pipeline`) and promotes again.
//!
//! Each call takes a [`Ctx`] carrying the store, the config, the tables, the clock and the year, so
//! the caller chooses them. Nothing here sleeps.
//!
//! The spec does not define "blocked" beyond "paused" (§6.7). A complete sequence is queued when it
//! is not paused and its histogram is known. A failure card (§6.9) blocks requeueing its output
//! until the card is dismissed or retried.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::codecs::VideoCodecs;
use crate::config::{Config, ConfigStore};
use crate::diagnosis::FailureKind;
use crate::events::{EVENT_RENDER_STARTED, EVENT_SEQUENCE_READY};
use crate::exif::{self, Histogram};
use crate::pipeline::{self, job_failure};
use crate::plan::{self, FAILURE_NOT_QUEUEABLE, Plan, PlanRequest};
use crate::process::{self, RunningProcess};
use crate::progress::ProgressReader;
use crate::scan::{ExportSequence, ScanResult};
use crate::store::{
    Failure, JobStatus, LogFields, LogLevel, RenderJob, Store, StoreResult, log_event,
};
use crate::thumbs::Thumbnailer;
use crate::tools::{ToolCommand, Tools};

/// How long after one sweep of stale partial files the next one runs (§6.6).
pub const SWEEP_INTERVAL_MS: i64 = 10 * 60 * 1000;

/// What a call needs from its caller: the store, the config and codec tables, the tools, the clock
/// and the calendar year (for `$YEAR` in paths).
pub struct Ctx<'a> {
    pub store: &'a Store,
    pub config: &'a mut ConfigStore,
    pub codecs: &'a VideoCodecs,
    pub tools: &'a Tools,
    pub now_ms: i64,
    pub year: i32,
}

/// A job whose ffmpeg process this daemon started and has not yet seen end.
pub(crate) struct Active {
    pub(crate) sequence_name: String,
    pub(crate) plan: Plan,
    pub(crate) process: RunningProcess,
    pub(crate) progress: ProgressReader,
    pub(crate) started_ms: i64,
    /// The histogram of the sequence, for the XMP step (§6.6 step 5). Empty when none was known.
    pub(crate) histogram: Histogram,
}

/// The render queue and the running processes.
pub struct Renders {
    pub(crate) data_dir: PathBuf,
    pub(crate) thumbs: Thumbnailer,
    /// Extra environment for the ffmpeg processes; empty in production, set by tests.
    pub(crate) env: Vec<(String, String)>,
    /// Running and suspended jobs by job id. A suspended job keeps its slot (§6.8).
    pub(crate) running: BTreeMap<String, Active>,
    /// The histogram of each sequence that has queued jobs, by sequence name.
    pub(crate) histograms: BTreeMap<String, Histogram>,
    /// When each sequence was first sighted (ms), from the engine (§6.7 promotion order).
    pub(crate) first_seen: BTreeMap<String, i64>,
    /// The export sequences of the last scan, by name. The sequence controls check against it.
    pub(crate) known_sequences: BTreeSet<String>,
    /// When each complete sequence was first seen complete, by sequence name.
    pub(crate) complete_since: BTreeMap<String, i64>,
    /// When the next sweep of stale partial files is due.
    pub(crate) next_sweep_ms: Option<i64>,
}

impl Renders {
    /// A queue whose renders go to `<data_dir>/renders` and whose thumbnails go through `thumbs`.
    pub fn new(data_dir: impl Into<PathBuf>, thumbs: Thumbnailer) -> Renders {
        Renders {
            data_dir: data_dir.into(),
            thumbs,
            env: Vec::new(),
            running: BTreeMap::new(),
            histograms: BTreeMap::new(),
            first_seen: BTreeMap::new(),
            known_sequences: BTreeSet::new(),
            complete_since: BTreeMap::new(),
            next_sweep_ms: None,
        }
    }

    /// Adds environment variables to every ffmpeg process this queue starts.
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Renders {
        self.env = env;
        self
    }

    /// The ids of the jobs whose process this queue holds, running or suspended.
    pub fn running_ids(&self) -> Vec<String> {
        self.running.keys().cloned().collect()
    }

    /// The latest complete progress block of a job this queue holds.
    pub fn progress(&self, job_id: &str) -> Option<crate::progress::ProgressBlock> {
        self.running
            .get(job_id)
            .and_then(|active| active.progress.latest())
    }

    /// Start-up (§6.6): sweeps stale partial files once. Later sweeps follow [`Renders::tick`].
    pub fn startup(&mut self, ctx: &mut Ctx<'_>) -> StoreResult<()> {
        self.sweep(ctx)
    }

    /// Everything a scan triggers, in order: queue what is missing, drop what is unwanted, promote.
    pub fn on_scan(
        &mut self,
        ctx: &mut Ctx<'_>,
        scan: &ScanResult,
        histograms: &BTreeMap<String, Histogram>,
        first_seen: &BTreeMap<String, i64>,
    ) -> StoreResult<()> {
        self.plan_jobs(ctx, scan, histograms, first_seen)?;
        self.remove_unwanted(ctx, scan)?;
        self.promote(ctx)?;
        Ok(())
    }

    /// One pass of the render loop: reaps the processes that ended, sweeps stale partials when due,
    /// and promotes queued jobs within the caps.
    pub fn tick(&mut self, ctx: &mut Ctx<'_>) -> StoreResult<()> {
        self.poll_running(ctx)?;
        if self.next_sweep_ms.is_some_and(|due| ctx.now_ms >= due) {
            self.sweep(ctx)?;
        }
        self.promote(ctx)?;
        Ok(())
    }

    /// Queues a job for every variant of every complete, unpaused sequence that has its histogram
    /// and is not already rendered, in flight or failed (§6.7). A variant whose plan cannot be made
    /// gets a `FAILURE_NOT_QUEUEABLE` card (§6.9). Returns how many jobs were queued.
    pub fn plan_jobs(
        &mut self,
        ctx: &mut Ctx<'_>,
        scan: &ScanResult,
        histograms: &BTreeMap<String, Histogram>,
        first_seen: &BTreeMap<String, i64>,
    ) -> StoreResult<usize> {
        self.first_seen = first_seen.clone();
        self.known_sequences = scan.exports.iter().map(|seq| seq.name.clone()).collect();
        let Some(config) = ctx.config.config().cloned() else {
            return Ok(0);
        };
        let mut in_flight = planned(&in_flight_jobs(ctx.store)?);
        let mut blocked: HashSet<String> = ctx
            .store
            .failures()?
            .into_iter()
            .map(|failure| failure.output_path)
            .collect();
        let mut queued = 0;
        for seq in &scan.exports {
            if config.paused_sequence_names.contains(&seq.name) {
                continue;
            }
            if seq.in_progress {
                self.complete_since.remove(&seq.name);
                continue;
            }
            if !self.complete_since.contains_key(&seq.name) {
                self.complete_since.insert(seq.name.clone(), ctx.now_ms);
                let fields = LogFields {
                    event: Some(EVENT_SEQUENCE_READY.to_owned()),
                    sequence_name: Some(seq.name.clone()),
                    ..LogFields::default()
                };
                note(
                    ctx,
                    LogLevel::Info,
                    &seq.name,
                    &format!("sequence {} is complete", seq.name),
                    fields,
                )?;
            }
            let Some(raw_dir) = raw_dir_of(scan, &seq.raw_name) else {
                continue;
            };
            let Some(histogram) = histograms.get(&raw_dir) else {
                continue;
            };
            self.histograms.insert(seq.name.clone(), histogram.clone());
            let metadata = exif::metadata(histogram);
            let export_path = export_path_of(seq);
            for render in &config.rendering {
                if !ctx.codecs.contains_key(&render.codec) {
                    continue;
                }
                let variant_key = render.variant_key();
                if is_paused_variant(&config, &seq.name, &variant_key) {
                    continue;
                }
                let filename = plan::output_filename(&config, ctx.codecs, render, &seq.name);
                let output_path = format!("{export_path}/{filename}");
                if blocked.contains(&output_path) {
                    continue;
                }
                if plan::already_rendered(&config, &export_path, &filename, &in_flight, ctx.year) {
                    continue;
                }
                let job_id = Uuid::new_v4().to_string();
                let request = PlanRequest {
                    config: &config,
                    codecs: ctx.codecs,
                    tools: ctx.tools,
                    export_path: &export_path,
                    sequence_dir: &seq.name,
                    render,
                    data_dir: &self.data_dir,
                    render_id: &job_id,
                    metadata: metadata.clone(),
                    total_frames: seq.raw_frames as i64,
                };
                match plan::plan_render(&request) {
                    Ok(plan) => {
                        let job = new_job(
                            &job_id,
                            &seq.name,
                            &export_path,
                            &variant_key,
                            &filename,
                            &plan,
                            &self.data_dir,
                            ctx.now_ms,
                        );
                        ctx.store.put_render_job(&job)?;
                        in_flight.push(plan::Job {
                            status: plan::JobStatus::Queued,
                            output_path: job.output_path.clone(),
                        });
                        queued += 1;
                    }
                    Err(err) => {
                        let detail = match &err {
                            plan::PlanError::NotQueueable { detail, .. } => detail.clone(),
                            plan::PlanError::UnknownCodec { .. } => String::new(),
                        };
                        let failure = Failure {
                            id: Uuid::new_v4().to_string(),
                            output_path: output_path.clone(),
                            kind: FAILURE_NOT_QUEUEABLE.to_owned(),
                            first_seen: ctx.now_ms,
                            last_seen: ctx.now_ms,
                            occurrences: 1,
                            sequence_name: seq.name.clone(),
                            export_path: export_path.clone(),
                            output_filename: filename.clone(),
                            variant_key: variant_key.clone(),
                            summary: err.to_string(),
                            detail,
                            exit_code: None,
                            command_line: String::new(),
                            tool_output_json: "[]".to_owned(),
                            variant_auto_paused: false,
                            render_id: None,
                        };
                        pipeline::record_failure(ctx, failure)?;
                        blocked.insert(output_path);
                    }
                }
            }
        }
        Ok(queued)
    }

    /// Deletes the queued jobs that are no longer wanted: their variant left the config or is paused,
    /// their output now exists, or their sequence is paused or gone (§6.7). A removed job becomes
    /// `cancelled`; the store has no delete, and a cancelled row is no longer queued. Returns how
    /// many were removed.
    pub fn remove_unwanted(&mut self, ctx: &mut Ctx<'_>, scan: &ScanResult) -> StoreResult<usize> {
        let Some(config) = ctx.config.config().cloned() else {
            return Ok(0);
        };
        let mut removed = 0;
        for mut job in ctx.store.render_jobs(Some(JobStatus::Queued))? {
            let reason = if !config
                .rendering
                .iter()
                .any(|r| r.variant_key() == job.variant_key)
            {
                Some("its variant left config.json")
            } else if is_paused_variant(&config, &job.sequence_name, &job.variant_key) {
                Some("its variant is paused")
            } else if config.paused_sequence_names.contains(&job.sequence_name) {
                Some("its sequence is paused")
            } else if !scan
                .exports
                .iter()
                .any(|e| e.name == job.sequence_name && export_path_of(e) == job.export_path)
            {
                Some("its sequence is gone")
            } else if plan::already_rendered(
                &config,
                &job.export_path,
                &job.output_filename,
                &[],
                ctx.year,
            ) {
                Some("its output exists")
            } else {
                None
            };
            let Some(reason) = reason else {
                continue;
            };
            let fields = LogFields {
                sequence_name: Some(job.sequence_name.clone()),
                output_filename: Some(job.output_filename.clone()),
                ..LogFields::default()
            };
            let message = format!("queued render of {} removed: {reason}", job.output_filename);
            let subject = job.sequence_name.clone();
            pipeline::finish_job(ctx, &mut job, JobStatus::Cancelled)?;
            note(ctx, LogLevel::Info, &subject, &message, fields)?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Starts queued jobs in order (§6.7) while the caps allow. Nothing starts while global pause is
    /// on. A job starts when the running and suspended jobs are below `max_processes.ffmpeg` and
    /// those of its sequence are below `max_cuncurrent_renders`. Returns how many started.
    pub fn promote(&mut self, ctx: &mut Ctx<'_>) -> StoreResult<usize> {
        if ctx.store.global_pause()? {
            return Ok(0);
        }
        let store = ctx.store;
        let tools = ctx.tools;
        let Some(config) = ctx.config.config().cloned() else {
            return Ok(0);
        };
        let Some(ffmpeg) = tools.ffmpeg.command.as_ref() else {
            return Ok(0);
        };
        let mut queued = store.render_jobs(Some(JobStatus::Queued))?;
        queued.sort_by_key(|job| self.order_key(&config, job));
        let mut started = 0;
        for job in queued {
            if self.running.len() as i64 >= config.max_processes.ffmpeg {
                break;
            }
            let same_sequence = self
                .running
                .values()
                .filter(|active| active.sequence_name == job.sequence_name)
                .count() as i64;
            if same_sequence >= config.max_cuncurrent_renders {
                continue;
            }
            if self.launch(ctx, ffmpeg, job)? {
                started += 1;
            }
        }
        Ok(started)
    }

    /// The promotion order (§6.7): the sequence's first sighting, then the variant's position in
    /// `rendering`, then the queue time and the id so ties are stable.
    fn order_key(&self, config: &Config, job: &RenderJob) -> (i64, usize, i64, String) {
        let sequence = self
            .first_seen
            .get(&job.sequence_name)
            .copied()
            .unwrap_or(i64::MAX);
        let position = config
            .rendering
            .iter()
            .position(|r| r.variant_key() == job.variant_key)
            .unwrap_or(usize::MAX);
        (sequence, position, job.queued_at, job.id.clone())
    }

    /// Starts the ffmpeg process of a queued job and records it as running. Returns whether it
    /// started; a job that cannot start is recorded as a failure and returns false.
    fn launch(
        &mut self,
        ctx: &mut Ctx<'_>,
        ffmpeg: &ToolCommand,
        mut job: RenderJob,
    ) -> StoreResult<bool> {
        let plan: Plan = match serde_json::from_str(&job.plan_json) {
            Ok(plan) => plan,
            Err(err) => {
                let summary = format!("the stored plan of the render is unreadable: {err}");
                pipeline::fail_job(ctx, &mut job, FailureKind::LaunchFailed, summary)?;
                return Ok(false);
            }
        };
        let mut argv: Vec<OsString> = vec![ffmpeg.program.clone()];
        argv.extend(ffmpeg.leading_args.iter().cloned());
        let plan_args = plan.argv();
        let command_line = format!("ffmpeg {}", plan_args.join(" "));
        argv.extend(plan_args.into_iter().map(OsString::from));

        let log_path = self
            .data_dir
            .join("renders")
            .join(format!("{}.log", job.id));
        let _ = fs::create_dir_all(self.data_dir.join("renders"));
        job.log_path = Some(log_path.to_string_lossy().into_owned());
        let env: Vec<(&str, &str)> = self
            .env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        match process::spawn_detached_with_env(&argv, &env, &log_path) {
            Err(err) => {
                let summary = format!("ffmpeg would not start: {err}");
                let mut failure = job_failure(
                    &job,
                    ctx.now_ms,
                    FailureKind::LaunchFailed.code(),
                    summary,
                    String::new(),
                );
                failure.command_line = command_line;
                pipeline::record_failure(ctx, failure)?;
                pipeline::finish_job(ctx, &mut job, JobStatus::Failed)?;
                Ok(false)
            }
            Ok(process) => {
                job.status = JobStatus::Running;
                job.pid = Some(i64::from(process.pid));
                job.pid_start_time = Some(process.start_time as i64);
                job.started_at = Some(ctx.now_ms);
                ctx.store.put_render_job(&job)?;
                let fields = LogFields {
                    event: Some(EVENT_RENDER_STARTED.to_owned()),
                    sequence_name: Some(job.sequence_name.clone()),
                    output_filename: Some(job.output_filename.clone()),
                    ..LogFields::default()
                };
                let message = format!("started {}", job.output_filename);
                note(ctx, LogLevel::Info, &job.sequence_name, &message, fields)?;
                let active = Active {
                    sequence_name: job.sequence_name.clone(),
                    progress: ProgressReader::new(plan.progress_path.clone()),
                    started_ms: ctx.now_ms,
                    histogram: self
                        .histograms
                        .get(&job.sequence_name)
                        .cloned()
                        .unwrap_or_default(),
                    process,
                    plan,
                };
                self.running.insert(job.id.clone(), active);
                Ok(true)
            }
        }
    }

    /// Sweeps the stale partial files and schedules the next sweep (§6.6).
    fn sweep(&mut self, ctx: &Ctx<'_>) -> StoreResult<()> {
        if let Some(config) = ctx.config.config() {
            let removed = pipeline::sweep_stale_partials(ctx.store, config, ctx.year)?;
            if removed > 0 {
                let message = format!("deleted {removed} stale partial render file(s)");
                note(
                    ctx,
                    LogLevel::Info,
                    "renders",
                    &message,
                    LogFields::default(),
                )?;
            }
        }
        self.next_sweep_ms = Some(ctx.now_ms + SWEEP_INTERVAL_MS);
        Ok(())
    }
}

/// Writes one log row through the store (§6.14), at the context's time.
pub(crate) fn note(
    ctx: &Ctx<'_>,
    level: LogLevel,
    subject: &str,
    message: &str,
    fields: LogFields,
) -> StoreResult<()> {
    log_event(ctx.store, ctx.now_ms, level, subject, message, &fields)?;
    Ok(())
}

/// The jobs that still own an output: queued, running or suspended.
pub(crate) fn in_flight_jobs(store: &Store) -> StoreResult<Vec<RenderJob>> {
    let mut jobs = store.render_jobs(Some(JobStatus::Queued))?;
    jobs.extend(store.render_jobs(Some(JobStatus::Running))?);
    jobs.extend(store.render_jobs(Some(JobStatus::Suspended))?);
    Ok(jobs)
}

/// The in-flight jobs as the planner needs them (§6.5 `already_rendered`).
fn planned(jobs: &[RenderJob]) -> Vec<plan::Job> {
    jobs.iter()
        .map(|job| plan::Job {
            status: match job.status {
                JobStatus::Queued => plan::JobStatus::Queued,
                JobStatus::Suspended => plan::JobStatus::Suspended,
                _ => plan::JobStatus::Running,
            },
            output_path: job.output_path.clone(),
        })
        .collect()
}

/// Whether the variant `variant_key` of `sequence_name` is in `paused_variants`.
pub(crate) fn is_paused_variant(config: &Config, sequence_name: &str, variant_key: &str) -> bool {
    config
        .paused_variants
        .iter()
        .any(|p| p.sequence_name == sequence_name && p.variant_key == variant_key)
}

/// The export directory a sequence sits in: the parent of `<export path>/<dir>`.
pub(crate) fn export_path_of(seq: &ExportSequence) -> String {
    Path::new(&seq.path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The raw directory of the raw sequence named `raw_name`, as the EXIF cache keys it.
fn raw_dir_of(scan: &ScanResult, raw_name: &str) -> Option<String> {
    scan.raw.iter().find(|raw| raw.name == raw_name).map(|raw| {
        Path::new(&raw.parent)
            .join(&raw.name)
            .to_string_lossy()
            .into_owned()
    })
}

/// A queued job built from its plan (§6.6 step 1).
#[allow(clippy::too_many_arguments)]
fn new_job(
    id: &str,
    sequence_name: &str,
    export_path: &str,
    variant_key: &str,
    output_filename: &str,
    plan: &Plan,
    data_dir: &Path,
    now_ms: i64,
) -> RenderJob {
    let log_path = data_dir.join("renders").join(format!("{id}.log"));
    RenderJob {
        id: id.to_owned(),
        sequence_name: sequence_name.to_owned(),
        export_path: export_path.to_owned(),
        variant_key: variant_key.to_owned(),
        output_filename: output_filename.to_owned(),
        output_path: plan.output_path.clone(),
        partial_path: plan.partial_path.clone(),
        plan_json: serde_json::to_string(plan).unwrap_or_else(|_| "{}".to_owned()),
        status: JobStatus::Queued,
        suspended_by: None,
        pid: None,
        pid_start_time: None,
        queued_at: now_ms,
        started_at: None,
        finished_at: None,
        progress_path: Some(plan.progress_path.clone()),
        log_path: Some(log_path.to_string_lossy().into_owned()),
        adopted: false,
        foreign: false,
    }
}
