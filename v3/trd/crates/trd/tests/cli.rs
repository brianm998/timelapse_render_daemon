// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Command-line behaviour of the trd binary. These tests start no service and touch no real
//! data: `version` and the not-yet-implemented commands do nothing else.

use std::process::{Command, Output};

fn trd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_trd"))
        .args(args)
        .output()
        .expect("the trd test binary starts")
}

#[test]
fn version_prints_name_version_and_platform() {
    let out = trd(&["version"]);
    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    let line = stdout.trim_end();
    let prefix = "trd 3.0.0 (";
    assert!(line.starts_with(prefix), "unexpected version line: {line}");
    assert!(line.ends_with(')'), "unexpected version line: {line}");
    let platform = &line[prefix.len()..line.len() - 1];
    assert!(!platform.is_empty(), "the platform is empty: {line}");
}

#[test]
fn service_without_an_action_is_a_usage_error() {
    // `service` is implemented (TRD-21), so it cannot serve as the not-implemented example. A
    // missing action is a clap usage error: exit 2, and no service-manager command runs.
    let out = trd(&["service"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).expect("utf-8 output");
    // clap names the binary as run: `trd` on Unix, `trd.exe` on Windows.
    assert!(
        stderr.contains("Usage: trd") && stderr.contains(" service [OPTIONS] <COMMAND>"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn no_arguments_prints_help_and_succeeds() {
    let out = trd(&[]);
    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    assert!(stdout.contains("Usage: trd"), "unexpected help: {stdout}");
}
