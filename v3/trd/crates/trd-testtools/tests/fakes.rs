// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The fake tools behave as their scripts say (spec 1 TRD-05, A3 and A4). Every fixture lives in a
//! temporary directory; no test touches the owner's data or a port.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use trd_core::tools::{ToolProblem, Tools};

const WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(10);

fn path_str(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

/// Runs a fake with `script` in `var` and `args`, and collects its output.
fn run(bin: &str, var: &str, script: &serde_json::Value, args: &[&str]) -> Output {
    Command::new(bin)
        .env(var, script.to_string())
        .args(args)
        .output()
        .expect("the fake starts")
}

fn ffmpeg(script: &serde_json::Value, args: &[&str]) -> Output {
    run(
        env!("CARGO_BIN_EXE_fake-ffmpeg"),
        "TRD_FAKE_FFMPEG",
        script,
        args,
    )
}

fn progress_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Polls `done` until it holds, failing after `WAIT`.
fn poll_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "the condition did not hold within {} s",
            WAIT.as_secs()
        );
        thread::sleep(POLL);
    }
}

#[test]
fn ffmpeg_answers_version() {
    let out = ffmpeg(&json!({}), &["-version"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("ffmpeg version"));
}

#[test]
fn ffmpeg_writes_one_progress_block_per_frame_and_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let progress = dir.path().join("progress.txt");
    let out_file = dir.path().join("render.mov");
    let out = ffmpeg(
        &json!({"frames": 3}),
        &[
            "-nostdin",
            "-i",
            "in.png",
            "-progress",
            path_str(&progress),
            path_str(&out_file),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let text = progress_text(&progress);
    assert_eq!(text.matches("frame=").count(), 3);
    assert_eq!(text.matches("progress=continue").count(), 2);
    assert_eq!(text.matches("progress=end").count(), 1);
    assert!(text.trim_end().ends_with("progress=end"));
    assert!(fs::metadata(&out_file).unwrap().len() > 0);
}

#[test]
fn ffmpeg_progress_can_stop_short_of_end() {
    let dir = tempfile::tempdir().unwrap();
    let progress = dir.path().join("progress.txt");
    let out = ffmpeg(
        &json!({"frames": 2, "progress_end": false, "write_output": false}),
        &[
            "-progress",
            path_str(&progress),
            path_str(&dir.path().join("x.mov")),
        ],
    );
    assert!(out.status.success());
    let text = progress_text(&progress);
    assert_eq!(text.matches("progress=continue").count(), 2);
    assert!(!text.contains("progress=end"));
}

#[test]
fn ffmpeg_scripted_exit_code_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let out_file = dir.path().join("render.mov");
    let script = json!({
        "exit_code": 3,
        "stderr_lines": ["Invalid data found when processing input"],
        "write_output": false,
    });
    let out = ffmpeg(&script, &["-i", "in.mov", path_str(&out_file)]);
    assert_eq!(out.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Invalid data found when processing input")
    );
    assert!(!out_file.exists());
}

#[test]
fn ffmpeg_rules_fail_only_the_matching_invocation() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("sandbox-a-render.mov");
    let bad = dir.path().join("sandbox-c-render.mov");
    let script = json!({
        "rules": [{
            "match": "sandbox-c",
            "exit_code": 1,
            "stderr_lines": ["Invalid data found when processing input"],
        }],
    });
    let ok = ffmpeg(&script, &["-i", "in.mov", path_str(&good)]);
    assert!(ok.status.success());
    assert!(good.is_file());

    let failed = ffmpeg(&script, &["-i", "in.mov", path_str(&bad)]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .contains("Invalid data found when processing input")
    );
}

#[test]
fn ffmpeg_writes_a_numbered_image_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let pattern = dir.path().join("frame_%04d.png");
    let out = ffmpeg(
        &json!({}),
        &["-i", "in.mov", "-frames:v", "3", path_str(&pattern)],
    );
    assert!(out.status.success());
    for number in 0..3 {
        assert!(dir.path().join(format!("frame_{number:04}.png")).is_file());
    }
    assert!(!dir.path().join("frame_0003.png").exists());

    // Without -frames:v one file, and -start_number moves the first number.
    let one = dir.path().join("one.%03d.png");
    assert!(
        ffmpeg(&json!({}), &["-i", "in.mov", path_str(&one)])
            .status
            .success()
    );
    assert!(dir.path().join("one.000.png").is_file());
    assert!(!dir.path().join("one.001.png").exists());

    let shifted = dir.path().join("b_%02d.png");
    let out = ffmpeg(
        &json!({}),
        &[
            "-start_number",
            "5",
            "-frames:v",
            "2",
            "-i",
            "in.mov",
            path_str(&shifted),
        ],
    );
    assert!(out.status.success());
    assert!(dir.path().join("b_05.png").is_file());
    assert!(dir.path().join("b_06.png").is_file());
}

#[test]
fn ffmpeg_writes_a_jpeg_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let thumb = dir.path().join("thumb.jpg");
    let out = ffmpeg(
        &json!({}),
        &[
            "-nostdin",
            "-hide_banner",
            "-ss",
            "1",
            "-i",
            "render.mov",
            "-frames:v",
            "1",
            "-vf",
            "scale=480:-2",
            "-q:v",
            "3",
            path_str(&thumb),
        ],
    );
    assert!(out.status.success());
    let bytes = fs::read(&thumb).unwrap();
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF]);
    assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9]);
}

#[test]
fn ffmpeg_waits_for_the_file_before_the_last_frame() {
    let dir = tempfile::tempdir().unwrap();
    let progress = dir.path().join("progress.txt");
    let go = dir.path().join("go");
    let out_file = dir.path().join("render.mov");
    let script = json!({"frames": 3, "wait_for_file": path_str(&go)});
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-ffmpeg"))
        .env("TRD_FAKE_FFMPEG", script.to_string())
        .args(["-progress", path_str(&progress), path_str(&out_file)])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the fake starts");

    // The first two blocks appear, then the fake waits for the file before the third.
    poll_until(|| progress_text(&progress).matches("frame=").count() == 2);
    assert!(
        child.try_wait().unwrap().is_none(),
        "the fake finished before the file appeared"
    );
    assert!(!out_file.exists());

    fs::write(&go, b"go").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(progress_text(&progress).matches("frame=").count(), 3);
    assert!(out_file.is_file());
}

#[test]
fn ffmpeg_reads_the_script_from_a_file_path() {
    let dir = tempfile::tempdir().unwrap();
    let script_file = dir.path().join("script.json");
    fs::write(&script_file, r#"{"exit_code": 4, "write_output": false}"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fake-ffmpeg"))
        .env("TRD_FAKE_FFMPEG", path_str(&script_file))
        .args(["-i", "in.mov", path_str(&dir.path().join("render.mov"))])
        .output()
        .expect("the fake starts");
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    assert!(!dir.path().join("render.mov").exists());
}

#[test]
fn ffmpeg_rejects_a_script_that_is_not_an_object() {
    let dir = tempfile::tempdir().unwrap();
    let script_file = dir.path().join("script.json");
    fs::write(&script_file, "[1, 2]").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fake-ffmpeg"))
        .env("TRD_FAKE_FFMPEG", path_str(&script_file))
        .args(["-i", "in.mov", path_str(&dir.path().join("render.mov"))])
        .output()
        .expect("the fake starts");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("must be a JSON object"));
    assert!(!dir.path().join("render.mov").exists());
}

#[test]
fn ffprobe_prints_the_scripted_dimensions() {
    let out = Command::new(env!("CARGO_BIN_EXE_fake-ffprobe"))
        .env(
            "TRD_FAKE_FFPROBE",
            json!({"width": 640, "height": 360}).to_string(),
        )
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=width,height",
            "clip.mov",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "width=640\nheight=360\n"
    );
}

#[test]
fn ffprobe_answers_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_fake-ffprobe"))
        .arg("-version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("ffprobe version"));
}

#[test]
fn exiftool_answers_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_fake-exiftool"))
        .arg("-ver")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "13.00");
}

#[test]
fn exiftool_prints_csv_rows_for_the_listed_files() {
    let dir = tempfile::tempdir().unwrap();
    let argfile = dir.path().join("args.txt");
    fs::write(
        &argfile,
        "-n\n/shots/IMG_0001.jpg\n/shots/IMG_0002.jpg\n/shots/IMG_0003.jpg\n",
    )
    .unwrap();
    let config = json!({"rows": {
        "IMG_0001.jpg": {"ISO": 100, "ExposureTime": "1/60"},
        "IMG_0002.jpg": {"Model": "A7, III"},
    }});
    let out = Command::new(env!("CARGO_BIN_EXE_fake-exiftool"))
        .env("TRD_FAKE_EXIFTOOL", config.to_string())
        .args(["-csv", "-@", path_str(&argfile)])
        .output()
        .unwrap();
    assert!(out.status.success());
    // IMG_0003 has no row and is left out; the comma in the model is quoted.
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "SourceFile,ExposureTime,ISO,Model\n\
         /shots/IMG_0001.jpg,1/60,100,\n\
         /shots/IMG_0002.jpg,,,\"A7, III\"\n"
    );
}

#[test]
fn exiftool_corrupt_write_returns_the_configured_output_and_code() {
    let config = json!({"write": {"exit_code": 1, "output": "Truncated mdat atom"}});
    let out = Command::new(env!("CARGO_BIN_EXE_fake-exiftool"))
        .env("TRD_FAKE_EXIFTOOL", config.to_string())
        .args([
            "-overwrite_original",
            "-tagsFromFile",
            "src.jpg",
            "clip.mov",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Truncated mdat atom"));
}

#[test]
fn exiftool_write_succeeds_by_default() {
    let out = Command::new(env!("CARGO_BIN_EXE_fake-exiftool"))
        .args(["-overwrite_original", "clip.mov"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

/// The bash that runs `fake-tools-dir.sh`. On Windows a bare `bash` finds WSL's launcher in
/// `System32` first, which fails on runners without a WSL distribution, so the test uses Git for
/// Windows' bash: `TRD_TEST_BASH` if set, else `<ProgramFiles>\Git\bin\bash.exe`, else `bash`.
fn bash_program() -> PathBuf {
    if let Some(bash) = std::env::var_os("TRD_TEST_BASH") {
        return PathBuf::from(bash);
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var) {
                let candidate = PathBuf::from(dir).join("Git").join("bin").join("bash.exe");
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("bash")
}

#[test]
fn fake_tools_dir_resolves() {
    let root = tempfile::tempdir().unwrap();
    let dest: PathBuf = root.path().join("tools");
    let bin_dir = Path::new(env!("CARGO_BIN_EXE_fake-ffmpeg"))
        .parent()
        .expect("the fakes' directory");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/fake-tools-dir.sh"
    );
    let status = Command::new(bash_program())
        .arg(script)
        .arg(&dest)
        .env("TRD_FAKE_BIN_DIR", bin_dir)
        .status()
        .expect("bash runs the script");
    assert!(status.success());

    let dest_var: OsString = dest.as_os_str().to_owned();
    let env = |key: &str| -> Option<OsString> {
        match key {
            "TRD_FFMPEG_DIR" | "TRD_EXIFTOOL_DIR" => Some(dest_var.clone()),
            _ => None,
        }
    };
    let tools = Tools::resolve(env, &root.path().join("bin").join("trd"));
    let suffix = std::env::consts::EXE_SUFFIX;
    for (report, name, version) in [
        (&tools.ffmpeg, "ffmpeg", "ffmpeg version"),
        (&tools.ffprobe, "ffprobe", "ffprobe version"),
        (&tools.exiftool, "exiftool", "13.00"),
    ] {
        assert!(report.info.available, "{name}: {:?}", report.info.problem);
        assert_eq!(report.info.path, Some(dest.join(format!("{name}{suffix}"))));
        assert!(
            report.info.version.starts_with(version),
            "{name}: {}",
            report.info.version
        );
        assert!(report.command.is_some());
    }
    assert_eq!(tools.ffmpeg.info.problem, None::<ToolProblem>);
}
