// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The client commands of trd (spec 1 §5, TRD-20): `trd status`, the controls, `pair`, `devices`,
//! `log`, `config`, and the offline `scan --once`.
//!
//! Every client command runs through [`client`], which always passes `--address` with the test
//! daemon's port, so no test reaches `127.0.0.1:50071` (the owner's `trd`). Each daemon starts with
//! `--sandbox --no-mdns --lan-bind off --port 0` and temporary data, log and config folders, and the
//! fake tools in a temporary folder. The only waits are for a state the daemon reaches, under a
//! hang guard.

// The view under test is the trd binary's own module, included by path: the binary is not a library.
#[path = "../src/status_view.rs"]
#[allow(dead_code)]
mod status_view;

mod cli_client {
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdout, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use trd_proto::trd::v1 as pb;
    use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
    use trd_testtools::{fake_bin_dir, make_tools_dir};

    use super::status_view::{self, Style};

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    /// The export directory the fixture sequence is named after.
    const SEQUENCE: &str = "LRT_0001";
    /// The variant key of the fixture's one rendering entry (`h264`, original size, 30 fps).
    const VARIANT: &str = "h264_ori_30";

    /// 2026-11-25 14:00:00 UTC, in milliseconds.
    const NOW_MS: i64 = 1_795_615_200_000;

    type Client = TrdServiceClient<tonic::transport::Channel>;

    /// Runs `trd --address <address> <args>` and waits for it. Every client test goes through here,
    /// so `--address` is never left to the default.
    fn client(address: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trd"))
            .arg("--address")
            .arg(address)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("the trd test binary starts")
    }

    fn stdout_of(out: &Output) -> String {
        String::from_utf8(out.stdout.clone()).expect("utf-8 output")
    }

    fn stderr_of(out: &Output) -> String {
        String::from_utf8(out.stderr.clone()).expect("utf-8 output")
    }

    fn assert_success(out: &Output) {
        assert!(
            out.status.success(),
            "exit {:?}; stderr: {}",
            out.status.code(),
            stderr_of(out)
        );
    }

    /// The temporary folders of one test. Removed when the test ends.
    struct Fixture {
        root: TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::Builder::new()
                .prefix("trd-cli-test-")
                .tempdir()
                .expect("a temporary directory");
            make_tools_dir(&fake_bin_dir(), &root.path().join("tools")).expect("the fake tools");
            Fixture { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.path().join(relative)
        }

        /// A source folder with the raw sequence `SEQUENCE` of three frames, and an export folder
        /// holding one of them: the sequence is in progress and never renders. Returns the config
        /// path, written under `config/`.
        fn with_sequence(&self) -> PathBuf {
            let source = self.path("source").join(SEQUENCE);
            let export = self.path("export");
            fs::create_dir_all(&source).expect("the source folder");
            fs::create_dir_all(export.join(SEQUENCE)).expect("the export folder");
            for frame in 1..=3 {
                fs::write(source.join(format!("frame_{frame:04}.tif")), b"raw").expect("a frame");
            }
            fs::write(export.join(SEQUENCE).join("frame_0001.tif"), b"out").expect("a frame");
            let config = self.path("config/config.json");
            fs::create_dir_all(config.parent().expect("a parent")).expect("the config folder");
            write_config(&config, &self.path("source"), &export);
            config
        }
    }

    /// A config that names one source, one export folder and one h264 rendering at 30 fps.
    fn write_config(config: &Path, source: &Path, export: &Path) {
        let text = json!({
            "source_paths": [source.to_string_lossy()],
            "export_paths": [export.to_string_lossy()],
            "backup_paths": [],
            "master_videos": [],
            "raw_types": ["tif"],
            "image_extensions": ["tif"],
            "image_name_prefix": "",
            "image_sequence_dirname_prefix": "",
            "rendering": [{"codec": "h264", "frame_rate": 30}],
        });
        fs::write(config, text.to_string()).expect("the config file");
    }

    /// A running `trd daemon` child on a loopback port of its own. Killed when dropped.
    struct Daemon {
        child: Child,
        #[allow(dead_code)]
        stdout: BufReader<ChildStdout>,
        address: String,
    }

    impl Daemon {
        /// Starts the daemon for `fixture` with `config` and waits for its `TRD_LISTENING` line.
        fn start(fixture: &Fixture, config: &Path) -> Daemon {
            let mut child = Command::new(env!("CARGO_BIN_EXE_trd"))
                .arg("daemon")
                .args(["--port", "0"])
                .args(["--lan-bind", "off"])
                .args(["--sandbox", "--no-mdns"])
                .arg("--data-dir")
                .arg(fixture.path("data"))
                .arg("--log-dir")
                .arg(fixture.path("logs"))
                .arg("--config")
                .arg(config)
                .env("TRD_FFMPEG_DIR", fixture.path("tools"))
                .env("TRD_EXIFTOOL_DIR", fixture.path("tools"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("trd daemon starts");
            let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
            let mut line = String::new();
            stdout.read_line(&mut line).expect("the listening line");
            let port = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix("loopback=127.0.0.1:"))
                .and_then(|port| port.parse::<u16>().ok())
                .unwrap_or_else(|| panic!("no loopback port in {line:?}"));
            Daemon {
                child,
                stdout,
                address: format!("127.0.0.1:{port}"),
            }
        }

        /// The loopback gRPC client: plaintext, every RPC allowed.
        async fn grpc(&self) -> Client {
            TrdServiceClient::connect(format!("http://{}", self.address))
                .await
                .expect("the loopback listener accepts")
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    /// Waits until the daemon's snapshot satisfies `ready`, and returns it.
    async fn snapshot_where(
        grpc: &mut Client,
        ready: impl Fn(&pb::Snapshot) -> bool,
    ) -> pb::Snapshot {
        let started = Instant::now();
        loop {
            let snapshot = grpc
                .get_snapshot(pb::GetSnapshotRequest {})
                .await
                .expect("a snapshot")
                .into_inner();
            if ready(&snapshot) {
                return snapshot;
            }
            assert!(
                started.elapsed() < HANG_GUARD,
                "the daemon never reached the state; last snapshot: {snapshot:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn config_of(grpc: &mut Client) -> pb::Config {
        grpc.get_config(pb::GetConfigRequest {})
            .await
            .expect("the config")
            .into_inner()
            .config
            .expect("a config with defaults applied")
    }

    /// A fixture snapshot with a fixed clock, so the golden text does not change.
    fn fixture_snapshot() -> pb::Snapshot {
        pb::Snapshot {
            revision: 7,
            generated_at_ms: NOW_MS,
            phase: pb::Phase::Rendering as i32,
            global_pause: false,
            problems: vec![pb::Problem {
                id: "tool:ffmpeg".to_owned(),
                severity: pb::Severity::Error as i32,
                title: "ffmpeg is missing".to_owned(),
                detail: "No ffmpeg was found beside trd.".to_owned(),
                fix: "Install the ffmpeg package.".to_owned(),
                ..Default::default()
            }],
            failures: vec![pb::FailureCard {
                id: "f-1".to_owned(),
                sequence_name: "LRT_0003".to_owned(),
                output_filename: "LRT_0003_h264.mov".to_owned(),
                kind_label: "Disk full".to_owned(),
                summary: "The disk ran out of space.".to_owned(),
                occurrences: 2,
                ..Default::default()
            }],
            in_progress: vec![
                pb::SequenceCard {
                    name: SEQUENCE.to_owned(),
                    export_path: "/exports".to_owned(),
                    expected_frames: 120,
                    exported_frames: 120,
                    stage: pb::Stage::Rendering as i32,
                    export_size_bytes: -1,
                    export_eta_seconds: -1,
                    variants: vec![pb::VariantRow {
                        codec: "ProResMQ".to_owned(),
                        resolution_label: "2160p".to_owned(),
                        frame_rate: 30,
                        status: pb::VariantStatus::VariantRunning as i32,
                        frame: 60,
                        total_frames: 120,
                        fps: 4.5,
                        eta_seconds: 14,
                        size_bytes: -1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                pb::SequenceCard {
                    name: "LRT_0002".to_owned(),
                    export_path: "/exports".to_owned(),
                    expected_frames: 1500,
                    exported_frames: 600,
                    stage: pb::Stage::Exporting as i32,
                    paused: true,
                    export_frames_per_minute: 41.0,
                    export_eta_seconds: 1200,
                    export_size_bytes: 2048,
                    ..Default::default()
                },
            ],
            recently_rendered: vec![pb::RenderedVideo {
                rendered_at_ms: NOW_MS - 12 * 60 * 1000,
                sequence_name: SEQUENCE.to_owned(),
                export_path: "/exports".to_owned(),
                output_filename: "LRT_0001_ProResMQ_2160p.mov".to_owned(),
                codec: "ProResMQ".to_owned(),
                resolution_label: "2160p".to_owned(),
                frame_rate: 30,
                frame_count: 120,
                size_bytes: 12_897_484,
                duration_seconds: 4.0,
                file_exists: true,
                ..Default::default()
            }],
            recently_rendered_total: 12,
            counts: Some(pb::Counts {
                rendering: 1,
                exporting: 1,
                failures: 1,
                rendered_last_24h: 3,
                ..Default::default()
            }),
            config_path: "/fixture/config.json".to_owned(),
            ..Default::default()
        }
    }

    /// The `trd status --plain` text of [`fixture_snapshot`]: the Activity view's sections in the
    /// overview's order, with no colour.
    const GOLDEN: &str = r#"Config: /fixture/config.json
Phase: rendering    Pause all rendering: off
1 rendering · 1 exporting · 1 failed · 3 rendered in 24 h

Problems
  ERROR  ffmpeg is missing
      No ffmpeg was found beside trd.
      Fix: Install the ffmpeg package.

Needs attention (1)
  [Disk full] LRT_0003_h264.mov  ×2
      The disk ran out of space.
      id f-1

In progress (2)
  LRT_0001  [rendering]
      [**********] 120 / 120 frames
      /exports
      ProResMQ · 2160p · 30 fps  [*****-----] 60 / 120 frames · 4.5 fps · ~14 s left  rendering
  LRT_0002  [exporting] (paused)
      [****------] 600 / 1,500 frames · 41 frames/min · ~20 min left · 2.0 KiB
      /exports

Recently rendered (newest 1 of 12)
  2026-11-25 13:48 · 12 min ago · LRT_0001_ProResMQ_2160p.mov
      ProResMQ · 2160p · 30 fps · 120 frames · 0:04 · 12.3 MiB
      /exports
"#;

    #[test]
    fn status_plain_golden() {
        let style = Style {
            bar_width: 10,
            color: false,
        };
        let lines = status_view::render(&fixture_snapshot(), style, NOW_MS, 0);
        assert_eq!(format!("{}\n", lines.join("\n")), GOLDEN);
    }

    #[test]
    fn status_plain_has_no_colour() {
        let style = Style {
            bar_width: 40,
            color: false,
        };
        let lines = status_view::render(&fixture_snapshot(), style, NOW_MS, 0);
        assert!(lines.iter().all(|line| !line.contains('\x1b')));
    }

    /// A loopback address with nothing listening on it: the port was free a moment ago.
    fn unused_address() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = listener.local_addr().expect("the port").port();
        drop(listener);
        format!("127.0.0.1:{port}")
    }

    /// `scan --once --config <config>` through the client helper. No daemon is involved.
    fn scan_once(config: &Path) -> Output {
        client(
            &unused_address(),
            &[
                "scan",
                "--once",
                "--config",
                config.to_str().expect("a UTF-8 path"),
            ],
        )
    }

    #[test]
    fn client_uses_test_address() {
        let address = unused_address();
        let out = client(&address, &["status", "--plain"]);
        assert_eq!(out.status.code(), Some(2), "stdout: {}", stdout_of(&out));
        let stderr = stderr_of(&out);
        let port = address.rsplit(':').next().expect("a port");
        assert!(
            stderr.contains(&format!("trd is not running on 127.0.0.1:{port}")),
            "unexpected stderr: {stderr}"
        );
    }

    #[test]
    fn scan_once_lists_complete_sequences() {
        let fixture = Fixture::new();
        let config = fixture.with_sequence();
        // A second raw sequence whose export is incomplete, and a complete one with frames exported.
        fs::create_dir_all(fixture.path("source/LRT_0004")).expect("a raw sequence");
        for frame in 1..=2 {
            fs::write(
                fixture.path(&format!("source/LRT_0004/frame_{frame:04}.tif")),
                b"raw",
            )
            .expect("a frame");
        }
        fs::create_dir_all(fixture.path("export/LRT_0004")).expect("an export folder");
        fs::write(fixture.path("export/LRT_0004/frame_0001.tif"), b"out").expect("a frame");

        let before = tree_listing(&fixture.path("export"));
        let out = scan_once(&config);
        assert_success(&out);
        let stdout = stdout_of(&out);
        assert!(
            stdout.contains("LRT_0001  1 of 3 frames  in progress"),
            "unexpected output: {stdout}"
        );
        assert!(
            stdout.contains("LRT_0004  1 of 2 frames  in progress"),
            "unexpected output: {stdout}"
        );
        assert!(
            !stdout.contains("would render"),
            "no sequence is complete yet: {stdout}"
        );
        assert_eq!(
            tree_listing(&fixture.path("export")),
            before,
            "scan wrote files"
        );

        // Complete the first export: its output names now show up.
        for frame in 2..=3 {
            fs::write(
                fixture.path(&format!("export/{SEQUENCE}/frame_{frame:04}.tif")),
                b"out",
            )
            .expect("a frame");
        }
        let out = scan_once(&config);
        assert_success(&out);
        let stdout = stdout_of(&out);
        assert!(
            stdout.contains(&format!("{SEQUENCE}  3 of 3 frames  complete")),
            "unexpected output: {stdout}"
        );
        let wanted: Vec<&str> = stdout
            .lines()
            .filter(|line| line.contains("would render"))
            .collect();
        assert_eq!(wanted.len(), 1, "one rendering entry: {stdout}");
        let output_path = wanted[0]
            .trim_start()
            .strip_prefix("would render ")
            .expect("the prefix")
            .to_owned();
        assert!(
            output_path.contains(&format!("{SEQUENCE}_")),
            "{output_path}"
        );
        assert!(output_path.ends_with(".mov"), "{output_path}");

        // An output that already exists is skipped.
        let existing = PathBuf::from(&output_path);
        fs::write(&existing, b"already here").expect("the existing output");
        let out = scan_once(&config);
        assert_success(&out);
        let stdout = stdout_of(&out);
        assert!(!stdout.contains("would render"), "skipped: {stdout}");
        assert!(
            stdout.contains("already rendered, skipped: 1"),
            "unexpected output: {stdout}"
        );
    }

    /// Every file under `root` with its length, for checking that nothing was written.
    fn tree_listing(root: &Path) -> Vec<(PathBuf, u64)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("a folder") {
                let entry = entry.expect("an entry");
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push((path.clone(), fs::metadata(&path).expect("metadata").len()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn config_validate_reads_a_file_offline() {
        let fixture = Fixture::new();
        let good = fixture.path("good.json");
        write_config(&good, &fixture.path("source"), &fixture.path("export"));
        let out = client(
            &unused_address(),
            &["config", "validate", good.to_str().expect("a UTF-8 path")],
        );
        assert_success(&out);
        assert!(stdout_of(&out).contains("is valid"), "{}", stdout_of(&out));

        let bad = fixture.path("bad.json");
        fs::write(&bad, "{not json").expect("the bad config");
        let out = client(
            &unused_address(),
            &["config", "validate", bad.to_str().expect("a UTF-8 path")],
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(
            stderr_of(&out).contains("not valid JSON"),
            "{}",
            stderr_of(&out)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_commands_change_the_daemon_state() {
        let fixture = Fixture::new();
        let config = fixture.with_sequence();
        let daemon = Daemon::start(&fixture, &config);
        let address = daemon.address.clone();
        let mut grpc = daemon.grpc().await;
        snapshot_where(&mut grpc, |snapshot| {
            snapshot
                .in_progress
                .iter()
                .any(|card| card.name == SEQUENCE)
        })
        .await;

        // Sequence pause and resume.
        assert_success(&client(&address, &["pause", SEQUENCE]));
        assert!(
            config_of(&mut grpc)
                .await
                .paused_sequence_names
                .contains(&SEQUENCE.to_owned())
        );
        assert_success(&client(&address, &["resume", SEQUENCE]));
        assert!(
            !config_of(&mut grpc)
                .await
                .paused_sequence_names
                .contains(&SEQUENCE.to_owned())
        );

        // Variant pause and resume.
        assert_success(&client(
            &address,
            &["pause", SEQUENCE, "--variant", VARIANT],
        ));
        let paused = config_of(&mut grpc).await.paused_variants;
        assert!(
            paused
                .iter()
                .any(|v| v.sequence_name == SEQUENCE && v.variant_key == VARIANT),
            "paused variants: {paused:?}"
        );
        assert_success(&client(
            &address,
            &["resume", SEQUENCE, "--variant", VARIANT],
        ));
        assert!(config_of(&mut grpc).await.paused_variants.is_empty());

        // Global pause.
        assert_success(&client(&address, &["pause-all"]));
        let snapshot = grpc
            .get_snapshot(pb::GetSnapshotRequest {})
            .await
            .expect("a snapshot")
            .into_inner();
        assert!(snapshot.global_pause);
        assert_success(&client(&address, &["resume-all"]));
        let snapshot = grpc
            .get_snapshot(pb::GetSnapshotRequest {})
            .await
            .expect("a snapshot")
            .into_inner();
        assert!(!snapshot.global_pause);

        assert_success(&client(&address, &["rescan"]));

        // Per-render and retry controls refuse ids the daemon does not know.
        for args in [
            vec!["render", "suspend", "no-such-render"],
            vec!["render", "resume", "no-such-render"],
            vec!["render", "stop", "no-such-render"],
            vec!["retry", "no-such-sequence", VARIANT],
        ] {
            let out = client(&address, &args);
            assert_eq!(out.status.code(), Some(1), "{args:?}");
            assert!(
                stderr_of(&out).starts_with("trd: "),
                "{args:?}: {}",
                stderr_of(&out)
            );
        }

        // Failures: none yet; dismissing an unknown one is refused; dismiss-all succeeds.
        let out = client(&address, &["failures"]);
        assert_success(&out);
        assert!(stdout_of(&out).contains("no failures"));
        let out = client(&address, &["failures", "--dismiss", "no-such-failure"]);
        assert_eq!(out.status.code(), Some(1));
        assert_success(&client(&address, &["failures", "--dismiss-all"]));

        // Config: the raw file, its path, and a new path read back.
        let out = client(&address, &["config", "show"]);
        assert_success(&out);
        let shown: Value = serde_json::from_str(&stdout_of(&out)).expect("the config JSON");
        assert_eq!(shown["rendering"][0]["codec"], "h264");
        let out = client(&address, &["config", "path"]);
        assert_eq!(stdout_of(&out).trim_end(), config.to_string_lossy());

        // Pairing: the §5 JSON object, then a device that is listed and revoked.
        let out = client(&address, &["devices"]);
        assert!(
            stdout_of(&out).contains("no paired devices"),
            "{}",
            stdout_of(&out)
        );
        let out = client(&address, &["pair", "--json"]);
        assert_success(&out);
        let pairing: Value = serde_json::from_str(&stdout_of(&out)).expect("the pairing JSON");
        let mut keys: Vec<&str> = pairing
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "code",
                "expires_at_ms",
                "fingerprint",
                "fingerprint_prefix",
                "lan_addresses",
                "lan_port",
                "server_name",
            ]
        );
        let code = pairing["code"].as_str().expect("the code").to_owned();
        assert_eq!(code.len(), 6);
        let fingerprint = pairing["fingerprint"].as_str().expect("the fingerprint");
        assert_eq!(pairing["fingerprint_prefix"], &fingerprint[..16]);

        let paired = grpc
            .pair(pb::PairRequest {
                code,
                device_name: "test phone".to_owned(),
                platform: pb::DevicePlatform::Ios as i32,
            })
            .await
            .expect("the pairing succeeds")
            .into_inner();
        let out = client(&address, &["devices"]);
        assert_success(&out);
        assert!(
            stdout_of(&out).contains(&paired.device_id),
            "{}",
            stdout_of(&out)
        );
        assert!(stdout_of(&out).contains("test phone"));
        assert_success(&client(
            &address,
            &["devices", "--revoke", paired.device_id.as_str()],
        ));
        let out = client(&address, &["devices"]);
        assert!(
            !stdout_of(&out).contains(&paired.device_id),
            "{}",
            stdout_of(&out)
        );

        // The config path moves; the log records it as its newest line.
        let moved = fixture.path("moved/config.json");
        assert_success(&client(
            &address,
            &["config", "set-path", moved.to_str().expect("a UTF-8 path")],
        ));
        let out = client(&address, &["config", "path"]);
        assert_eq!(stdout_of(&out).trim_end(), moved.to_string_lossy());
        let out = client(&address, &["log", "--limit", "5"]);
        assert_success(&out);
        let last = stdout_of(&out)
            .lines()
            .last()
            .expect("a log line")
            .to_owned();
        assert!(last.contains("config path set to"), "newest last: {last}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pause_and_resume_refuse_an_unknown_sequence() {
        let fixture = Fixture::new();
        let config = fixture.with_sequence();
        let daemon = Daemon::start(&fixture, &config);
        let address = daemon.address.clone();
        let mut grpc = daemon.grpc().await;
        snapshot_where(&mut grpc, |snapshot| !snapshot.in_progress.is_empty()).await;
        let out = client(&address, &["pause", "no-such-sequence"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr_of(&out).starts_with("trd: "), "{}", stderr_of(&out));
    }
}
