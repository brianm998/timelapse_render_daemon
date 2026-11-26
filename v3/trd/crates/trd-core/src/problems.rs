// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Problems raised by the daemon and their dismissal (§6.10).
//!
//! A problem has a stable id (`fda`, `tool:<name>`, `config`, `config-issues`, `codec:<name>`,
//! `path:<path>`). The engine reports the problems that hold at each tick; [`Problems::reconcile`]
//! keeps the `since_ms` of a problem that persists and drops one that is no longer reported. The
//! lan and mdns problems are raised by the trd crate, not here.

use std::collections::BTreeMap;

use crate::fda;
use crate::scan::PathProblem;
use crate::tools::ToolProblem;

/// The id of the config.json missing or unparseable problem.
pub const ID_CONFIG: &str = "config";
/// The id of the problem for config.json entries that were ignored.
pub const ID_CONFIG_ISSUES: &str = "config-issues";
/// The id of the problem of a LAN listener that cannot bind (§8.2).
pub const ID_LAN: &str = "lan";

/// How bad a problem is. The proto's `SEVERITY_ERROR` is the higher one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// One problem as a snapshot reports it (§7 `Problem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub id: String,
    pub severity: Severity,
    /// The proto `ProblemKind` name, such as `PROBLEM_TOOL_MISSING`.
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    pub fix: String,
    pub paths: Vec<String>,
    /// When the problem was first reported, in ms. Kept while it persists.
    pub since_ms: i64,
}

/// The problems that currently hold, by id.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Problems {
    held: BTreeMap<String, Problem>,
}

impl Problems {
    /// An empty set.
    pub fn new() -> Problems {
        Problems::default()
    }

    /// Replaces the set with `reported`. A problem that is already held keeps its `since_ms`; a new
    /// one takes `now_ms`; a held one that is not reported is removed. When an id is reported twice,
    /// the first report wins.
    pub fn reconcile(&mut self, now_ms: i64, reported: Vec<Problem>) {
        let mut next: BTreeMap<String, Problem> = BTreeMap::new();
        for mut problem in reported {
            if next.contains_key(&problem.id) {
                continue;
            }
            problem.since_ms = self
                .held
                .get(&problem.id)
                .map_or(now_ms, |held| held.since_ms);
            next.insert(problem.id.clone(), problem);
        }
        self.held = next;
    }

    /// The problems, errors first, then by id (§7).
    pub fn list(&self) -> Vec<Problem> {
        let mut out: Vec<Problem> = self.held.values().cloned().collect();
        out.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// The problem with `id`, if it holds.
    pub fn get(&self, id: &str) -> Option<&Problem> {
        self.held.get(id)
    }
}

/// The Full Disk Access problem (§6.12). Its text is the constants of [`crate::fda`].
pub fn fda_problem() -> Problem {
    Problem {
        id: fda::PROBLEM_ID.to_owned(),
        severity: Severity::Error,
        kind: "PROBLEM_FULL_DISK_ACCESS_MISSING",
        title: fda::TITLE.to_owned(),
        detail: fda::DETAIL.to_owned(),
        fix: fda::FIX.to_owned(),
        paths: Vec::new(),
        since_ms: 0,
    }
}

/// The problem of a tool that is missing or broken (§6.15). ffmpeg and ffprobe are errors: nothing
/// is promoted without them. exiftool is a warning: without it no EXIF job or XMP embed runs.
pub fn tool_problem(name: &'static str, problem: &ToolProblem) -> Problem {
    let severity = if name == "exiftool" {
        Severity::Warning
    } else {
        Severity::Error
    };
    let (title, detail, paths) = match problem {
        ToolProblem::Missing { tried } => {
            let paths: Vec<String> = tried
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            (
                format!("{name} is not installed"),
                format!(
                    "trd could not find {name}. It looked in: {}",
                    paths.join(", ")
                ),
                paths,
            )
        }
        ToolProblem::Broken { path, detail } => {
            let path = path.to_string_lossy().into_owned();
            (
                format!("{name} does not run"),
                format!("{path}: {detail}"),
                vec![path],
            )
        }
    };
    let env = if name == "exiftool" {
        "TRD_EXIFTOOL_DIR"
    } else {
        "TRD_FFMPEG_DIR"
    };
    Problem {
        id: format!("tool:{name}"),
        severity,
        kind: match problem {
            ToolProblem::Missing { .. } => "PROBLEM_TOOL_MISSING",
            ToolProblem::Broken { .. } => "PROBLEM_TOOL_BROKEN",
        },
        title,
        detail,
        fix: format!(
            "Install {name}, or set {env} to the folder that holds it, then wait for the next check."
        ),
        paths,
        since_ms: 0,
    }
}

/// The problem of a config.json that is missing (`PROBLEM_CONFIG_MISSING`) or does not parse
/// (`PROBLEM_CONFIG_INVALID`), at `path`.
pub fn config_problem(missing: bool, path: &str) -> Problem {
    if missing {
        Problem {
            id: ID_CONFIG.to_owned(),
            severity: Severity::Error,
            kind: "PROBLEM_CONFIG_MISSING",
            title: "config.json is missing".to_owned(),
            detail: format!("There is no configuration at {path}, so trd scans nothing."),
            fix: "Create config.json at that path, or point trd at another file with trd config set-path.".to_owned(),
            paths: vec![path.to_owned()],
            since_ms: 0,
        }
    } else {
        Problem {
            id: ID_CONFIG.to_owned(),
            severity: Severity::Error,
            kind: "PROBLEM_CONFIG_INVALID",
            title: "config.json does not parse".to_owned(),
            detail: format!(
                "{path} is not valid JSON. trd keeps the last good configuration until it parses."
            ),
            fix: "Correct the JSON syntax, or save the file again from the app.".to_owned(),
            paths: vec![path.to_owned()],
            since_ms: 0,
        }
    }
}

/// The problem of config.json entries that were ignored or are suspect (`PROBLEM_CONFIG_ISSUES`).
/// `messages` holds `field: message` for each issue; `error` is whether any of them is an ERROR.
pub fn config_issues_problem(
    path: &str,
    messages: Vec<String>,
    fields: Vec<String>,
    error: bool,
) -> Problem {
    Problem {
        id: ID_CONFIG_ISSUES.to_owned(),
        severity: if error {
            Severity::Error
        } else {
            Severity::Warning
        },
        kind: "PROBLEM_CONFIG_ISSUES",
        title: "config.json has entries that trd cannot use".to_owned(),
        detail: messages.join("; "),
        fix: format!(
            "Correct the listed entries in {path}; trd checks the file again when it changes."
        ),
        paths: fields,
        since_ms: 0,
    }
}

/// The problem of a rendering codec that the codec table does not have (`PROBLEM_UNKNOWN_CODEC`).
pub fn codec_problem(codec: &str) -> Problem {
    Problem {
        id: format!("codec:{codec}"),
        severity: Severity::Error,
        kind: "PROBLEM_UNKNOWN_CODEC",
        title: format!("The codec {codec} is not known"),
        detail: "The rendering entries that use this codec are skipped.".to_owned(),
        fix:
            "Add the codec to video_codecs.json, or remove its rendering entries from config.json."
                .to_owned(),
        paths: Vec::new(),
        since_ms: 0,
    }
}

/// The problem of the LAN listener that cannot bind (`PROBLEM_LAN_LISTENER_FAILED`, §8.2). The trd
/// crate raises it: `error` is the bind error.
pub fn lan_problem(error: &str) -> Problem {
    Problem {
        id: ID_LAN.to_owned(),
        severity: Severity::Error,
        kind: "PROBLEM_LAN_LISTENER_FAILED",
        title: "The LAN listener is not running".to_owned(),
        detail: format!(
            "trd cannot bind the LAN address: {error}. Paired devices cannot reach trd until it can. trd tries again every 30 seconds."
        ),
        fix: "Free the address named by --lan-bind, or start trd with another --lan-bind address."
            .to_owned(),
        paths: Vec::new(),
        since_ms: 0,
    }
}

/// The problem of a configured folder that cannot be read (`PROBLEM_PATH_UNREACHABLE`).
pub fn path_problem(problem: &PathProblem) -> Problem {
    Problem {
        id: format!("path:{}", problem.path),
        severity: Severity::Warning,
        kind: "PROBLEM_PATH_UNREACHABLE",
        title: "A folder cannot be read".to_owned(),
        detail: problem.reason.clone(),
        fix: "Check that the folder exists and that trd can read it. trd tries again on the next scan.".to_owned(),
        paths: vec![problem.path.clone()],
        since_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(id: &str, severity: Severity) -> Problem {
        Problem {
            id: id.to_owned(),
            severity,
            kind: "PROBLEM_CONFIG_MISSING",
            title: String::new(),
            detail: String::new(),
            fix: String::new(),
            paths: Vec::new(),
            since_ms: 0,
        }
    }

    #[test]
    fn a_persisting_problem_keeps_its_since_ms() {
        let mut set = Problems::new();
        set.reconcile(100, vec![problem("fda", Severity::Error)]);
        set.reconcile(200, vec![problem("fda", Severity::Error)]);
        assert_eq!(set.get("fda").map(|p| p.since_ms), Some(100));
    }

    #[test]
    fn a_resolved_problem_is_removed_and_a_new_one_starts_at_now() {
        let mut set = Problems::new();
        set.reconcile(100, vec![problem("fda", Severity::Error)]);
        set.reconcile(200, Vec::new());
        assert!(set.get("fda").is_none());
        set.reconcile(300, vec![problem("fda", Severity::Error)]);
        assert_eq!(set.get("fda").map(|p| p.since_ms), Some(300));
    }

    #[test]
    fn the_lan_problem_is_an_error_that_names_the_bind_error() {
        let problem = lan_problem("address already in use");
        assert_eq!(problem.id, ID_LAN);
        assert_eq!(problem.severity, Severity::Error);
        assert_eq!(problem.kind, "PROBLEM_LAN_LISTENER_FAILED");
        assert!(problem.detail.contains("address already in use"));
    }

    #[test]
    fn errors_come_first_then_by_id() {
        let mut set = Problems::new();
        set.reconcile(
            0,
            vec![
                problem("b", Severity::Warning),
                problem("z", Severity::Error),
                problem("a", Severity::Warning),
                problem("m", Severity::Error),
            ],
        );
        let ids: Vec<String> = set.list().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["m", "z", "a", "b"]);
    }
}
