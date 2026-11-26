// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The live autostart test (spec 1 §10.4, overview §15.2). It runs only on the CI runners of
//! `v3-system-tests.yml`: it installs the service for the runner's user with this test's `trd`
//! binary and a temporary config, waits for the daemon to answer on the loopback port, restarts
//! it, uninstalls it, and checks that nothing answers any more. On a developer's Mac it is
//! ignored, and `system_test!` refuses it unless CI is set.

use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use trd_proto::trd::v1 as pb;
use trd_proto::trd::v1::trd_service_client::TrdServiceClient;
use trd_testtools::system_test;

/// The loopback address of the daemon the service starts. The service has no `--port` flag, so
/// the default port applies.
const LOOPBACK: &str = "http://127.0.0.1:50071";
/// How long a wait for the service may take.
const WAIT: Duration = Duration::from_secs(60);
/// How long the daemon may take to stop answering after uninstall.
const STOP_WAIT: Duration = Duration::from_secs(30);
/// The pause between two looks.
const POLL: Duration = Duration::from_millis(500);

/// Runs the trd binary under test with the given arguments.
fn trd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trd"))
        .args(args)
        .output()
        .expect("the trd test binary starts")
}

/// What `trd service status` printed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Status {
    /// `running`, `stopped` or `not installed`; empty when the line is missing.
    state: String,
    pid: Option<String>,
    last_run: Option<String>,
}

fn status() -> Status {
    let output = trd(&["service", "status"]);
    assert!(
        output.status.success(),
        "trd service status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(|value| value.trim().to_string())
    };
    Status {
        state: field("status:").unwrap_or_default(),
        pid: field("pid:"),
        last_run: field("last run:"),
    }
}

/// Whether the daemon answers `GetServerInfo` on the loopback port.
async fn answers() -> bool {
    match TrdServiceClient::connect(LOOPBACK).await {
        Ok(mut client) => client
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .is_ok(),
        Err(_) => false,
    }
}

/// Polls `trd service status` until `ready` holds, and returns that status.
async fn wait_for_status(what: &str, ready: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + WAIT;
    loop {
        let current = status();
        if ready(&current) {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; last status: {current:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Polls the loopback port until it answers (`expected` true) or stops answering (false).
async fn wait_for_answer(expected: bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while answers().await != expected {
        assert!(
            Instant::now() < deadline,
            "the loopback port did not {} within {timeout:?}",
            if expected { "answer" } else { "stop answering" }
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Uninstalls the service when the test ends, so a failed assertion does not leave it installed.
struct UninstallOnDrop;

impl Drop for UninstallOnDrop {
    fn drop(&mut self) {
        let _ = trd(&["service", "uninstall"]);
    }
}

system_test!(
    TRD_SERVICE_TEST,
    async fn service_install_status_restart_uninstall() {
        let dir = TempDir::new().expect("a temporary folder");
        let config = dir.path().join("config.json");
        let config = config.to_str().expect("a UTF-8 path");
        let binary = env!("CARGO_BIN_EXE_trd");
        let _cleanup = UninstallOnDrop;

        let installed = trd(&["service", "install", "--binary", binary, "--config", config]);
        assert!(
            installed.status.success(),
            "install failed: {}",
            String::from_utf8_lossy(&installed.stderr)
        );
        wait_for_status("the service running", |s| s.state == "running").await;
        wait_for_answer(true, WAIT).await;

        let before = status();
        let restarted = trd(&["service", "restart"]);
        assert!(
            restarted.status.success(),
            "restart failed: {}",
            String::from_utf8_lossy(&restarted.stderr)
        );
        // macOS and Linux show a new pid; Windows shows a new last run time.
        wait_for_status("a restarted service", |s| {
            s.state == "running" && (s.pid != before.pid || s.last_run != before.last_run)
        })
        .await;
        wait_for_answer(true, WAIT).await;

        let removed = trd(&["service", "uninstall"]);
        assert!(
            removed.status.success(),
            "uninstall failed: {}",
            String::from_utf8_lossy(&removed.stderr)
        );
        assert_eq!(
            status().state,
            "not installed",
            "the service is still installed"
        );
        wait_for_answer(false, STOP_WAIT).await;
    }
);
