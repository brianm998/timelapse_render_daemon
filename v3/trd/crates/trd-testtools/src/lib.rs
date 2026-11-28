// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Test helpers (spec 1 TRD-05): the fake ffmpeg, ffprobe and exiftool binaries, a helper that
//! builds a directory of them for `TRD_FFMPEG_DIR` and `TRD_EXIFTOOL_DIR`, and the guard for
//! system tests (overview §10 rule 1 and §15.2).

pub mod system_test;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub use system_test::{require_system_test, system_test_allowed};

/// The fake binaries and the names each has in a tools directory.
pub const FAKE_TOOLS: [(&str, &str); 3] = [
    ("fake-ffmpeg", "ffmpeg"),
    ("fake-ffprobe", "ffprobe"),
    ("fake-exiftool", "exiftool"),
];

/// Reads a JSON object from the environment variable `var`. The value is the object itself when it
/// starts with `{`, else the path of a file holding it. An unset or blank variable is an empty
/// object.
pub fn json_object_from_env(var: &str) -> Result<Map<String, Value>, String> {
    let text = match std::env::var(var) {
        Ok(text) if !text.trim().is_empty() => text,
        _ => return Ok(Map::new()),
    };
    let text = if text.trim_start().starts_with('{') {
        text
    } else {
        let path = text.clone();
        fs::read_to_string(&path).map_err(|err| format!("cannot read {var} file {path}: {err}"))?
    };
    match serde_json::from_str::<Value>(&text).map_err(|err| format!("bad {var}: {err}"))? {
        Value::Object(map) => Ok(map),
        _ => Err(format!("{var} must be a JSON object")),
    }
}

/// The executable suffix of the platform.
fn exe_suffix() -> &'static str {
    if cfg!(windows) { ".exe" } else { "" }
}

/// The directory holding the built fake binaries: `TRD_FAKE_BIN_DIR` when set, else the Cargo
/// target directory the running test was built in.
pub fn fake_bin_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TRD_FAKE_BIN_DIR") {
        return PathBuf::from(dir);
    }
    let exe = std::env::current_exe().expect("the test executable's path");
    // The test binary sits in target/<profile>/deps, the fakes in target/<profile>.
    for dir in exe.ancestors().skip(1).take(3) {
        if dir.join(format!("fake-ffmpeg{}", exe_suffix())).is_file() {
            return dir.to_path_buf();
        }
    }
    panic!(
        "the fake tools are not built: run cargo build -p trd-testtools, or set TRD_FAKE_BIN_DIR"
    );
}

/// Copies the fake binaries from `bin_dir` into `dest` under the names ffmpeg, ffprobe and
/// exiftool. On macOS each copy is ad-hoc signed, since a copied binary's signature names the old
/// file. The copies can be run as soon as this returns, even while other threads start processes
/// (see `copy_program`).
pub fn make_tools_dir(bin_dir: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for (bin, name) in FAKE_TOOLS {
        let to = dest.join(format!("{name}{}", exe_suffix()));
        copy_program(&bin_dir.join(format!("{bin}{}", exe_suffix())), &to)?;
        sign_ad_hoc(&to)?;
    }
    Ok(())
}

/// Copies a program with `/bin/cp`, so that this process never holds the copy open for writing.
/// A child forked by another thread while such a descriptor is open inherits it until its own
/// exec, and running the copy in that window fails with `ETXTBSY` ("Text file busy").
#[cfg(unix)]
fn copy_program(from: &Path, to: &Path) -> io::Result<()> {
    let status = std::process::Command::new("/bin/cp")
        .arg(from)
        .arg(to)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "cp failed for {}: {status}",
            from.display()
        )))
    }
}

#[cfg(not(unix))]
fn copy_program(from: &Path, to: &Path) -> io::Result<()> {
    fs::copy(from, to).map(|_| ())
}

#[cfg(target_os = "macos")]
fn sign_ad_hoc(path: &Path) -> io::Result<()> {
    use std::process::Command;

    let status = Command::new("/usr/bin/codesign")
        .args(["-s", "-", "-f"])
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "codesign failed for {}: {status}",
            path.display()
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn sign_ad_hoc(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// A temporary directory holding the fake tools, removed when dropped. It lives in the system
/// temporary directory, never in the worktree.
pub struct FakeTools {
    dir: tempfile::TempDir,
}

impl FakeTools {
    /// Builds a tools directory from the fake binaries (see `fake_bin_dir`).
    pub fn new() -> io::Result<FakeTools> {
        let dir = tempfile::Builder::new()
            .prefix("trd-fake-tools-")
            .tempdir()?;
        make_tools_dir(&fake_bin_dir(), dir.path())?;
        Ok(FakeTools { dir })
    }

    /// The directory: pass it as `TRD_FFMPEG_DIR` and `TRD_EXIFTOOL_DIR`.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}
