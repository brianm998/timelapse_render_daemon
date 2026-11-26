// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the `make develop` sandbox (TRD-28, spec 1 §12): `sandbox.py` builds the tree, drips
//! sequence A, resets the tree, and a `trd daemon --sandbox` started on the tree renders, fails and
//! hides the sequences §12 says it must.
//!
//! Everything runs in temporary folders with the fake tools. The daemon starts with `--sandbox
//! --no-mdns --port 0 --lan-bind 127.0.0.1:0`, so no test binds 50071 or 50072, reads the owner's
//! data, or signals a process it did not start. The only waits are polls of a state the daemon
//! reaches, under a hang guard.

mod sandbox {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdout, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use tempfile::TempDir;
    use trd_proto::trd::v1 as pb;
    use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
    use trd_testtools::{fake_bin_dir, make_tools_dir};

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(120);
    /// How often the snapshot is polled.
    const POLL: Duration = Duration::from_millis(200);
    /// The sandbox tool, from the crate directory (crates/trd) up to v3/trd.
    const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../sandbox/sandbox.py");
    /// Sequence C fails: the fake ffmpeg exits 1 for every render whose arguments name it.
    const FAKE_FFMPEG: &str = r#"{"rules":[{"match":"sandbox-c","exit_code":1,"stderr_lines":["bad source frame: LRT_00012.tif"]}]}"#;
    /// The sandbox frames are 320 by 213.
    const FAKE_FFPROBE: &str = r#"{"width":320,"height":213}"#;
    /// Sequence A's export folder, which init leaves empty and drip fills.
    const A_EXPORT: &str = "export/LRT_2026_01_01-sandbox-a";
    /// Sequence G's export folder, which the daemon deletes after its renders.
    const G_EXPORT: &str = "export/LRT_2026_01_01-sandbox-g-merge";

    /// The temporary folders of one test, removed when the test ends. The fake tools live beside
    /// the tree, never in the worktree.
    struct Fixture {
        root: TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::Builder::new()
                .prefix("trd-sandbox-test-")
                .tempdir()
                .expect("a temporary directory");
            make_tools_dir(&fake_bin_dir(), &root.path().join("tools")).expect("the fake tools");
            Fixture { root }
        }

        fn tools(&self) -> PathBuf {
            self.root.path().join("tools")
        }

        /// The sandbox folder, which init creates.
        fn tree(&self) -> PathBuf {
            self.root.path().join("tree")
        }
    }

    /// The Python 3 interpreter: `PYTHON` when set (the Makefile's variable), else the first of
    /// `python3` and `python` that is Python 3. Windows runners often have only `python`.
    fn python() -> OsString {
        if let Some(chosen) = std::env::var_os("PYTHON") {
            return chosen;
        }
        for candidate in ["python3", "python"] {
            let is_python3 = Command::new(candidate)
                .args([
                    "-c",
                    "import sys; sys.exit(0 if sys.version_info[0] == 3 else 1)",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if is_python3 {
                return OsString::from(candidate);
            }
        }
        OsString::from("python3")
    }

    /// Runs `sandbox.py` with `args` and the fake tools, and returns its output.
    fn sandbox(fixture: &Fixture, args: &[&OsStr]) -> Output {
        Command::new(python())
            .arg("-I")
            .arg(SCRIPT)
            .args(args)
            .env("TRD_FFMPEG_DIR", fixture.tools())
            .env("TRD_EXIFTOOL_DIR", fixture.tools())
            .env_remove("TRD_FAKE_FFMPEG")
            .env_remove("TRD_FAKE_FFPROBE")
            .stdin(Stdio::null())
            .output()
            .expect("the Python 3 interpreter runs sandbox.py")
    }

    /// Runs `sandbox.py` with the sandbox folder `dir` and `extra` arguments, and asserts success.
    fn expect_ok(fixture: &Fixture, dir: &Path, command: &str, extra: &[&str]) -> String {
        let mut args: Vec<&OsStr> = vec![OsStr::new(command), OsStr::new("--dir"), dir.as_os_str()];
        args.extend(extra.iter().map(OsStr::new));
        let output = sandbox(fixture, &args);
        assert!(
            output.status.success(),
            "sandbox.py {command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Builds the tree in the fixture's sandbox folder.
    fn build_tree(fixture: &Fixture) {
        expect_ok(fixture, &fixture.tree(), "init", &[]);
    }

    /// The regular, non-hidden files directly in `dir`.
    fn frame_count(dir: &Path) -> usize {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        !entry.file_name().to_string_lossy().starts_with('.')
                            && entry.path().is_file()
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    /// A running `trd daemon --sandbox` on the tree. Killed when dropped.
    struct Daemon {
        child: Child,
        #[allow(dead_code)]
        stdout: BufReader<ChildStdout>,
        port: u16,
    }

    impl Daemon {
        /// Starts the daemon on `tree` and waits for its `TRD_LISTENING` line.
        fn start(fixture: &Fixture, tree: &Path) -> Daemon {
            let mut child = Command::new(env!("CARGO_BIN_EXE_trd"))
                .arg("daemon")
                .arg("--sandbox")
                .arg("--no-mdns")
                .args(["--port", "0"])
                .args(["--lan-bind", "127.0.0.1:0"])
                .arg("--config")
                .arg(tree.join("config.json"))
                .arg("--data-dir")
                .arg(tree.join("data"))
                .arg("--log-dir")
                .arg(tree.join("logs"))
                .env("TRD_FFMPEG_DIR", fixture.tools())
                .env("TRD_EXIFTOOL_DIR", fixture.tools())
                .env("TRD_FAKE_FFMPEG", FAKE_FFMPEG)
                .env("TRD_FAKE_FFPROBE", FAKE_FFPROBE)
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
                port,
            }
        }

        /// A plaintext client on the loopback listener.
        async fn client(&self) -> TrdServiceClient<tonic::transport::Channel> {
            TrdServiceClient::connect(format!("http://127.0.0.1:{}", self.port))
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

    /// The renders of the sequences whose export folder name contains `sequence`.
    fn renders_of<'a>(snapshot: &'a pb::Snapshot, sequence: &str) -> Vec<&'a pb::RenderedVideo> {
        snapshot
            .recently_rendered
            .iter()
            .filter(|video| video.sequence_name.contains(sequence))
            .collect()
    }

    /// Fails when a snapshot mentions sequence D or E anywhere: §12 says they are never shown.
    fn assert_hidden(snapshot: &pb::Snapshot) {
        let text = format!("{snapshot:?}");
        for hidden in ["sandbox-d", "sandbox-e"] {
            assert!(
                !text.contains(hidden),
                "the snapshot mentions {hidden}: {snapshot:?}"
            );
        }
    }

    /// A2: `init` builds a valid tree, `drip` completes sequence A, `path` prints the folder, and
    /// `init --reset` recreates the tree and removes what the old tree held.
    #[test]
    fn init_drip_path_and_reset() {
        let fixture = Fixture::new();
        let tree = fixture.tree();
        let dir = tree.as_path();

        build_tree(&fixture);
        expect_ok(&fixture, dir, "init", &["--check"]);
        assert_eq!(
            frame_count(&tree.join(A_EXPORT)),
            0,
            "A starts with no exported frames"
        );

        // Running init again on a valid tree changes nothing.
        expect_ok(&fixture, dir, "init", &[]);
        assert_eq!(frame_count(&tree.join(A_EXPORT)), 0);

        expect_ok(&fixture, dir, "drip", &["--fps", "1000"]);
        assert_eq!(
            frame_count(&tree.join(A_EXPORT)),
            60,
            "drip completes A's 60 frames"
        );
        expect_ok(&fixture, dir, "init", &["--check"]);

        let printed = expect_ok(&fixture, dir, "path", &[]);
        assert_eq!(printed.trim_end(), dir.to_str().expect("a UTF-8 path"));

        let stale = tree.join("export").join("left-over-from-the-old-tree.txt");
        fs::write(&stale, b"old").expect("a file in the old tree");
        expect_ok(&fixture, dir, "init", &["--reset"]);
        assert!(!stale.exists(), "--reset removes the old tree's files");
        expect_ok(&fixture, dir, "init", &["--check"]);
        assert_eq!(
            frame_count(&tree.join(A_EXPORT)),
            0,
            "--reset starts A over"
        );
    }

    /// Polls the daemon until B has its three renders, C has failed, G has its three renders and
    /// G's export folder is gone. Checks every snapshot on the way for D and E, and fails at the
    /// hang guard.
    async fn run_until_settled(daemon: &Daemon, tree: &Path) {
        let mut client = daemon.client().await;

        let started = Instant::now();
        loop {
            let snapshot = client
                .get_snapshot(pb::GetSnapshotRequest {})
                .await
                .expect("a snapshot")
                .into_inner();
            assert_hidden(&snapshot);

            let mut b_variants: Vec<String> = renders_of(&snapshot, "sandbox-b")
                .into_iter()
                .map(|video| video.variant_key.clone())
                .collect();
            b_variants.sort_unstable();
            b_variants.dedup();
            let c_failed = snapshot
                .failures
                .iter()
                .any(|failure| failure.sequence_name.contains("sandbox-c"));
            let g_renders = renders_of(&snapshot, "sandbox-g").len();
            let g_gone = !tree.join(G_EXPORT).exists();

            if b_variants.len() == 3 && c_failed && g_renders == 3 && g_gone {
                // The final snapshot: B's renders exist on disk, C made no render.
                for video in renders_of(&snapshot, "sandbox-b") {
                    assert!(
                        Path::new(&video.output_path).is_file(),
                        "B's render {} is on disk",
                        video.output_path
                    );
                }
                assert!(
                    renders_of(&snapshot, "sandbox-c").is_empty(),
                    "C renders nothing"
                );
                break;
            }
            assert!(
                started.elapsed() < HANG_GUARD,
                "the daemon did not reach the expected state within {HANG_GUARD:?}: {snapshot:?}"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// A3: a daemon started on the tree renders B's three variants, fails C, never shows D or E,
    /// and deletes G's export folder after G's renders.
    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_renders_fails_and_hides_sequences() {
        let fixture = Fixture::new();
        let tree = fixture.tree();
        build_tree(&fixture);
        let daemon = Daemon::start(&fixture, &tree);
        run_until_settled(&daemon, &tree).await;
    }

    /// After a daemon run has deleted G's export folder, `init` without `--reset` still accepts the
    /// tree (spec §12: init is idempotent), and `--check` passes.
    #[tokio::test(flavor = "multi_thread")]
    async fn init_accepts_tree_after_daemon_run() {
        let fixture = Fixture::new();
        let tree = fixture.tree();
        let dir = tree.as_path();
        build_tree(&fixture);
        {
            let daemon = Daemon::start(&fixture, &tree);
            run_until_settled(&daemon, &tree).await;
        }
        assert!(
            !tree.join(G_EXPORT).exists(),
            "the daemon deleted G's export"
        );

        let printed = expect_ok(&fixture, dir, "init", &[]);
        assert!(
            printed.contains("sandbox already present"),
            "init reuses the tree: {printed:?}"
        );
        expect_ok(&fixture, dir, "init", &["--check"]);
    }
}
