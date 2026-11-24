// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of re-adoption and foreign renders (TRD-13, spec 1 §6.11).
//!
//! Every test takes its process list from a [`FakeProcessTable`]; none reads the owner's processes.
//! Each test works in its own temporary directory. The fake pids are not real processes: a start
//! time that matches no real process is never alive, so a finished render is decided by its files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tempfile::TempDir;
use trd_core::adopt::{ForeignCommand, adopt_foreign, parse_foreign, readopt};
use trd_core::codecs::{self, VideoCodecs};
use trd_core::config::ConfigStore;
use trd_core::paths::Clock;
use trd_core::plan::Plan;
use trd_core::process::{FakeProcessTable, ProcessEntry};
use trd_core::queue::{Ctx, Renders};
use trd_core::store::{JobStatus, RenderJob, Store};
use trd_core::thumbs::Thumbnailer;
use trd_core::tools::{ResolvedTool, ToolInfo, Tools};

/// The calendar year the fixtures use for `$YEAR`.
const YEAR: i32 = 2026;
/// The queue's clock, in milliseconds.
const NOW: i64 = 1_000_000;
/// The start time (seconds since the epoch) a fixture render was recorded with.
const START: i64 = 1_700_000_000;
/// The pid of a fixture's own render.
const OWN_PID: u32 = 4242;
/// The pid of a foreign ffmpeg.
const FOREIGN_PID: u32 = 5555;

struct FixedClock;

impl Clock for FixedClock {
    fn instant(&self) -> Instant {
        Instant::now()
    }

    fn year(&self) -> i32 {
        YEAR
    }
}

/// Everything one test needs: the store, the config (none on disk, so no after-render steps), the
/// tools (none available: the XMP step is skipped for an empty histogram), and the queue.
struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    store: Store,
    config: ConfigStore,
    codecs: VideoCodecs,
    tools: Tools,
    renders: Renders,
}

/// Builds the [`Ctx`] of a call from the fields of a [`Fixture`], so the borrows stay disjoint from
/// `renders`.
macro_rules! ctx {
    ($f:ident) => {
        Ctx {
            store: &$f.store,
            config: &mut $f.config,
            codecs: &$f.codecs,
            tools: &$f.tools,
            now_ms: NOW,
            year: YEAR,
        }
    };
}

fn unavailable(name: &'static str) -> ResolvedTool {
    ResolvedTool {
        info: ToolInfo {
            name,
            path: None,
            version: String::new(),
            available: false,
            problem: None,
        },
        command: None,
    }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path().to_path_buf();
    let data = root.join("data");
    fs::create_dir_all(data.join("renders")).expect("the renders directory");
    let store = Store::open(&data.join("trd.sqlite3")).expect("the store");
    let config = ConfigStore::new(root.join("config.json"), Arc::new(FixedClock));
    let thumbs = Thumbnailer::new(
        &data,
        Box::new(|args: &[String]| {
            let Some(output) = args.last() else {
                return Err("no output argument".to_owned());
            };
            fs::write(output, b"jpg").map_err(|err| err.to_string())
        }),
    );
    Fixture {
        _dir: dir,
        root: root.clone(),
        store,
        config,
        codecs: codecs::builtin(),
        tools: Tools {
            ffmpeg: unavailable("ffmpeg"),
            ffprobe: unavailable("ffprobe"),
            exiftool: unavailable("exiftool"),
        },
        renders: Renders::new(&data, thumbs),
    }
}

/// The plan of a render of sequence `seq` into the export directory of the fixture.
fn plan(root: &Path) -> Plan {
    let export = root.join("export");
    fs::create_dir_all(&export).expect("the export directory");
    let data = root.join("data").join("renders");
    Plan {
        input_dir: root.join("seq").to_string_lossy().into_owned(),
        image_name_prefix: "LRT_".to_owned(),
        image_suffix: ".tif".to_owned(),
        frame_number_digits: 4,
        start_number: None,
        output_path: export.join("seq.mov").to_string_lossy().into_owned(),
        progress_path: data.join("job.progress").to_string_lossy().into_owned(),
        partial_path: export
            .join(".seq.partial.mov")
            .to_string_lossy()
            .into_owned(),
        frame_rate: 25,
        aspect: "16:9".to_owned(),
        width: None,
        height: None,
        codec: codecs::builtin()
            .get("ProResUHQ")
            .cloned()
            .expect("the built-in ProResUHQ codec"),
        metadata: BTreeMap::new(),
        total_frames: 3,
    }
}

/// A row of `render_jobs` for the render `plan` with the given status and recorded process.
fn own_job(id: &str, plan: &Plan, status: JobStatus, pid: u32, start: i64) -> RenderJob {
    let output = Path::new(&plan.output_path);
    RenderJob {
        id: id.to_owned(),
        sequence_name: "seq".to_owned(),
        export_path: output
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        variant_key: "ProResUHQ_25".to_owned(),
        output_filename: output
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        output_path: plan.output_path.clone(),
        partial_path: plan.partial_path.clone(),
        plan_json: serde_json::to_string(plan).expect("the plan as JSON"),
        status,
        suspended_by: None,
        pid: Some(i64::from(pid)),
        pid_start_time: Some(start),
        queued_at: NOW,
        started_at: Some(NOW),
        finished_at: None,
        progress_path: Some(plan.progress_path.clone()),
        log_path: None,
        adopted: false,
        foreign: false,
    }
}

/// The process table entry of the ffmpeg that writes `plan`'s partial file.
fn own_entry(plan: &Plan, pid: u32, start: i64) -> ProcessEntry {
    ProcessEntry {
        pid,
        name: "ffmpeg".to_owned(),
        args: vec![
            "-i".to_owned(),
            format!("{}/LRT_%04d.tif", plan.input_dir),
            "-progress".to_owned(),
            plan.progress_path.clone(),
            plan.partial_path.clone(),
        ],
        start_time: start as u64,
    }
}

/// A foreign ffmpeg that writes `output`.
fn foreign_entry(pid: u32, start: i64, output: &str) -> ProcessEntry {
    ProcessEntry {
        pid,
        name: "ffmpeg".to_owned(),
        args: vec![
            "-framerate".to_owned(),
            "25".to_owned(),
            "-i".to_owned(),
            "/rp/tmp/LRT_xxx/LRT_%05d.tiff".to_owned(),
            "-c:v".to_owned(),
            "prores_ks".to_owned(),
            output.to_owned(),
        ],
        start_time: start as u64,
    }
}

fn write_progress(plan: &Plan, text: &str) {
    fs::write(&plan.progress_path, text).expect("the progress file");
}

fn write_partial(plan: &Plan) {
    fs::write(&plan.partial_path, b"partial video bytes").expect("the partial file");
}

#[test]
fn readopt_alive() {
    let mut f = fixture();
    let plan = plan(&f.root);
    write_progress(&plan, "frame=12\nfps=25\nprogress=continue\n");
    write_partial(&plan);
    f.store
        .put_render_job(&own_job("job-1", &plan, JobStatus::Running, OWN_PID, START))
        .expect("the row");
    let mut table = FakeProcessTable::new(vec![own_entry(&plan, OWN_PID, START)]);

    let report = readopt(&mut ctx!(f), &mut f.renders, &mut table).expect("readopt");

    assert_eq!(report.adopted, 1, "the live render is adopted");
    assert_eq!(f.renders.running_ids(), vec!["job-1".to_owned()]);
    let progress = f.renders.progress("job-1").expect("the adopted progress");
    assert_eq!(progress.frame, 12, "progress is read at once");
    let job = f
        .store
        .render_job("job-1")
        .expect("the row")
        .expect("a row");
    assert_eq!(job.status, JobStatus::Running, "the status is unchanged");
}

#[test]
fn readopt_finished_while_down() {
    let mut f = fixture();
    let plan = plan(&f.root);
    write_progress(&plan, "frame=3\nfps=25\nprogress=end\n");
    write_partial(&plan);
    f.store
        .put_render_job(&own_job("job-1", &plan, JobStatus::Running, OWN_PID, START))
        .expect("the row");
    let mut table = FakeProcessTable::new(Vec::new());

    let report = readopt(&mut ctx!(f), &mut f.renders, &mut table).expect("readopt");

    assert_eq!(report.finished, 1, "the render that ended is finished");
    assert!(f.renders.running_ids().is_empty(), "nothing is held");
    let job = f
        .store
        .render_job("job-1")
        .expect("the row")
        .expect("a row");
    assert_eq!(job.status, JobStatus::Done);
    assert!(
        Path::new(&plan.output_path).exists(),
        "the partial file was renamed"
    );
    assert!(!Path::new(&plan.partial_path).exists());
    let videos = f.store.rendered_videos().expect("the history");
    assert_eq!(videos.len(), 1, "one history row");
    assert_eq!(videos[0].source, "TRD");
}

#[test]
fn pid_reuse() {
    let mut f = fixture();
    let plan = plan(&f.root);
    write_progress(&plan, "frame=3\nfps=25\nprogress=continue\n");
    write_partial(&plan);
    f.store
        .put_render_job(&own_job("job-1", &plan, JobStatus::Running, OWN_PID, START))
        .expect("the row");
    // The pid now belongs to another process, started long after the recorded render.
    let mut table = FakeProcessTable::new(vec![own_entry(&plan, OWN_PID, START + 600)]);

    let report = readopt(&mut ctx!(f), &mut f.renders, &mut table).expect("readopt");

    assert_eq!(report.adopted, 0, "a recycled pid is not adopted");
    assert!(f.renders.running_ids().is_empty());
    let job = f
        .store
        .render_job("job-1")
        .expect("the row")
        .expect("a row");
    assert_eq!(job.status, JobStatus::Failed);
    let failures = f.store.failures().expect("the failures");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, "FAILURE_PRODUCED_NO_OUTPUT");
    assert!(
        !Path::new(&plan.partial_path).exists(),
        "the partial file is dropped"
    );
}

#[test]
fn no_adopt_foreign_keeps_own() {
    let mut f = fixture();
    let plan = plan(&f.root);
    write_progress(&plan, "frame=12\nfps=25\nprogress=continue\n");
    write_partial(&plan);
    f.store
        .put_render_job(&own_job("job-1", &plan, JobStatus::Running, OWN_PID, START))
        .expect("the row");
    let foreign_output = f.root.join("export").join("foreign.mov");
    let mut table = FakeProcessTable::new(vec![
        own_entry(&plan, OWN_PID, START),
        foreign_entry(FOREIGN_PID, START + 30, &foreign_output.to_string_lossy()),
    ]);

    // With --no-adopt-foreign only readopt runs.
    let report = readopt(&mut ctx!(f), &mut f.renders, &mut table).expect("readopt");

    assert_eq!(report.adopted, 1);
    assert_eq!(
        f.renders.running_ids(),
        vec!["job-1".to_owned()],
        "own render re-adopted"
    );
    assert!(
        f.renders.foreign_ids().is_empty(),
        "the foreign ffmpeg is not adopted"
    );
    let rows = f.store.render_jobs(None).expect("the rows");
    assert_eq!(rows.len(), 1, "no row is made for the foreign ffmpeg");
}

#[test]
fn parse_foreign_ffmpeg_line() {
    let entry = foreign_entry(FOREIGN_PID, START, "/rp/tmp/foo.mov");
    assert_eq!(
        parse_foreign(&entry),
        Some(ForeignCommand {
            input_pattern: "/rp/tmp/LRT_xxx/LRT_%05d.tiff".to_owned(),
            output_path: "/rp/tmp/foo.mov".to_owned(),
        })
    );
}

#[test]
fn parse_rejects_non_ffmpeg() {
    let mut other = foreign_entry(FOREIGN_PID, START, "/rp/tmp/foo.mov");
    other.name = "exiftool".to_owned();
    assert_eq!(parse_foreign(&other), None, "only ffmpeg is a render");

    let mut no_pattern = foreign_entry(FOREIGN_PID, START, "/rp/tmp/foo.mov");
    no_pattern.args[3] = "/rp/tmp/LRT_xxx/LRT_0001.tiff".to_owned();
    assert_eq!(
        parse_foreign(&no_pattern),
        None,
        "the input needs a % pattern"
    );

    let partial = foreign_entry(FOREIGN_PID, START, "/rp/tmp/.foo.partial.mov");
    assert_eq!(parse_foreign(&partial), None, "a partial file is trd's own");

    let not_video = foreign_entry(FOREIGN_PID, START, "/rp/tmp/foo.txt");
    assert_eq!(
        parse_foreign(&not_video),
        None,
        "the output must be a video"
    );
}

#[test]
fn adopt_foreign_skips_partial_and_owned_outputs() {
    let mut f = fixture();
    let owned_output = f.root.join("export").join("owned.mov");
    let partial_output = f.root.join("export").join(".p.partial.mov");
    fs::create_dir_all(f.root.join("export")).expect("the export directory");
    // A queued render already owns owned.mov.
    let plan = plan(&f.root);
    let mut queued = own_job("job-q", &plan, JobStatus::Queued, 0, 0);
    queued.pid = None;
    queued.pid_start_time = None;
    queued.output_path = owned_output.to_string_lossy().into_owned();
    f.store.put_render_job(&queued).expect("the row");
    let mut table = FakeProcessTable::new(vec![
        foreign_entry(6000, START, &partial_output.to_string_lossy()),
        foreign_entry(6001, START, &owned_output.to_string_lossy()),
    ]);

    let adopted = adopt_foreign(&mut ctx!(f), &mut f.renders, &mut table).expect("adopt");

    assert_eq!(adopted, 0);
    assert!(f.renders.foreign_ids().is_empty());
}

#[test]
fn adopt_foreign_adopts_and_finishes_by_output() {
    let mut f = fixture();
    let output = f.root.join("export").join("foreign.mov");
    fs::create_dir_all(f.root.join("export")).expect("the export directory");
    let mut table = FakeProcessTable::new(vec![foreign_entry(
        FOREIGN_PID,
        START,
        &output.to_string_lossy(),
    )]);

    let adopted = adopt_foreign(&mut ctx!(f), &mut f.renders, &mut table).expect("adopt");
    assert_eq!(adopted, 1);
    assert!(
        f.renders.running_ids().is_empty(),
        "a foreign render takes no slot of the queue"
    );
    let rows = f
        .store
        .render_jobs(Some(JobStatus::Running))
        .expect("the rows");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].foreign && rows[0].adopted);
    assert_eq!(rows[0].output_path, output.to_string_lossy().into_owned());

    // The process is gone and its output exists: the render succeeded, with source ADOPTED.
    fs::write(&output, b"video").expect("the output");
    f.renders
        .poll_foreign(&mut ctx!(f))
        .expect("the foreign render is reaped");

    assert!(f.renders.foreign_ids().is_empty());
    let job = f
        .store
        .render_job(&rows[0].id)
        .expect("the row")
        .expect("a row");
    assert_eq!(job.status, JobStatus::Done);
    let videos = f.store.rendered_videos().expect("the history");
    assert_eq!(videos.len(), 1);
    assert_eq!(videos[0].source, "ADOPTED");
}

#[test]
fn foreign_render_without_output_fails() {
    let mut f = fixture();
    let output = f.root.join("export").join("missing.mov");
    fs::create_dir_all(f.root.join("export")).expect("the export directory");
    let mut table = FakeProcessTable::new(vec![foreign_entry(
        FOREIGN_PID,
        START,
        &output.to_string_lossy(),
    )]);
    adopt_foreign(&mut ctx!(f), &mut f.renders, &mut table).expect("adopt");

    f.renders
        .poll_foreign(&mut ctx!(f))
        .expect("the foreign render is reaped");

    let rows = f.store.render_jobs(None).expect("the rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, JobStatus::Failed);
    let failures = f.store.failures().expect("the failures");
    assert_eq!(failures[0].kind, "FAILURE_PRODUCED_NO_OUTPUT");
    assert!(f.store.rendered_videos().expect("the history").is_empty());
}
