// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Pause, resume, suspend, stop and retry controls (§6.8, overview §6.9).
//!
//! Every control is one method on [`Renders`]. A control that applies writes one INFO log row
//! naming the action and its target (§6.14). A control that does not apply returns
//! [`ControlError::FailedPrecondition`]; an unknown id returns [`ControlError::NotFound`]. A
//! rejected control writes nothing.

use std::fmt;
use std::path::Path;

use crate::config::{Config, ConfigError, PausedVariant};
use crate::pipeline::{self, remove_file_quietly};
use crate::process::{self, RunningProcess, TERMINATE_GRACE};
use crate::queue::{Ctx, Renders, in_flight_jobs, is_paused_variant, note};
use crate::store::{Failure, JobStatus, LogFields, LogLevel, RenderJob, StoreError, SuspendedBy};

/// What a control asks of its caller. `rescan` is set by Retry and Rescan now: the caller runs a
/// scan at once (§6.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub rescan: bool,
}

impl Outcome {
    fn done() -> Outcome {
        Outcome { rescan: false }
    }
}

/// Why a control was not applied.
#[derive(Debug)]
pub enum ControlError {
    /// The id names nothing.
    NotFound(String),
    /// The control does not apply to the current state.
    FailedPrecondition(String),
    /// The store, the config file or a process could not be changed.
    Internal(String),
}

impl ControlError {
    /// The status code name of the API (overview §6.9): `NOT_FOUND`, `FAILED_PRECONDITION`, `INTERNAL`.
    pub fn code(&self) -> &'static str {
        match self {
            ControlError::NotFound(_) => "NOT_FOUND",
            ControlError::FailedPrecondition(_) => "FAILED_PRECONDITION",
            ControlError::Internal(_) => "INTERNAL",
        }
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::NotFound(message)
            | ControlError::FailedPrecondition(message)
            | ControlError::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<StoreError> for ControlError {
    fn from(err: StoreError) -> ControlError {
        ControlError::Internal(err.to_string())
    }
}

fn map_config(err: ConfigError) -> ControlError {
    match err {
        ConfigError::StaleRevision { .. } => {
            ControlError::FailedPrecondition("config.json changed on disk; try again".to_owned())
        }
        ConfigError::Refused(issues) => {
            let first = issues
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_default();
            ControlError::FailedPrecondition(format!("config.json would be invalid: {first}"))
        }
        ConfigError::Unparseable(text) => {
            ControlError::FailedPrecondition(format!("config.json does not parse: {text}"))
        }
        ConfigError::Io(err) => ControlError::Internal(err.to_string()),
    }
}

fn require_config(ctx: &Ctx<'_>) -> Result<Config, ControlError> {
    ctx.config
        .config()
        .cloned()
        .ok_or_else(|| ControlError::FailedPrecondition("config.json is not loaded".to_owned()))
}

fn require_variant(config: &Config, variant_key: &str) -> Result<(), ControlError> {
    if config
        .rendering
        .iter()
        .any(|render| render.variant_key() == variant_key)
    {
        Ok(())
    } else {
        Err(ControlError::NotFound(format!("no variant {variant_key}")))
    }
}

fn edit_config(ctx: &mut Ctx<'_>, edit: impl FnOnce(&mut Config)) -> Result<(), ControlError> {
    let revision = ctx.config.revision().to_owned();
    ctx.config
        .apply(edit, &revision, ctx.codecs)
        .map(|_| ())
        .map_err(map_config)
}

fn load_job(ctx: &Ctx<'_>, id: &str) -> Result<RenderJob, ControlError> {
    ctx.store
        .render_job(id)?
        .ok_or_else(|| ControlError::NotFound(format!("no render {id}")))
}

fn process_error(what: &str, pid: i64, err: std::io::Error) -> ControlError {
    ControlError::Internal(format!("cannot {what} process {pid}: {err}"))
}

/// Writes the INFO row of a control that applied (§6.14).
fn record(
    ctx: &Ctx<'_>,
    message: &str,
    sequence: Option<&str>,
    output: Option<&str>,
) -> Result<(), ControlError> {
    let fields = LogFields {
        sequence_name: sequence.map(str::to_owned),
        output_filename: output.map(str::to_owned),
        ..LogFields::default()
    };
    note(ctx, LogLevel::Info, "controls", message, fields)?;
    Ok(())
}

impl Renders {
    /// Pause sequence (overview §6.9): the sequence is added to `paused_sequence_names`, and its
    /// queued, running and suspended renders end with their partial files deleted.
    pub fn pause_sequence(
        &mut self,
        ctx: &mut Ctx<'_>,
        name: &str,
    ) -> Result<Outcome, ControlError> {
        let config = require_config(ctx)?;
        self.require_sequence(ctx, &config, name)?;
        if config.paused_sequence_names.iter().any(|n| n == name) {
            return Err(ControlError::FailedPrecondition(format!(
                "sequence {name} is already paused"
            )));
        }
        // The renders end first: when one cannot be ended, the config is left as it was.
        for job in in_flight_jobs(ctx.store)?
            .into_iter()
            .filter(|job| job.sequence_name == name)
        {
            self.cancel_job(ctx, job)?;
        }
        let target = name.to_owned();
        edit_config(ctx, move |cfg| cfg.paused_sequence_names.push(target))?;
        record(ctx, &format!("pause sequence {name}"), Some(name), None)?;
        Ok(Outcome::done())
    }

    /// Resume sequence: removes it from `paused_sequence_names`. The next scan queues what is missing.
    pub fn resume_sequence(
        &mut self,
        ctx: &mut Ctx<'_>,
        name: &str,
    ) -> Result<Outcome, ControlError> {
        let config = require_config(ctx)?;
        self.require_sequence(ctx, &config, name)?;
        if !config.paused_sequence_names.iter().any(|n| n == name) {
            return Err(ControlError::FailedPrecondition(format!(
                "sequence {name} is not paused"
            )));
        }
        let target = name.to_owned();
        edit_config(ctx, move |cfg| {
            cfg.paused_sequence_names.retain(|n| *n != target)
        })?;
        record(ctx, &format!("resume sequence {name}"), Some(name), None)?;
        Ok(Outcome::done())
    }

    /// Pause variant: adds the variant to `paused_variants`, and ends its in-flight renders.
    pub fn pause_variant(
        &mut self,
        ctx: &mut Ctx<'_>,
        sequence: &str,
        variant_key: &str,
    ) -> Result<Outcome, ControlError> {
        let config = require_config(ctx)?;
        self.require_sequence(ctx, &config, sequence)?;
        require_variant(&config, variant_key)?;
        if is_paused_variant(&config, sequence, variant_key) {
            return Err(ControlError::FailedPrecondition(format!(
                "{sequence}/{variant_key} is already paused"
            )));
        }
        // The renders end first: when one cannot be ended, the config is left as it was.
        for job in in_flight_jobs(ctx.store)?
            .into_iter()
            .filter(|job| job.sequence_name == sequence && job.variant_key == variant_key)
        {
            self.cancel_job(ctx, job)?;
        }
        push_pause(ctx, sequence, variant_key)?;
        record(
            ctx,
            &format!("pause variant {sequence}/{variant_key}"),
            Some(sequence),
            None,
        )?;
        Ok(Outcome::done())
    }

    /// Resume variant: removes the variant from `paused_variants`.
    pub fn resume_variant(
        &mut self,
        ctx: &mut Ctx<'_>,
        sequence: &str,
        variant_key: &str,
    ) -> Result<Outcome, ControlError> {
        let config = require_config(ctx)?;
        self.require_sequence(ctx, &config, sequence)?;
        require_variant(&config, variant_key)?;
        if !is_paused_variant(&config, sequence, variant_key) {
            return Err(ControlError::FailedPrecondition(format!(
                "{sequence}/{variant_key} is not paused"
            )));
        }
        let (seq, key) = (sequence.to_owned(), variant_key.to_owned());
        edit_config(ctx, move |cfg| {
            cfg.paused_variants
                .retain(|p| !(p.sequence_name == seq && p.variant_key == key))
        })?;
        record(
            ctx,
            &format!("resume variant {sequence}/{variant_key}"),
            Some(sequence),
            None,
        )?;
        Ok(Outcome::done())
    }

    /// Suspend render: SIGSTOP (`NtSuspendProcess` on Windows) for a running job. It keeps its slot.
    pub fn suspend_render(&mut self, ctx: &mut Ctx<'_>, id: &str) -> Result<Outcome, ControlError> {
        let mut job = load_job(ctx, id)?;
        if job.status != JobStatus::Running {
            return Err(ControlError::FailedPrecondition(format!(
                "render {id} is {}, not running",
                job.status.as_str()
            )));
        }
        let pid = self.live_pid(&job).ok_or_else(|| {
            ControlError::FailedPrecondition(format!("render {id} has no live process"))
        })?;
        process::suspend(pid).map_err(|err| process_error("suspend", i64::from(pid), err))?;
        job.status = JobStatus::Suspended;
        job.suspended_by = Some(SuspendedBy::User);
        ctx.store.put_render_job(&job)?;
        record(
            ctx,
            &format!("suspend render {id}"),
            Some(&job.sequence_name),
            Some(&job.output_filename),
        )?;
        Ok(Outcome::done())
    }

    /// Resume render: SIGCONT (`NtResumeProcess` on Windows) for a render suspended by the user. A
    /// render suspended by Pause rendering is resumed by Resume rendering instead.
    pub fn resume_render(&mut self, ctx: &mut Ctx<'_>, id: &str) -> Result<Outcome, ControlError> {
        let mut job = load_job(ctx, id)?;
        if job.status != JobStatus::Suspended {
            return Err(ControlError::FailedPrecondition(format!(
                "render {id} is {}, not suspended",
                job.status.as_str()
            )));
        }
        if job.suspended_by == Some(SuspendedBy::Global) {
            return Err(ControlError::FailedPrecondition(format!(
                "render {id} was suspended by pause rendering; resume rendering instead"
            )));
        }
        let pid = self.live_pid(&job).ok_or_else(|| {
            ControlError::FailedPrecondition(format!(
                "render {id} has no live process; stop it instead"
            ))
        })?;
        process::resume(pid).map_err(|err| process_error("resume", i64::from(pid), err))?;
        job.status = JobStatus::Running;
        job.suspended_by = None;
        ctx.store.put_render_job(&job)?;
        record(
            ctx,
            &format!("resume render {id}"),
            Some(&job.sequence_name),
            Some(&job.output_filename),
        )?;
        Ok(Outcome::done())
    }

    /// Stop render: ends a queued, running or suspended render, deletes its partial file, and pauses
    /// its variant so the next scan does not queue it again.
    pub fn stop_render(&mut self, ctx: &mut Ctx<'_>, id: &str) -> Result<Outcome, ControlError> {
        let job = load_job(ctx, id)?;
        if !matches!(
            job.status,
            JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
        ) {
            return Err(ControlError::FailedPrecondition(format!(
                "render {id} is {}, so it cannot be stopped",
                job.status.as_str()
            )));
        }
        let config = require_config(ctx)?;
        let paused = is_paused_variant(&config, &job.sequence_name, &job.variant_key);
        let (sequence, output) = (job.sequence_name.clone(), job.output_filename.clone());
        let (variant_sequence, variant_key) = (job.sequence_name.clone(), job.variant_key.clone());
        // The render ends first: when it cannot be ended, the config is left as it was.
        self.cancel_job(ctx, job)?;
        if !paused {
            push_pause(ctx, &variant_sequence, &variant_key)?;
        }
        record(
            ctx,
            &format!("stop render {id}"),
            Some(&sequence),
            Some(&output),
        )?;
        Ok(Outcome::done())
    }

    /// Retry: dismisses the variant's failure cards, clears its pause and corrupt strikes, and asks
    /// for a scan so the variant is queued again. Applies to a failed or paused variant.
    pub fn retry(
        &mut self,
        ctx: &mut Ctx<'_>,
        sequence: &str,
        variant_key: &str,
    ) -> Result<Outcome, ControlError> {
        let config = require_config(ctx)?;
        self.require_sequence(ctx, &config, sequence)?;
        require_variant(&config, variant_key)?;
        let paused = is_paused_variant(&config, sequence, variant_key);
        let failures: Vec<Failure> = ctx
            .store
            .failures()?
            .into_iter()
            .filter(|f| f.sequence_name == sequence && f.variant_key == variant_key)
            .collect();
        let failed_job = ctx
            .store
            .render_jobs(Some(JobStatus::Failed))?
            .iter()
            .any(|job| job.sequence_name == sequence && job.variant_key == variant_key);
        if !paused && failures.is_empty() && !failed_job {
            return Err(ControlError::FailedPrecondition(format!(
                "{sequence}/{variant_key} is neither failed nor paused"
            )));
        }
        if paused {
            let (seq, key) = (sequence.to_owned(), variant_key.to_owned());
            edit_config(ctx, move |cfg| {
                cfg.paused_variants
                    .retain(|p| !(p.sequence_name == seq && p.variant_key == key))
            })?;
        }
        for failure in &failures {
            ctx.store.delete_failure(&failure.id)?;
        }
        ctx.store.set_corrupt_strikes(sequence, variant_key, 0)?;
        record(
            ctx,
            &format!("retry {sequence}/{variant_key}"),
            Some(sequence),
            None,
        )?;
        Ok(Outcome { rescan: true })
    }

    /// Pause all rendering: global pause is set, and every running render is suspended with
    /// `suspended_by = global`. Nothing new starts until Resume rendering.
    pub fn pause_all_rendering(&mut self, ctx: &mut Ctx<'_>) -> Result<Outcome, ControlError> {
        if ctx.store.global_pause()? {
            return Err(ControlError::FailedPrecondition(
                "rendering is already paused".to_owned(),
            ));
        }
        ctx.store.set_global_pause(true)?;
        for mut job in ctx.store.render_jobs(Some(JobStatus::Running))? {
            let Some(pid) = self.live_pid(&job) else {
                skip_job(ctx, &job, "has no live process")?;
                continue;
            };
            if let Err(err) = process::suspend(pid) {
                skip_job(ctx, &job, &format!("could not be suspended: {err}"))?;
                continue;
            }
            job.status = JobStatus::Suspended;
            job.suspended_by = Some(SuspendedBy::Global);
            ctx.store.put_render_job(&job)?;
        }
        record(ctx, "pause all rendering", None, None)?;
        Ok(Outcome::done())
    }

    /// Resume rendering: global pause is cleared, the renders suspended by Pause rendering continue,
    /// and the queue runs. Renders suspended by the user stay suspended.
    pub fn resume_all_rendering(&mut self, ctx: &mut Ctx<'_>) -> Result<Outcome, ControlError> {
        if !ctx.store.global_pause()? {
            return Err(ControlError::FailedPrecondition(
                "rendering is not paused".to_owned(),
            ));
        }
        for mut job in ctx.store.render_jobs(Some(JobStatus::Suspended))? {
            if job.suspended_by != Some(SuspendedBy::Global) {
                continue;
            }
            let Some(pid) = self.live_pid(&job) else {
                skip_job(ctx, &job, "has no live process; it stays suspended")?;
                continue;
            };
            if let Err(err) = process::resume(pid) {
                skip_job(ctx, &job, &format!("could not be resumed: {err}"))?;
                continue;
            }
            job.status = JobStatus::Running;
            job.suspended_by = None;
            ctx.store.put_render_job(&job)?;
        }
        ctx.store.set_global_pause(false)?;
        record(ctx, "resume all rendering", None, None)?;
        self.promote(ctx)?;
        Ok(Outcome::done())
    }

    /// Dismiss failure: the card goes away. The same failure, when it happens again, makes a new card.
    pub fn dismiss_failure(
        &mut self,
        ctx: &mut Ctx<'_>,
        id: &str,
    ) -> Result<Outcome, ControlError> {
        let failure = ctx
            .store
            .failure(id)?
            .ok_or_else(|| ControlError::NotFound(format!("no failure {id}")))?;
        ctx.store.delete_failure(id)?;
        record(
            ctx,
            &format!("dismiss failure {id}"),
            Some(&failure.sequence_name),
            Some(&failure.output_filename),
        )?;
        Ok(Outcome::done())
    }

    /// Rescan now: asks the caller to run a scan at once.
    pub fn rescan_now(&mut self, ctx: &mut Ctx<'_>) -> Result<Outcome, ControlError> {
        record(ctx, "rescan now", None, None)?;
        Ok(Outcome { rescan: true })
    }

    /// A sequence is known when the last scan saw it, when it is paused (so it can be resumed after it
    /// left the disk), or when a job of it is in flight. Anything else is NOT_FOUND.
    fn require_sequence(
        &self,
        ctx: &Ctx<'_>,
        config: &Config,
        name: &str,
    ) -> Result<(), ControlError> {
        let known = self.known_sequences.contains(name)
            || config.paused_sequence_names.iter().any(|n| n == name)
            || in_flight_jobs(ctx.store)?
                .iter()
                .any(|job| job.sequence_name == name);
        if known {
            Ok(())
        } else {
            Err(ControlError::NotFound(format!("no sequence {name}")))
        }
    }

    /// The pid to signal for a job, or `None`. Only a process this queue holds, or one recorded in the
    /// row whose start time still matches, is signalled: a recycled pid has another start time, and
    /// nothing signals a process the daemon did not start (overview §10 rule 2).
    fn live_pid(&self, job: &RenderJob) -> Option<u32> {
        if let Some(active) = self.running.get(&job.id) {
            return Some(active.process.pid);
        }
        let pid = u32::try_from(job.pid?).ok()?;
        let start = u64::try_from(job.pid_start_time?).ok()?;
        process::is_alive(pid, start).then_some(pid)
    }

    /// Ends a job: a process this queue holds, or one recorded in the job row that is still alive, is
    /// terminated (a suspended one is resumed first, §6.8). Its partial file is deleted and the row
    /// becomes `cancelled`.
    fn cancel_job(&mut self, ctx: &Ctx<'_>, mut job: RenderJob) -> Result<(), ControlError> {
        if let Some(mut active) = self.running.remove(&job.id) {
            let pid = active.process.pid;
            if let Err(err) = process::terminate(&mut active.process, TERMINATE_GRACE) {
                // The process may still be running: its entry goes back, so the next tick reaps it
                // and the row keeps its state.
                self.running.insert(job.id.clone(), active);
                return Err(process_error("end", i64::from(pid), err));
            }
        } else if let (Some(pid), Some(start)) = (job.pid, job.pid_start_time) {
            let live_job = matches!(job.status, JobStatus::Running | JobStatus::Suspended);
            if live_job && process::is_alive(pid as u32, start as u64) {
                let mut recorded = RunningProcess::adopted(pid as u32, start as u64);
                process::terminate(&mut recorded, TERMINATE_GRACE)
                    .map_err(|err| process_error("end", pid, err))?;
            }
        }
        remove_file_quietly(Path::new(&job.partial_path));
        pipeline::finish_job(ctx, &mut job, JobStatus::Cancelled)?;
        Ok(())
    }
}

/// Writes the WARN row of a job that a global pause or resume could not act on. The job keeps its
/// state, and the row says so.
fn skip_job(ctx: &Ctx<'_>, job: &RenderJob, why: &str) -> Result<(), ControlError> {
    let fields = LogFields {
        sequence_name: Some(job.sequence_name.clone()),
        output_filename: Some(job.output_filename.clone()),
        ..LogFields::default()
    };
    let message = format!(
        "render of {} {why}; its state is unchanged",
        job.output_filename
    );
    note(ctx, LogLevel::Warn, &job.sequence_name, &message, fields)?;
    Ok(())
}

/// Adds a variant to `paused_variants` (no-op when it is there already).
fn push_pause(ctx: &mut Ctx<'_>, sequence: &str, variant_key: &str) -> Result<(), ControlError> {
    let (seq, key) = (sequence.to_owned(), variant_key.to_owned());
    edit_config(ctx, move |cfg| {
        if !is_paused_variant(cfg, &seq, &key) {
            cfg.paused_variants.push(PausedVariant {
                sequence_name: seq,
                variant_key: key,
            });
        }
    })
}
