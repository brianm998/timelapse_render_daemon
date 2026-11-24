// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the render queue, the completion pipeline and the controls (TRD-12, spec 1 §6.6–§6.9).
//!
//! Unix only: the XMP step runs exiftool, and the stand-in exiftool is a `/bin/sh` script. Every test
//! is inside `mod queue`, so `cargo test -p trd-core queue` selects them all. Each test works in its
//! own temporary directory with the fake ffmpeg copied into it, and drives the queue tick by tick
//! with a manual clock. The only waits are for a fake ffmpeg process to end or to be released, under
//! a hang guard; no test asserts how long anything takes.

#![cfg(unix)]

mod queue {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::{Map, Value, json};
    use tempfile::TempDir;
    use trd_core::codecs::{self, VideoCodecs};
    use trd_core::config::{Config, ConfigStore, MaxProcesses, RenderType};
    use trd_core::controls::{ControlError, Outcome};
    use trd_core::events::{
        EVENT_RENDER_FAILED, EVENT_RENDER_FINISHED, EVENT_RENDER_STARTED, EVENT_SEQUENCE_DELETED,
        EVENT_SEQUENCE_FINISHED, EVENT_SEQUENCE_READY,
    };
    use trd_core::exif::Histogram;
    use trd_core::paths::Clock;
    use trd_core::pipeline::{RENDER_LOG_LIMIT, retain_render_logs};
    use trd_core::plan;
    use trd_core::queue::{Ctx, Renders, SWEEP_INTERVAL_MS};
    use trd_core::scan::{ExportSequence, RawSequence, ScanResult};
    use trd_core::store::{Failure, JobStatus, LogLevel, LogRow, RenderJob, Store, SuspendedBy};
    use trd_core::thumbs::Thumbnailer;
    use trd_core::tools::{ResolvedTool, ToolCommand, ToolInfo, Tools};
    use trd_testtools::FakeTools;

    /// The calendar year the fixtures use for `$YEAR`.
    const YEAR: i32 = 2026;
    /// Stops a test that waits for something that never happens. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);
    const POLL: Duration = Duration::from_millis(20);

    /// A clock that gives the fixed year. The queue's own time is the manual clock of [`World`].
    struct FixedClock;

    impl Clock for FixedClock {
        fn instant(&self) -> Instant {
            Instant::now()
        }

        fn year(&self) -> i32 {
            YEAR
        }
    }

    /// Builds the [`Ctx`] of a call from the fields of a [`World`], so the borrows stay disjoint from
    /// `renders`.
    macro_rules! ctx {
        ($w:expr) => {
            Ctx {
                store: &$w.store,
                config: &mut $w.config,
                codecs: &$w.codecs,
                tools: &$w.tools,
                now_ms: $w.now,
                year: YEAR,
            }
        };
    }

    /// The fixture settings of one test.
    struct Setup {
        rendering: Vec<RenderType>,
        processes: i64,
        per_sequence: i64,
        delete_after: Vec<String>,
        /// The fake ffmpeg's behaviour, besides the hold.
        script: Map<String, Value>,
        /// Whether the fake ffmpeg waits for the release file before it writes its output.
        hold: bool,
    }

    impl Setup {
        fn new() -> Setup {
            Setup {
                rendering: vec![pro_res(1280, 720)],
                processes: 2,
                per_sequence: 1,
                delete_after: Vec::new(),
                script: Map::new(),
                hold: false,
            }
        }
    }

    fn pro_res(width: i64, height: i64) -> RenderType {
        RenderType {
            codec: "ProResUHQ".to_owned(),
            frame_rate: 25,
            resolution: Some(vec![width, height]),
        }
    }

    /// A stand-in exiftool: a `/bin/sh` script that ignores its arguments.
    fn sh_tool(script: &str) -> ToolCommand {
        ToolCommand {
            program: "/bin/sh".into(),
            leading_args: vec!["-c".into(), script.into(), "exiftool".into()],
        }
    }

    fn resolved(name: &'static str, command: Option<ToolCommand>) -> ResolvedTool {
        ResolvedTool {
            info: ToolInfo {
                name,
                path: None,
                version: String::new(),
                available: command.is_some(),
                problem: None,
            },
            command,
        }
    }

    /// Everything one test needs: the directories, the store, the config, the tools, the queue and
    /// the manual clock.
    struct World {
        _tools: FakeTools,
        _dir: TempDir,
        store: Store,
        config: ConfigStore,
        codecs: VideoCodecs,
        tools: Tools,
        renders: Renders,
        now: i64,
        first_seen: BTreeMap<String, i64>,
        rendering: Vec<RenderType>,
        processes: i64,
        per_sequence: i64,
        export: PathBuf,
        raw: PathBuf,
        data: PathBuf,
        config_path: PathBuf,
        release: PathBuf,
        hold: bool,
    }

    impl World {
        fn new(setup: Setup) -> World {
            let fakes = FakeTools::new().expect("the fake tools");
            let dir = tempfile::tempdir().expect("a temporary directory");
            let root = dir.path().to_path_buf();
            let export = root.join("export");
            let master = root.join("master");
            let raw = root.join("raw");
            let data = root.join("data");
            for path in [&export, &master, &raw, &data] {
                fs::create_dir_all(path).expect("a fixture directory");
            }
            let release = root.join("release");
            let config_path = root.join("config.json");

            let config = Config {
                export_paths: vec![export.to_string_lossy().into_owned()],
                master_videos: vec![master.to_string_lossy().into_owned()],
                rendering: setup.rendering.clone(),
                max_processes: MaxProcesses {
                    ffmpeg: setup.processes,
                    exiftool: 2,
                },
                max_cuncurrent_renders: setup.per_sequence,
                image_name_prefix: "LRT_".to_owned(),
                image_sequence_dirname_prefix: "LRT_".to_owned(),
                image_extensions: vec!["tif".to_owned(), "tiff".to_owned()],
                delete_sequence_after_render_regexes: setup.delete_after.clone(),
                ..Config::default()
            };
            fs::write(&config_path, config.to_json_pretty()).expect("config.json");
            let codecs = codecs::builtin();
            let mut config_store = ConfigStore::new(&config_path, Arc::new(FixedClock));
            config_store.reload_if_changed(&codecs);
            assert!(
                config_store.config().is_some(),
                "the fixture config loads: {:?}",
                config_store.issues()
            );

            let store = Store::open(&data.join("trd.sqlite3")).expect("the store");
            let tools = Tools {
                ffmpeg: resolved(
                    "ffmpeg",
                    Some(ToolCommand::direct(&fakes.path().join("ffmpeg"))),
                ),
                ffprobe: resolved("ffprobe", None),
                exiftool: resolved("exiftool", Some(sh_tool("exit 0"))),
            };

            let mut script = setup.script.clone();
            if setup.hold {
                script.insert("wait_for_file".to_owned(), json!(release.to_string_lossy()));
            }
            let thumbs = Thumbnailer::new(
                &data,
                Box::new(|args: &[String]| {
                    let Some(output) = args.last() else {
                        return Err("no output argument".to_owned());
                    };
                    fs::write(output, b"jpg").map_err(|err| err.to_string())
                }),
            );
            let renders = Renders::new(&data, thumbs).with_env(vec![(
                "TRD_FAKE_FFMPEG".to_owned(),
                Value::Object(script).to_string(),
            )]);

            World {
                _tools: fakes,
                _dir: dir,
                store,
                config: config_store,
                codecs,
                tools,
                renders,
                now: 1_000_000,
                first_seen: BTreeMap::new(),
                rendering: setup.rendering,
                processes: setup.processes,
                per_sequence: setup.per_sequence,
                export,
                raw,
                data,
                config_path,
                release,
                hold: setup.hold,
            }
        }

        /// Creates the export directory of a sequence with three numbered frames.
        fn add_sequence(&self, name: &str) {
            let seq = self.export.join(name);
            fs::create_dir_all(&seq).expect("a sequence directory");
            for index in 1..=3 {
                fs::write(seq.join(format!("LRT_{index:04}.tif")), b"x").expect("a frame");
            }
        }

        /// A scan that lists the named sequences as complete, each with its histogram.
        fn scan_of(&self, names: &[&str]) -> (ScanResult, BTreeMap<String, Histogram>) {
            let mut scan = ScanResult::default();
            let mut histograms = BTreeMap::new();
            for name in names {
                scan.raw.push(RawSequence {
                    name: (*name).to_owned(),
                    parent: self.raw.to_string_lossy().into_owned(),
                    frames: 3,
                    from_backup: false,
                });
                scan.exports.push(ExportSequence {
                    path: self.export.join(name).to_string_lossy().into_owned(),
                    name: (*name).to_owned(),
                    raw_name: (*name).to_owned(),
                    raw_frames: 3,
                    frames: 3,
                    export_frames_per_minute: 0.0,
                    export_eta_seconds: -1,
                    export_size: -1,
                    in_progress: false,
                });
                let raw_dir = self.raw.join(name).to_string_lossy().into_owned();
                histograms.insert(raw_dir, one_tag_histogram());
            }
            (scan, histograms)
        }

        /// One scan of the named sequences: queue, remove and promote (§6.7).
        fn scan(&mut self, names: &[&str]) {
            let (scan, histograms) = self.scan_of(names);
            let first_seen = self.first_seen.clone();
            self.renders
                .on_scan(&mut ctx!(self), &scan, &histograms, &first_seen)
                .expect("a scan");
            self.check_caps();
        }

        /// One tick of the render loop, the clock moving on by a millisecond.
        fn tick(&mut self) {
            self.now += 1;
            self.renders.tick(&mut ctx!(self)).expect("a tick");
            self.check_caps();
        }

        /// Ticks until `done` holds. Every tick checks the caps.
        fn drive(&mut self, mut done: impl FnMut(&World) -> bool) {
            let deadline = Instant::now() + HANG_GUARD;
            while !done(self) {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for the renders"
                );
                self.tick();
                thread::sleep(POLL);
            }
        }

        /// Checks that running and suspended jobs stay within `max_processes` and
        /// `max_cuncurrent_renders` (§6.7).
        fn check_caps(&self) {
            let mut busy = self.jobs(JobStatus::Running);
            busy.extend(self.jobs(JobStatus::Suspended));
            assert!(
                busy.len() as i64 <= self.processes,
                "{} jobs hold slots, above {}",
                busy.len(),
                self.processes
            );
            for job in &busy {
                let same = busy
                    .iter()
                    .filter(|other| other.sequence_name == job.sequence_name)
                    .count() as i64;
                assert!(
                    same <= self.per_sequence,
                    "{same} jobs of {} hold slots, above {}",
                    job.sequence_name,
                    self.per_sequence
                );
            }
        }

        fn jobs(&self, status: JobStatus) -> Vec<RenderJob> {
            self.store.render_jobs(Some(status)).expect("the jobs")
        }

        fn all_jobs(&self) -> Vec<RenderJob> {
            self.store.render_jobs(None).expect("the jobs")
        }

        /// The jobs of a variant of a sequence, in any state.
        fn jobs_of(&self, sequence: &str, variant_key: &str) -> Vec<RenderJob> {
            self.all_jobs()
                .into_iter()
                .filter(|job| job.sequence_name == sequence && job.variant_key == variant_key)
                .collect()
        }

        /// Whether any job is queued, running or suspended.
        fn in_flight(&self) -> bool {
            [JobStatus::Queued, JobStatus::Running, JobStatus::Suspended]
                .into_iter()
                .any(|status| !self.jobs(status).is_empty())
        }

        /// The key of the variant at `index` in the rendering list.
        fn key(&self, index: usize) -> String {
            self.rendering[index].variant_key()
        }

        /// The output path of variant `index` of a sequence.
        fn output(&self, sequence: &str, index: usize) -> PathBuf {
            let config = self.config.config().expect("the config");
            let filename =
                plan::output_filename(config, &self.codecs, &self.rendering[index], sequence);
            self.export.join(filename)
        }

        /// Lets a held fake ffmpeg write its output.
        fn release(&self) {
            assert!(self.hold, "only a held fixture is released");
            fs::write(&self.release, b"").expect("the release file");
        }

        /// The log rows with `event` set to `event`, oldest first.
        fn events(&self, event: &str) -> Vec<LogRow> {
            self.log_rows()
                .into_iter()
                .filter(|row| row.fields.event.as_deref() == Some(event))
                .collect()
        }

        fn log_rows(&self) -> Vec<LogRow> {
            self.store.log_rows_after(0).expect("the log")
        }

        /// The number of INFO rows written by the controls.
        fn control_rows(&self) -> usize {
            self.log_rows()
                .into_iter()
                .filter(|row| row.level == LogLevel::Info && row.subject == "controls")
                .count()
        }

        /// The job of a variant that is in flight, the only one there is.
        fn only_in_flight(&self, sequence: &str, variant_key: &str) -> RenderJob {
            let mut jobs: Vec<RenderJob> = self
                .jobs_of(sequence, variant_key)
                .into_iter()
                .filter(|job| {
                    matches!(
                        job.status,
                        JobStatus::Queued | JobStatus::Running | JobStatus::Suspended
                    )
                })
                .collect();
            assert_eq!(
                jobs.len(),
                1,
                "one job in flight for {sequence}/{variant_key}"
            );
            jobs.remove(0)
        }
    }

    /// Runs one control and checks the log: an applied control writes exactly one INFO row, a
    /// rejected one writes none.
    fn control<T>(
        w: &mut World,
        action: impl FnOnce(&mut World) -> Result<T, ControlError>,
    ) -> Result<T, ControlError> {
        let before = w.control_rows();
        let result = action(w);
        let after = w.control_rows();
        match &result {
            Ok(_) => assert_eq!(after, before + 1, "one INFO row for an applied control"),
            Err(_) => assert_eq!(after, before, "a rejected control writes no row"),
        }
        result
    }

    fn one_tag_histogram() -> Histogram {
        let mut histogram = Histogram::new();
        histogram.insert("Make".to_owned(), BTreeMap::from([("Canon".to_owned(), 3)]));
        histogram
    }

    /// A release file that is written when the test is done with a held fixture: the held processes
    /// end, so the next call drains the queue.
    fn drain(w: &mut World) {
        if w.hold && !w.release.exists() {
            w.release();
        }
        w.drive(|w| !w.in_flight());
    }

    #[test]
    fn sequence_ready_announced_once() {
        let mut w = World::new(Setup::new());
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        assert_eq!(
            w.events(EVENT_SEQUENCE_READY).len(),
            1,
            "a complete sequence is announced"
        );
        w.scan(&["seqA"]);
        assert_eq!(
            w.events(EVENT_SEQUENCE_READY).len(),
            1,
            "and announced once, not per scan"
        );
        w.drive(|w| !w.in_flight());
    }

    #[test]
    fn plan_failure_recorded_once() {
        let mut w = World::new(Setup::new());
        // A complete sequence with no frames in it: no variant can be planned.
        fs::create_dir_all(w.export.join("seqA")).expect("an empty sequence directory");
        w.scan(&["seqA"]);
        w.scan(&["seqA"]);
        let cards = w.store.failures().expect("the failures");
        assert_eq!(cards.len(), 1, "the unplannable variant gets one card");
        assert_eq!(cards[0].kind, "FAILURE_NOT_QUEUEABLE");
        assert!(w.all_jobs().is_empty(), "no job is queued for it");
    }

    #[test]
    fn caps() {
        let mut setup = Setup::new();
        setup.rendering = (0..3).map(|i| pro_res(1280, 720 - 60 * i)).collect();
        setup.processes = 2;
        setup.per_sequence = 1;
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.add_sequence("seqB");
        w.scan(&["seqA", "seqB"]);
        assert_eq!(w.jobs(JobStatus::Queued).len(), 4, "six jobs, two started");
        assert_eq!(w.jobs(JobStatus::Running).len(), 2);
        w.release();
        w.drive(|w| !w.in_flight());
        assert_eq!(w.jobs(JobStatus::Done).len(), 6, "every variant rendered");
    }

    #[test]
    fn completion() {
        let mut setup = Setup::new();
        setup.rendering = vec![pro_res(1280, 720), pro_res(1920, 1080)];
        setup.processes = 1;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        w.drive(|w| !w.in_flight());

        let videos = w.store.rendered_videos().expect("the history");
        assert_eq!(videos.len(), 2, "one history row per variant");
        for video in &videos {
            assert_eq!(video.source, "TRD");
            assert!(
                video.xmp_embedded,
                "the XMP step ran with the stand-in exiftool"
            );
            assert!(
                Path::new(&video.output_path).is_file(),
                "the final file exists"
            );
            assert!(video.thumbnail_path.is_some(), "the poster frame was made");
        }
        assert_eq!(w.events(EVENT_RENDER_FINISHED).len(), 2);
        let finished = w.events(EVENT_SEQUENCE_FINISHED);
        assert_eq!(
            finished.len(),
            1,
            "only the last variant finishes the sequence"
        );
        let total: i64 = videos.iter().map(|video| video.size_bytes).sum();
        assert_eq!(finished[0].fields.video_count, Some(2));
        assert_eq!(finished[0].fields.total_bytes, Some(total));
    }

    #[test]
    fn auto_pause_after_three() {
        let mut setup = Setup::new();
        setup.processes = 1;
        let mut w = World::new(setup);
        w.tools.exiftool.command = Some(sh_tool("echo 'Truncated mdat atom'; exit 1"));
        w.add_sequence("seqA");
        let key = w.key(0);
        for attempt in 1..=3 {
            if attempt > 1 {
                for failure in w.store.failures().expect("the failures") {
                    w.renders
                        .dismiss_failure(&mut ctx!(w), &failure.id)
                        .expect("dismiss the card");
                }
            }
            w.scan(&["seqA"]);
            w.drive(|w| !w.in_flight());
            assert!(
                !w.output("seqA", 0).exists(),
                "a corrupt render leaves no final file"
            );
        }
        let job = w.jobs_of("seqA", &key);
        assert!(job.iter().all(|job| job.status == JobStatus::Failed));
        assert_eq!(w.store.corrupt_strikes("seqA", &key).expect("strikes"), 3);
        let config = w.config.config().expect("the config");
        assert!(
            config
                .paused_variants
                .iter()
                .any(|p| p.sequence_name == "seqA" && p.variant_key == key),
            "the third corrupt render pauses the variant in config.json"
        );
        let on_disk = fs::read_to_string(&w.config_path).expect("config.json");
        assert!(
            on_disk.contains("paused_variants"),
            "the pause is written to disk"
        );
        let last = w
            .store
            .failures()
            .expect("the failures")
            .into_iter()
            .next()
            .expect("the last card");
        assert_eq!(last.kind, "FAILURE_STRUCTURAL_CORRUPTION");
        assert!(last.variant_auto_paused);
        assert!(
            !Path::new(&w.jobs_of("seqA", &key)[0].partial_path).exists(),
            "the corrupt partial file is deleted"
        );
    }

    #[test]
    fn stop() {
        let mut setup = Setup::new();
        setup.processes = 1;
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        let job = w.only_in_flight("seqA", &key);
        assert_eq!(job.status, JobStatus::Running);
        fs::write(&job.partial_path, b"partial").expect("a partial file");

        control(&mut w, |w| w.renders.stop_render(&mut ctx!(w), &job.id)).expect("stop");

        let stopped = w
            .store
            .render_job(&job.id)
            .expect("the job")
            .expect("a job");
        assert_eq!(stopped.status, JobStatus::Cancelled);
        assert!(
            !Path::new(&job.partial_path).exists(),
            "the partial file is deleted"
        );
        assert!(w.renders.running_ids().is_empty(), "the process is gone");
        let config = w.config.config().expect("the config");
        assert!(
            config
                .paused_variants
                .iter()
                .any(|p| p.sequence_name == "seqA" && p.variant_key == key),
            "stop pauses the variant"
        );
        drain(&mut w);
    }

    #[test]
    fn retry() {
        let mut setup = Setup::new();
        setup.processes = 1;
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        let job = w.only_in_flight("seqA", &key);
        control(&mut w, |w| w.renders.stop_render(&mut ctx!(w), &job.id)).expect("stop");
        assert!(w.config.config().expect("the config").paused_variants.len() == 1);
        w.store
            .set_corrupt_strikes("seqA", &key, 2)
            .expect("strikes");

        let outcome =
            control(&mut w, |w| w.renders.retry(&mut ctx!(w), "seqA", &key)).expect("retry");
        assert_eq!(outcome, Outcome { rescan: true }, "retry asks for a scan");
        assert!(
            w.config
                .config()
                .expect("the config")
                .paused_variants
                .is_empty(),
            "retry clears the pause"
        );
        assert_eq!(w.store.corrupt_strikes("seqA", &key).expect("strikes"), 0);

        w.scan(&["seqA"]);
        assert_eq!(
            w.jobs_of("seqA", &key)
                .iter()
                .filter(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
                .count(),
            1,
            "the scan after retry queues the variant again"
        );
        drain(&mut w);
    }

    #[test]
    fn global_pause() {
        let mut setup = Setup::new();
        setup.rendering = (0..4).map(|i| pro_res(1280, 720 - 60 * i)).collect();
        setup.processes = 3;
        setup.per_sequence = 3;
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        assert_eq!(w.jobs(JobStatus::Running).len(), 3);
        assert_eq!(w.jobs(JobStatus::Queued).len(), 1);
        let by_user = w.jobs(JobStatus::Running)[0].clone();
        control(&mut w, |w| {
            w.renders.suspend_render(&mut ctx!(w), &by_user.id)
        })
        .expect("suspend one render");

        control(&mut w, |w| w.renders.pause_all_rendering(&mut ctx!(w))).expect("pause all");
        let suspended = w.jobs(JobStatus::Suspended);
        assert_eq!(suspended.len(), 3, "every running render is suspended");
        let global = suspended
            .iter()
            .filter(|job| job.suspended_by == Some(SuspendedBy::Global))
            .count();
        assert_eq!(
            global, 2,
            "the two not suspended by the user are suspended by global"
        );
        for _ in 0..3 {
            w.tick();
        }
        assert_eq!(w.jobs(JobStatus::Queued).len(), 1, "nothing new starts");
        assert_eq!(w.jobs(JobStatus::Running).len(), 0);

        let global_one = suspended
            .iter()
            .find(|job| job.suspended_by == Some(SuspendedBy::Global))
            .expect("a render suspended by pause rendering")
            .clone();
        assert_code(
            control(&mut w, |w| {
                w.renders.resume_render(&mut ctx!(w), &global_one.id)
            }),
            "FAILED_PRECONDITION",
        );

        control(&mut w, |w| w.renders.resume_all_rendering(&mut ctx!(w))).expect("resume all");
        let still_user = w
            .jobs(JobStatus::Suspended)
            .into_iter()
            .filter(|job| job.suspended_by == Some(SuspendedBy::User))
            .count();
        assert_eq!(
            still_user, 1,
            "the render suspended by the user stays suspended"
        );
        assert_eq!(w.jobs(JobStatus::Running).len(), 2, "the others continue");
        assert_eq!(
            w.jobs(JobStatus::Queued).len(),
            1,
            "the suspended render keeps its slot, so the queued one waits"
        );

        control(&mut w, |w| {
            w.renders.resume_render(&mut ctx!(w), &by_user.id)
        })
        .expect("resume the render the user suspended");
        w.release();
        drain(&mut w);
        assert_eq!(w.jobs(JobStatus::Done).len(), 4);
    }

    #[test]
    fn delete_waits_for_all_variants() {
        let mut setup = Setup::new();
        setup.rendering = vec![pro_res(1280, 720), pro_res(1920, 1080)];
        setup.processes = 1;
        setup.delete_after = vec![".*".to_owned()];
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let seq_dir = w.export.join("seqA");
        w.drive(|w| w.jobs(JobStatus::Done).len() == 1);
        assert!(
            seq_dir.is_dir(),
            "the export directory stays until every variant is rendered"
        );
        assert!(w.events(EVENT_SEQUENCE_DELETED).is_empty());
        w.drive(|w| !w.in_flight());
        assert!(
            !seq_dir.exists(),
            "the export directory is deleted at the last variant"
        );
        assert_eq!(w.events(EVENT_SEQUENCE_DELETED).len(), 1);
    }

    #[test]
    fn failed_not_requeued() {
        let mut setup = Setup::new();
        setup.script.insert("exit_code".to_owned(), json!(1));
        setup.script.insert(
            "stderr_lines".to_owned(),
            json!(["Unknown encoder 'prores_ks'"]),
        );
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        let key = w.key(0);
        w.scan(&["seqA"]);
        w.drive(|w| !w.in_flight());
        assert_eq!(w.store.failures().expect("the failures").len(), 1);
        assert_eq!(w.jobs_of("seqA", &key).len(), 1);

        w.scan(&["seqA"]);
        assert_eq!(
            w.jobs_of("seqA", &key).len(),
            1,
            "the next scan does not queue the failed variant again"
        );

        control(&mut w, |w| w.renders.retry(&mut ctx!(w), "seqA", &key)).expect("retry");
        w.scan(&["seqA"]);
        assert_eq!(
            w.jobs_of("seqA", &key).len(),
            2,
            "retry lets the next scan queue it again"
        );
        w.drive(|w| !w.in_flight());
    }

    #[test]
    fn promotion_order() {
        let mut setup = Setup::new();
        setup.rendering = vec![pro_res(1280, 720), pro_res(1920, 1080)];
        setup.processes = 1;
        setup.per_sequence = 1;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.add_sequence("seqB");
        w.first_seen.insert("seqA".to_owned(), 100);
        w.first_seen.insert("seqB".to_owned(), 50);
        w.scan(&["seqA", "seqB"]);
        w.drive(|w| !w.in_flight());

        let started: Vec<(String, String)> = w
            .events(EVENT_RENDER_STARTED)
            .into_iter()
            .map(|row| {
                (
                    row.fields.sequence_name.unwrap_or_default(),
                    row.fields.output_filename.unwrap_or_default(),
                )
            })
            .collect();
        let expected: Vec<(String, String)> = [("seqB", 0), ("seqB", 1), ("seqA", 0), ("seqA", 1)]
            .into_iter()
            .map(|(seq, index)| {
                let name = plan::output_filename(
                    w.config.config().expect("the config"),
                    &w.codecs,
                    &w.rendering[index],
                    seq,
                );
                (seq.to_owned(), name)
            })
            .collect();
        assert_eq!(
            started, expected,
            "by sequence first sighting, then position in rendering"
        );
    }

    #[test]
    fn unwanted_jobs_removed() {
        let mut setup = Setup::new();
        setup.rendering = vec![pro_res(1280, 720), pro_res(1920, 1080)];
        setup.processes = 4;
        setup.per_sequence = 4;
        let mut w = World::new(setup);
        for name in ["seqA", "seqB", "seqC", "seqD"] {
            w.add_sequence(name);
        }
        control(&mut w, |w| w.renders.pause_all_rendering(&mut ctx!(w))).expect("pause all");
        w.scan(&["seqA", "seqB", "seqC", "seqD"]);
        assert_eq!(
            w.jobs(JobStatus::Queued).len(),
            8,
            "every variant is queued, none starts"
        );

        let rev = w.config.revision().to_owned();
        w.config
            .apply(|cfg| cfg.rendering.truncate(1), &rev, &w.codecs)
            .expect("drop the second variant");
        w.scan(&["seqA", "seqB", "seqC", "seqD"]);
        assert_eq!(
            w.jobs(JobStatus::Queued).len(),
            4,
            "a variant that left the config loses its jobs"
        );

        fs::write(w.output("seqB", 0), b"done").expect("an output appears");
        w.scan(&["seqA", "seqB", "seqC", "seqD"]);
        assert_eq!(
            w.jobs(JobStatus::Queued).len(),
            3,
            "a job whose output exists goes"
        );

        let rev = w.config.revision().to_owned();
        w.config
            .apply(
                |cfg| cfg.paused_sequence_names.push("seqC".to_owned()),
                &rev,
                &w.codecs,
            )
            .expect("pause seqC in config.json");
        w.scan(&["seqA", "seqB", "seqC", "seqD"]);
        assert_eq!(
            w.jobs(JobStatus::Queued).len(),
            2,
            "a paused sequence loses its jobs"
        );

        w.scan(&["seqA", "seqB", "seqC"]);
        let remaining: Vec<String> = w
            .jobs(JobStatus::Queued)
            .into_iter()
            .map(|job| job.sequence_name)
            .collect();
        assert_eq!(
            remaining,
            vec!["seqA".to_owned()],
            "only seqA's job is left: seqD is gone"
        );
    }

    #[test]
    fn rename_on_success() {
        let mut setup = Setup::new();
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        let job = w.only_in_flight("seqA", &key);
        w.release();
        w.drive(|w| !w.in_flight());
        let done = w
            .store
            .render_job(&job.id)
            .expect("the job")
            .expect("a job");
        assert_eq!(done.status, JobStatus::Done);
        assert!(
            Path::new(&job.output_path).is_file(),
            "the partial file became the final name"
        );
        assert!(
            !Path::new(&job.partial_path).exists(),
            "no partial file is left"
        );
        assert!(!Path::new(&job.progress_path.expect("a progress path")).exists());
    }

    #[test]
    fn final_name_taken() {
        let mut setup = Setup::new();
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        let job = w.only_in_flight("seqA", &key);
        fs::write(&job.output_path, b"existing").expect("a final file appears mid-render");
        w.release();
        w.drive(|w| !w.in_flight());

        assert_eq!(
            fs::read(&job.output_path).expect("the final file"),
            b"existing",
            "the final file is left alone"
        );
        assert!(
            !Path::new(&job.partial_path).exists(),
            "the partial file is deleted"
        );
        let done = w
            .store
            .render_job(&job.id)
            .expect("the job")
            .expect("a job");
        assert_eq!(
            done.status,
            JobStatus::Done,
            "the variant counts as already rendered"
        );
        assert!(
            w.store.rendered_videos().expect("the history").is_empty(),
            "no history row is written for a render that was not kept"
        );
    }

    #[test]
    fn stale_partial_sweep() {
        let mut setup = Setup::new();
        setup.processes = 1;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        let orphan = w.export.join(".orphan.partial.mov");
        fs::write(&orphan, b"x").expect("an orphan");
        w.renders.startup(&mut ctx!(w)).expect("start-up");
        assert!(
            !orphan.exists(),
            "start-up deletes an orphaned partial file"
        );

        control(&mut w, |w| w.renders.pause_all_rendering(&mut ctx!(w))).expect("pause all");
        w.scan(&["seqA"]);
        let owned = PathBuf::from(&w.jobs(JobStatus::Queued)[0].partial_path);
        fs::write(&owned, b"x").expect("the live job's partial file");
        let later = w.export.join(".later.partial.mov");
        fs::write(&later, b"x").expect("a later orphan");

        w.tick();
        assert!(later.exists(), "the sweep waits for its interval");
        w.now += SWEEP_INTERVAL_MS;
        w.tick();
        assert!(!later.exists(), "the 10-minute sweep deletes the orphan");
        assert!(owned.exists(), "a file a live job owns is never deleted");
    }

    #[test]
    fn render_log_retention() {
        let mut setup = Setup::new();
        setup.processes = 1;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        w.drive(|w| !w.in_flight());
        let job = w.jobs_of("seqA", &key).remove(0);
        assert!(
            !Path::new(&job.progress_path.clone().expect("a progress path")).exists(),
            "the progress file is deleted when the job ends"
        );
        let log = PathBuf::from(job.log_path.clone().expect("a log path"));
        assert!(log.is_file(), "the render log is kept");

        for index in 1..=205_i64 {
            let id = format!("old-{index}");
            let log_path = w.data.join("renders").join(format!("{id}.log"));
            fs::write(&log_path, b"old").expect("an old log");
            let old = RenderJob {
                id,
                queued_at: index,
                status: JobStatus::Done,
                log_path: Some(log_path.to_string_lossy().into_owned()),
                ..job.clone()
            };
            w.store.put_render_job(&old).expect("an old job");
        }
        let deleted = retain_render_logs(&w.store, RENDER_LOG_LIMIT).expect("retention");
        assert_eq!(deleted, 6, "206 logs, the newest 200 are kept");
        assert!(!w.data.join("renders/old-6.log").exists());
        assert!(w.data.join("renders/old-7.log").exists());
        assert!(log.is_file(), "the newest log is kept");
    }

    #[test]
    fn failure_recorded_and_merged() {
        let mut setup = Setup::new();
        setup.processes = 1;
        setup.script.insert("exit_code".to_owned(), json!(1));
        setup.script.insert(
            "stderr_lines".to_owned(),
            json!(["Unknown encoder 'prores_ks'"]),
        );
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key = w.key(0);
        w.drive(|w| !w.in_flight());

        let cards = w.store.failures().expect("the failures");
        assert_eq!(cards.len(), 1);
        let card = cards[0].clone();
        assert_eq!(card.kind, "FAILURE_FFMPEG_REJECTED_ARGUMENTS");
        assert_eq!(card.occurrences, 1);
        assert_eq!(card.exit_code, Some(1));
        assert!(
            card.tool_output_json.contains("Unknown encoder"),
            "the log tail is kept"
        );
        assert!(
            w.events(EVENT_RENDER_FAILED)
                .iter()
                .any(|row| row.level == LogLevel::Error
                    && row.fields.failure_id.as_deref() == Some(&card.id)),
            "the first occurrence is logged at ERROR with the full report"
        );

        let original = w.jobs_of("seqA", &key).remove(0);
        let repeat = RenderJob {
            id: "repeat-job".to_owned(),
            status: JobStatus::Queued,
            pid: None,
            pid_start_time: None,
            started_at: None,
            finished_at: None,
            log_path: Some(
                w.data
                    .join("renders/repeat-job.log")
                    .to_string_lossy()
                    .into_owned(),
            ),
            progress_path: Some(
                w.data
                    .join("renders/repeat-job.progress")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..original
        };
        w.store.put_render_job(&repeat).expect("a repeat job");
        w.drive(|w| !w.in_flight());

        let merged = w.store.failures().expect("the failures");
        assert_eq!(merged.len(), 1, "the repeat adds no card");
        assert_eq!(merged[0].id, card.id, "the card keeps its id");
        assert_eq!(merged[0].occurrences, 2);
    }

    #[test]
    fn controls_logged_and_validated() {
        let mut setup = Setup::new();
        setup.rendering = vec![pro_res(1280, 720), pro_res(1920, 1080)];
        setup.processes = 1;
        setup.per_sequence = 1;
        setup.hold = true;
        let mut w = World::new(setup);
        w.add_sequence("seqA");
        w.scan(&["seqA"]);
        let key0 = w.key(0);
        let key1 = w.key(1);
        let running = w.jobs(JobStatus::Running).remove(0);

        control(&mut w, |w| w.renders.pause_all_rendering(&mut ctx!(w))).expect("pause all");
        assert_code(
            control(&mut w, |w| w.renders.pause_all_rendering(&mut ctx!(w))),
            "FAILED_PRECONDITION",
        );
        control(&mut w, |w| w.renders.resume_all_rendering(&mut ctx!(w))).expect("resume all");
        assert_code(
            control(&mut w, |w| w.renders.resume_all_rendering(&mut ctx!(w))),
            "FAILED_PRECONDITION",
        );

        assert_code(
            control(&mut w, |w| {
                w.renders.resume_render(&mut ctx!(w), &running.id)
            }),
            "FAILED_PRECONDITION",
        );
        control(&mut w, |w| {
            w.renders.suspend_render(&mut ctx!(w), &running.id)
        })
        .expect("suspend");
        assert_code(
            control(&mut w, |w| {
                w.renders.suspend_render(&mut ctx!(w), &running.id)
            }),
            "FAILED_PRECONDITION",
        );
        assert_code(
            control(&mut w, |w| w.renders.suspend_render(&mut ctx!(w), "nope")),
            "NOT_FOUND",
        );
        control(&mut w, |w| {
            w.renders.resume_render(&mut ctx!(w), &running.id)
        })
        .expect("resume");

        assert_code(
            control(&mut w, |w| w.renders.dismiss_failure(&mut ctx!(w), "nope")),
            "NOT_FOUND",
        );
        let card = Failure {
            id: "card-1".to_owned(),
            output_path: w.output("seqA", 0).to_string_lossy().into_owned(),
            kind: "FAILURE_UNKNOWN".to_owned(),
            first_seen: 1,
            last_seen: 1,
            occurrences: 1,
            sequence_name: "seqA".to_owned(),
            export_path: w.export.to_string_lossy().into_owned(),
            output_filename: "card.mov".to_owned(),
            variant_key: key0.clone(),
            summary: "a card".to_owned(),
            detail: String::new(),
            exit_code: None,
            command_line: String::new(),
            tool_output_json: "[]".to_owned(),
            variant_auto_paused: false,
            render_id: None,
        };
        w.store.record_failure(&card).expect("a card");
        control(&mut w, |w| {
            w.renders.dismiss_failure(&mut ctx!(w), "card-1")
        })
        .expect("dismiss");

        control(&mut w, |w| {
            w.renders.pause_variant(&mut ctx!(w), "seqA", &key1)
        })
        .expect("pause the queued variant");
        assert_code(
            control(&mut w, |w| {
                w.renders.pause_variant(&mut ctx!(w), "seqA", &key1)
            }),
            "FAILED_PRECONDITION",
        );
        assert_code(
            control(&mut w, |w| {
                w.renders.pause_variant(&mut ctx!(w), "seqA", "bogus")
            }),
            "NOT_FOUND",
        );
        control(&mut w, |w| {
            w.renders.resume_variant(&mut ctx!(w), "seqA", &key1)
        })
        .expect("resume the variant");
        assert_code(
            control(&mut w, |w| {
                w.renders.resume_variant(&mut ctx!(w), "seqA", &key1)
            }),
            "FAILED_PRECONDITION",
        );
        assert_code(
            control(&mut w, |w| w.renders.resume_sequence(&mut ctx!(w), "seqA")),
            "FAILED_PRECONDITION",
        );
        control(&mut w, |w| w.renders.pause_sequence(&mut ctx!(w), "seqA")).expect("pause seqA");
        assert_code(
            control(&mut w, |w| w.renders.pause_sequence(&mut ctx!(w), "seqA")),
            "FAILED_PRECONDITION",
        );
        control(&mut w, |w| w.renders.resume_sequence(&mut ctx!(w), "seqA")).expect("resume seqA");
        assert_code(
            control(&mut w, |w| w.renders.stop_render(&mut ctx!(w), &running.id)),
            "FAILED_PRECONDITION",
        );
        assert_code(
            control(&mut w, |w| w.renders.stop_render(&mut ctx!(w), "nope")),
            "NOT_FOUND",
        );
        assert_code(
            control(&mut w, |w| w.renders.retry(&mut ctx!(w), "seqA", &key1)),
            "FAILED_PRECONDITION",
        );
        let rescanned = control(&mut w, |w| w.renders.rescan_now(&mut ctx!(w))).expect("rescan");
        assert!(rescanned.rescan);

        w.release();
        drain(&mut w);
        let infos = w.control_rows();
        assert!(
            infos >= 10,
            "every applied control wrote its row, {infos} in all"
        );
        let messages: Vec<String> = w
            .log_rows()
            .into_iter()
            .filter(|row| row.subject == "controls")
            .map(|row| row.message)
            .collect();
        assert!(messages.iter().any(|m| m.contains("pause all rendering")));
        assert!(
            messages
                .iter()
                .any(|m| m.contains("dismiss failure card-1"))
        );
        assert!(messages.iter().any(|m| m.contains("pause variant seqA/")));
        assert!(messages.iter().any(|m| m.contains("rescan now")));
    }

    fn assert_code<T: std::fmt::Debug>(result: Result<T, ControlError>, code: &str) {
        match result {
            Err(err) => assert_eq!(err.code(), code, "the code of {err}"),
            Ok(value) => panic!("expected {code}, got {value:?}"),
        }
    }
}
