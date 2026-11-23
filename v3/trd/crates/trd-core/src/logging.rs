// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! trd.log: the log file, its levels and the log events (§6.15).
//!
//! The rolling file appender names its files `trd.YYYY-MM-DD.log` (prefix `trd`, suffix `log`,
//! one per day, the newest 14 kept), not a bare `trd.log`: `tracing-appender` always puts the date
//! in the name.

use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::Path;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::dirs::Dirs;

/// The file-name prefix of the rolling log files.
pub const LOG_FILE_PREFIX: &str = "trd";
/// The file-name suffix of the rolling log files.
pub const LOG_FILE_SUFFIX: &str = "log";
/// How many daily log files are kept.
pub const MAX_LOG_FILES: usize = 14;

/// Starts logging: a daily rolling file in `dirs.logs`, and stderr when it is a terminal.
///
/// In daemon mode (`foreground == false`) the process's stderr is first redirected to
/// `trd-stderr.log` in the log directory, so a panic message is kept. The returned guard flushes
/// the file writer when dropped; the caller keeps it until the process exits.
///
/// Must be called once per process: a second call fails because a global subscriber is set.
pub fn init(dirs: &Dirs, level: Level, foreground: bool) -> io::Result<WorkerGuard> {
    fs::create_dir_all(&dirs.logs)?;
    // Decided before any redirect: a terminal on stderr gets its own copy of the log lines.
    let stderr_is_terminal = io::stderr().is_terminal();

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(&dirs.logs)
        .map_err(io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let redirected = !foreground;
    if redirected {
        redirect_stderr(&dirs.stderr_log())?;
    }

    let file_layer = fmt::layer().with_writer(writer).with_ansi(false);
    let stderr_layer =
        (stderr_is_terminal && !redirected).then(|| fmt::layer().with_writer(io::stderr));

    tracing_subscriber::registry()
        .with(LevelFilter::from_level(level))
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(io::Error::other)?;
    Ok(guard)
}

/// Points the process's stderr (file descriptor 2) at `path`, appending to it.
#[cfg(unix)]
pub fn redirect_stderr(path: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let file = OpenOptions::new().create(true).append(true).open(path)?;
    // SAFETY: both descriptors are open: `file` owns the first, fd 2 is the process's stderr.
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // `file` closes its own descriptor here; fd 2 keeps the file open.
    Ok(())
}

/// Points the process's stderr at `path`, appending to it.
#[cfg(windows)]
pub fn redirect_stderr(path: &Path) -> io::Result<()> {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};

    let file = OpenOptions::new().create(true).append(true).open(path)?;
    // The handle now belongs to the standard error handle, so it is not closed with `file`.
    let handle = file.into_raw_handle();
    // SAFETY: `handle` is a valid file handle opened above and is not closed elsewhere.
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Restores file descriptor 2 from the copy taken before the test redirected it.
    struct RestoreStderr(i32);

    impl Drop for RestoreStderr {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a duplicate of the original fd 2 made by `dup`, owned here.
            unsafe {
                libc::dup2(self.0, libc::STDERR_FILENO);
                libc::close(self.0);
            }
        }
    }

    #[test]
    fn stderr_redirect_restores_fd() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trd-stderr.log");

        // SAFETY: duplicating the process's own fd 2 into a new descriptor.
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        assert!(saved >= 0, "dup of stderr failed");
        let restore = RestoreStderr(saved);

        redirect_stderr(&path).unwrap();
        let line = b"written through fd 2\n";
        // SAFETY: the buffer is valid for `line.len()` bytes and fd 2 is open.
        let written = unsafe { libc::write(libc::STDERR_FILENO, line.as_ptr().cast(), line.len()) };
        assert_eq!(written, line.len() as isize);

        drop(restore);
        assert_eq!(fs::read(&path).unwrap(), line);
    }
}
