// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! A scripted stand-in for ffprobe (spec 1 §6.15, TRD-05). `TRD_FAKE_FFPROBE` holds
//! `{"width": N, "height": N}`, inline or as a file path; the defaults are 1920 x 1080. The fake
//! prints `width=` and `height=` lines. `-version` prints a version line and exits 0.

use std::env;
use std::process::ExitCode;

use serde_json::Value;

const SCRIPT_VAR: &str = "TRD_FAKE_FFPROBE";

fn main() -> ExitCode {
    let args: Vec<String> = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if args.iter().any(|arg| arg == "-version") {
        println!("ffprobe version fake-1.0");
        return ExitCode::SUCCESS;
    }
    match trd_testtools::json_object_from_env(SCRIPT_VAR) {
        Ok(script) => {
            let dimension = |key: &str, default: u64| {
                script.get(key).and_then(Value::as_u64).unwrap_or(default)
            };
            println!("width={}", dimension("width", 1920));
            println!("height={}", dimension("height", 1080));
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("fake-ffprobe: {message}");
            ExitCode::from(2)
        }
    }
}
