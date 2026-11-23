// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Default data, log and config directories per platform (§6.13, overview §4).
//!
//! The paths are computed from a home directory and an [`Env`] value passed in by the caller, so
//! no test ever reads the real home or the real environment. [`Env::system`] is the only place
//! that reads them.

use std::env;
use std::path::{Path, PathBuf};

/// The operating system whose layout applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    /// The OS this binary was built for.
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// The inputs of the path rules: the home directory and the variables that move the locations.
///
/// A variable that is `None` is unset. An `XDG_*` variable that holds a relative path is treated
/// as unset too, as the XDG Base Directory specification requires; either way the rules fall back
/// to the home directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub localappdata: Option<PathBuf>,
}

impl Env {
    /// The real environment of this process. Only the daemon's start-up calls this.
    pub fn system() -> Env {
        let home = env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default();
        let var = |name: &str| env::var_os(name).map(PathBuf::from);
        Env {
            home,
            xdg_config_home: var("XDG_CONFIG_HOME"),
            xdg_data_home: var("XDG_DATA_HOME"),
            xdg_state_home: var("XDG_STATE_HOME"),
            appdata: var("APPDATA"),
            localappdata: var("LOCALAPPDATA"),
        }
    }
}

/// The default locations of the daemon's files on one OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// The default config file (used when neither `--config` nor the `config_path` setting is set).
    pub config_default: PathBuf,
    /// The data directory: `trd.sqlite3`, `identity/`, `thumbnails/`, `renders/`.
    pub data: PathBuf,
    /// The log directory: the rolling `trd` log files and `trd-stderr.log`.
    pub logs: PathBuf,
}

/// Returns the variable's value when it is an absolute POSIX path (it starts with `/`).
///
/// The XDG rules apply to the Linux layout, so the test is the POSIX one whatever host runs the
/// code. `Path::is_absolute` would reject `/xdg/config` on Windows and make the tests host-dependent.
fn absolute(value: &Option<PathBuf>) -> Option<&Path> {
    value
        .as_deref()
        .filter(|p| p.to_string_lossy().starts_with('/'))
}

impl Dirs {
    /// The directories of `os` for the given environment.
    pub fn for_os(os: Os, env: &Env) -> Dirs {
        let home = env.home.as_path();
        match os {
            Os::MacOs => {
                let support = home
                    .join("Library")
                    .join("Application Support")
                    .join("TimelapseRenderDaemon");
                Dirs {
                    config_default: support.join("config.json"),
                    data: support.join("trd"),
                    logs: home
                        .join("Library")
                        .join("Logs")
                        .join("TimelapseRenderDaemon"),
                }
            }
            Os::Linux => {
                let config = absolute(&env.xdg_config_home)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| home.join(".config"));
                let data = absolute(&env.xdg_data_home)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| home.join(".local").join("share"));
                let state = absolute(&env.xdg_state_home)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| home.join(".local").join("state"));
                Dirs {
                    config_default: config.join("trd").join("config.json"),
                    data: data.join("trd"),
                    logs: state.join("trd"),
                }
            }
            Os::Windows => {
                // Windows always sets these to absolute paths, so they are used as given.
                let roaming = env
                    .appdata
                    .clone()
                    .unwrap_or_else(|| home.join("AppData").join("Roaming"));
                let local = env
                    .localappdata
                    .clone()
                    .unwrap_or_else(|| home.join("AppData").join("Local"));
                let app = "TimelapseRenderDaemon";
                Dirs {
                    config_default: roaming.join(app).join("config.json"),
                    data: local.join(app).join("data"),
                    logs: local.join(app).join("logs"),
                }
            }
        }
    }

    /// Applies `--data-dir` and `--log-dir`; a `None` leaves that directory unchanged.
    pub fn with_overrides(mut self, data_dir: Option<PathBuf>, log_dir: Option<PathBuf>) -> Dirs {
        if let Some(data) = data_dir {
            self.data = data;
        }
        if let Some(logs) = log_dir {
            self.logs = logs;
        }
        self
    }

    /// The database file inside the data directory.
    pub fn database(&self) -> PathBuf {
        self.data.join("trd.sqlite3")
    }

    /// The stderr file that daemon mode redirects the process's stderr to.
    pub fn stderr_log(&self) -> PathBuf {
        self.logs.join("trd-stderr.log")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_at(home: &str) -> Env {
        Env {
            home: PathBuf::from(home),
            ..Env::default()
        }
    }

    #[test]
    fn dirs_per_os() {
        let home = Path::new("/fake/home");

        let mac = Dirs::for_os(Os::MacOs, &env_at("/fake/home"));
        assert_eq!(
            mac.config_default,
            home.join("Library/Application Support/TimelapseRenderDaemon/config.json")
        );
        assert_eq!(
            mac.data,
            home.join("Library/Application Support/TimelapseRenderDaemon/trd")
        );
        assert_eq!(mac.logs, home.join("Library/Logs/TimelapseRenderDaemon"));

        // XDG unset: the defaults under the home directory.
        let linux = Dirs::for_os(Os::Linux, &env_at("/fake/home"));
        assert_eq!(linux.config_default, home.join(".config/trd/config.json"));
        assert_eq!(linux.data, home.join(".local/share/trd"));
        assert_eq!(linux.logs, home.join(".local/state/trd"));

        // XDG set to absolute paths: they replace the defaults.
        let xdg = Env {
            xdg_config_home: Some(PathBuf::from("/xdg/config")),
            xdg_data_home: Some(PathBuf::from("/xdg/data")),
            xdg_state_home: Some(PathBuf::from("/xdg/state")),
            ..env_at("/fake/home")
        };
        let linux_xdg = Dirs::for_os(Os::Linux, &xdg);
        assert_eq!(
            linux_xdg.config_default,
            Path::new("/xdg/config/trd/config.json")
        );
        assert_eq!(linux_xdg.data, Path::new("/xdg/data/trd"));
        assert_eq!(linux_xdg.logs, Path::new("/xdg/state/trd"));

        // A relative XDG value is ignored.
        let relative = Env {
            xdg_config_home: Some(PathBuf::from("relative/config")),
            xdg_data_home: Some(PathBuf::from("relative/data")),
            xdg_state_home: Some(PathBuf::from("relative/state")),
            ..env_at("/fake/home")
        };
        assert_eq!(Dirs::for_os(Os::Linux, &relative), linux);

        let win_home = Path::new("C:/Users/fake");
        let win = Dirs::for_os(Os::Windows, &env_at("C:/Users/fake"));
        assert_eq!(
            win.config_default,
            win_home.join("AppData/Roaming/TimelapseRenderDaemon/config.json")
        );
        assert_eq!(
            win.data,
            win_home.join("AppData/Local/TimelapseRenderDaemon/data")
        );
        assert_eq!(
            win.logs,
            win_home.join("AppData/Local/TimelapseRenderDaemon/logs")
        );

        let win_set = Env {
            appdata: Some(PathBuf::from("C:/Roam")),
            localappdata: Some(PathBuf::from("C:/Local")),
            ..env_at("C:/Users/fake")
        };
        let win_env = Dirs::for_os(Os::Windows, &win_set);
        assert_eq!(
            win_env.config_default,
            Path::new("C:/Roam/TimelapseRenderDaemon/config.json")
        );
        assert_eq!(
            win_env.data,
            Path::new("C:/Local/TimelapseRenderDaemon/data")
        );
        assert_eq!(
            win_env.logs,
            Path::new("C:/Local/TimelapseRenderDaemon/logs")
        );
    }

    #[test]
    fn dir_overrides() {
        let base = Dirs::for_os(Os::Linux, &env_at("/fake/home"));

        let both = base.clone().with_overrides(
            Some(PathBuf::from("/o/data")),
            Some(PathBuf::from("/o/logs")),
        );
        assert_eq!(both.data, Path::new("/o/data"));
        assert_eq!(both.logs, Path::new("/o/logs"));
        assert_eq!(both.config_default, base.config_default);

        let data_only = base
            .clone()
            .with_overrides(Some(PathBuf::from("/o/data")), None);
        assert_eq!(data_only.data, Path::new("/o/data"));
        assert_eq!(data_only.logs, base.logs);
        assert_eq!(data_only.config_default, base.config_default);

        let logs_only = base
            .clone()
            .with_overrides(None, Some(PathBuf::from("/o/logs")));
        assert_eq!(logs_only.data, base.data);
        assert_eq!(logs_only.logs, Path::new("/o/logs"));
        assert_eq!(logs_only.config_default, base.config_default);

        assert_eq!(base.clone().with_overrides(None, None), base);
    }
}
