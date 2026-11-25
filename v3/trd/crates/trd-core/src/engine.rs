// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The engine that ties scanning, queueing and rendering together (§6).
//!
//! [`Engine`] owns the state: the store, the config, the tools, the scanner, the render queue, the
//! EXIF histograms, the problems and the snapshot. It runs start-up once, then a tick every
//! `scan_interval_seconds` (or when a control or a config change asks for a scan). A tick checks
//! Full Disk Access, reloads the config, scans, collects missing histograms, queues, promotes and
//! publishes. Controls arrive over an mpsc channel as [`Control`]s and are applied one at a time.
//!
//! The snapshot is published on a `watch` channel as an `Arc<Snapshot>`. Its revision goes up by one
//! on every change of content and stays put otherwise, so a tick that changes nothing publishes
//! nothing.
//!
//! Tests drive the engine directly: [`Engine::startup`], [`Engine::tick`] and [`Engine::apply`] are
//! callable without the run loop, and the clock is injected.
//!
//! An adopted foreign render (§6.11 step 3) is never signalled here. The engine's shutdown only
//! drops its handles, and only [`Control::StopRender`] ends an adopted job.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::adopt::{AdoptPolicy, adopt_at_startup};
use crate::codecs::VideoCodecs;
use crate::config::{
    Config, ConfigStore, PROBLEM_CONFIG_INVALID, PROBLEM_CONFIG_ISSUES, PROBLEM_CONFIG_MISSING,
    PROBLEM_UNKNOWN_CODEC, Status,
};
use crate::controls::ControlError;
use crate::events::EVENT_CONFIG_RELOADED;
use crate::exif::{self, ExifError, Histogram};
use crate::fda::{FdaGuard, FdaState};
use crate::history_import;
use crate::paths::Clock;
use crate::plan;
use crate::problems::{self, Problem, Problems};
use crate::process::ProcessTable;
use crate::queue::{Ctx, Renders, export_path_of, is_paused_variant};
use crate::scan::{ExportSequence, ScanFs, ScanResult, Scanner};
use crate::snapshot::{
    Counts, FailureCard, FileExistsCache, PhaseInputs, RECENT_LIMIT, RenderedVideo as SnapVideo,
    SequenceCard, Snapshot, Stage, StageInputs, VariantRow, VariantStatus, mismatch_note, order,
    phase_of, stage_of,
};
use crate::store::{
    DAY_MS, Failure, JobStatus, LogFields, LogLevel, RenderJob, RenderedVideo, Store, StoreResult,
    log_event,
};
use crate::thumbs::{self, Thumbnailer};
use crate::tools::{ResolvedTool, ToolInfo, Tools};

/// How often Full Disk Access is checked again (§6.12).
pub const FDA_RECHECK: Duration = Duration::from_secs(30);
/// How often the tools are resolved again (§6.15).
pub const TOOLS_RECHECK: Duration = Duration::from_secs(5 * 60);
/// How long a failed histogram collection waits before it is tried again.
pub const EXIF_RETRY: Duration = Duration::from_secs(60);
/// How often retention is applied (§6.13).
const PRUNE_EVERY_MS: i64 = 60 * 60 * 1000;
/// The scan interval when the config is not loaded.
const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// The retention window when the config is not loaded (§6.13).
const DEFAULT_HISTORY_DAYS: i64 = 30;

/// The controls a client sends (§6.8, §6.9). Each one is the matching method of
/// [`Renders`](crate::queue::Renders).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    PauseSequence(String),
    ResumeSequence(String),
    PauseVariant {
        sequence: String,
        variant_key: String,
    },
    ResumeVariant {
        sequence: String,
        variant_key: String,
    },
    SuspendRender(String),
    ResumeRender(String),
    StopRender(String),
    Retry {
        sequence: String,
        variant_key: String,
    },
    PauseAllRendering,
    ResumeAllRendering,
    DismissFailure(String),
    RescanNow,
}

/// Full Disk Access, as the engine asks it (§6.12). The real one is [`MacDiskAccess`]; tests
/// script their own.
pub trait DiskAccess: Send {
    fn check(&self) -> FdaState;
}

/// The real check: [`FdaGuard`] against the home directory.
pub struct MacDiskAccess {
    guard: FdaGuard,
    home: PathBuf,
}

impl MacDiskAccess {
    /// `enabled` is false for `--no-fda-check`.
    pub fn new(enabled: bool, home: impl Into<PathBuf>) -> MacDiskAccess {
        MacDiskAccess {
            guard: FdaGuard::new(enabled),
            home: home.into(),
        }
    }
}

impl DiskAccess for MacDiskAccess {
    fn check(&self) -> FdaState {
        self.guard.check(&self.home)
    }
}

/// Everything an engine is built from.
pub struct EngineOptions {
    pub store: Store,
    pub config_path: PathBuf,
    /// Where the renders' logs and partial files go: `<data_dir>/renders`.
    pub data_dir: PathBuf,
    pub codecs: VideoCodecs,
    /// Resolves the tools. Called at start-up and every [`TOOLS_RECHECK`].
    pub resolve_tools: Box<dyn FnMut() -> Tools + Send>,
    pub disk_access: Box<dyn DiskAccess>,
    pub scan_fs: Arc<dyn ScanFs>,
    /// The monotonic instant and the calendar year.
    pub clock: Arc<dyn Clock>,
    /// The wall-clock time in ms since the Unix epoch.
    pub now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub table: Box<dyn ProcessTable + Send>,
    pub policy: AdoptPolicy,
    pub thumbs: Thumbnailer,
    /// Extra environment for the ffmpeg processes; empty in production.
    pub env: Vec<(String, String)>,
    /// The Swift history file to import once (§6.17), if any.
    pub history_path: Option<PathBuf>,
}

/// A command for the run loop.
enum Command {
    Control {
        control: Control,
        reply: oneshot::Sender<Result<(), ControlError>>,
    },
    Shutdown,
}

/// The client side of an engine: sends controls and reads the snapshot. Cheap to clone.
#[derive(Clone)]
pub struct EngineHandle {
    commands: mpsc::Sender<Command>,
    snapshot: watch::Receiver<Arc<Snapshot>>,
}

impl EngineHandle {
    /// The latest snapshot.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.borrow().clone()
    }

    /// A receiver that wakes on every published snapshot.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshot.clone()
    }

    /// Applies a control on the engine's task and waits for its result.
    pub async fn control(&self, control: Control) -> Result<(), ControlError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Control { control, reply })
            .await
            .map_err(|_| ControlError::Internal("the engine is not running".to_owned()))?;
        answer
            .await
            .unwrap_or_else(|_| Err(ControlError::Internal("the engine stopped".to_owned())))
    }

    /// Asks the run loop to end. The engine drops its handles and signals no process.
    pub async fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown).await;
    }
}

/// The engine: the state, the run loop and the tick.
pub struct Engine {
    store: Store,
    config: ConfigStore,
    codecs: VideoCodecs,
    tools: Tools,
    resolve_tools: Box<dyn FnMut() -> Tools + Send>,
    tools_checked: Option<Instant>,
    disk_access: Box<dyn DiskAccess>,
    fda_checked: Option<Instant>,
    fda_missing: bool,
    scanner: Scanner,
    renders: Renders,
    table: Box<dyn ProcessTable + Send>,
    policy: AdoptPolicy,
    clock: Arc<dyn Clock>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    history_path: Option<PathBuf>,
    /// Set once the tools are resolved.
    started: bool,
    /// Set once the start-up work that touches the configured folders has run (see [`Engine::begin`]).
    begun: bool,
    /// Set once a tick has read the config file (never while Full Disk Access is missing).
    config_checked: bool,
    /// Set once the first tick has run; until then the phase is `PHASE_STARTING`.
    ticked: bool,
    rescan: bool,
    /// The last scan, and when it ran.
    scan: Option<ScanResult>,
    scan_instant: Option<Instant>,
    scan_completed_ms: i64,
    scan_duration_ms: u32,
    /// The histogram of each raw directory, by its path.
    histograms: BTreeMap<String, Histogram>,
    /// Histogram collections in flight, by raw directory.
    exif_jobs: BTreeMap<String, JoinHandle<Result<Histogram, ExifError>>>,
    /// When a failed collection may be tried again, by raw directory.
    exif_retry: BTreeMap<String, Instant>,
    /// When each sequence was first sighted, by name (§6.7 promotion order).
    first_seen: BTreeMap<String, i64>,
    /// The last frame count seen per sequence, and when it changed.
    frame_seen: BTreeMap<String, (u64, i64)>,
    problems: Problems,
    /// The codec names of the rendering entries that the codec table lacks, from the last reload.
    unknown_codecs: Vec<String>,
    file_cache: FileExistsCache,
    prune_due_ms: i64,
    tx: watch::Sender<Arc<Snapshot>>,
    commands: mpsc::Receiver<Command>,
    /// The snapshot last published.
    published: Snapshot,
}

/// Builds the [`Ctx`] of a call from the engine's fields, so the borrows stay disjoint.
macro_rules! ctx {
    ($engine:expr, $now:expr) => {
        Ctx {
            store: &$engine.store,
            config: &mut $engine.config,
            codecs: &$engine.codecs,
            tools: &$engine.tools,
            now_ms: $now,
            year: $engine.clock.year(),
        }
    };
}

impl Engine {
    /// An engine and its handle. The engine publishes the `PHASE_STARTING` snapshot (revision 1) at
    /// once; nothing is read or started until [`Engine::startup`] or [`Engine::run`].
    pub fn new(options: EngineOptions) -> (Engine, EngineHandle) {
        let (commands_tx, commands_rx) = mpsc::channel(64);
        let config_path = options.config_path.to_string_lossy().into_owned();
        let initial = Snapshot {
            revision: 1,
            generated_at_ms: 0,
            phase: crate::snapshot::Phase::Starting,
            global_pause: false,
            problems: Vec::new(),
            failures: Vec::new(),
            in_progress: Vec::new(),
            recently_rendered: Vec::new(),
            recently_rendered_total: 0,
            counts: Counts::default(),
            last_scan_completed_ms: 0,
            last_scan_duration_ms: 0,
            config_path,
        };
        let (tx, rx) = watch::channel(Arc::new(initial.clone()));
        let scanner = Scanner::new(options.scan_fs, options.clock.clone());
        let renders = Renders::new(&options.data_dir, options.thumbs).with_env(options.env);
        let engine = Engine {
            config: ConfigStore::new(&options.config_path, options.clock.clone()),
            store: options.store,
            codecs: options.codecs,
            tools: unresolved_tools(),
            resolve_tools: options.resolve_tools,
            tools_checked: None,
            disk_access: options.disk_access,
            fda_checked: None,
            fda_missing: false,
            scanner,
            renders,
            table: options.table,
            policy: options.policy,
            clock: options.clock,
            now_ms: options.now_ms,
            history_path: options.history_path,
            started: false,
            begun: false,
            config_checked: false,
            ticked: false,
            rescan: false,
            scan: None,
            scan_instant: None,
            scan_completed_ms: 0,
            scan_duration_ms: 0,
            histograms: BTreeMap::new(),
            exif_jobs: BTreeMap::new(),
            exif_retry: BTreeMap::new(),
            first_seen: BTreeMap::new(),
            frame_seen: BTreeMap::new(),
            problems: Problems::new(),
            unknown_codecs: Vec::new(),
            file_cache: FileExistsCache::default(),
            prune_due_ms: 0,
            tx,
            commands: commands_rx,
            published: initial,
        };
        let handle = EngineHandle {
            commands: commands_tx,
            snapshot: rx,
        };
        (engine, handle)
    }

    /// The last published snapshot.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.tx.borrow().clone()
    }

    /// The render queue, for reading its state.
    pub fn renders(&self) -> &Renders {
        &self.renders
    }

    /// Runs the engine until it is shut down or every handle is dropped. Start-up runs first.
    pub async fn run(mut self) {
        self.tick().await;
        loop {
            let wait = self.scan_interval();
            let next = tokio::select! {
                command = self.commands.recv() => Some(command),
                _ = tokio::time::sleep(wait) => None,
            };
            match next {
                None => self.tick().await,
                Some(None) | Some(Some(Command::Shutdown)) => break,
                Some(Some(Command::Control { control, reply })) => {
                    let result = self.apply(control);
                    let _ = reply.send(result);
                }
            }
        }
    }

    /// Start-up, first half (§6.15): resolves the tools. Runs once; later calls return at once.
    /// The rest of start-up is [`Engine::begin`], which waits for Full Disk Access and a config.
    pub async fn startup(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let instant = self.clock.instant();
        self.tools = (self.resolve_tools)();
        self.tools_checked = Some(instant);
    }

    /// Start-up, second half (§6.6, §6.11, §6.17): re-adopts the renders the database records and the
    /// foreign ffmpeg processes the policy allows, imports the Swift history once and sweeps stale
    /// partial files. Each step reads the configured folders or needs the config's settings, so it
    /// runs once, at the first tick that has Full Disk Access and a loaded config (§6.12).
    fn begin(&mut self, now: i64) {
        self.begun = true;
        let report = {
            let mut ctx = ctx!(self, now);
            adopt_at_startup(self.policy, &mut ctx, &mut self.renders, &mut *self.table)
        };
        match report {
            Ok(report) => tracing::info!(
                adopted_own = report.own.adopted,
                adopted_foreign = report.foreign,
                "start-up adoption done"
            ),
            Err(err) => tracing::warn!(error = %err, "start-up adoption failed"),
        }

        if let Some(path) = self.history_path.clone() {
            let days = self.history_days();
            if let Err(err) = history_import::import_swift_history(&path, &self.store, days, now) {
                tracing::warn!(error = %err, "the Swift render history could not be imported");
            }
        }

        {
            let mut ctx = ctx!(self, now);
            if let Err(err) = self.renders.startup(&mut ctx) {
                tracing::warn!(error = %err, "the start-up sweep failed");
            }
        }
    }

    /// One pass of the engine (§6.1, §6.3): tools if due, then [`Engine::run_tick`], then publish.
    /// A failure inside the pass is logged and the snapshot still publishes.
    pub async fn tick(&mut self) {
        self.startup().await;
        let now = self.now();
        let instant = self.clock.instant();
        if self.tools_due(instant) {
            self.tools = (self.resolve_tools)();
            self.tools_checked = Some(instant);
        }
        if let Err(err) = self.run_tick(now, instant).await {
            tracing::warn!(error = %err, "the engine tick failed");
        }
        self.ticked = true;
        self.publish(now);
    }

    /// Applies one control (§6.8, §6.9) and publishes. A control that does not apply changes nothing
    /// and returns its error. A control that asks for a scan sets the next tick to scan.
    pub fn apply(&mut self, control: Control) -> Result<(), ControlError> {
        let now = self.now();
        let outcome = {
            let mut ctx = ctx!(self, now);
            match control {
                Control::PauseSequence(name) => self.renders.pause_sequence(&mut ctx, &name),
                Control::ResumeSequence(name) => self.renders.resume_sequence(&mut ctx, &name),
                Control::PauseVariant {
                    sequence,
                    variant_key,
                } => self
                    .renders
                    .pause_variant(&mut ctx, &sequence, &variant_key),
                Control::ResumeVariant {
                    sequence,
                    variant_key,
                } => self
                    .renders
                    .resume_variant(&mut ctx, &sequence, &variant_key),
                Control::SuspendRender(id) => self.renders.suspend_render(&mut ctx, &id),
                Control::ResumeRender(id) => self.renders.resume_render(&mut ctx, &id),
                Control::StopRender(id) => self.renders.stop_render(&mut ctx, &id),
                Control::Retry {
                    sequence,
                    variant_key,
                } => self.renders.retry(&mut ctx, &sequence, &variant_key),
                Control::PauseAllRendering => self.renders.pause_all_rendering(&mut ctx),
                Control::ResumeAllRendering => self.renders.resume_all_rendering(&mut ctx),
                Control::DismissFailure(id) => self.renders.dismiss_failure(&mut ctx, &id),
                Control::RescanNow => self.renders.rescan_now(&mut ctx),
            }
        };
        let result = match outcome {
            Ok(outcome) => {
                if outcome.rescan {
                    self.rescan = true;
                }
                Ok(())
            }
            Err(err) => Err(err),
        };
        self.publish(now);
        result
    }

    /// Ends the engine. Its handles are dropped; no process is signalled, adopted renders included.
    pub fn shutdown(self) {}

    fn now(&self) -> i64 {
        (self.now_ms)()
    }

    fn tools_due(&self, instant: Instant) -> bool {
        self.tools_checked
            .is_none_or(|at| instant.saturating_duration_since(at) >= TOOLS_RECHECK)
    }

    fn fda_due(&self, instant: Instant) -> bool {
        self.fda_checked
            .is_none_or(|at| instant.saturating_duration_since(at) >= FDA_RECHECK)
    }

    fn scan_interval(&self) -> Duration {
        self.config
            .config()
            .map_or(DEFAULT_SCAN_INTERVAL, |config| {
                Duration::from_secs_f64(config.scan_interval_seconds.max(0.5))
            })
    }

    fn history_days(&self) -> i64 {
        self.config
            .config()
            .map_or(DEFAULT_HISTORY_DAYS, |c| c.recent_render_history_days)
    }

    fn scan_due(&self, instant: Instant, interval: Duration) -> bool {
        self.scan.is_none()
            || self
                .scan_instant
                .is_none_or(|at| instant.saturating_duration_since(at) >= interval)
    }

    /// The body of one tick (§6.1, §6.3, §6.11, §6.12).
    async fn run_tick(&mut self, now: i64, instant: Instant) -> StoreResult<()> {
        if self.fda_due(instant) {
            self.fda_missing = self.disk_access.check() == FdaState::Missing;
            self.fda_checked = Some(instant);
        }
        if self.fda_missing {
            // Nothing is read, scanned, reaped, promoted or collected while access is missing (§6.12).
            // Loading the config stats the configured folders, so it waits too. A render that is
            // running keeps running; its completion is picked up once access is back.
            self.scan = None;
            return Ok(());
        }

        if self.config.reload_if_changed(&self.codecs) {
            self.rescan = true;
            self.unknown_codecs = self.read_unknown_codecs();
            if matches!(self.config.status(), Status::Loaded) {
                self.log_config_reloaded(now)?;
            }
        }
        self.config_checked = true;
        let Some(config) = self.config.config().cloned() else {
            self.scan = None;
            self.poll_and_promote(now)?;
            return Ok(());
        };
        if !self.begun {
            self.begin(now);
        }

        let interval = self.scan_interval();
        let due = self.rescan || self.scan_due(instant, interval);
        if due {
            self.rescan = false;
            self.scan_now(&config, now, instant).await;
        }
        let collected = self.collect_histograms(&config, now, instant).await?;
        if due || collected {
            self.queue(now)?;
        }
        self.poll_and_promote(now)?;
        self.apply_retention(now);
        Ok(())
    }

    /// One scan (§6.3): backups in the background when due, the scan, then the sizes.
    async fn scan_now(&mut self, config: &Config, now: i64, instant: Instant) {
        let started = self.clock.instant();
        if !config.backup_paths.is_empty() && self.scanner.backups_due() {
            let scanner = self.scanner.clone();
            let paths = config.backup_paths.clone();
            tokio::spawn(async move { scanner.refresh_backups(&paths).await });
        }
        let result = self.scanner.scan(config).await;
        self.scanner.measure_sizes().await;
        let duration = self.clock.instant().saturating_duration_since(started);
        self.scan_duration_ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);

        for seq in &result.exports {
            self.first_seen.entry(seq.name.clone()).or_insert(now);
            match self.frame_seen.get(&seq.name) {
                Some((frames, _)) if *frames == seq.frames => {}
                _ => {
                    self.frame_seen.insert(seq.name.clone(), (seq.frames, now));
                }
            }
        }
        self.scan = Some(result);
        self.scan_instant = Some(instant);
        self.scan_completed_ms = now;
    }

    /// Queues what the last scan wants (§6.7). A sequence whose export has more frames than its raw
    /// sequence is not handed over, so it never renders.
    fn queue(&mut self, now: i64) -> StoreResult<()> {
        if self.config.config().is_none() {
            return Ok(());
        }
        let Some(scan) = self.scan.as_ref() else {
            return Ok(());
        };
        let queueable = ScanResult {
            raw: scan.raw.clone(),
            exports: scan
                .exports
                .iter()
                .filter(|seq| seq.frames <= seq.raw_frames)
                .cloned()
                .collect(),
            unreachable: Vec::new(),
        };
        let mut ctx = ctx!(self, now);
        self.renders
            .on_scan(&mut ctx, &queueable, &self.histograms, &self.first_seen)
    }

    /// Reaps finished histogram collections, stores them, and starts one for each complete sequence
    /// that has no histogram (§6.4). Returns whether a histogram became known.
    async fn collect_histograms(
        &mut self,
        config: &Config,
        now: i64,
        instant: Instant,
    ) -> StoreResult<bool> {
        let mut changed = false;
        let finished: Vec<String> = self
            .exif_jobs
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(raw_dir, _)| raw_dir.clone())
            .collect();
        for raw_dir in finished {
            let Some(handle) = self.exif_jobs.remove(&raw_dir) else {
                continue;
            };
            match handle.await {
                Ok(Ok(histogram)) => {
                    let sidecar = exif::sidecar_path(Path::new(&raw_dir));
                    if let Err(err) = exif::write_sidecar(
                        &histogram,
                        &sidecar,
                        Path::new(&raw_dir),
                        &self.store,
                        now,
                    ) {
                        tracing::warn!(raw_dir = %raw_dir, error = %err, "the EXIF histogram could not be saved");
                    }
                    self.histograms.insert(raw_dir, histogram);
                    changed = true;
                }
                Ok(Err(err)) => {
                    tracing::warn!(raw_dir = %raw_dir, error = %err, "the EXIF collection failed");
                    self.exif_retry.insert(raw_dir, instant + EXIF_RETRY);
                }
                Err(err) => {
                    tracing::warn!(raw_dir = %raw_dir, error = %err, "the EXIF collection ended abnormally");
                    self.exif_retry.insert(raw_dir, instant + EXIF_RETRY);
                }
            }
        }

        let wanted: Vec<String> = match self.scan.as_ref() {
            None => return Ok(changed),
            Some(scan) => scan
                .exports
                .iter()
                .filter(|seq| seq.frames == seq.raw_frames)
                .filter_map(|seq| raw_dir_of(scan, &seq.raw_name))
                .collect(),
        };
        let max_processes = usize::try_from(config.max_processes.exiftool)
            .unwrap_or(1)
            .max(1);
        for raw_dir in wanted {
            if self.histograms.contains_key(&raw_dir)
                || self.exif_jobs.contains_key(&raw_dir)
                || self
                    .exif_retry
                    .get(&raw_dir)
                    .is_some_and(|at| instant < *at)
            {
                continue;
            }
            let path = Path::new(&raw_dir);
            match exif::read_histogram(&exif::sidecar_path(path), path, &self.store) {
                Ok(Some(histogram)) => {
                    self.histograms.insert(raw_dir, histogram);
                    changed = true;
                    continue;
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(raw_dir = %raw_dir, error = %err, "the EXIF histogram cannot be read");
                    continue;
                }
            }
            let tool: ResolvedTool = self.tools.exiftool.clone();
            if tool.command.is_none() {
                continue;
            }
            let dir = PathBuf::from(&raw_dir);
            let raw_types = config.raw_types.clone();
            let handle = tokio::task::spawn_blocking(move || {
                exif::collect(&dir, &raw_types, max_processes, &tool)
            });
            self.exif_jobs.insert(raw_dir, handle);
        }
        Ok(changed)
    }

    /// Promotes queued jobs and reaps finished renders (§6.7, §6.11).
    fn poll_and_promote(&mut self, now: i64) -> StoreResult<()> {
        let mut ctx = ctx!(self, now);
        self.renders.tick(&mut ctx, &mut *self.table)
    }

    /// Applies the retention of §6.13 hourly, once a config is loaded: its window is the config's, so
    /// no pass runs on the default window before the config says what it is.
    fn apply_retention(&mut self, now: i64) {
        if self.config.config().is_none() || now < self.prune_due_ms {
            return;
        }
        self.prune_due_ms = now + PRUNE_EVERY_MS;
        if let Err(err) = thumbs::prune_history(&self.store, now, self.history_days()) {
            tracing::warn!(error = %err, "the retention pass failed");
        }
    }

    /// The codec names of the `rendering` entries of config.json that the codec table lacks. The
    /// loader drops those entries, so the names are read from the file. Empty unless the loader
    /// raised `PROBLEM_UNKNOWN_CODEC`.
    fn read_unknown_codecs(&self) -> Vec<String> {
        if !self.config.problem_ids().contains(&PROBLEM_UNKNOWN_CODEC) {
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(self.config.path()) else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Vec::new();
        };
        let mut codecs: Vec<String> = value
            .get("rendering")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("codec").and_then(serde_json::Value::as_str))
            .filter(|codec| !self.codecs.contains_key(*codec))
            .map(str::to_owned)
            .collect();
        codecs.sort();
        codecs.dedup();
        codecs
    }

    fn log_config_reloaded(&self, now: i64) -> StoreResult<()> {
        let fields = LogFields {
            event: Some(EVENT_CONFIG_RELOADED.to_owned()),
            ..LogFields::default()
        };
        let message = format!("config.json reloaded from {}", self.config.path().display());
        log_event(
            &self.store,
            now,
            LogLevel::Info,
            "config",
            &message,
            &fields,
        )?;
        Ok(())
    }

    /// The problems that hold now (§6.10). The set keeps their `since_ms`.
    fn reported_problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if self.fda_missing {
            out.push(problems::fda_problem());
        }
        for info in self.tools.infos() {
            if let Some(problem) = &info.problem {
                out.push(problems::tool_problem(info.name, problem));
            }
        }
        let path = self.config.path().to_string_lossy().into_owned();
        // A config that has not been read yet is not reported as missing.
        let ids = if self.config_checked {
            self.config.problem_ids()
        } else {
            Vec::new()
        };
        if ids.contains(&PROBLEM_CONFIG_MISSING) {
            out.push(problems::config_problem(true, &path));
        }
        if ids.contains(&PROBLEM_CONFIG_INVALID) {
            out.push(problems::config_problem(false, &path));
        }
        if ids.contains(&PROBLEM_CONFIG_ISSUES) {
            let issues = self.config.issues();
            let messages = issues
                .iter()
                .map(|issue| format!("{}: {}", issue.field, issue.message))
                .collect();
            let fields = issues.iter().map(|issue| issue.field.clone()).collect();
            let error = issues.iter().any(|issue| issue.is_error());
            out.push(problems::config_issues_problem(
                &path, messages, fields, error,
            ));
        }
        for codec in &self.unknown_codecs {
            out.push(problems::codec_problem(codec));
        }
        if let Some(scan) = &self.scan {
            for problem in &scan.unreachable {
                out.push(problems::path_problem(problem));
            }
        }
        out
    }

    /// Builds the snapshot from the state (§6.10). Reconciles the problem set as a side effect.
    fn build(&mut self, now: i64) -> StoreResult<Snapshot> {
        let instant = self.clock.instant();
        let year = self.clock.year();
        let reported = self.reported_problems();
        self.problems.reconcile(now, reported);

        let config = self.config.config().cloned();
        let jobs = self.store.render_jobs(None)?;
        let mut failures = self.store.failures()?;
        let global_pause = self.store.global_pause()?;
        let mut rows = self.store.rendered_videos()?;
        rows.sort_by(|a, b| b.rendered_at.cmp(&a.rendered_at).then(a.id.cmp(&b.id)));

        let mut cards = match &config {
            Some(config) => self.cards(config, &jobs, &failures, now, year),
            None => Vec::new(),
        };
        order(&mut cards);

        let phase = phase_of(PhaseInputs {
            started: self.ticked,
            config_loaded: config.is_some(),
            fda_missing: self.fda_missing,
            global_pause,
            job_running: jobs.iter().any(|job| job.status == JobStatus::Running),
            sequence_exporting: cards.iter().any(|card| card.stage == Stage::Exporting),
        });

        let counts = Counts {
            rendering: count_jobs(&jobs, JobStatus::Running),
            suspended: count_jobs(&jobs, JobStatus::Suspended),
            queued: count_jobs(&jobs, JobStatus::Queued),
            exporting: count(&cards, |card| card.stage == Stage::Exporting),
            paused_sequences: count(&cards, |card| card.paused),
            failures: u32::try_from(failures.len()).unwrap_or(u32::MAX),
            rendered_last_24h: u32::try_from(
                rows.iter()
                    .filter(|row| row.rendered_at >= now - DAY_MS)
                    .count(),
            )
            .unwrap_or(u32::MAX),
        };

        let days = config
            .as_ref()
            .map_or(DEFAULT_HISTORY_DAYS, |c| c.recent_render_history_days);
        let cutoff = now - days * DAY_MS;
        let recently_rendered_total =
            u32::try_from(rows.iter().filter(|row| row.rendered_at >= cutoff).count())
                .unwrap_or(u32::MAX);
        let recent: Vec<&RenderedVideo> = rows.iter().take(RECENT_LIMIT).collect();
        let mut kept_paths = Vec::with_capacity(recent.len());
        let mut recently_rendered = Vec::with_capacity(recent.len());
        for row in recent {
            // While access is missing no output is probed: the last answer held stands, or none is
            // known and the row shows `false` until access is back.
            let file_exists = if self.fda_missing {
                self.file_cache.held(&row.output_path).unwrap_or(false)
            } else {
                self.file_cache
                    .check(&row.output_path, instant, |path| Path::new(path).exists())
            };
            kept_paths.push(row.output_path.clone());
            recently_rendered.push(snap_video(row, file_exists));
        }
        self.file_cache.retain(&kept_paths);

        failures.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));

        Ok(Snapshot {
            revision: 0,
            generated_at_ms: now,
            phase,
            global_pause,
            problems: self.problems.list(),
            failures: failures.iter().map(failure_card).collect(),
            in_progress: cards,
            recently_rendered,
            recently_rendered_total,
            counts,
            last_scan_completed_ms: self.scan_completed_ms,
            last_scan_duration_ms: self.scan_duration_ms,
            config_path: self.config.path().to_string_lossy().into_owned(),
        })
    }

    /// The sequence cards of the last scan that are not finished (§6.10).
    fn cards(
        &self,
        config: &Config,
        jobs: &[RenderJob],
        failures: &[Failure],
        now: i64,
        year: i32,
    ) -> Vec<SequenceCard> {
        let Some(scan) = self.scan.as_ref() else {
            return Vec::new();
        };
        let mut cards = Vec::new();
        for seq in &scan.exports {
            let export_path = export_path_of(seq);
            let mismatch = seq.frames > seq.raw_frames;
            let complete = !seq.in_progress && !mismatch;
            let raw = scan.raw.iter().find(|raw| raw.name == seq.raw_name);
            let has_histogram = raw_dir_of(scan, &seq.raw_name)
                .is_some_and(|dir| self.histograms.contains_key(&dir));
            let paused = config.paused_sequence_names.contains(&seq.name);
            let variants = self.variants(
                config,
                seq,
                &export_path,
                paused,
                complete && has_histogram,
                jobs,
                failures,
                year,
            );
            if variants.iter().all(|row| row.status.is_finished()) {
                continue;
            }
            let stage = stage_of(StageInputs {
                paused,
                count_mismatch: mismatch,
                exporting: seq.in_progress,
                rendering: variants.iter().any(|row| {
                    matches!(
                        row.status,
                        VariantStatus::Running | VariantStatus::Suspended
                    )
                }),
                queued: variants
                    .iter()
                    .any(|row| row.status == VariantStatus::Queued),
                collecting: complete && !has_histogram,
            });
            cards.push(SequenceCard {
                id: format!("{export_path}/{}", seq.name),
                name: seq.name.clone(),
                export_path: export_path.clone(),
                raw_sequence_name: seq.raw_name.clone(),
                raw_sequence_location: raw.map(|r| r.parent.clone()).unwrap_or_default(),
                raw_from_backup: raw.is_some_and(|r| r.from_backup),
                expected_frames: u32::try_from(seq.raw_frames).unwrap_or(u32::MAX),
                exported_frames: u32::try_from(seq.frames).unwrap_or(u32::MAX),
                stage,
                paused,
                export_frames_per_minute: seq.export_frames_per_minute,
                export_eta_seconds: seq.export_eta_seconds,
                export_size_bytes: seq.export_size,
                first_seen_ms: self.first_seen.get(&seq.name).copied().unwrap_or(now),
                last_frame_ms: self.frame_seen.get(&seq.name).map_or(0, |(_, at)| *at),
                variants,
                note: if mismatch {
                    mismatch_note(seq.frames, seq.raw_frames)
                } else {
                    String::new()
                },
            });
        }
        cards
    }

    /// The variant rows of one sequence, in the config's `rendering` order (§6.10).
    #[allow(clippy::too_many_arguments)]
    fn variants(
        &self,
        config: &Config,
        seq: &ExportSequence,
        export_path: &str,
        paused_sequence: bool,
        ready: bool,
        jobs: &[RenderJob],
        failures: &[Failure],
        year: i32,
    ) -> Vec<VariantRow> {
        let mut rows = Vec::with_capacity(config.rendering.len());
        for render in &config.rendering {
            let variant_key = render.variant_key();
            let codec_known = self.codecs.contains_key(&render.codec);
            let filename = if codec_known {
                plan::output_filename(config, &self.codecs, render, &seq.name)
            } else {
                String::new()
            };
            let output_path = if filename.is_empty() {
                String::new()
            } else {
                format!("{export_path}/{filename}")
            };
            let mut row = VariantRow {
                variant_key: variant_key.clone(),
                codec: render.codec.clone(),
                resolution_label: render.resolution_label(),
                frame_rate: u32::try_from(render.frame_rate).unwrap_or(0),
                output_filename: filename.clone(),
                output_path: output_path.clone(),
                status: VariantStatus::Waiting,
                render_id: String::new(),
                frame: 0,
                total_frames: seq.raw_frames,
                fps: 0.0,
                eta_seconds: -1,
                progress_known: false,
                adopted: false,
                size_bytes: -1,
                message: String::new(),
                failure_id: String::new(),
            };
            if !codec_known {
                row.status = VariantStatus::UnknownCodec;
                rows.push(row);
                continue;
            }

            let in_flight = jobs.iter().find(|job| {
                job.output_path == output_path
                    && matches!(
                        job.status,
                        JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
                    )
            });
            if let Some(job) = in_flight {
                row.render_id = job.id.clone();
                row.adopted = job.adopted;
                row.status = match job.status {
                    JobStatus::Queued => VariantStatus::Queued,
                    JobStatus::Suspended => VariantStatus::Suspended,
                    _ => VariantStatus::Running,
                };
                if job.foreign {
                    // An adopted render has no progress file: its row shows indeterminate progress.
                    row.progress_known = false;
                } else if let Some(block) = self.renders.progress(&job.id) {
                    row.frame = block.frame;
                    row.fps = block.fps;
                    row.progress_known = true;
                    row.eta_seconds = eta_seconds(block.frame, seq.raw_frames, block.fps);
                }
                rows.push(row);
                continue;
            }
            if let Some(job) = jobs
                .iter()
                .find(|job| job.output_path == output_path && job.status == JobStatus::Done)
            {
                row.status = VariantStatus::Done;
                row.render_id = job.id.clone();
                row.size_bytes = file_size(&output_path);
                rows.push(row);
                continue;
            }
            if let Some(failure) = failures
                .iter()
                .find(|failure| failure.output_path == output_path)
            {
                row.status = VariantStatus::Failed;
                row.failure_id = failure.id.clone();
                row.message = failure.summary.clone();
                rows.push(row);
                continue;
            }
            if paused_sequence || is_paused_variant(config, &seq.name, &variant_key) {
                row.status = VariantStatus::Paused;
                rows.push(row);
                continue;
            }
            if plan::already_rendered(config, export_path, &filename, &[], year) {
                row.status = VariantStatus::AlreadyRendered;
                row.size_bytes = file_size(&output_path);
                rows.push(row);
                continue;
            }
            row.status = if ready {
                VariantStatus::Queued
            } else {
                VariantStatus::Waiting
            };
            rows.push(row);
        }
        rows
    }

    /// Publishes the snapshot built from the state, if its content changed.
    fn publish(&mut self, now: i64) {
        let mut next = match self.build(now) {
            Ok(next) => next,
            Err(err) => {
                tracing::warn!(error = %err, "the snapshot could not be built");
                return;
            }
        };
        if next.same_content(&self.published) {
            return;
        }
        next.revision = self.published.revision + 1;
        self.published = next.clone();
        self.tx.send_replace(Arc::new(next));
    }
}

/// The histogram key of a raw sequence: the raw directory, `<parent>/<name>`.
fn raw_dir_of(scan: &ScanResult, raw_name: &str) -> Option<String> {
    scan.raw.iter().find(|raw| raw.name == raw_name).map(|raw| {
        Path::new(&raw.parent)
            .join(&raw.name)
            .to_string_lossy()
            .into_owned()
    })
}

/// The tools before the first resolution: none is found.
fn unresolved_tools() -> Tools {
    let unresolved = |name: &'static str| ResolvedTool {
        info: ToolInfo {
            name,
            path: None,
            version: String::new(),
            available: false,
            problem: None,
        },
        command: None,
    };
    Tools {
        ffmpeg: unresolved("ffmpeg"),
        ffprobe: unresolved("ffprobe"),
        exiftool: unresolved("exiftool"),
    }
}

/// Seconds until a render at `frame` of `total` frames ends at `fps`; -1 when unknown.
fn eta_seconds(frame: u64, total: u64, fps: f64) -> i64 {
    if fps > 0.0 && total > frame {
        ((total - frame) as f64 / fps) as i64
    } else {
        -1
    }
}

/// The size of a file in bytes; -1 when it cannot be read.
fn file_size(path: &str) -> i64 {
    std::fs::metadata(path).map_or(-1, |meta| meta.len() as i64)
}

fn count_jobs(jobs: &[RenderJob], status: JobStatus) -> u32 {
    u32::try_from(jobs.iter().filter(|job| job.status == status).count()).unwrap_or(u32::MAX)
}

fn count<T>(items: &[T], keep: impl Fn(&T) -> bool) -> u32 {
    u32::try_from(items.iter().filter(|item| keep(item)).count()).unwrap_or(u32::MAX)
}

fn snap_video(row: &RenderedVideo, file_exists: bool) -> SnapVideo {
    SnapVideo {
        id: row.id.clone(),
        rendered_at_ms: row.rendered_at,
        sequence_name: row.sequence_name.clone(),
        export_path: row.export_path.clone(),
        output_filename: row.output_filename.clone(),
        output_path: row.output_path.clone(),
        variant_key: row.variant_key.clone(),
        codec: row.codec.clone(),
        resolution_label: row.resolution_label.clone(),
        frame_rate: row.frame_rate,
        frame_count: row.frame_count,
        size_bytes: row.size_bytes,
        width: row.width,
        height: row.height,
        duration_seconds: row.duration_seconds,
        render_seconds: row.render_seconds as i64,
        has_thumbnail: row.thumbnail_path.is_some(),
        file_exists,
        xmp_embedded: row.xmp_embedded,
        source: row.source.clone(),
    }
}

fn failure_card(failure: &Failure) -> FailureCard {
    FailureCard {
        id: failure.id.clone(),
        kind: failure.kind.clone(),
        summary: failure.summary.clone(),
        detail: failure.detail.clone(),
        sequence_name: failure.sequence_name.clone(),
        output_filename: failure.output_filename.clone(),
        output_path: failure.output_path.clone(),
        variant_key: failure.variant_key.clone(),
        first_seen_ms: failure.first_seen,
        last_seen_ms: failure.last_seen,
        occurrences: failure.occurrences,
    }
}
