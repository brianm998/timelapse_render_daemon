// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Tests of the process runner (§6.6, §6.8, TRD-08) with the fake ffmpeg.
//!
//! Every test is inside `mod process`, so `cargo test -p trd-core process` selects them all. Each
//! test copies the fake ffmpeg into its own temporary directory and runs it detached there. The
//! only deadline is a hang guard: no test asserts how long anything takes.

mod process {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use trd_core::process::{
        self, ExitInfo, ProcessError, ProcessState, RunningProcess, TERMINATE_GRACE,
    };
    use trd_core::progress::ProgressReader;
    use trd_testtools::FakeTools;

    /// Stops a test that waits for something that never happens. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);
    const POLL: Duration = Duration::from_millis(20);

    /// Polls `done` until it holds, failing at the hang guard.
    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + HANG_GUARD;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(POLL);
        }
    }

    /// Waits until the child has ended, and returns how it ended.
    fn wait_for_exit(process: &mut RunningProcess) -> ExitInfo {
        let mut found = None;
        wait_until("the process to exit", || {
            found = process.try_wait().expect("try_wait");
            found.is_some()
        });
        found.expect("an exit status")
    }

    /// A temporary directory with its own copy of the fake ffmpeg.
    struct Sandbox {
        _tools: FakeTools,
        dir: TempDir,
        ffmpeg: PathBuf,
    }

    impl Sandbox {
        fn new() -> Sandbox {
            let tools = FakeTools::new().expect("the fake tools");
            let name = if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            };
            let ffmpeg = tools.path().join(name);
            Sandbox {
                _tools: tools,
                dir: tempfile::tempdir().expect("a temporary directory"),
                ffmpeg,
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn log(&self) -> PathBuf {
            self.path("render.log")
        }
    }

    /// The ffmpeg command line the daemon builds: `-progress <file>`, an input pattern, then the
    /// output, which is always the last argument.
    fn argv(ffmpeg: &Path, progress: &Path, output: &Path) -> Vec<OsString> {
        vec![
            ffmpeg.as_os_str().to_owned(),
            OsString::from("-progress"),
            progress.as_os_str().to_owned(),
            OsString::from("-i"),
            OsString::from("in%03d.png"),
            output.as_os_str().to_owned(),
        ]
    }

    /// Starts the fake ffmpeg with `script` as its TRD_FAKE_FFMPEG behaviour.
    fn spawn(sandbox: &Sandbox, progress: &Path, output: &Path, script: Value) -> RunningProcess {
        let text = script.to_string();
        process::spawn_detached_with_env(
            &argv(&sandbox.ffmpeg, progress, output),
            &[("TRD_FAKE_FFMPEG", text.as_str())],
            &sandbox.log(),
        )
        .expect("the fake ffmpeg starts")
    }

    fn text(path: &Path) -> &str {
        path.to_str().expect("a UTF-8 test path")
    }

    #[test]
    fn progress_blocks_then_exit_code() {
        let sandbox = Sandbox::new();
        let progress = sandbox.path("render.progress");
        let output = sandbox.path("out.mov");
        let release = sandbox.path("release");
        // Two blocks are written at once; the last waits for the release file.
        let mut child = spawn(
            &sandbox,
            &progress,
            &output,
            json!({"frames": 3, "wait_for_file": text(&release)}),
        );

        let mut reader = ProgressReader::new(&progress);
        wait_until("the second progress block", || {
            reader
                .poll()
                .expect("poll the progress file")
                .is_some_and(|block| block.frame == 2)
        });
        assert!(!reader.latest().expect("a block").ended);

        fs::write(&release, b"").expect("release the fake");
        assert_eq!(
            wait_for_exit(&mut child),
            ExitInfo {
                code: Some(0),
                by_signal: None
            }
        );
        let last = reader.poll().expect("poll").expect("a block");
        assert_eq!(last.frame, 3);
        assert!(last.ended, "the last block says progress=end");
        assert!(output.is_file(), "the output was written");
    }

    #[test]
    fn suspend_resume() {
        let sandbox = Sandbox::new();
        let output = sandbox.path("out.mov");
        let release = sandbox.path("release");
        let mut child = spawn(
            &sandbox,
            &sandbox.path("render.progress"),
            &output,
            json!({"frames": 1, "wait_for_file": text(&release)}),
        );
        let pid = child.pid;

        process::suspend(pid).expect("suspend the fake");
        wait_until("the fake to be stopped", || {
            process::process_state(pid) == ProcessState::Stopped
        });
        assert_eq!(process::process_state(pid), ProcessState::Stopped);

        fs::write(&release, b"").expect("release the fake");
        process::resume(pid).expect("resume the fake");
        assert_eq!(
            wait_for_exit(&mut child),
            ExitInfo {
                code: Some(0),
                by_signal: None
            }
        );
        assert!(output.is_file(), "the resumed fake wrote its output");
    }

    #[test]
    fn terminate_suspended() {
        let sandbox = Sandbox::new();
        let output = sandbox.path("out.mov");
        // The release file never appears: only terminate ends this fake.
        let mut child = spawn(
            &sandbox,
            &sandbox.path("render.progress"),
            &output,
            json!({"frames": 1, "wait_for_file": text(&sandbox.path("never"))}),
        );
        let pid = child.pid;

        process::suspend(pid).expect("suspend the fake");
        wait_until("the fake to be stopped", || {
            process::process_state(pid) == ProcessState::Stopped
        });

        let exit = process::terminate(&mut child, TERMINATE_GRACE)
            .expect("terminate the suspended fake")
            .expect("a child reports how it ended");
        #[cfg(unix)]
        assert_eq!(exit.by_signal, Some(libc::SIGTERM));
        #[cfg(not(unix))]
        assert!(
            exit.code.is_some(),
            "TerminateProcess ends the fake with an exit code"
        );
        assert_eq!(process::process_state(pid), ProcessState::Gone);
        assert!(
            !output.exists(),
            "the fake was ended before it wrote output"
        );
    }

    #[cfg(unix)]
    const HELPER_DIR: &str = "TRD_PROCESS_TEST_HELPER_DIR";
    #[cfg(unix)]
    const HELPER_FFMPEG: &str = "TRD_PROCESS_TEST_HELPER_FFMPEG";
    #[cfg(unix)]
    const HELPER_MARK: &str = "PROCESS_TEST_CHILD ";

    /// The helper run by `own_process_group`: it is the "parent" whose group the test signals. It
    /// starts a detached fake and reports the fake's pid and start time, then waits to be signalled.
    #[cfg(unix)]
    fn run_helper(dir: &Path, ffmpeg: &Path) {
        let output = dir.join("out.mov");
        let release = dir.join("release");
        let child = process::spawn_detached_with_env(
            &argv(ffmpeg, &dir.join("render.progress"), &output),
            &[(
                "TRD_FAKE_FFMPEG",
                json!({"frames": 1, "wait_for_file": text(&release)})
                    .to_string()
                    .as_str(),
            )],
            &dir.join("render.log"),
        )
        .expect("the helper's fake starts");
        println!("{HELPER_MARK}{} {}", child.pid, child.start_time);
        thread::sleep(HANG_GUARD);
    }

    /// A render runs in a session and process group of its own, so a daemon that dies while the
    /// render is stopped does not get it SIGHUPped as an orphaned group.
    #[cfg(unix)]
    #[test]
    fn render_has_own_session() {
        let dir = TempDir::new().expect("a temp dir");
        let argv = vec![OsString::from("/bin/sleep"), OsString::from("60")];
        let mut child = process::spawn_detached(&argv, &dir.path().join("log")).expect("spawn");
        let pid = child.pid as libc::pid_t;
        // SAFETY: getsid and getpgid only read process attributes.
        let (child_sid, child_pgid, own_sid) =
            unsafe { (libc::getsid(pid), libc::getpgid(pid), libc::getsid(0)) };
        process::terminate(&mut child, TERMINATE_GRACE).expect("end the child");
        assert_eq!(child_sid, pid, "the child leads a session of its own");
        assert_eq!(child_pgid, pid, "and a process group of its own");
        assert_ne!(child_sid, own_sid);
    }

    #[cfg(unix)]
    #[test]
    fn own_process_group() {
        if let (Some(dir), Some(ffmpeg)) = (
            std::env::var_os(HELPER_DIR),
            std::env::var_os(HELPER_FFMPEG),
        ) {
            run_helper(Path::new(&dir), Path::new(&ffmpeg));
            return;
        }

        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::Pid as UnixPid;
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let sandbox = Sandbox::new();
        let exe = std::env::current_exe().expect("this test binary");
        let mut helper = Command::new(exe)
            .args([
                "--exact",
                "process::own_process_group",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HELPER_DIR, sandbox.dir.path())
            .env(HELPER_FFMPEG, &sandbox.ffmpeg)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("start the helper");

        let stdout = helper.stdout.take().expect("the helper's stdout");
        let mut report = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("a line from the helper");
            // libtest prints "test process::own_process_group ... " without a newline under
            // --nocapture, so the marker can land mid-line.
            if let Some(index) = line.find(HELPER_MARK) {
                report = Some(line[index + HELPER_MARK.len()..].to_owned());
                break;
            }
        }
        let report = report.expect("the helper reports its fake's pid");
        let mut fields = report.split_whitespace();
        let child_pid: u32 = fields.next().expect("pid").parse().expect("a pid");
        let child_start: u64 = fields.next().expect("start").parse().expect("a start time");

        // SIGTERM to the helper's process group, which is the parent's group in this test.
        killpg(UnixPid::from_raw(helper.id() as i32), Signal::SIGTERM).expect("signal the group");
        let status = helper.wait().expect("the helper ends");
        assert!(!status.success(), "the helper was ended by the signal");

        assert!(
            process::is_alive(child_pid, child_start),
            "the detached fake survives its parent's process group being signalled"
        );

        let mut orphan = RunningProcess::adopted(child_pid, child_start);
        process::terminate(&mut orphan, TERMINATE_GRACE).expect("end the orphan");
        assert!(!process::is_alive(child_pid, child_start));
    }

    #[test]
    fn fast_exit() {
        let sandbox = Sandbox::new();
        let output = sandbox.path("out.mov");
        let mut child = spawn(
            &sandbox,
            &sandbox.path("render.progress"),
            &output,
            json!({}),
        );
        assert!(child.is_child());
        assert_eq!(
            wait_for_exit(&mut child),
            ExitInfo {
                code: Some(0),
                by_signal: None
            }
        );
        assert!(output.is_file());
    }

    #[test]
    fn non_zero_exit() {
        let sandbox = Sandbox::new();
        let mut child = spawn(
            &sandbox,
            &sandbox.path("render.progress"),
            &sandbox.path("out.mov"),
            json!({"exit_code": 3, "stderr_lines": ["fake failure"]}),
        );
        assert_eq!(
            wait_for_exit(&mut child),
            ExitInfo {
                code: Some(3),
                by_signal: None
            }
        );
        let log = fs::read_to_string(sandbox.log()).expect("the log");
        assert!(
            log.contains("fake failure"),
            "stderr goes to the log: {log}"
        );
    }

    #[test]
    fn missing_binary_is_launch_failed() {
        let sandbox = Sandbox::new();
        let argv = vec![
            sandbox.path("no-such-ffmpeg").into_os_string(),
            OsString::from("-version"),
        ];
        let err = process::spawn_detached(&argv, &sandbox.log())
            .expect_err("a missing binary does not start");
        assert!(
            matches!(err, ProcessError::LaunchFailed { .. }),
            "got {err:?}"
        );
    }
}
