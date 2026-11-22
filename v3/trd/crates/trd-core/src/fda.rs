// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The macOS Full Disk Access guard (spec 1 §6.12).
//!
//! On macOS, before the first scan and then every 30 s, the daemon tries to open
//! `~/Library/Application Support/com.apple.TCC/TCC.db` read-only. A permission error means Full
//! Disk Access is missing and nothing protected may be touched. Opening `TCC.db` never shows a
//! prompt. `ENOENT` counts as granted, and on other platforms the guard does nothing.
//!
//! The probe is platform-independent so its tests run on the Linux CI runner; only the real
//! check is gated to macOS. The problem text is defined here for TRD-16 to report. The severity
//! (ERROR) and the phase (`PHASE_WAITING_FOR_DISK_ACCESS`) are set by TRD-16, since the problem
//! and phase types are not defined in this module.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Stable problem id for the missing-access problem (§6.12, §6.10).
pub const PROBLEM_ID: &str = "fda";

/// Problem title (§6.12).
pub const TITLE: &str = "Full Disk Access is needed";

/// Problem detail (§6.12).
pub const DETAIL: &str = "trd reads and writes your export, source, backup and video folders, which macOS protects. Until access is granted trd scans nothing, so macOS never asks for permission while you are away.";

/// Problem fix (§6.12).
pub const FIX: &str = "Open System Settings → Privacy & Security → Full Disk Access, add /Applications/Timelapse Render Daemon.app (the + button), and turn it on. trd checks again every 30 seconds.";

/// Result of a Full Disk Access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdaState {
    Granted,
    Missing,
}

/// The guard. `enabled` is false when `--no-fda-check` (or `--sandbox`) is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdaGuard {
    enabled: bool,
}

impl FdaGuard {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Checks Full Disk Access for the user whose home directory is `home`.
    ///
    /// Returns `Granted` without touching the filesystem when the guard is disabled or the
    /// target is not macOS.
    pub fn check(&self, home: &Path) -> FdaState {
        if !self.enabled || !cfg!(target_os = "macos") {
            return FdaState::Granted;
        }
        probe(home, |p| File::open(p))
    }
}

/// The protected file probed for Full Disk Access.
pub fn tcc_db_path(home: &Path) -> PathBuf {
    home.join("Library/Application Support/com.apple.TCC/TCC.db")
}

/// Classifies the result of opening `TCC.db` for `home`.
///
/// `PermissionDenied` (EPERM and EACCES) means missing access. `NotFound` means granted. Any
/// other error also counts as granted, so an unexpected error never blocks scanning. The
/// handle returned by `open` is dropped at once.
pub fn probe<F>(home: &Path, open: F) -> FdaState
where
    F: FnOnce(&Path) -> io::Result<File>,
{
    match open(&tcc_db_path(home)) {
        Ok(_file) => FdaState::Granted,
        Err(e) => match e.kind() {
            io::ErrorKind::PermissionDenied => FdaState::Missing,
            _ => FdaState::Granted,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home() -> PathBuf {
        PathBuf::from("/nonexistent-trd-test-home")
    }

    #[test]
    fn permission_denied_is_missing() {
        let state = probe(&fake_home(), |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(state, FdaState::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn eperm_and_eacces_raw_errors_are_missing() {
        // EPERM is 1 and EACCES is 13 on macOS and Linux.
        for errno in [1, 13] {
            let state = probe(&fake_home(), |_| Err(io::Error::from_raw_os_error(errno)));
            assert_eq!(state, FdaState::Missing, "errno {errno}");
        }
    }

    #[test]
    fn not_found_is_granted() {
        let state = probe(&fake_home(), |_| {
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert_eq!(state, FdaState::Granted);
    }

    #[test]
    fn other_errors_are_granted() {
        let state = probe(&fake_home(), |_| Err(io::Error::from(io::ErrorKind::Other)));
        assert_eq!(state, FdaState::Granted);
    }

    #[test]
    fn successful_open_is_granted() {
        let dir = std::env::temp_dir().join(format!("trd-fda-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp fixture dir");
        let fixture = dir.join("fixture.db");
        std::fs::write(&fixture, b"").expect("write temp fixture");
        let state = probe(&fake_home(), |_| File::open(&fixture));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(state, FdaState::Granted);
    }

    #[test]
    fn disabled_guard_is_granted_without_touching_home() {
        let home = fake_home();
        assert_eq!(FdaGuard::new(false).check(&home), FdaState::Granted);
    }

    #[test]
    fn enabled_guard_on_non_macos_is_granted() {
        if cfg!(target_os = "macos") {
            return;
        }
        assert_eq!(FdaGuard::new(true).check(&fake_home()), FdaState::Granted);
    }

    #[test]
    fn tcc_db_path_joins_spec_path() {
        assert_eq!(
            tcc_db_path(Path::new("/home/u")),
            PathBuf::from("/home/u/Library/Application Support/com.apple.TCC/TCC.db")
        );
    }

    #[test]
    fn problem_text_matches_spec_6_12() {
        assert_eq!(PROBLEM_ID, "fda");
        assert_eq!(TITLE, "Full Disk Access is needed");
        assert_eq!(
            DETAIL,
            "trd reads and writes your export, source, backup and video folders, which macOS protects. Until access is granted trd scans nothing, so macOS never asks for permission while you are away."
        );
        assert_eq!(
            FIX,
            "Open System Settings → Privacy & Security → Full Disk Access, add /Applications/Timelapse Render Daemon.app (the + button), and turn it on. trd checks again every 30 seconds."
        );
    }
}
