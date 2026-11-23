// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! A scripted stand-in for exiftool (spec 1 §6.4, §6.15, TRD-05). `TRD_FAKE_EXIFTOOL` holds
//! `{"rows": {"<file name>": {"Tag": "value"}}, "write": {"exit_code": N, "output": "…"}}`,
//! inline or as a file path.
//!
//! - `-ver` prints the version, `13.00`.
//! - `-csv -@ <argfile>` prints CSV for each file the argument file lists (one per line; lines
//!   starting with `-` are options) that `rows` has a key for, by file name or full path. The
//!   header is `SourceFile` and then the tags those rows hold, sorted.
//! - Any other invocation is a write: it prints `write.output` and exits with `write.exit_code`
//!   (both default to nothing), so a test can simulate `Truncated mdat atom`.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Map, Value};

const SCRIPT_VAR: &str = "TRD_FAKE_EXIFTOOL";

fn main() -> ExitCode {
    let args: Vec<String> = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let result = if args.iter().any(|arg| arg == "-ver") {
        println!("13.00");
        Ok(0)
    } else if args.iter().any(|arg| arg == "-csv") {
        csv(&args)
    } else {
        write()
    };
    match result {
        Ok(code) => ExitCode::from((code & 0xff) as u8),
        Err(message) => {
            eprintln!("fake-exiftool: {message}");
            ExitCode::from(2)
        }
    }
}

fn csv(args: &[String]) -> Result<i32, String> {
    let position = args
        .iter()
        .position(|arg| arg == "-@")
        .ok_or("-csv needs -@ <argfile>")?;
    let argfile = args.get(position + 1).ok_or("-@ needs a file name")?;
    let config = trd_testtools::json_object_from_env(SCRIPT_VAR)?;
    let rows = config
        .get("rows")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let listed =
        fs::read_to_string(argfile).map_err(|err| format!("cannot read {argfile}: {err}"))?;

    let matched: Vec<(&str, &Map<String, Value>)> = listed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('-'))
        .filter_map(|file| {
            let name = Path::new(file).file_name()?.to_string_lossy().into_owned();
            let tags = rows
                .get(&name)
                .or_else(|| rows.get(file))
                .and_then(Value::as_object)?;
            Some((file, tags))
        })
        .collect();
    if matched.is_empty() {
        return Ok(0);
    }

    let tags: BTreeSet<&str> = matched
        .iter()
        .flat_map(|(_, tags)| tags.keys().map(String::as_str))
        .collect();
    let mut header = vec!["SourceFile".to_string()];
    header.extend(tags.iter().map(|tag| tag.to_string()));
    println!("{}", csv_line(&header));
    for (file, row) in matched {
        let mut fields = vec![file.to_string()];
        fields.extend(
            tags.iter()
                .map(|tag| row.get(*tag).map(value_text).unwrap_or_default()),
        );
        println!("{}", csv_line(&fields));
    }
    Ok(0)
}

/// The output and exit code of a write.
fn write() -> Result<i32, String> {
    let config = trd_testtools::json_object_from_env(SCRIPT_VAR)?;
    let write = config.get("write").and_then(Value::as_object);
    let code = write
        .and_then(|write| write.get("exit_code"))
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok())
        .unwrap_or(0);
    if let Some(output) = write
        .and_then(|write| write.get("output"))
        .and_then(Value::as_str)
        .filter(|output| !output.is_empty())
    {
        println!("{output}");
    }
    Ok(code)
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn csv_line(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| csv_field(field))
        .collect::<Vec<_>>()
        .join(",")
}

/// A CSV field, quoted when it holds a comma, a quote or a line break.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}
