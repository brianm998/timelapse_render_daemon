// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The Windows logon task (spec 1 §10.3 and §10.4): the scheduled task `TimelapseRenderDaemon`
//! runs `trdw daemon` when the user logs on. It is created from an XML definition written as
//! UTF-16 LE with a byte-order mark.

use std::path::{Path, PathBuf};

use super::{Service, State, Target, fill, show_status, xml_text};

/// The task's name.
pub const TASK: &str = "TimelapseRenderDaemon";
const TEMPLATE: &str = include_str!("../../../../packaging/windows/task.xml.in");

/// The windowed build of trd that the task starts: `trdw.exe` beside the given `trd` executable.
pub fn trdw_path(binary: &Path) -> PathBuf {
    binary.with_file_name("trdw.exe")
}

/// The task definition for `user`, running `trdw daemon [--config config]` (§10.3).
pub fn render_task(user: &str, trdw: &Path, config: Option<&Path>) -> String {
    let mut arguments = "daemon".to_string();
    if let Some(config) = config {
        arguments.push_str(&format!(" --config \"{}\"", config.display()));
    }
    let user = xml_text(user);
    let command = xml_text(&trdw.display().to_string());
    let arguments = xml_text(&arguments);
    fill(
        TEMPLATE,
        &[
            ("USER", user.as_str()),
            ("COMMAND", command.as_str()),
            ("ARGUMENTS", arguments.as_str()),
        ],
    )
}

/// The task file's bytes: UTF-16 LE behind a byte-order mark. schtasks rejects some UTF-8 files.
pub fn encode_task(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// The text of task-file bytes written by [`encode_task`]. None when the mark or the length is
/// wrong, or the units are not valid UTF-16.
#[cfg(test)]
pub fn decode_task(bytes: &[u8]) -> Option<String> {
    let body = bytes.strip_prefix(&[0xFFu8, 0xFE])?;
    let chunks = body.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return None;
    }
    let units: Vec<u16> = chunks
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// The task definition install writes, for this target.
fn task_xml(t: &Target) -> String {
    render_task(&t.user, &trdw_path(&t.binary), t.config.as_deref())
}

/// The task file, beside the per-user data.
fn task_file(t: &Target) -> PathBuf {
    t.home
        .join("AppData")
        .join("Local")
        .join("TimelapseRenderDaemon")
        .join("TimelapseRenderDaemon.xml")
}

/// Splits one CSV line. Quotes group a field, and `""` inside quotes is one quote character.
pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            other => field.push(other),
        }
    }
    fields.push(field);
    fields
}

/// Reads `schtasks /Query /V /FO CSV` output: the Status and Last Run Time columns of the task.
pub fn parse_query(text: &str) -> State {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let (Some(header), Some(row)) = (lines.next(), lines.next()) else {
        return State {
            installed: true,
            ..State::default()
        };
    };
    let header = parse_csv_line(header);
    let row = parse_csv_line(row);
    let field = |name: &str| {
        header
            .iter()
            .position(|column| column == name)
            .and_then(|index| row.get(index))
            .cloned()
    };
    State {
        installed: true,
        running: field("Status").as_deref() == Some("Running"),
        pid: None,
        last_run: field("Last Run Time"),
    }
}

/// The task's state: installed when its query succeeds.
fn query(t: &Target) -> Result<State, String> {
    let output = t.exec("schtasks", &["/Query", "/TN", TASK, "/V", "/FO", "CSV"])?;
    if !output.success {
        return Ok(State::default());
    }
    Ok(parse_query(&output.stdout))
}

/// The Windows implementation of [`Service`].
pub struct LogonTask;

impl Service for LogonTask {
    fn install(&self, t: &Target) -> Result<(), String> {
        let file = task_file(t);
        let xml = task_xml(t);
        // A dry run prints `xml` (readable text), not the UTF-16 bytes that are written.
        t.write_file(&file, &encode_task(&xml), &xml)?;
        let path = file.display().to_string();
        t.exec_ok(
            "schtasks",
            &["/Create", "/TN", TASK, "/XML", path.as_str(), "/F"],
        )?;
        t.exec_ok("schtasks", &["/Run", "/TN", TASK])?;
        t.report(&format!("created task {TASK} and started it"));
        Ok(())
    }

    fn uninstall(&self, t: &Target) -> Result<(), String> {
        // Deleting a task does not stop a running trdw, so the task ends first.
        t.exec_warn("schtasks", &["/End", "/TN", TASK]);
        t.exec_ok("schtasks", &["/Delete", "/TN", TASK, "/F"])?;
        t.remove_file(&task_file(t))?;
        t.report(&format!("uninstalled {TASK}"));
        Ok(())
    }

    fn status(&self, t: &Target) -> Result<(), String> {
        show_status(t, &query(t)?);
        Ok(())
    }

    fn restart(&self, t: &Target) -> Result<(), String> {
        t.exec_warn("schtasks", &["/End", "/TN", TASK]);
        t.exec_ok("schtasks", &["/Run", "/TN", TASK])?;
        t.report(&format!("restarted {TASK}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::service::testing::{files_under, put_file, target};
    use crate::service::{CommandOutput, RecordingRunner, write_listing};

    const USER: &str = "EXAMPLE\\example";
    const TRDW: &str = r"C:\Users\example\AppData\Local\Programs\TimelapseRenderDaemon\trdw.exe";
    const CONFIG: &str = r"C:\Users\example\AppData\Roaming\TimelapseRenderDaemon\config.json";

    #[test]
    fn task_xml_matches_golden() {
        let bytes = encode_task(&render_task(USER, Path::new(TRDW), Some(Path::new(CONFIG))));
        assert_eq!(&bytes[..2], &[0xFFu8, 0xFE], "the byte-order mark");
        assert_eq!(
            decode_task(&bytes).as_deref(),
            Some(include_str!(
                "../../tests/golden/service/windows.task.xml.golden"
            ))
        );
    }

    #[test]
    fn task_template_replaces_a_running_instance() {
        let xml = render_task(USER, Path::new(TRDW), Some(Path::new(CONFIG)));
        assert!(xml.contains("<MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>"));
    }

    #[test]
    fn decode_rejects_a_missing_mark_and_an_odd_length() {
        assert_eq!(decode_task(b"<Task/>"), None);
        assert_eq!(decode_task(&[0xFFu8, 0xFE, 0x41]), None);
        assert_eq!(decode_task(&encode_task("ok")).as_deref(), Some("ok"));
    }

    #[test]
    fn install_writes_the_task_then_creates_and_runs_it() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let mut t = target(home.path(), &runner);
        t.binary = PathBuf::from("/opt/trd/trd.exe");
        LogonTask.install(&t).expect("install");
        let file = task_file(&t);
        let bytes = std::fs::read(&file).expect("the task file is written");
        assert_eq!(&bytes[..2], &[0xFFu8, 0xFE]);
        let xml = decode_task(&bytes).expect("UTF-16 LE with a mark");
        // Compare against the path the code builds: Windows joins with `\`, Unix with `/`.
        let command = format!(
            "<Command>\"{}\"</Command>",
            trdw_path(Path::new("/opt/trd/trd.exe")).display()
        );
        assert!(xml.contains(&command), "{xml}");
        assert!(xml.contains("<Arguments>daemon</Arguments>"), "{xml}");
        assert_eq!(
            runner.calls(),
            [
                format!(
                    "schtasks /Create /TN TimelapseRenderDaemon /XML {} /F",
                    file.display()
                ),
                "schtasks /Run /TN TimelapseRenderDaemon".to_string(),
            ]
        );
    }

    #[test]
    fn dry_run_install_prints_the_readable_task_xml() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let mut t = target(home.path(), &runner);
        t.binary = PathBuf::from("/opt/trd/trd.exe");
        t.dry_run = true;
        LogonTask.install(&t).expect("dry-run install");
        assert_eq!(files_under(home.path()), Vec::<PathBuf>::new());
        let xml = task_xml(&t);
        let listing = write_listing(&task_file(&t), encode_task(&xml).len(), &xml);
        let command = format!(
            "<Command>\"{}\"</Command>",
            trdw_path(Path::new("/opt/trd/trd.exe")).display()
        );
        assert!(listing.contains(&command), "{listing}");
        assert!(
            listing.contains("<Arguments>daemon</Arguments>"),
            "{listing}"
        );
        assert!(!listing.contains('\u{FFFD}'), "{listing}");
        assert_eq!(
            runner.calls(),
            [
                format!(
                    "schtasks /Create /TN TimelapseRenderDaemon /XML {} /F",
                    task_file(&t).display()
                ),
                "schtasks /Run /TN TimelapseRenderDaemon".to_string(),
            ]
        );
    }

    #[test]
    fn uninstall_ends_the_task_before_deleting_it() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        put_file(&task_file(&t));
        LogonTask.uninstall(&t).expect("uninstall");
        assert_eq!(
            runner.calls(),
            [
                "schtasks /End /TN TimelapseRenderDaemon",
                "schtasks /Delete /TN TimelapseRenderDaemon /F",
            ]
        );
        assert_eq!(files_under(home.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn restart_ends_then_runs_the_task() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::new();
        let t = target(home.path(), &runner);
        LogonTask.restart(&t).expect("restart");
        assert_eq!(
            runner.calls(),
            [
                "schtasks /End /TN TimelapseRenderDaemon",
                "schtasks /Run /TN TimelapseRenderDaemon",
            ]
        );
    }

    #[test]
    fn status_reads_the_csv_report() {
        let home = tempfile::tempdir().expect("a temporary home");
        let csv = concat!(
            "\"HostName\",\"TaskName\",\"Status\",\"Last Run Time\"\n",
            "\"HOST\",\"\\TimelapseRenderDaemon\",\"Running\",\"11/25/2026 9:00:00 AM\"\n",
        );
        let runner = RecordingRunner::scripted(vec![CommandOutput::ok(csv)]);
        let t = target(home.path(), &runner);
        assert_eq!(
            query(&t).expect("query"),
            State {
                installed: true,
                running: true,
                pid: None,
                last_run: Some("11/25/2026 9:00:00 AM".to_string()),
            }
        );
    }

    #[test]
    fn status_of_a_missing_task_is_not_installed() {
        let home = tempfile::tempdir().expect("a temporary home");
        let runner = RecordingRunner::scripted(vec![CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "ERROR: The system cannot find the file specified.".to_string(),
        }]);
        let t = target(home.path(), &runner);
        assert_eq!(query(&t).expect("query"), State::default());
    }

    #[test]
    fn csv_line_keeps_quoted_commas_and_quotes() {
        assert_eq!(
            parse_csv_line("\"a, b\",c,\"say \"\"hi\"\"\""),
            ["a, b", "c", "say \"hi\""]
        );
    }
}
