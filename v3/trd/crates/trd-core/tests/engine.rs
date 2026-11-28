// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the engine loop, the snapshot, the problems and the events (TRD-16, spec 1 §6.1, §6.3,
//! §6.10–§6.12, §6.14).
//!
//! Unix only: the EXIF step and the renders run stand-in tools. Every test works in its own
//! temporary directory with the fake tools copied into it, a manual clock and a manual wall clock,
//! and drives the engine tick by tick. Nothing reads the owner's config, data directory or ports,
//! and the real process table is never used: each test lists its processes in a
//! `FakeProcessTable`. The only waits are for a fake ffmpeg or an EXIF stand-in to be released, under
//! a hang guard; no test asserts how long anything takes.

#![cfg(unix)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use trd_core::adopt::AdoptPolicy;
use trd_core::codecs;
use trd_core::config::{Config, MaxProcesses, PausedVariant, RenderType};
use trd_core::controls::ControlError;
use trd_core::engine::{Control, DiskAccess, Engine, EngineHandle, EngineOptions};
use trd_core::events::{EVENT_CONFIG_RELOADED, EVENT_RENDER_FINISHED};
use trd_core::fda::FdaState;
use trd_core::paths::{Clock, ManualClock};
use trd_core::process::{self, FakeProcessTable, ProcessEntry, ProcessState, RunningProcess};
use trd_core::scan::{DirItem, RealFs, ScanFs};
use trd_core::snapshot::{Phase, SequenceCard, Severity, Snapshot, Stage, VariantStatus};
use trd_core::store::{JobStatus, LogRow, RenderedVideo as StoredVideo, Store};
use trd_core::thumbs::Thumbnailer;
use trd_core::tools::{ResolvedTool, ToolCommand, ToolInfo, ToolProblem, Tools};
use trd_testtools::FakeTools;

/// The calendar year the fixtures use for `$YEAR`.
const YEAR: i32 = 2026;
/// Stops a test that waits for something that never happens. It is not a timing assertion.
const HANG_GUARD: Duration = Duration::from_secs(60);
/// Pause between two ticks of a wait.
const POLL: Duration = Duration::from_millis(20);
/// How far the manual clock moves on each [`World::tick`]; more than the 2 s scan interval.
const TICK_STEP: Duration = Duration::from_secs(3);
/// The name of every sequence's export directory prefix (`image_sequence_dirname_prefix`).
const PREFIX: &str = "LRT_";

/// The fixture's variant: ProRes at 1280 x 720.
fn pro_res() -> RenderType {
    RenderType {
        codec: "ProResUHQ".to_owned(),
        frame_rate: 25,
        resolution: Some(vec![1280, 720]),
    }
}

/// A render type whose codec the table does not have: loaded from disk it is skipped, and
/// `codec:NoSuchCodec` is reported.
fn unknown_codec() -> RenderType {
    RenderType {
        codec: "NoSuchCodec".to_owned(),
        frame_rate: 25,
        resolution: None,
    }
}

/// What the fixture starts with. The default has a config, no histogram yet, ffmpeg present, one
/// ProRes variant, and no foreign render.
struct Opts {
    config: bool,
    exif_released: bool,
    ffmpeg_present: bool,
    rendering: Vec<RenderType>,
    paused_sequences: Vec<String>,
    paused_variants: Vec<(String, String)>,
    foreign: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            config: true,
            exif_released: false,
            ffmpeg_present: true,
            rendering: vec![pro_res()],
            paused_sequences: Vec::new(),
            paused_variants: Vec::new(),
            foreign: false,
        }
    }
}

/// The directories and tool paths of one fixture. Cloned into every engine a test builds.
#[derive(Clone)]
struct Paths {
    export: PathBuf,
    export_other: PathBuf,
    source: PathBuf,
    master: PathBuf,
    data: PathBuf,
    config: PathBuf,
    release_exif: PathBuf,
    release_render: PathBuf,
    fake_bin: PathBuf,
}

/// The state the engine's injected seams read. The test moves it through the fixture.
#[derive(Clone)]
struct Shared {
    disk: Arc<Mutex<FdaState>>,
    ffmpeg_present: Arc<AtomicBool>,
    clock: Arc<ManualClock>,
    now: Arc<AtomicI64>,
    blocked: Arc<AtomicBool>,
    touched: Arc<AtomicUsize>,
    lists: Arc<AtomicUsize>,
}

/// Full Disk Access as the test sets it.
struct ScriptedDisk(Arc<Mutex<FdaState>>);

impl DiskAccess for ScriptedDisk {
    fn check(&self) -> FdaState {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The real filesystem, except while `blocked` is set: then every call counts as a touch and fails.
struct GuardedFs {
    blocked: Arc<AtomicBool>,
    touched: Arc<AtomicUsize>,
    lists: Arc<AtomicUsize>,
}

impl GuardedFs {
    fn gate(&self) -> io::Result<()> {
        if self.blocked.load(Ordering::SeqCst) {
            self.touched.fetch_add(1, Ordering::SeqCst);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"));
        }
        Ok(())
    }
}

impl ScanFs for GuardedFs {
    fn list(&self, dir: &Path) -> io::Result<Vec<DirItem>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.gate()?;
        RealFs.list(dir)
    }

    fn mtime(&self, dir: &Path) -> io::Result<std::time::SystemTime> {
        self.gate()?;
        RealFs.mtime(dir)
    }

    fn size(&self, dir: &Path) -> io::Result<u64> {
        self.gate()?;
        RealFs.size(dir)
    }

    fn read(&self, file: &Path) -> io::Result<Vec<u8>> {
        self.gate()?;
        RealFs.read(file)
    }
}

/// A tool that is there, or missing with no paths tried.
fn tool(name: &'static str, command: Option<ToolCommand>) -> ResolvedTool {
    let problem = command
        .is_none()
        .then(|| ToolProblem::Missing { tried: Vec::new() });
    ResolvedTool {
        info: ToolInfo {
            name,
            path: None,
            version: String::new(),
            available: command.is_some(),
            problem,
        },
        command,
    }
}

/// A stand-in exiftool: a shell script that waits until `release` exists, then prints one CSV row,
/// so every collected histogram names one tag value. The script ignores its arguments.
fn exif_tool(release: &Path) -> ToolCommand {
    let script = format!(
        "while [ ! -f '{}' ]; do sleep 0.05; done; printf 'SourceFile,Model\\nx,Model-A\\n'",
        release.display()
    );
    ToolCommand {
        program: "/bin/sh".into(),
        leading_args: vec!["-c".into(), script.into(), "exiftool".into()],
    }
}

/// The tools the fixture resolves: the fake ffmpeg (present or missing as `shared` says), the fake
/// ffprobe, and the EXIF stand-in.
fn resolve_tools(paths: &Paths, shared: &Shared) -> Tools {
    let ffmpeg = if shared.ffmpeg_present.load(Ordering::SeqCst) {
        tool(
            "ffmpeg",
            Some(ToolCommand::direct(&paths.fake_bin.join("ffmpeg"))),
        )
    } else {
        tool("ffmpeg", None)
    };
    Tools {
        ffmpeg,
        ffprobe: tool(
            "ffprobe",
            Some(ToolCommand::direct(&paths.fake_bin.join("ffprobe"))),
        ),
        exiftool: tool("exiftool", Some(exif_tool(&paths.release_exif))),
    }
}

/// Builds an engine over `paths` and `shared`, listing `processes` as the process table.
fn build_engine(
    paths: &Paths,
    shared: &Shared,
    processes: Vec<ProcessEntry>,
) -> (Engine, EngineHandle) {
    let thumbs = Thumbnailer::new(
        &paths.data,
        Box::new(|args: &[String]| {
            let Some(output) = args.last() else {
                return Err("no output argument".to_owned());
            };
            fs::write(output, b"jpg").map_err(|err| err.to_string())
        }),
    );
    let fake_script = json!({ "wait_for_file": paths.release_render.to_string_lossy() });
    let resolve_paths = paths.clone();
    let resolve_shared = shared.clone();
    let now = shared.now.clone();
    let store = Store::open(&paths.data.join("trd.sqlite3")).expect("the store");
    Engine::new(EngineOptions {
        store,
        config_path: paths.config.clone(),
        data_dir: paths.data.clone(),
        codecs: codecs::builtin(),
        explicit_codecs: None,
        resolve_tools: Box::new(move || resolve_tools(&resolve_paths, &resolve_shared)),
        disk_access: Box::new(ScriptedDisk(shared.disk.clone())),
        scan_fs: Arc::new(GuardedFs {
            blocked: shared.blocked.clone(),
            touched: shared.touched.clone(),
            lists: shared.lists.clone(),
        }),
        clock: shared.clock.clone() as Arc<dyn Clock>,
        now_ms: Arc::new(move || now.load(Ordering::SeqCst)),
        table: Box::new(FakeProcessTable::new(processes)),
        policy: AdoptPolicy::All,
        thumbs,
        env: vec![("TRD_FAKE_FFMPEG".to_owned(), fake_script.to_string())],
        history_path: None,
        external_problems: Default::default(),
    })
}

/// One fixture: the directories, the engine and its handle, and a second store connection that the
/// test reads through.
struct World {
    _dir: TempDir,
    _fakes: FakeTools,
    paths: Paths,
    shared: Shared,
    rendering: Vec<RenderType>,
    paused_sequences: Vec<String>,
    paused_variants: Vec<(String, String)>,
    engine: Option<Engine>,
    handle: EngineHandle,
    reader: Store,
    /// The real fake-ffmpeg child a foreign-render test started.
    foreign_child: Option<RunningProcess>,
}

impl World {
    fn new(opts: Opts) -> World {
        let fakes = FakeTools::new().expect("the fake tools");
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().to_path_buf();
        let paths = Paths {
            export: root.join("export"),
            export_other: root.join("export-other"),
            source: root.join("source"),
            master: root.join("master"),
            data: root.join("data"),
            config: root.join("config.json"),
            release_exif: root.join("release-exif"),
            release_render: root.join("release-render"),
            fake_bin: fakes.path().to_path_buf(),
        };
        for path in [
            &paths.export,
            &paths.export_other,
            &paths.source,
            &paths.master,
            &paths.data,
        ] {
            fs::create_dir_all(path).expect("a fixture directory");
        }
        if opts.exif_released {
            fs::write(&paths.release_exif, b"").expect("the EXIF release");
        }
        let shared = Shared {
            disk: Arc::new(Mutex::new(FdaState::Granted)),
            ffmpeg_present: Arc::new(AtomicBool::new(opts.ffmpeg_present)),
            clock: Arc::new(ManualClock::new(YEAR)),
            now: Arc::new(AtomicI64::new(1_000_000_000)),
            blocked: Arc::new(AtomicBool::new(false)),
            touched: Arc::new(AtomicUsize::new(0)),
            lists: Arc::new(AtomicUsize::new(0)),
        };
        let (foreign_child, processes) = if opts.foreign {
            let (child, entries) = start_foreign_render(&paths);
            (Some(child), entries)
        } else {
            (None, Vec::new())
        };
        let (engine, handle) = build_engine(&paths, &shared, processes);
        let world = World {
            _dir: dir,
            _fakes: fakes,
            paths: paths.clone(),
            shared: shared.clone(),
            rendering: opts.rendering.clone(),
            paused_sequences: opts.paused_sequences.clone(),
            paused_variants: opts.paused_variants.clone(),
            engine: Some(engine),
            handle,
            reader: Store::open(&paths.data.join("trd.sqlite3")).expect("a reader"),
            foreign_child,
        };
        if opts.config {
            world.write_config(|_| {});
        }
        world
    }

    /// The config the fixture writes: one sequence source and export, ProRes, and the fixture's
    /// paused entries. `edit` changes it before it is written.
    fn base_config(&self, edit: impl FnOnce(&mut Config)) -> Config {
        let mut config = Config {
            export_paths: vec![self.paths.export.to_string_lossy().into_owned()],
            source_paths: vec![self.paths.source.to_string_lossy().into_owned()],
            master_videos: vec![self.paths.master.to_string_lossy().into_owned()],
            rendering: self.rendering.clone(),
            max_processes: MaxProcesses {
                ffmpeg: 1,
                exiftool: 1,
            },
            max_cuncurrent_renders: 1,
            image_name_prefix: PREFIX.to_owned(),
            image_sequence_dirname_prefix: PREFIX.to_owned(),
            image_extensions: vec!["tif".to_owned()],
            raw_types: vec!["tif".to_owned()],
            scan_interval_seconds: 2.0,
            recent_render_history_days: 30,
            paused_sequence_names: self.paused_sequences.clone(),
            paused_variants: self
                .paused_variants
                .iter()
                .map(|(sequence, key)| PausedVariant {
                    sequence_name: sequence.clone(),
                    variant_key: key.clone(),
                })
                .collect(),
            ..Config::default()
        };
        edit(&mut config);
        config
    }

    /// Writes config.json: the base config with `edit` applied.
    fn write_config(&self, edit: impl FnOnce(&mut Config)) {
        let config = self.base_config(edit);
        fs::write(&self.paths.config, config.to_json_pretty()).expect("config.json");
    }

    /// Creates a raw sequence of `frames` tif files in the source folder.
    fn add_raw(&self, name: &str, frames: u32) {
        let dir = self.paths.source.join(name);
        fs::create_dir_all(&dir).expect("a raw directory");
        for index in 1..=frames {
            fs::write(dir.join(format!("raw_{index:04}.tif")), b"x").expect("a raw frame");
        }
    }

    /// Sets the export of raw sequence `name` to `frames` frames, creating it if needed. The export
    /// directory is `LRT_<name>`.
    fn set_export(&self, name: &str, frames: u32) {
        let dir = self.paths.export.join(format!("{PREFIX}{name}"));
        fs::create_dir_all(&dir).expect("an export directory");
        for index in 1..=frames {
            let frame = dir.join(format!("{PREFIX}{index:04}.tif"));
            if !frame.exists() {
                fs::write(frame, b"x").expect("an exported frame");
            }
        }
    }

    fn engine(&mut self) -> &mut Engine {
        self.engine.as_mut().expect("an engine")
    }

    /// The latest snapshot.
    fn snap(&self) -> Arc<Snapshot> {
        self.handle.snapshot()
    }

    /// Moves the clocks: `instant` by `by`, the wall clock by one second.
    fn advance(&self, by: Duration) {
        self.shared.clock.advance(by);
        self.shared.now.fetch_add(1_000, Ordering::SeqCst);
    }

    /// One tick with the clock moved on by [`TICK_STEP`], so a scan is due.
    async fn tick(&mut self) {
        self.advance(TICK_STEP);
        self.engine().tick().await;
    }

    /// Ticks until `done` holds on the snapshot. Every tick moves the clock on.
    async fn drive(&mut self, mut done: impl FnMut(&Snapshot) -> bool) {
        let deadline = Instant::now() + HANG_GUARD;
        loop {
            self.tick().await;
            if done(&self.snap()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the engine"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Log rows of one event, oldest first.
    fn events(&self, event: &str) -> Vec<LogRow> {
        self.reader
            .log_rows_after(0)
            .expect("the log")
            .into_iter()
            .filter(|row| row.fields.event.as_deref() == Some(event))
            .collect()
    }

    /// The render jobs of one status.
    fn jobs(&self, status: JobStatus) -> Vec<trd_core::store::RenderJob> {
        self.reader.render_jobs(Some(status)).expect("the jobs")
    }

    /// A rendered-video row for the store, as a finished render would record it.
    fn store_video(&self, id: &str, rendered_at: i64, output_path: &str) {
        let video = StoredVideo {
            id: id.to_owned(),
            rendered_at,
            sequence_name: "LRT_OLD".to_owned(),
            export_path: self.paths.export.to_string_lossy().into_owned(),
            output_filename: format!("{id}.mov"),
            output_path: output_path.to_owned(),
            variant_key: "ProResUHQ_1280x720_25".to_owned(),
            codec: "ProResUHQ".to_owned(),
            resolution_label: "1280x720".to_owned(),
            frame_rate: 25,
            frame_count: 5,
            size_bytes: 10,
            width: 1280,
            height: 720,
            duration_seconds: 0.2,
            render_seconds: 1.0,
            thumbnail_path: None,
            xmp_embedded: false,
            source: "TRD".to_owned(),
        };
        self.reader
            .put_rendered_video(&video)
            .expect("a rendered row");
    }
}

/// Starts a fake ffmpeg that the test itself owns, held until `release_render`, and returns the
/// child with the process table entry that lists it as a foreign render (§6.11 step 2).
fn start_foreign_render(paths: &Paths) -> (RunningProcess, Vec<ProcessEntry>) {
    let sequence = paths.export.join("LRT_S1");
    fs::create_dir_all(&sequence).expect("the foreign render's folder");
    let output = sequence.join("LRT_S1_foreign.mov");
    let pattern = sequence.join("LRT_%04d.tif");
    let progress = paths.data.join("foreign.progress");
    // The progress argument makes the fake hold until the release file, as a running render does.
    let args: Vec<String> = vec![
        "-i".to_owned(),
        pattern.to_string_lossy().into_owned(),
        "-progress".to_owned(),
        progress.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
    ];
    let mut argv: Vec<std::ffi::OsString> = vec![paths.fake_bin.join("ffmpeg").into()];
    argv.extend(args.iter().map(|arg| arg.into()));
    let wait = json!({ "wait_for_file": paths.release_render.to_string_lossy() }).to_string();
    let env = [("TRD_FAKE_FFMPEG", wait.as_str())];
    let log = paths.data.join("foreign.log");
    let child = process::spawn_detached_with_env(&argv, &env, &log).expect("the foreign render");
    let entry = ProcessEntry {
        pid: child.pid,
        name: "ffmpeg".to_owned(),
        args,
        start_time: child.start_time,
        zombie: false,
    };
    (child, vec![entry])
}

/// The card of the sequence `name` in the snapshot, if it is listed.
fn card<'a>(snap: &'a Snapshot, name: &str) -> Option<&'a SequenceCard> {
    snap.in_progress.iter().find(|card| card.name == name)
}

/// The problem with `id`, if it holds.
fn problem<'a>(snap: &'a Snapshot, id: &str) -> Option<&'a trd_core::snapshot::Problem> {
    snap.problems.iter().find(|p| p.id == id)
}

/// Pauses every render, and resumes it, through the engine's controls.
fn apply(world: &mut World, control: Control) -> Result<(), ControlError> {
    world.engine().apply(control)
}

/// The scan interval of the run-loop tests: far beyond [`HANG_GUARD`], so a wait cannot be
/// satisfied by the periodic tick.
const LONG_INTERVAL: Duration = Duration::from_secs(3600);

/// Writes a config with [`LONG_INTERVAL`], starts the engine's run loop on its own task and waits
/// for the first tick's snapshot.
async fn start_run_loop(world: &mut World) -> tokio::task::JoinHandle<()> {
    world.write_config(|config| config.scan_interval_seconds = LONG_INTERVAL.as_secs_f64());
    let engine = world.engine.take().expect("an engine");
    let run = tokio::spawn(engine.run());
    wait_for(world, |s| s.phase != Phase::Starting).await;
    run
}

/// Waits, without ticking or moving the manual clock, until `done` holds on the snapshot.
async fn wait_for(world: &World, mut done: impl FnMut(&Snapshot) -> bool) {
    let deadline = Instant::now() + HANG_GUARD;
    while !done(&world.snap()) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the engine"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Shuts the run loop down and joins its task.
async fn stop_run_loop(world: &World, run: tokio::task::JoinHandle<()>) {
    world.handle.shutdown().await;
    run.await.expect("the run loop");
}

#[tokio::test]
async fn rescan_now_scans_at_once() {
    let mut w = World::new(Opts::default());
    let run = start_run_loop(&mut w).await;

    // A partial export: no histogram collection or render starts. The manual clock stays put, so
    // only the control can cause the scan that finds it.
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.handle
        .control(Control::RescanNow)
        .await
        .expect("rescan now");
    wait_for(&w, |s| card(s, "LRT_S1").is_some()).await;

    stop_run_loop(&w, run).await;
}

#[tokio::test]
async fn retry_scans_at_once() {
    let key = pro_res().variant_key();
    let mut w = World::new(Opts {
        paused_variants: vec![("LRT_S1".to_owned(), key.clone())],
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    let run = start_run_loop(&mut w).await;
    assert!(card(&w.snap(), "LRT_S1").is_some(), "the first scan");

    w.add_raw("S2", 5);
    w.set_export("S2", 3);
    w.handle
        .control(Control::Retry {
            sequence: "LRT_S1".to_owned(),
            variant_key: key,
        })
        .await
        .expect("retry");
    wait_for(&w, |s| card(s, "LRT_S2").is_some()).await;

    stop_run_loop(&w, run).await;
}

#[tokio::test]
async fn controls_keep_the_tick_deadline() {
    let mut w = World::new(Opts::default());
    let run = start_run_loop(&mut w).await;
    // The next periodic tick will scan.
    w.advance(LONG_INTERVAL);
    let scanned = w.shared.lists.load(Ordering::SeqCst);

    // Tokio's time moves only by `advance` from here, as long as this test never sleeps on it.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2000)).await;
    w.handle
        .control(Control::PauseAllRendering)
        .await
        .expect("pause");
    assert_eq!(
        w.shared.lists.load(Ordering::SeqCst),
        scanned,
        "neither the time so far nor the control causes a scan"
    );

    // 4000 s after the first tick, but only 2000 s after the control.
    tokio::time::advance(Duration::from_secs(2000)).await;
    let deadline = Instant::now() + HANG_GUARD;
    while w.shared.lists.load(Ordering::SeqCst) == scanned {
        assert!(
            Instant::now() < deadline,
            "the control postponed the periodic tick"
        );
        tokio::task::yield_now().await;
    }

    tokio::time::resume();
    stop_run_loop(&w, run).await;
}

#[tokio::test]
async fn lifecycle() {
    let mut w = World::new(Opts {
        exif_released: false,
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::Exporting, "a sequence is exporting");
    assert_eq!(
        card(&snap, "LRT_S1").map(|c| c.stage),
        Some(Stage::Exporting)
    );

    // The export completes, but no global pause keeps the queue from running yet: pause first, so
    // the job stays queued after its histogram arrives.
    w.set_export("S1", 5);
    apply(&mut w, Control::PauseAllRendering).expect("pause");
    w.tick().await;
    assert_eq!(
        card(&w.snap(), "LRT_S1").map(|c| c.stage),
        Some(Stage::CollectingMetadata),
        "a complete sequence waits for its histogram"
    );

    fs::write(&w.paths.release_exif, b"").expect("the EXIF release");
    w.drive(|s| card(s, "LRT_S1").is_some_and(|c| c.stage == Stage::Queued))
        .await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::Paused);
    assert_eq!(
        card(&snap, "LRT_S1").map(|c| c.variants[0].status),
        Some(VariantStatus::Queued)
    );

    apply(&mut w, Control::ResumeAllRendering).expect("resume");
    w.drive(|s| {
        card(s, "LRT_S1").is_some_and(|c| {
            c.stage == Stage::Rendering && c.variants[0].status == VariantStatus::Running
        })
    })
    .await;
    assert_eq!(w.snap().phase, Phase::Rendering);

    // The render finishes: the sequence leaves in_progress and its video heads recently_rendered in
    // the same snapshot.
    fs::write(&w.paths.release_render, b"").expect("the render release");
    w.drive(|s| card(s, "LRT_S1").is_none() && !s.recently_rendered.is_empty())
        .await;
    let snap = w.snap();
    let video = &snap.recently_rendered[0];
    assert!(video.file_exists, "the rendered file exists");
    assert!(video.output_path.ends_with(&video.output_filename));
    assert_eq!(snap.recently_rendered_total, 1);
    assert_eq!(w.jobs(JobStatus::Done).len(), 1, "the job is done");
    assert_eq!(w.events(EVENT_RENDER_FINISHED).len(), 1);
}

#[tokio::test]
async fn ordering() {
    // Every rank of §6.10 in one snapshot: rendering (0), queued (1), exporting (2), count mismatch
    // (3), blocked (4) and paused (5).
    let mut w = World::new(Opts {
        exif_released: true,
        rendering: vec![pro_res(), unknown_codec()],
        paused_sequences: vec!["LRT_P".to_owned()],
        paused_variants: vec![("LRT_B".to_owned(), pro_res().variant_key())],
        ..Opts::default()
    });

    // R is seen first and renders; its render is held until the end of the test.
    w.add_raw("R", 5);
    w.set_export("R", 5);
    w.drive(|s| card(s, "LRT_R").is_some_and(|c| c.stage == Stage::Rendering))
        .await;

    // Qa and Qb are complete and queued behind R, Qa seen before Qb.
    w.add_raw("Qa", 5);
    w.set_export("Qa", 5);
    w.drive(|s| card(s, "LRT_Qa").is_some_and(|c| c.stage == Stage::Queued))
        .await;
    w.add_raw("Qb", 5);
    w.set_export("Qb", 5);
    w.drive(|s| card(s, "LRT_Qb").is_some_and(|c| c.stage == Stage::Queued))
        .await;

    // X, Y and Z are exporting; Y and Z share a fraction, so the name decides.
    for (name, frames) in [("X", 1), ("Y", 4), ("Z", 4)] {
        w.add_raw(name, 5);
        w.set_export(name, frames);
    }
    // M exports more frames than its raw sequence has.
    w.add_raw("M", 5);
    w.set_export("M", 6);
    // B is complete, its known variant is paused and its other variant has an unknown codec.
    w.add_raw("B", 5);
    w.set_export("B", 5);
    // P is paused as a sequence.
    w.add_raw("P", 5);
    w.set_export("P", 2);

    let expected = [
        "LRT_R", "LRT_Qa", "LRT_Qb", "LRT_Y", "LRT_Z", "LRT_X", "LRT_M", "LRT_B", "LRT_P",
    ];
    w.drive(|s| {
        let names: Vec<&str> = s.in_progress.iter().map(|c| c.name.as_str()).collect();
        names == expected
    })
    .await;

    let snap = w.snap();
    let stages: Vec<Stage> = snap.in_progress.iter().map(|c| c.stage).collect();
    assert_eq!(
        stages,
        [
            Stage::Rendering,
            Stage::Queued,
            Stage::Queued,
            Stage::Exporting,
            Stage::Exporting,
            Stage::Exporting,
            Stage::CountMismatch,
            Stage::Blocked,
            Stage::Paused,
        ]
    );
    // Let R's fake ffmpeg finish, so it does not outlive the test.
    fs::write(&w.paths.release_render, b"").expect("the render release");
}

#[tokio::test]
async fn fda_blocks_scanning() {
    let mut w = World::new(Opts {
        exif_released: true,
        ..Opts::default()
    });
    // One render slot: S1 runs and S2 waits in the queue.
    w.add_raw("S1", 5);
    w.set_export("S1", 5);
    w.add_raw("S2", 5);
    w.set_export("S2", 5);
    // A finished render whose file the snapshot reports with `file_exists`.
    let old = w.paths.export.join("LRT_OLD").join("old.mov");
    fs::create_dir_all(old.parent().expect("a parent")).expect("a directory");
    fs::write(&old, b"x").expect("the old render");
    let now = w.shared.now.load(Ordering::SeqCst);
    w.store_video("old", now, &old.to_string_lossy());
    w.drive(|s| s.counts.rendering == 1 && s.counts.queued == 1)
        .await;

    let ids = |jobs: Vec<trd_core::store::RenderJob>| -> Vec<String> {
        jobs.into_iter().map(|job| job.id).collect()
    };
    let running = ids(w.jobs(JobStatus::Running));
    let queued = ids(w.jobs(JobStatus::Queued));
    assert_eq!((running.len(), queued.len()), (1, 1), "one runs, one waits");
    let old_path = old.to_string_lossy().into_owned();
    let listed = |snap: &Snapshot| {
        snap.recently_rendered
            .iter()
            .find(|row| row.output_path == old_path)
            .map(|row| row.file_exists)
    };
    assert_eq!(listed(&w.snap()), Some(true));

    // Access goes missing. The running render's slot frees up and the file goes away, and the
    // recheck is due: nothing may be reaped, promoted or probed.
    *w.shared.disk.lock().expect("the disk state") = FdaState::Missing;
    w.shared.blocked.store(true, Ordering::SeqCst);
    fs::write(&w.paths.release_render, b"").expect("the render release");
    fs::remove_file(&old).expect("the old render is removed");
    w.advance(Duration::from_secs(120));
    w.engine().tick().await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::WaitingForDiskAccess);
    let fda = problem(&snap, "fda").expect("the fda problem");
    assert_eq!(fda.severity, Severity::Error);
    assert!(snap.in_progress.is_empty(), "no configured path is listed");
    assert_eq!(
        listed(&snap),
        Some(true),
        "the file is not probed while access is missing"
    );
    assert_eq!(ids(w.jobs(JobStatus::Running)), running, "nothing reaped");
    assert_eq!(ids(w.jobs(JobStatus::Queued)), queued, "nothing promoted");

    // Still blocked on the next tick, and nothing touched the filesystem.
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::WaitingForDiskAccess);
    assert_eq!(
        w.shared.touched.load(Ordering::SeqCst),
        0,
        "no path was touched"
    );
    assert_eq!(ids(w.jobs(JobStatus::Queued)), queued, "still not promoted");

    // Access granted: the finished render is reaped, S2 is promoted and renders to the end.
    *w.shared.disk.lock().expect("the disk state") = FdaState::Granted;
    w.shared.blocked.store(false, Ordering::SeqCst);
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    assert!(
        problem(&w.snap(), "fda").is_none(),
        "the problem is gone once access is granted"
    );
    assert_eq!(listed(&w.snap()), Some(false), "the file is probed again");
    w.drive(|s| s.counts.rendering == 0 && s.counts.queued == 0)
        .await;
}

#[tokio::test]
async fn fda_at_launch_waits_before_config() {
    // Access is missing from the first tick, and a config file exists that is not read yet.
    let mut w = World::new(Opts {
        config: false,
        ..Opts::default()
    });
    *w.shared.disk.lock().expect("the disk state") = FdaState::Missing;
    w.shared.blocked.store(true, Ordering::SeqCst);
    w.write_config(|_| {});
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::WaitingForDiskAccess);
    assert!(problem(&snap, "fda").is_some(), "the fda problem");
    assert!(
        problem(&snap, "config").is_none(),
        "the config is not read while access is missing, so it is not reported missing"
    );
    assert_eq!(w.shared.touched.load(Ordering::SeqCst), 0);

    *w.shared.disk.lock().expect("the disk state") = FdaState::Granted;
    w.shared.blocked.store(false, Ordering::SeqCst);
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    assert_eq!(
        w.snap().phase,
        Phase::Idle,
        "the config is read once access is granted"
    );
}

#[tokio::test]
async fn retention_uses_loaded_config() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    // No config at launch: the retention window is the config's, so no pass runs before it loads.
    let mut w = World::new(Opts {
        config: false,
        ..Opts::default()
    });
    let now = w.shared.now.load(Ordering::SeqCst);
    w.store_video("old", now - 60 * DAY, "/nonexistent/old.mov");
    w.write_config(|c| c.recent_render_history_days = 90);
    w.tick().await;
    let rows = w.reader.rendered_videos().expect("the rows");
    assert!(
        rows.iter().any(|row| row.id == "old"),
        "a row inside the config's 90-day window survives start-up"
    );
}

#[tokio::test]
async fn config_reload_event() {
    let mut w = World::new(Opts::default());
    w.tick().await;
    let before = w.events(EVENT_CONFIG_RELOADED).len();
    // A scan lists the export folders; the count shows whether one ran.
    let scanned_at = w.shared.lists.load(Ordering::SeqCst);

    // Without the clock moving, no scan is due: only the config change asks for one.
    w.write_config(|c| c.scan_interval_seconds = 2.5);
    w.shared.now.fetch_add(1_000, Ordering::SeqCst);
    w.engine().tick().await;
    assert_eq!(
        w.events(EVENT_CONFIG_RELOADED).len(),
        before + 1,
        "one EVENT_CONFIG_RELOADED for the edit"
    );
    assert!(
        w.shared.lists.load(Ordering::SeqCst) > scanned_at,
        "the config change triggered a rescan"
    );

    // No edit: no event, and no scan either.
    let after_edit = w.shared.lists.load(Ordering::SeqCst);
    w.shared.now.fetch_add(1_000, Ordering::SeqCst);
    w.engine().tick().await;
    assert_eq!(w.events(EVENT_CONFIG_RELOADED).len(), before + 1);
    assert_eq!(w.shared.lists.load(Ordering::SeqCst), after_edit);
}

#[tokio::test]
async fn phases() {
    // Starting: before any tick.
    let mut w = World::new(Opts {
        config: false,
        exif_released: true,
        ..Opts::default()
    });
    assert_eq!(w.snap().phase, Phase::Starting);

    // No config: nothing is scanned.
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::NoConfig);

    // Idle: a config and nothing to render.
    w.write_config(|_| {});
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::Idle);

    // Exporting: a sequence is still growing.
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::Exporting);

    // Waiting for disk access.
    *w.shared.disk.lock().expect("the disk state") = FdaState::Missing;
    w.shared.blocked.store(true, Ordering::SeqCst);
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    assert_eq!(w.snap().phase, Phase::WaitingForDiskAccess);
    *w.shared.disk.lock().expect("the disk state") = FdaState::Granted;
    w.shared.blocked.store(false, Ordering::SeqCst);
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    assert_eq!(w.snap().phase, Phase::Exporting);

    // Rendering: the export completes and its render runs, held until the release file exists.
    w.set_export("S1", 5);
    w.drive(|s| s.phase == Phase::Rendering).await;

    // Paused: global pause suspends the render.
    apply(&mut w, Control::PauseAllRendering).expect("pause");
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::Paused);

    // Back to rendering, then the render ends so the test leaves nothing running.
    apply(&mut w, Control::ResumeAllRendering).expect("resume");
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::Rendering);
    fs::write(&w.paths.release_render, b"").expect("the render release");
    w.drive(|s| s.phase != Phase::Rendering).await;
}

#[tokio::test]
async fn problems_lifecycle() {
    let mut w = World::new(Opts {
        config: false,
        ffmpeg_present: false,
        ..Opts::default()
    });
    let missing_source = w.paths.data.join("missing-source");
    w.tick().await;
    let snap = w.snap();
    let ffmpeg_since = problem(&snap, "tool:ffmpeg")
        .expect("ffmpeg is missing")
        .since_ms;
    let config_since = problem(&snap, "config")
        .expect("config is missing")
        .since_ms;

    // Both persist: their since_ms does not move.
    w.tick().await;
    let snap = w.snap();
    assert_eq!(
        problem(&snap, "tool:ffmpeg").map(|p| p.since_ms),
        Some(ffmpeg_since)
    );
    assert_eq!(
        problem(&snap, "config").map(|p| p.since_ms),
        Some(config_since)
    );

    // A config with an ignored entry, an unknown codec and a missing source folder.
    w.write_config(|c| {
        c.export_paths = vec!["exports".to_owned()];
        c.rendering = vec![pro_res(), unknown_codec()];
        c.source_paths
            .push(missing_source.to_string_lossy().into_owned());
    });
    w.tick().await;
    let snap = w.snap();
    assert!(
        problem(&snap, "config").is_none(),
        "the config is no longer missing"
    );
    let issues = problem(&snap, "config-issues").expect("config-issues");
    assert_eq!(issues.severity, Severity::Error);
    let codec_since = problem(&snap, "codec:NoSuchCodec")
        .expect("the codec problem")
        .since_ms;
    let path_id = format!("path:{}", missing_source.to_string_lossy());
    let path_since = problem(&snap, &path_id).expect("the path problem").since_ms;
    let issues_since = issues.since_ms;

    w.tick().await;
    let snap = w.snap();
    assert_eq!(
        problem(&snap, "config-issues").map(|p| p.since_ms),
        Some(issues_since)
    );
    assert_eq!(
        problem(&snap, "codec:NoSuchCodec").map(|p| p.since_ms),
        Some(codec_since)
    );
    assert_eq!(
        problem(&snap, &path_id).map(|p| p.since_ms),
        Some(path_since)
    );

    // ffmpeg is installed: the next tool check clears its problem.
    w.shared.ffmpeg_present.store(true, Ordering::SeqCst);
    w.advance(Duration::from_secs(5 * 60));
    w.engine().tick().await;
    assert!(
        problem(&w.snap(), "tool:ffmpeg").is_none(),
        "tool problem resolved"
    );

    // The source folder is created: the path problem clears.
    fs::create_dir_all(&missing_source).expect("the source folder");
    w.tick().await;
    assert!(
        problem(&w.snap(), &path_id).is_none(),
        "path problem resolved"
    );

    // Full Disk Access: the problem appears, keeps its since_ms, and clears.
    *w.shared.disk.lock().expect("the disk state") = FdaState::Missing;
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    let fda_since = problem(&w.snap(), "fda").expect("the fda problem").since_ms;
    w.engine().tick().await;
    assert_eq!(
        problem(&w.snap(), "fda").map(|p| p.since_ms),
        Some(fda_since)
    );
    *w.shared.disk.lock().expect("the disk state") = FdaState::Granted;
    w.advance(Duration::from_secs(31));
    w.engine().tick().await;
    assert!(problem(&w.snap(), "fda").is_none(), "fda resolved");
}

#[tokio::test]
async fn snapshot_counters() {
    let mut w = World::new(Opts {
        exif_released: true,
        ..Opts::default()
    });
    let initial = w.snap().revision;
    assert_eq!(initial, 1, "the starting snapshot is revision 1");

    // The first tick changes the phase once: one step.
    w.tick().await;
    let after_first = w.snap().revision;
    assert_eq!(after_first, initial + 1);

    // A tick that changes nothing publishes nothing (no scan is due).
    w.shared.now.fetch_add(1_000, Ordering::SeqCst);
    w.engine().tick().await;
    assert_eq!(
        w.snap().revision,
        after_first,
        "an unchanged tick keeps the revision"
    );

    // A control that applies: one step.
    apply(&mut w, Control::PauseAllRendering).expect("pause");
    assert_eq!(w.snap().revision, after_first + 1);
    assert!(w.snap().global_pause);

    // A control that does not apply: no step.
    let refused = apply(&mut w, Control::ResumeSequence("LRT_NOPE".to_owned()));
    assert!(refused.is_err());
    assert_eq!(
        w.snap().revision,
        after_first + 1,
        "a refused control changes nothing"
    );
    apply(&mut w, Control::ResumeAllRendering).expect("resume");

    // Rendered rows: two within the window, one older than 30 days. The total and the last 24 h
    // count only the window; the recent list holds all three.
    let now = w.shared.now.load(Ordering::SeqCst);
    let before_rows = w.snap().revision;
    w.store_video("a", now - 3_600_000, "/none/a.mov");
    w.store_video("b", now - 7_200_000, "/none/b.mov");
    w.store_video("c", now - 40 * 86_400_000, "/none/c.mov");
    w.engine().tick().await;
    let snap = w.snap();
    assert_eq!(
        snap.revision,
        before_rows + 1,
        "the new rows are one change"
    );
    assert_eq!(snap.recently_rendered.len(), 3);
    assert_eq!(snap.recently_rendered_total, 2);
    assert_eq!(snap.counts.rendered_last_24h, 2);
    assert!(snap.recently_rendered.iter().all(|row| !row.file_exists));

    // A sequence exporting: a scan is due and the snapshot changes once.
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    let before_scan = w.snap().revision;
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.revision, before_scan + 1);
    assert_eq!(snap.counts.exporting, 1);
    assert_eq!(snap.phase, Phase::Exporting);
    assert!(snap.last_scan_completed_ms > 0, "the scan time is recorded");
}

#[tokio::test]
async fn count_mismatch() {
    let mut w = World::new(Opts {
        exif_released: true,
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 6);
    w.drive(|s| card(s, "LRT_S1").is_some()).await;
    let snap = w.snap();
    let seq = card(&snap, "LRT_S1").expect("the mismatched sequence");
    assert_eq!(seq.stage, Stage::CountMismatch);
    assert_eq!(
        seq.note,
        "6 frames exported but the raw sequence has 5; trd renders only when the counts are equal"
    );

    // Several more ticks, histogram available: it still never renders.
    for _ in 0..3 {
        w.tick().await;
    }
    assert!(w.jobs(JobStatus::Queued).is_empty());
    assert!(w.jobs(JobStatus::Running).is_empty());
    let seq = card(&w.snap(), "LRT_S1").expect("still listed").clone();
    assert_eq!(seq.stage, Stage::CountMismatch);
    assert!(
        seq.variants
            .iter()
            .all(|v| v.status == VariantStatus::Waiting)
    );
}

#[tokio::test]
async fn file_exists_cache() {
    let mut w = World::new(Opts::default());
    let rendered = w.paths.data.join("kept.mov");
    fs::write(&rendered, b"video").expect("a rendered file");
    let now = w.shared.now.load(Ordering::SeqCst);
    w.store_video("kept", now, &rendered.to_string_lossy());

    // The first check: the file exists.
    w.engine().tick().await;
    assert!(w.snap().recently_rendered[0].file_exists);

    // The file goes away, but the answer is kept for a minute.
    fs::remove_file(&rendered).expect("remove the file");
    w.advance(Duration::from_secs(30));
    w.engine().tick().await;
    assert!(
        w.snap().recently_rendered[0].file_exists,
        "the answer is cached"
    );

    // At a minute the file is checked again.
    w.advance(Duration::from_secs(30));
    w.engine().tick().await;
    assert!(
        !w.snap().recently_rendered[0].file_exists,
        "the file is checked again"
    );
}

#[tokio::test]
async fn foreign_render_left_alone() {
    let mut w = World::new(Opts {
        foreign: true,
        exif_released: true,
        ..Opts::default()
    });
    let pid = w.foreign_child.as_ref().expect("the foreign child").pid;
    let start = w
        .foreign_child
        .as_ref()
        .expect("the foreign child")
        .start_time;

    // Start-up adopts it (§6.11 step 2): a running row, foreign, with no slot.
    w.tick().await;
    let running = w.jobs(JobStatus::Running);
    assert_eq!(running.len(), 1, "the foreign render is adopted");
    assert!(running[0].foreign);
    assert_eq!(w.engine().renders().foreign_ids().len(), 1);
    assert_eq!(w.snap().phase, Phase::Rendering);

    // Global pause does not suspend it.
    apply(&mut w, Control::PauseAllRendering).expect("pause");
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::Paused);
    assert_eq!(
        process::process_state(pid),
        ProcessState::Running,
        "the foreign render is still running after a global pause"
    );
    assert_eq!(
        w.jobs(JobStatus::Running).len(),
        1,
        "its row is still running"
    );
    assert!(w.jobs(JobStatus::Suspended).is_empty());

    // Pausing its sequence does not signal it either: only its own stop ends it (§6.11 step 3).
    apply(&mut w, Control::PauseSequence("LRT_S1".to_owned())).expect("pause the sequence");
    w.tick().await;
    assert_eq!(
        process::process_state(pid),
        ProcessState::Running,
        "the foreign render is still running after its sequence is paused"
    );
    assert_eq!(
        w.jobs(JobStatus::Running).len(),
        1,
        "its row is still running after its sequence is paused"
    );

    // A config reload that no longer covers its folder does not signal it.
    w.write_config(|c| c.export_paths = vec![w.paths.export_other.to_string_lossy().into_owned()]);
    w.tick().await;
    assert_eq!(process::process_state(pid), ProcessState::Running);

    // Engine shutdown drops the handles and signals nothing.
    let engine = w.engine.take().expect("the engine");
    engine.shutdown();
    assert!(
        process::is_alive(pid, start),
        "the foreign render outlives the engine"
    );
    assert_eq!(process::process_state(pid), ProcessState::Running);

    // Only an explicit stop ends it. A new engine adopts it again, then stops it.
    let processes = vec![ProcessEntry {
        pid,
        name: "ffmpeg".to_owned(),
        args: vec![
            "-i".to_owned(),
            w.paths
                .export
                .join("LRT_S1")
                .join("LRT_%04d.tif")
                .to_string_lossy()
                .into_owned(),
            w.paths
                .export
                .join("LRT_S1")
                .join("LRT_S1_foreign.mov")
                .to_string_lossy()
                .into_owned(),
        ],
        start_time: start,
        zombie: false,
    }];
    let (engine, handle) = build_engine(&w.paths, &w.shared, processes);
    w.engine = Some(engine);
    w.handle = handle;
    w.tick().await;
    let job = w.jobs(JobStatus::Running).remove(0);
    assert!(job.foreign);
    assert!(process::is_alive(pid, start), "adoption does not signal it");

    apply(&mut w, Control::StopRender(job.id.clone())).expect("the explicit stop");
    assert_eq!(
        w.reader
            .render_job(&job.id)
            .expect("the job")
            .expect("the row")
            .status,
        JobStatus::Cancelled
    );
    // The stopped process ends; its child handle reports the exit.
    let child = w.foreign_child.as_mut().expect("the foreign child");
    let deadline = Instant::now() + HANG_GUARD;
    while child.try_wait().expect("the exit status").is_none() {
        assert!(Instant::now() < deadline, "the stopped render did not end");
        tokio::time::sleep(POLL).await;
    }
}

// ---- Switching the config path while the engine runs (spec 1 §6.1) ----

/// Writes a config of the fixture's kind at `path`, with `edit` applied.
fn write_config_at(w: &World, path: &Path, edit: impl FnOnce(&mut Config)) {
    fs::write(path, w.base_config(edit).to_json_pretty()).expect("a config file");
}

/// The path of a second config file, next to the first.
fn second_config(w: &World) -> PathBuf {
    w.paths.config.with_file_name("config-second.json")
}

#[tokio::test]
async fn switch_scans_the_new_folders() {
    let mut w = World::new(Opts::default());
    // S1 renders (held); Qb is complete and queued behind it.
    w.add_raw("S1", 5);
    w.set_export("S1", 5);
    w.drive(|s| card(s, "LRT_S1").is_some_and(|c| c.stage == Stage::Rendering))
        .await;
    w.add_raw("Qb", 5);
    w.set_export("Qb", 5);
    w.drive(|s| card(s, "LRT_Qb").is_some_and(|c| c.stage == Stage::Queued))
        .await;
    assert_eq!(w.jobs(JobStatus::Running).len(), 1);
    assert_eq!(w.jobs(JobStatus::Queued).len(), 1);

    // The second config looks at other folders, where N exists.
    let source_other = w.paths.source.with_file_name("source-other");
    let raw = source_other.join("N");
    fs::create_dir_all(&raw).expect("the other source");
    for index in 1..=5 {
        fs::write(raw.join(format!("raw_{index:04}.tif")), b"x").expect("a raw frame");
    }
    let export_n = w.paths.export_other.join(format!("{PREFIX}N"));
    fs::create_dir_all(&export_n).expect("the other export");
    for index in 1..=2 {
        fs::write(export_n.join(format!("{PREFIX}{index:04}.tif")), b"x").expect("a frame");
    }
    let path = second_config(&w);
    write_config_at(&w, &path, |c| {
        c.export_paths = vec![w.paths.export_other.to_string_lossy().into_owned()];
        c.source_paths = vec![source_other.to_string_lossy().into_owned()];
    });

    w.engine()
        .set_config_path(path.clone(), codecs::builtin(), None);
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.config_path, path.to_string_lossy());
    assert!(card(&snap, "LRT_N").is_some(), "the new sequence appears");
    assert!(card(&snap, "LRT_Qb").is_none(), "the old sequence is gone");
    assert!(w.jobs(JobStatus::Queued).is_empty(), "its queued job went");
    assert_eq!(
        w.jobs(JobStatus::Running).len(),
        1,
        "the render that was running is not stopped"
    );
    // Let S1's fake ffmpeg finish, so it does not outlive the test.
    fs::write(&w.paths.release_render, b"").expect("the render release");
}

#[tokio::test]
async fn switch_to_a_missing_or_invalid_file_gives_no_config() {
    let mut w = World::new(Opts::default());
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.drive(|s| card(s, "LRT_S1").is_some()).await;
    assert_ne!(w.snap().phase, Phase::NoConfig);

    // A missing file.
    let missing = w.paths.config.with_file_name("nothing-here.json");
    w.engine()
        .set_config_path(missing.clone(), codecs::builtin(), None);
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::NoConfig);
    assert!(snap.in_progress.is_empty(), "the old config is not used");
    let found = problem(&snap, "config").expect("the config problem");
    assert_eq!(found.kind, "PROBLEM_CONFIG_MISSING");
    assert_eq!(found.paths, vec![missing.to_string_lossy().into_owned()]);

    // A file that does not parse.
    let invalid = second_config(&w);
    fs::write(&invalid, b"{ not json").expect("an invalid file");
    w.engine()
        .set_config_path(invalid.clone(), codecs::builtin(), None);
    w.tick().await;
    let snap = w.snap();
    assert_eq!(snap.phase, Phase::NoConfig);
    assert!(snap.in_progress.is_empty());
    let found = problem(&snap, "config").expect("the config problem");
    assert_eq!(found.kind, "PROBLEM_CONFIG_INVALID");
    assert_eq!(found.paths, vec![invalid.to_string_lossy().into_owned()]);
}

#[tokio::test]
async fn setting_the_current_path_keeps_the_last_good_config() {
    let mut w = World::new(Opts::default());
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.drive(|s| card(s, "LRT_S1").is_some()).await;

    fs::write(&w.paths.config, b"{ broken").expect("a broken file");
    let path = w.paths.config.clone();
    w.engine().set_config_path(path, codecs::builtin(), None);
    w.tick().await;
    let snap = w.snap();
    assert_ne!(snap.phase, Phase::NoConfig, "the last good config is kept");
    assert!(card(&snap, "LRT_S1").is_some());
    let found = problem(&snap, "config").expect("the broken file is reported");
    assert_eq!(found.kind, "PROBLEM_CONFIG_INVALID");
}

#[tokio::test]
async fn switch_uses_the_codec_table_passed_with_it() {
    let mut w = World::new(Opts::default());
    w.tick().await;
    let mut table = codecs::builtin();
    let extra = table["ProResUHQ"].clone();
    table.insert("ExtraCodec".to_owned(), extra);

    let path = second_config(&w);
    write_config_at(&w, &path, |c| {
        c.rendering = vec![RenderType {
            codec: "ExtraCodec".to_owned(),
            frame_rate: 25,
            resolution: Some(vec![1280, 720]),
        }];
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.engine().set_config_path(path, table, None);
    w.tick().await;
    let snap = w.snap();
    assert!(
        problem(&snap, "codec:ExtraCodec").is_none(),
        "no unknown-codec problem: {:?}",
        snap.problems
    );
    let sequence = card(&snap, "LRT_S1").expect("the sequence");
    assert!(
        sequence
            .variants
            .iter()
            .all(|v| v.status != VariantStatus::UnknownCodec),
        "the codec is a known variant"
    );
    assert_eq!(sequence.variants.len(), 1);
}

#[tokio::test]
async fn first_config_imports_its_history_once() {
    let mut w = World::new(Opts {
        config: false,
        ..Opts::default()
    });
    w.tick().await;
    assert_eq!(w.snap().phase, Phase::NoConfig);
    assert!(!w.reader.swift_history_imported().expect("the setting"));

    let history = w.paths.config.with_file_name("history.json");
    fs::write(&history, b"[]").expect("a history file");
    let path = second_config(&w);
    write_config_at(&w, &path, |_| {});
    w.engine()
        .set_config_path(path, codecs::builtin(), Some(history));
    w.tick().await;
    assert_ne!(w.snap().phase, Phase::NoConfig);
    assert!(
        w.reader.swift_history_imported().expect("the setting"),
        "the chosen config's history is imported at the first loaded tick"
    );
}

#[tokio::test]
async fn later_switch_does_not_import_history() {
    let mut w = World::new(Opts::default());
    w.tick().await;
    let history = w.paths.config.with_file_name("history.json");
    fs::write(&history, b"[]").expect("a history file");
    let path = second_config(&w);
    write_config_at(&w, &path, |_| {});
    w.engine()
        .set_config_path(path, codecs::builtin(), Some(history));
    w.tick().await;
    assert!(
        !w.reader.swift_history_imported().expect("the setting"),
        "start-up work does not run a second time"
    );
}

#[tokio::test]
async fn switch_loads_the_new_backups_and_drops_the_old() {
    let mut w = World::new(Opts::default());
    let root = w.paths.config.parent().expect("the root").to_path_buf();
    let (old, new) = (root.join("backup-old"), root.join("backup-new"));
    for (dir, name) in [(&old, "B1"), (&new, "B2")] {
        fs::create_dir_all(dir).expect("a backup folder");
        fs::write(
            dir.join(format!("{name}.json")),
            br#"{"number_source_images":5}"#,
        )
        .expect("a backup sidecar");
    }
    w.set_export("B1", 5);
    w.set_export("B2", 5);
    let old_text = old.to_string_lossy().into_owned();
    w.write_config(|c| c.backup_paths = vec![old_text]);
    w.drive(|s| card(s, "LRT_B1").is_some_and(|c| c.raw_from_backup))
        .await;

    let path = second_config(&w);
    let new_text = new.to_string_lossy().into_owned();
    write_config_at(&w, &path, |c| c.backup_paths = vec![new_text]);
    w.engine().set_config_path(path, codecs::builtin(), None);
    w.drive(|s| card(s, "LRT_B2").is_some_and(|c| c.raw_from_backup))
        .await;
    assert!(
        card(&w.snap(), "LRT_B1").is_none_or(|c| !c.raw_from_backup),
        "the old config's backup candidates are gone"
    );
}

// ---- The codec table reloads while the engine runs (spec 1 §6.2) ----

fn extra_codec_entry() -> RenderType {
    RenderType {
        codec: "ExtraCodec".to_owned(),
        frame_rate: 25,
        resolution: Some(vec![1280, 720]),
    }
}

fn table_with_extra() -> codecs::VideoCodecs {
    let mut table = codecs::builtin();
    let extra = table["ProResUHQ"].clone();
    table.insert("ExtraCodec".to_owned(), extra);
    table
}

fn codec_file(w: &World) -> PathBuf {
    codecs::sibling_codecs_path(&w.paths.config)
}

fn only_variant_is_known(w: &World) -> bool {
    let snap = w.snap();
    card(&snap, "LRT_S1").is_some_and(|c| {
        c.variants.len() == 1 && c.variants.iter().all(|v| v.status != VariantStatus::UnknownCodec)
    })
}

#[tokio::test]
async fn codec_file_edit_reaches_the_engine() {
    let mut w = World::new(Opts {
        rendering: vec![extra_codec_entry()],
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.tick().await;
    assert!(problem(&w.snap(), "codec:ExtraCodec").is_some());

    let text = serde_json::to_string(&table_with_extra()).expect("json");
    fs::write(codec_file(&w), text).expect("a codec file");
    w.tick().await;
    let snap = w.snap();
    assert!(
        problem(&snap, "codec:ExtraCodec").is_none(),
        "the unknown-codec problem clears: {:?}",
        snap.problems
    );
    assert!(only_variant_is_known(&w), "the entry plans");
}

#[tokio::test]
async fn bad_codec_file_keeps_last_good_table() {
    let mut w = World::new(Opts {
        rendering: vec![extra_codec_entry()],
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    let text = serde_json::to_string(&table_with_extra()).expect("json");
    fs::write(codec_file(&w), text).expect("a codec file");
    w.tick().await;
    assert!(problem(&w.snap(), "codec:ExtraCodec").is_none());

    fs::write(codec_file(&w), "{ this is not json, at all").expect("a bad codec file");
    w.tick().await;
    let snap = w.snap();
    assert!(
        problem(&snap, "codec:ExtraCodec").is_none(),
        "still on the last good table: {:?}",
        snap.problems
    );
    assert!(only_variant_is_known(&w));
}

#[tokio::test]
async fn set_codecs_swaps_the_table() {
    let mut w = World::new(Opts {
        rendering: vec![extra_codec_entry()],
        ..Opts::default()
    });
    w.add_raw("S1", 5);
    w.set_export("S1", 3);
    w.tick().await;
    assert!(problem(&w.snap(), "codec:ExtraCodec").is_some());

    w.engine().set_codecs(table_with_extra());
    w.tick().await;
    assert!(problem(&w.snap(), "codec:ExtraCodec").is_none());
    assert!(only_variant_is_known(&w));
}
