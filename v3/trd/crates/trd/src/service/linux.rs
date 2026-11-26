// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The Linux systemd user unit (spec 1 §10.2 and §10.4): `~/.config/systemd/user/trd.service`,
//! managed with `systemctl --user`.

use std::path::{Path, PathBuf};

use super::{Service, State, Target, fill, show_status};

/// The unit's name, without `.service`.
pub const UNIT: &str = "trd";
/// The unit the .deb installs (§10.2). When it exists, install writes no per-user unit.
pub const PACKAGE_UNIT: &str = "/usr/lib/systemd/user/trd.service";
const TEMPLATE: &str = include_str!("../../../../packaging/linux/trd.service");

/// The unit for `program daemon [--config config]` (§10.2).
pub fn render_unit(program: &Path, config: Option<&Path>) -> String {
    let mut words = vec![program.display().to_string(), "daemon".to_string()];
    if let Some(config) = config {
        words.push("--config".to_string());
        words.push(config.display().to_string());
    }
    let exec_start: Vec<String> = words.iter().map(|word| quote_arg(word)).collect();
    fill(TEMPLATE, &[("EXEC_START", exec_start.join(" ").as_str())])
}

/// One word of `ExecStart`. A plain word stays as it is; any other word is double-quoted, with
/// `\`, `"`, `$` and `%` escaped the way systemd reads them.
pub fn quote_arg(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/-_.:=,+@".contains(c));
    if plain {
        return word.to_string();
    }
    let mut quoted = String::from("\"");
    for c in word.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '$' => quoted.push_str("$$"),
            '%' => quoted.push_str("%%"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// The per-user unit file.
fn user_unit(t: &Target) -> PathBuf {
    t.home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{UNIT}.service"))
}

/// Reads `systemctl --user show -p ActiveState,MainPID`: whether the unit is active, and its pid.
pub fn parse_show(text: &str) -> (bool, Option<u32>) {
    let mut active = false;
    let mut pid = None;
    for line in text.lines() {
        if let Some(state) = line.strip_prefix("ActiveState=") {
            active = state.trim() == "active";
        } else if let Some(value) = line.strip_prefix("MainPID=") {
            pid = value.trim().parse::<u32>().ok().filter(|pid| *pid > 0);
        }
    }
    (active, pid)
}

/// The unit's state: installed when a unit file exists, running when systemd reports it active.
fn query(t: &Target) -> Result<State, String> {
    let installed = user_unit(t).exists() || t.system_unit.exists();
    let output = t.exec(
        "systemctl",
        &["--user", "show", UNIT, "-p", "ActiveState,MainPID"],
    )?;
    let (active, pid) = parse_show(&output.stdout);
    Ok(State {
        installed,
        running: output.success && active,
        pid,
        last_run: None,
    })
}

/// The Linux implementation of [`Service`].
pub struct SystemdUser;

impl Service for SystemdUser {
    fn install(&self, t: &Target) -> Result<(), String> {
        let unit = user_unit(t);
        if t.system_unit.exists() {
            println!(
                "{} is installed by the package; no per-user unit is written",
                t.system_unit.display()
            );
        } else {
            let text = render_unit(&t.binary, t.config.as_deref());
            t.write_file(&unit, text.as_bytes())?;
        }
        t.exec_ok("systemctl", &["--user", "daemon-reload"])?;
        t.exec_ok("systemctl", &["--user", "enable", "--now", UNIT])?;
        t.report(&format!("enabled and started {UNIT}"));
        Ok(())
    }

    fn uninstall(&self, t: &Target) -> Result<(), String> {
        t.exec_warn("systemctl", &["--user", "disable", "--now", UNIT]);
        t.remove_file(&user_unit(t))?;
        t.exec_warn("systemctl", &["--user", "daemon-reload"]);
        if t.system_unit.exists() {
            println!(
                "{} is installed by the package and is not removed",
                t.system_unit.display()
            );
        }
        t.report(&format!("uninstalled {UNIT}"));
        Ok(())
    }

    fn status(&self, t: &Target) -> Result<(), String> {
        show_status(t, &query(t)?);
        Ok(())
    }

    fn restart(&self, t: &Target) -> Result<(), String> {
        t.exec_ok("systemctl", &["--user", "restart", UNIT])?;
        t.report(&format!("restarted {UNIT}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::service::testing::{files_under, put_file, target};
    use crate::service::{CommandOutput, RecordingRunner};

    const PROGRAM: &str = "/usr/lib/trd/trd";
    const CONFIG: &str = "/home/example/.config/trd/config.json";

    #[test]
    fn unit_matches_golden() {
        let text = render_unit(Path::new(PROGRAM), Some(Path::new(CONFIG)));
        assert_eq!(
            text,
            include_str!("../../tests/golden/service/linux.service.golden")
        );
    }

    #[test]
    fn quote_arg_leaves_plain_words_and_quotes_the_rest() {
        assert_eq!(quote_arg("/usr/lib/trd/trd"), "/usr/lib/trd/trd");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg("50%"), "\"50%%\"");
        assert_eq!(quote_arg("$HOME"), "\"$$HOME\"");
        assert_eq!(quote_arg("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn install_writes_the_unit_then_reloads_and_enables_it() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let mut t = target(home.path(), &runner);
        t.binary = PathBuf::from(PROGRAM);
        t.config = Some(PathBuf::from(CONFIG));
        SystemdUser.install(&t).expect("install");
        let unit = user_unit(&t);
        assert_eq!(
            std::fs::read_to_string(&unit).expect("the unit is written"),
            render_unit(Path::new(PROGRAM), Some(Path::new(CONFIG)))
        );
        assert_eq!(
            runner.calls(),
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable --now trd",
            ]
        );
    }

    #[test]
    fn install_writes_no_unit_when_the_package_unit_exists() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        put_file(&t.system_unit);
        SystemdUser.install(&t).expect("install");
        assert!(!user_unit(&t).exists(), "a per-user unit was written");
        assert_eq!(
            runner.calls(),
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable --now trd",
            ]
        );
    }

    #[test]
    fn uninstall_disables_removes_the_unit_and_reloads() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        put_file(&user_unit(&t));
        SystemdUser.uninstall(&t).expect("uninstall");
        assert_eq!(
            runner.calls(),
            [
                "systemctl --user disable --now trd",
                "systemctl --user daemon-reload",
            ]
        );
        assert_eq!(files_under(home.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn restart_restarts_the_unit() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        SystemdUser.restart(&t).expect("restart");
        assert_eq!(runner.calls(), ["systemctl --user restart trd"]);
    }

    #[test]
    fn status_reads_the_active_state_and_pid() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner =
            RecordingRunner::scripted(vec![CommandOutput::ok("ActiveState=active\nMainPID=77\n")]);
        let t = target(home.path(), &runner);
        put_file(&user_unit(&t));
        assert_eq!(
            query(&t).expect("query"),
            State {
                installed: true,
                running: true,
                pid: Some(77),
                last_run: None,
            }
        );
    }

    #[test]
    fn status_of_an_inactive_unit_has_no_pid() {
        assert_eq!(
            parse_show("ActiveState=inactive\nMainPID=0\n"),
            (false, None)
        );
    }
}
