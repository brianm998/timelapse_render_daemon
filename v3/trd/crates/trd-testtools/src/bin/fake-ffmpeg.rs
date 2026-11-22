// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! A scripted stand-in for ffmpeg (spec 1 §6.6, §6.15, TRD-05). Its behaviour comes from the JSON
//! in `TRD_FAKE_FFMPEG` (inline when it starts with `{`, else the path of a file):
//!
//! - `frames` (default 1): one progress block per frame, written to the `-progress <file>` file;
//! - `exit_code` (default 0) and `stderr_lines` (default none): the exit status and stderr text;
//! - `write_output` (default true): whether the last argument is written;
//! - `wait_for_file`: when set, the last progress block waits until this file exists (120 s at most),
//!   so a test can suspend the fake at a known point;
//! - `progress_end` (default true): whether the last block says `progress=end`;
//! - `rules`: a list of objects with a `match` substring. The first rule whose `match` occurs in
//!   the joined arguments overrides, for that invocation, the fields it names.
//!
//! The last argument is an image sequence when it holds a `%0Nd` pattern: `-frames:v` files
//! (default 1), numbered from `-start_number` (default 0). It is a JPEG when it ends in `.jpg` or
//! `.jpeg`, and otherwise a small file. `-version` prints a version line and exits 0.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Map, Value};

const SCRIPT_VAR: &str = "TRD_FAKE_FFMPEG";
const WAIT_LIMIT: Duration = Duration::from_secs(120);
const WAIT_POLL: Duration = Duration::from_millis(20);
/// A header-only JPEG (start of image, JFIF segment, end of image). It is recognised by its magic
/// bytes; it does not decode.
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9,
];
const RENDER_BYTES: &[u8] = b"fake render\n";

/// The behaviour of one invocation.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct Behaviour {
    frames: u32,
    exit_code: i32,
    stderr_lines: Vec<String>,
    write_output: bool,
    wait_for_file: Option<String>,
    progress_end: bool,
}

impl Default for Behaviour {
    fn default() -> Self {
        Behaviour {
            frames: 1,
            exit_code: 0,
            stderr_lines: Vec::new(),
            write_output: true,
            wait_for_file: None,
            progress_end: true,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    match run(&args) {
        Ok(code) => ExitCode::from((code & 0xff) as u8),
        Err(message) => {
            eprintln!("fake-ffmpeg: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-version") {
        println!("ffmpeg version fake-1.0");
        return Ok(0);
    }
    let script = trd_testtools::json_object_from_env(SCRIPT_VAR)?;
    let behaviour = behaviour_for(&script, &args.join(" "))?;
    for line in &behaviour.stderr_lines {
        eprintln!("{line}");
    }
    let output = args.last().ok_or("no output argument")?;
    if let Some(progress) = arg_value(args, "-progress") {
        write_progress(Path::new(progress), &behaviour)?;
    }
    if behaviour.write_output {
        write_outputs(args, output)?;
    }
    Ok(behaviour.exit_code)
}

/// The script's fields, overlaid by the first matching rule.
fn behaviour_for(script: &Map<String, Value>, joined: &str) -> Result<Behaviour, String> {
    let mut merged = script.clone();
    merged.remove("rules");
    if let Some(rules) = script.get("rules").and_then(Value::as_array) {
        for rule in rules {
            let fields = rule.as_object().ok_or("each rule must be an object")?;
            let pattern = fields
                .get("match")
                .and_then(Value::as_str)
                .ok_or("each rule needs a match string")?;
            if joined.contains(pattern) {
                for (key, value) in fields {
                    if key != "match" {
                        merged.insert(key.clone(), value.clone());
                    }
                }
                break;
            }
        }
    }
    serde_json::from_value(Value::Object(merged)).map_err(|err| format!("bad {SCRIPT_VAR}: {err}"))
}

/// Writes one progress block per frame, flushing each, and waits before the last one if asked.
fn write_progress(path: &Path, behaviour: &Behaviour) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    for frame in 1..=behaviour.frames {
        let last = frame == behaviour.frames;
        if let Some(wait) = behaviour.wait_for_file.as_deref().filter(|_| last) {
            wait_for(Path::new(wait))?;
        }
        let state = if last && behaviour.progress_end {
            "end"
        } else {
            "continue"
        };
        // 25 frames a second, so each frame advances the output time by 40 ms.
        let out_time_us = u64::from(frame - 1) * 40_000;
        let block = format!(
            "frame={frame}\nfps=25.00\ntotal_size=0\nout_time_us={out_time_us}\nspeed=1.00x\nprogress={state}\n"
        );
        file.write_all(block.as_bytes())
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn wait_for(path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + WAIT_LIMIT;
    while !path.exists() {
        if Instant::now() >= deadline {
            return Err(format!(
                "{} did not appear within {} s",
                path.display(),
                WAIT_LIMIT.as_secs()
            ));
        }
        thread::sleep(WAIT_POLL);
    }
    Ok(())
}

/// Writes the output named by the last argument: a sequence, a JPEG or a small file.
fn write_outputs(args: &[String], output: &str) -> Result<(), String> {
    if let Some((start, end, width)) = sequence_spec(output) {
        let count = number_arg(args, "-frames:v")?.unwrap_or(1);
        let first = number_arg(args, "-start_number")?.unwrap_or(0);
        for index in 0..count {
            let number = first + index;
            let name = format!(
                "{}{:0width$}{}",
                &output[..start],
                number,
                &output[end..],
                width = width
            );
            write_file(&name, RENDER_BYTES)?;
        }
        Ok(())
    } else if is_jpeg(output) {
        write_file(output, JPEG)
    } else {
        write_file(output, RENDER_BYTES)
    }
}

fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|err| format!("cannot write {path}: {err}"))
}

/// The byte range of the first `%0Nd` pattern and its width N.
fn sequence_spec(pattern: &str) -> Option<(usize, usize, usize)> {
    let bytes = pattern.as_bytes();
    for start in 0..bytes.len() {
        if bytes[start] != b'%' || bytes.get(start + 1) != Some(&b'0') {
            continue;
        }
        let digits_start = start + 2;
        let mut end = digits_start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > digits_start && bytes.get(end) == Some(&b'd') {
            let width = pattern[digits_start..end].parse().ok()?;
            return Some((start, end + 1, width));
        }
    }
    None
}

fn is_jpeg(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg")
}

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| arg == flag)?;
    args.get(position + 1).map(String::as_str)
}

fn number_arg(args: &[String], flag: &str) -> Result<Option<u32>, String> {
    arg_value(args, flag)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("{flag} needs a number, not {value}"))
        })
        .transpose()
}
