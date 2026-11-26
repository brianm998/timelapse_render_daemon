// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The macOS LaunchAgent (spec 1 §10.1 and §10.4): `~/Library/LaunchAgents/com.timelapserender.trd.plist`,
//! loaded into the user's GUI domain with `launchctl`.

use std::path::{Path, PathBuf};

use super::{Service, State, Target, fill, show_status, xml_text};

/// The launchd label, which is also the agent's file name.
pub const LABEL: &str = "com.timelapserender.trd";
/// The agent the .pkg installs (§10.1). When it exists, install does nothing.
pub const PACKAGE_PLIST: &str = "/Library/LaunchAgents/com.timelapserender.trd.plist";
const TEMPLATE: &str = include_str!("../../../../packaging/macos/com.timelapserender.trd.plist");

/// The agent for `program daemon [--config config]` (§10.1).
pub fn render_plist(program: &Path, config: Option<&Path>) -> String {
    let mut arguments = vec![program.display().to_string(), "daemon".to_string()];
    if let Some(config) = config {
        arguments.push("--config".to_string());
        arguments.push(config.display().to_string());
    }
    let items: String = arguments
        .iter()
        .map(|argument| format!("<string>{}</string>", xml_text(argument)))
        .collect();
    fill(TEMPLATE, &[("PROGRAM_ARGUMENTS", items.as_str())])
}

/// The per-user agent file.
fn user_plist(t: &Target) -> PathBuf {
    t.home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// The service as launchctl names it: `gui/<uid>/<label>`.
fn service_name(t: &Target) -> String {
    format!("gui/{}/{LABEL}", t.uid)
}

/// Reads `launchctl print` output: whether the state is `running`, and the pid.
pub fn parse_print(text: &str) -> (bool, Option<u32>) {
    let mut running = false;
    let mut pid = None;
    for line in text.lines().map(str::trim) {
        if line == "state = running" {
            running = true;
        } else if let Some(value) = line.strip_prefix("pid = ") {
            pid = value.trim().parse().ok();
        }
    }
    (running, pid)
}

/// The agent's state: installed when its file exists, running when launchd reports it so.
fn query(t: &Target) -> Result<State, String> {
    let installed = user_plist(t).exists() || t.system_plist.exists();
    let name = service_name(t);
    let output = t.exec("launchctl", &["print", name.as_str()])?;
    let (running, pid) = if output.success {
        parse_print(&output.stdout)
    } else {
        (false, None)
    };
    Ok(State {
        installed,
        running,
        pid,
        last_run: None,
    })
}

/// The macOS implementation of [`Service`].
pub struct LaunchAgent;

impl Service for LaunchAgent {
    fn install(&self, t: &Target) -> Result<(), String> {
        if t.system_plist.exists() {
            println!(
                "already installed: {} (installed by the package); nothing to do",
                t.system_plist.display()
            );
            return Ok(());
        }
        let plist = user_plist(t);
        let text = render_plist(&t.binary, t.config.as_deref());
        t.write_file(&plist, text.as_bytes(), &text)?;
        let domain = format!("gui/{}", t.uid);
        let path = plist.display().to_string();
        // A previous install may still be loaded, and bootstrap refuses a label that is loaded.
        // Booting it out first makes install re-runnable; when nothing is loaded this fails, which
        // is expected, so its result is ignored.
        let _ = t.exec("launchctl", &["bootout", service_name(t).as_str()]);
        t.exec_ok("launchctl", &["bootstrap", domain.as_str(), path.as_str()])?;
        t.report(&format!("installed {} and started it", plist.display()));
        Ok(())
    }

    fn uninstall(&self, t: &Target) -> Result<(), String> {
        let name = service_name(t);
        t.exec_warn("launchctl", &["bootout", name.as_str()]);
        t.remove_file(&user_plist(t))?;
        if t.system_plist.exists() {
            println!(
                "{} was installed by the package and is not removed; to remove it: sudo rm {}",
                t.system_plist.display(),
                t.system_plist.display()
            );
        }
        t.report(&format!("uninstalled {LABEL}"));
        Ok(())
    }

    fn status(&self, t: &Target) -> Result<(), String> {
        show_status(t, &query(t)?);
        Ok(())
    }

    fn restart(&self, t: &Target) -> Result<(), String> {
        let name = service_name(t);
        t.exec_ok("launchctl", &["kickstart", "-k", name.as_str()])?;
        t.report(&format!("restarted {name}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::service::testing::{files_under, put_file, target};
    use crate::service::{CommandOutput, RecordingRunner};

    const PROGRAM: &str = "/Applications/Timelapse Render Daemon.app/Contents/MacOS/trd";
    const CONFIG: &str = "/Users/example/Library/Application Support/trd/config.json";

    #[test]
    fn plist_matches_golden() {
        let text = render_plist(Path::new(PROGRAM), Some(Path::new(CONFIG)));
        assert_eq!(
            text,
            include_str!("../../tests/golden/service/macos.plist.golden")
        );
    }

    #[test]
    fn plist_without_config_has_no_config_argument() {
        let text = render_plist(Path::new(PROGRAM), None);
        assert!(text.contains(&format!(
            "<string>{PROGRAM}</string><string>daemon</string></array>"
        )));
    }

    #[test]
    fn install_writes_the_agent_and_bootstraps_it() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let mut t = target(home.path(), &runner);
        t.binary = PathBuf::from(PROGRAM);
        t.config = Some(PathBuf::from(CONFIG));
        LaunchAgent.install(&t).expect("install");
        let plist = user_plist(&t);
        assert_eq!(
            std::fs::read_to_string(&plist).expect("the agent is written"),
            render_plist(Path::new(PROGRAM), Some(Path::new(CONFIG)))
        );
        assert_eq!(
            runner.calls(),
            [
                "launchctl bootout gui/501/com.timelapserender.trd".to_string(),
                format!("launchctl bootstrap gui/501 {}", plist.display()),
            ]
        );
    }

    #[test]
    fn reinstall_boots_out_the_loaded_agent_before_bootstrapping() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::scripted(vec![
            // The bootout of the loaded agent succeeds; the bootstrap that follows also succeeds.
            CommandOutput::ok(""),
            CommandOutput::ok(""),
        ]);
        let mut t = target(home.path(), &runner);
        t.binary = PathBuf::from(PROGRAM);
        put_file(&user_plist(&t));
        LaunchAgent.install(&t).expect("reinstall");
        let plist = user_plist(&t);
        assert_eq!(
            runner.calls(),
            [
                "launchctl bootout gui/501/com.timelapserender.trd".to_string(),
                format!("launchctl bootstrap gui/501 {}", plist.display()),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&plist).expect("the agent is rewritten"),
            render_plist(Path::new(PROGRAM), None)
        );
    }

    #[test]
    fn reinstall_succeeds_when_nothing_was_loaded() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::scripted(vec![
            // bootout fails because the agent is not loaded; that is ignored.
            CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "Boot-out failed: 3: No such process".to_string(),
            },
            CommandOutput::ok(""),
        ]);
        let t = target(home.path(), &runner);
        LaunchAgent
            .install(&t)
            .expect("install after a failed bootout");
        assert_eq!(runner.calls().len(), 2, "{:?}", runner.calls());
    }

    #[test]
    fn install_does_nothing_when_the_package_agent_exists() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        put_file(&t.system_plist);
        LaunchAgent.install(&t).expect("install");
        assert!(
            runner.calls().is_empty(),
            "commands ran: {:?}",
            runner.calls()
        );
        assert!(!user_plist(&t).exists(), "a per-user agent was written");
    }

    #[test]
    fn uninstall_boots_out_and_removes_only_the_user_agent() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        put_file(&user_plist(&t));
        LaunchAgent.uninstall(&t).expect("uninstall");
        assert_eq!(
            runner.calls(),
            ["launchctl bootout gui/501/com.timelapserender.trd"]
        );
        assert!(!user_plist(&t).exists());
        assert_eq!(files_under(home.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn restart_kickstarts_the_agent() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        LaunchAgent.restart(&t).expect("restart");
        assert_eq!(
            runner.calls(),
            ["launchctl kickstart -k gui/501/com.timelapserender.trd"]
        );
    }

    #[test]
    fn status_reads_the_state_and_pid() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::scripted(vec![CommandOutput::ok(
            "gui/501/com.timelapserender.trd = {\n\tstate = running\n\tpid = 4321\n}\n",
        )]);
        let t = target(home.path(), &runner);
        put_file(&user_plist(&t));
        assert_eq!(
            query(&t).expect("query"),
            State {
                installed: true,
                running: true,
                pid: Some(4321),
                last_run: None,
            }
        );
    }

    #[test]
    fn status_of_a_missing_agent_is_not_installed() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::scripted(vec![CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "Could not find service".to_string(),
        }]);
        let t = target(home.path(), &runner);
        assert_eq!(query(&t).expect("query"), State::default());
    }

    #[test]
    fn parse_print_ignores_other_lines() {
        assert_eq!(
            parse_print("\tlast exit code = 0\n\tstate = waiting\n"),
            (false, None)
        );
    }
}
