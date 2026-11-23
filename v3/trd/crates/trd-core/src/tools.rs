// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Locating and checking the bundled ffmpeg, ffprobe and exiftool (§6.4, §6.5, §6.15).
//!
//! For each tool the first existing candidate wins. The candidates are searched in the order of
//! §6.15: the `TRD_FFMPEG_DIR` or `TRD_EXIFTOOL_DIR` directory, beside the executable, the
//! development tree, then `PATH`. Every path checked is recorded, so a missing tool's problem lists
//! them all. The found tool's version command then runs (at most 30 s); a tool that fails it is
//! broken.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Id of the problem raised for a tool that is not found (§6.15).
pub const PROBLEM_TOOL_MISSING: &str = "PROBLEM_TOOL_MISSING";
/// Id of the problem raised for a tool whose version command fails (§6.15).
pub const PROBLEM_TOOL_BROKEN: &str = "PROBLEM_TOOL_BROKEN";

/// The longest a version command may run before its tool counts as broken.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_POLL: Duration = Duration::from_millis(20);

/// The tool names, as `ToolInfo.name` carries them.
pub const FFMPEG: &str = "ffmpeg";
pub const FFPROBE: &str = "ffprobe";
pub const EXIFTOOL: &str = "exiftool";

/// How to run one tool: the program and the arguments that come before the tool's own. A Unix
/// exiftool script runs as `<perl> <script>`; every other tool runs directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommand {
    pub program: OsString,
    pub leading_args: Vec<OsString>,
}

impl ToolCommand {
    /// A command that runs the file itself.
    pub fn direct(path: &Path) -> Self {
        ToolCommand {
            program: path.as_os_str().to_owned(),
            leading_args: Vec::new(),
        }
    }

    /// A `Command` for this tool with its leading arguments in place.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.leading_args);
        command
    }
}

/// Why a tool is not available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProblem {
    /// No candidate exists; `tried` lists every path checked, in search order.
    Missing { tried: Vec<PathBuf> },
    /// The tool was found at `path`, but its version command failed with `detail`.
    Broken { path: PathBuf, detail: String },
}

impl ToolProblem {
    /// The problem id (§6.10).
    pub fn id(&self) -> &'static str {
        match self {
            ToolProblem::Missing { .. } => PROBLEM_TOOL_MISSING,
            ToolProblem::Broken { .. } => PROBLEM_TOOL_BROKEN,
        }
    }
}

/// The state of one tool, as the proto `ToolInfo` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: &'static str,
    /// The file found, if any.
    pub path: Option<PathBuf>,
    /// The first line of the version command; empty unless the tool is available.
    pub version: String,
    pub available: bool,
    pub problem: Option<ToolProblem>,
}

/// A tool's report and, when it is available, the command that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTool {
    pub info: ToolInfo,
    pub command: Option<ToolCommand>,
}

/// The three tools, resolved at start-up and again every 5 minutes (§6.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: ResolvedTool,
    pub ffprobe: ResolvedTool,
    pub exiftool: ResolvedTool,
}

impl Tools {
    /// Resolves the three tools for the running platform. `env` reads one environment variable;
    /// `exe` is the path of the running executable.
    pub fn resolve(env: impl Fn(&str) -> Option<OsString>, exe: &Path) -> Tools {
        resolve_with(
            &env,
            exe,
            std::env::consts::OS,
            std::env::consts::ARCH,
            &run_version_command,
        )
    }

    /// The three reports in the order ffmpeg, ffprobe, exiftool.
    pub fn infos(&self) -> Vec<ToolInfo> {
        vec![
            self.ffmpeg.info.clone(),
            self.ffprobe.info.clone(),
            self.exiftool.info.clone(),
        ]
    }
}

/// Resolves the tools for the platform `os`/`arch` (the names `std::env::consts` uses), probing
/// each found tool with `probe`.
pub fn resolve_with(
    env: &dyn Fn(&str) -> Option<OsString>,
    exe: &Path,
    os: &str,
    arch: &str,
    probe: &dyn Fn(&ToolCommand, &str) -> Result<String, String>,
) -> Tools {
    let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let search = Search {
        env,
        dev_v3: dev_v3_dir(&exe_dir),
        dev_target: dev_target(os, arch),
        path_dirs: env("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<PathBuf>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect(),
        exe_dir,
        os,
    };
    Tools {
        ffmpeg: resolve_tool(
            FFMPEG,
            &search.ffmpeg_family_candidates(FFMPEG),
            probe,
            "-version",
            false,
        ),
        ffprobe: resolve_tool(
            FFPROBE,
            &search.ffmpeg_family_candidates(FFPROBE),
            probe,
            "-version",
            false,
        ),
        exiftool: resolve_tool(EXIFTOOL, &search.exiftool_candidates(), probe, "-ver", true),
    }
}

/// The `v3` directory of the development tree: the nearest `v3` directory among the executable's
/// ancestors that holds a `VERSION` file (the `v3/VERSION` of §6.15). Its `ffmpeg/dist` and
/// `exiftool/dist` directories are searched.
fn dev_v3_dir(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir
        .ancestors()
        .map(|dir| dir.join("v3"))
        .find(|v3| v3.join("VERSION").is_file())
}

/// The `v3/ffmpeg/dist` directory name for a platform, from `std::env::consts` names.
pub fn dev_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", _) => Some("macos-universal"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

/// The Perl interpreter for a Unix exiftool script: `/usr/bin/perl` when it exists, else `perl`
/// from `PATH`.
pub fn perl_program(exists: impl Fn(&Path) -> bool) -> OsString {
    let usr_bin = Path::new("/usr/bin/perl");
    if exists(usr_bin) {
        usr_bin.as_os_str().to_owned()
    } else {
        OsString::from("perl")
    }
}

/// Where each tool may be, for one resolution.
struct Search<'a> {
    env: &'a dyn Fn(&str) -> Option<OsString>,
    exe_dir: PathBuf,
    dev_v3: Option<PathBuf>,
    dev_target: Option<&'static str>,
    path_dirs: Vec<PathBuf>,
    os: &'a str,
}

impl Search<'_> {
    /// `name` with the executable suffix of the platform.
    fn file_name(&self, base: &str) -> String {
        if self.os == "windows" {
            format!("{base}.exe")
        } else {
            base.to_string()
        }
    }

    /// The directory an environment variable names, when it is set and not empty.
    fn env_dir(&self, key: &str) -> Option<PathBuf> {
        (self.env)(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    /// Candidates for ffmpeg or ffprobe, in search order.
    fn ffmpeg_family_candidates(&self, base: &str) -> Vec<PathBuf> {
        let file = self.file_name(base);
        let mut out = Vec::new();
        if let Some(dir) = self.env_dir("TRD_FFMPEG_DIR") {
            out.push(dir.join(&file));
        }
        out.push(self.exe_dir.join(&file));
        if let (Some(v3), Some(target)) = (&self.dev_v3, self.dev_target) {
            out.push(v3.join("ffmpeg").join("dist").join(target).join(&file));
        }
        out.extend(self.path_dirs.iter().map(|dir| dir.join(&file)));
        out
    }

    /// Candidates for exiftool, in search order: the bundle layouts of §6.15 by platform.
    fn exiftool_candidates(&self) -> Vec<PathBuf> {
        let file = self.file_name(EXIFTOOL);
        let mut out = Vec::new();
        if let Some(dir) = self.env_dir("TRD_EXIFTOOL_DIR") {
            out.push(dir.join(&file));
        }
        out.push(match self.os {
            "macos" => self
                .exe_dir
                .join("..")
                .join("Resources")
                .join("exiftool")
                .join("exiftool"),
            _ => self.exe_dir.join("exiftool").join(&file),
        });
        if let Some(v3) = &self.dev_v3 {
            let dist = if self.os == "windows" {
                "windows"
            } else {
                "unix"
            };
            out.push(v3.join("exiftool").join("dist").join(dist).join(&file));
        }
        out.extend(self.path_dirs.iter().map(|dir| dir.join(&file)));
        out
    }
}

/// Picks the first existing candidate and probes it. `wrap_scripts` lets a `#!` file run through
/// Perl, which is how the Unix exiftool runs.
fn resolve_tool(
    name: &'static str,
    candidates: &[PathBuf],
    probe: &dyn Fn(&ToolCommand, &str) -> Result<String, String>,
    flag: &str,
    wrap_scripts: bool,
) -> ResolvedTool {
    let Some(found) = candidates.iter().find(|path| path.is_file()) else {
        return ResolvedTool {
            info: ToolInfo {
                name,
                path: None,
                version: String::new(),
                available: false,
                problem: Some(ToolProblem::Missing {
                    tried: candidates.to_vec(),
                }),
            },
            command: None,
        };
    };
    let command = if wrap_scripts && starts_with_shebang(found) {
        ToolCommand {
            program: perl_program(|path| path.is_file()),
            leading_args: vec![found.as_os_str().to_owned()],
        }
    } else {
        ToolCommand::direct(found)
    };
    let detail = match probe(&command, flag) {
        Ok(version) if !version.trim().is_empty() => {
            return ResolvedTool {
                info: ToolInfo {
                    name,
                    path: Some(found.clone()),
                    version: version.trim().to_string(),
                    available: true,
                    problem: None,
                },
                command: Some(command),
            };
        }
        Ok(_) => format!("{flag} printed nothing"),
        Err(detail) => detail,
    };
    ResolvedTool {
        info: ToolInfo {
            name,
            path: Some(found.clone()),
            version: String::new(),
            available: false,
            problem: Some(ToolProblem::Broken {
                path: found.clone(),
                detail,
            }),
        },
        command: None,
    }
}

/// True when the file begins with `#!`, a script rather than a binary.
fn starts_with_shebang(path: &Path) -> bool {
    let mut head = [0u8; 2];
    File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .map(|read| read == 2 && head == *b"#!")
        .unwrap_or(false)
}

/// The real version probe: runs the command with `flag` and waits at most `PROBE_TIMEOUT`.
fn run_version_command(command: &ToolCommand, flag: &str) -> Result<String, String> {
    let mut child = command
        .command()
        .arg(flag)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot start: {err}"))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{flag} did not finish within {} s",
                        PROBE_TIMEOUT.as_secs()
                    ));
                }
                thread::sleep(PROBE_POLL);
            }
            Err(err) => return Err(format!("cannot wait: {err}")),
        }
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("{flag} exited with {status}")
        } else {
            format!("{flag} exited with {status}: {stderr}")
        });
    }
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{flag} printed nothing"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::ffi::OsStr;

    /// A fixed environment for one resolution.
    #[derive(Default)]
    struct Env(Vec<(String, OsString)>);

    impl Env {
        fn with(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
            self.0.push((key.to_string(), value.as_ref().to_owned()));
            self
        }

        fn get(&self, key: &str) -> Option<OsString> {
            self.0
                .iter()
                .find(|(name, _)| name.as_str() == key)
                .map(|(_, value)| value.clone())
        }
    }

    /// A probe that answers every tool with a fixed version.
    fn ok_probe(_: &ToolCommand, flag: &str) -> Result<String, String> {
        Ok(format!("fake {flag}"))
    }

    /// A temporary tree: `bin/trd` is the executable, `env/` the environment directory, `v3/` the
    /// development tree (its `VERSION` file is what marks it), `path1/` and `path2/` the `PATH`
    /// directories.
    struct Tree {
        root: tempfile::TempDir,
    }

    impl Tree {
        fn new() -> Self {
            Tree {
                root: tempfile::tempdir().expect("a temporary directory"),
            }
        }

        fn root(&self) -> &Path {
            self.root.path()
        }

        fn exe(&self) -> PathBuf {
            self.root().join("bin").join("trd")
        }

        fn beside(&self) -> PathBuf {
            self.root().join("bin")
        }

        fn env_dir(&self) -> PathBuf {
            self.root().join("env")
        }

        fn dev_v3(&self) -> PathBuf {
            self.root().join("v3")
        }

        fn path_dirs(&self) -> Vec<PathBuf> {
            vec![self.root().join("path1"), self.root().join("path2")]
        }

        fn path_var(&self) -> OsString {
            std::env::join_paths(self.path_dirs()).expect("joinable paths")
        }
    }

    /// Creates a file (and its directories) holding bytes that are not a script.
    fn write(path: &Path) {
        write_bytes(path, b"binary");
    }

    fn write_bytes(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a directory");
        }
        std::fs::write(path, bytes).expect("a file");
    }

    fn resolve(
        tree: &Tree,
        env: &Env,
        os: &str,
        arch: &str,
        probe: &dyn Fn(&ToolCommand, &str) -> Result<String, String>,
    ) -> Tools {
        resolve_with(&|key: &str| env.get(key), &tree.exe(), os, arch, probe)
    }

    #[test]
    fn order_follows_the_four_steps() {
        let tree = Tree::new();
        let env_file = tree.env_dir().join("ffmpeg");
        let beside = tree.beside().join("ffmpeg");
        let dev = tree.dev_v3().join("ffmpeg/dist/linux-x64/ffmpeg");
        let path_file = tree.path_dirs()[0].join("ffmpeg");
        for file in [&env_file, &beside, &path_file] {
            write(file);
        }
        write(&tree.dev_v3().join("VERSION"));
        write(&dev);
        let with_env = Env::default()
            .with("TRD_FFMPEG_DIR", tree.env_dir())
            .with("PATH", tree.path_var());
        let found = |env: &Env| {
            resolve(&tree, env, "linux", "x86_64", &ok_probe)
                .ffmpeg
                .info
                .path
        };

        // Each step wins once the steps before it are gone.
        assert_eq!(found(&with_env), Some(env_file.clone()));
        std::fs::remove_file(&env_file).unwrap();
        assert_eq!(found(&with_env), Some(beside.clone()));
        std::fs::remove_file(&beside).unwrap();
        assert_eq!(found(&with_env), Some(dev.clone()));
        std::fs::remove_file(&dev).unwrap();
        assert_eq!(found(&with_env), Some(path_file.clone()));
    }

    #[test]
    fn missing_env_dir_falls_through() {
        let tree = Tree::new();
        let beside = tree.beside().join("ffmpeg");
        write(&beside);
        // A directory that does not exist, and an empty value (which counts as unset).
        for value in [
            tree.root().join("nowhere").into_os_string(),
            OsString::new(),
        ] {
            let env = Env::default().with("TRD_FFMPEG_DIR", value);
            let tools = resolve(&tree, &env, "linux", "x86_64", &ok_probe);
            assert_eq!(tools.ffmpeg.info.path, Some(beside.clone()));
            assert!(tools.ffmpeg.info.available);
        }
    }

    #[test]
    fn exiftool_follows_the_same_order() {
        let tree = Tree::new();
        let env_file = tree.env_dir().join("exiftool");
        let beside = tree.beside().join("exiftool").join("exiftool");
        let dev = tree.dev_v3().join("exiftool/dist/unix/exiftool");
        let path_file = tree.path_dirs()[1].join("exiftool");
        for file in [&env_file, &beside, &path_file, &dev] {
            write(file);
        }
        write(&tree.dev_v3().join("VERSION"));
        let env = Env::default()
            .with("TRD_EXIFTOOL_DIR", tree.env_dir())
            .with("PATH", tree.path_var());
        let found = |env: &Env| {
            resolve(&tree, env, "linux", "x86_64", &ok_probe)
                .exiftool
                .info
                .path
        };
        assert_eq!(found(&env), Some(env_file.clone()));
        std::fs::remove_file(&env_file).unwrap();
        assert_eq!(found(&env), Some(beside.clone()));
        std::fs::remove_file(&beside).unwrap();
        assert_eq!(found(&env), Some(dev.clone()));
        std::fs::remove_file(&dev).unwrap();
        assert_eq!(found(&env), Some(path_file.clone()));
    }

    #[test]
    fn exiftool_bundle_layouts() {
        // Where exiftool sits beside the executable on each platform, relative to its directory.
        let cases: [(&str, &[&str]); 3] = [
            ("macos", &["..", "Resources", "exiftool", "exiftool"]),
            ("linux", &["exiftool", "exiftool"]),
            ("windows", &["exiftool", "exiftool.exe"]),
        ];
        for (os, parts) in cases {
            let tree = Tree::new();
            let mut file = tree.beside();
            for part in parts {
                file.push(part);
            }
            write(&file);
            let tools = resolve(&tree, &Env::default(), os, "x86_64", &ok_probe);
            assert_eq!(
                tools.exiftool.info.path.as_deref(),
                Some(file.as_path()),
                "{os}"
            );
        }
    }

    #[test]
    fn windows_names_use_the_exe_suffix() {
        let tree = Tree::new();
        let ffmpeg = tree.beside().join("ffmpeg.exe");
        let ffprobe = tree.beside().join("ffprobe.exe");
        write(&ffmpeg);
        write(&ffprobe);
        let tools = resolve(&tree, &Env::default(), "windows", "x86_64", &ok_probe);
        assert_eq!(tools.ffmpeg.info.path, Some(ffmpeg));
        assert_eq!(tools.ffprobe.info.path, Some(ffprobe));
    }

    #[test]
    fn dev_tree_needs_the_version_file() {
        let tree = Tree::new();
        write(&tree.dev_v3().join("ffmpeg/dist/linux-x64/ffmpeg"));
        let tools = resolve(&tree, &Env::default(), "linux", "x86_64", &ok_probe);
        assert!(!tools.ffmpeg.info.available);
        assert_eq!(
            tools.ffmpeg.info.problem,
            Some(ToolProblem::Missing {
                tried: vec![tree.beside().join("ffmpeg")],
            })
        );
    }

    #[test]
    fn dev_target_maps_os_and_arch() {
        let cases = [
            ("macos", "aarch64", Some("macos-universal")),
            ("macos", "x86_64", Some("macos-universal")),
            ("linux", "x86_64", Some("linux-x64")),
            ("linux", "aarch64", Some("linux-arm64")),
            ("windows", "x86_64", Some("windows-x64")),
            ("linux", "riscv64", None),
            ("windows", "aarch64", None),
            ("freebsd", "x86_64", None),
        ];
        for (os, arch, target) in cases {
            assert_eq!(dev_target(os, arch), target, "{os}/{arch}");
        }
    }

    #[test]
    fn perl_program_prefers_usr_bin_perl() {
        assert_eq!(perl_program(|_| true), OsString::from("/usr/bin/perl"));
        assert_eq!(perl_program(|_| false), OsString::from("perl"));
    }

    #[test]
    fn exiftool_script_runs_through_perl() {
        let tree = Tree::new();
        let script = tree.beside().join("exiftool").join("exiftool");
        write_bytes(&script, b"#!/usr/bin/env perl\nprint \"13.00\\n\";\n");
        let seen = RefCell::new(Vec::new());
        let probe = |command: &ToolCommand, flag: &str| {
            seen.borrow_mut().push((command.clone(), flag.to_string()));
            Ok("13.00".to_string())
        };
        let tools = resolve(&tree, &Env::default(), "linux", "x86_64", &probe);
        let expected = ToolCommand {
            program: perl_program(|path| path.is_file()),
            leading_args: vec![script.as_os_str().to_owned()],
        };
        assert_eq!(tools.exiftool.command, Some(expected.clone()));
        assert_eq!(tools.exiftool.info.version, "13.00");
        assert_eq!(*seen.borrow(), vec![(expected, "-ver".to_string())]);
    }

    #[test]
    fn exiftool_binary_runs_directly() {
        let tree = Tree::new();
        let binary = tree.beside().join("exiftool").join("exiftool");
        write_bytes(&binary, b"\x7fELF binary");
        let tools = resolve(&tree, &Env::default(), "linux", "x86_64", &ok_probe);
        assert_eq!(tools.exiftool.command, Some(ToolCommand::direct(&binary)));
    }

    #[test]
    fn infos_lists_the_three_tools_in_order() {
        let tree = Tree::new();
        for name in ["ffmpeg", "ffprobe", "exiftool"] {
            write(&tree.beside().join(name));
        }
        let tools = resolve(&tree, &Env::default(), "linux", "x86_64", &ok_probe);

        let infos = tools.infos();
        let names: Vec<&str> = infos.iter().map(|info| info.name).collect();
        assert_eq!(names, ["ffmpeg", "ffprobe", "exiftool"]);
        assert_eq!(infos[0], tools.ffmpeg.info);
        assert_eq!(infos[1], tools.ffprobe.info);
        assert_eq!(infos[2], tools.exiftool.info);
        for info in &infos {
            assert!(info.available, "{}: {:?}", info.name, info.problem);
            assert_eq!(info.problem, None);
            assert_eq!(info.path, Some(tree.beside().join(info.name)));
        }
        assert_eq!(infos[0].version, "fake -version");
        assert_eq!(infos[1].version, "fake -version");
        assert_eq!(infos[2].version, "fake -ver");
    }

    #[test]
    fn missing_tool_problem() {
        let tree = Tree::new();
        write(&tree.dev_v3().join("VERSION"));
        let env = Env::default()
            .with("TRD_FFMPEG_DIR", tree.env_dir())
            .with("TRD_EXIFTOOL_DIR", tree.env_dir())
            .with("PATH", tree.path_var());
        let tools = resolve(&tree, &env, "linux", "x86_64", &ok_probe);

        let ffmpeg = &tools.ffmpeg;
        assert!(!ffmpeg.info.available);
        assert_eq!(ffmpeg.info.path, None);
        assert_eq!(ffmpeg.command, None);
        let problem = ffmpeg.info.problem.as_ref().expect("a problem");
        assert_eq!(problem.id(), PROBLEM_TOOL_MISSING);
        assert_eq!(
            problem,
            &ToolProblem::Missing {
                tried: vec![
                    tree.env_dir().join("ffmpeg"),
                    tree.beside().join("ffmpeg"),
                    tree.dev_v3().join("ffmpeg/dist/linux-x64/ffmpeg"),
                    tree.path_dirs()[0].join("ffmpeg"),
                    tree.path_dirs()[1].join("ffmpeg"),
                ],
            }
        );
        assert_eq!(
            tools.exiftool.info.problem,
            Some(ToolProblem::Missing {
                tried: vec![
                    tree.env_dir().join("exiftool"),
                    tree.beside().join("exiftool").join("exiftool"),
                    tree.dev_v3().join("exiftool/dist/unix/exiftool"),
                    tree.path_dirs()[0].join("exiftool"),
                    tree.path_dirs()[1].join("exiftool"),
                ],
            })
        );
        assert_eq!(
            tools.ffprobe.info.problem.as_ref().map(ToolProblem::id),
            Some(PROBLEM_TOOL_MISSING)
        );
    }

    #[test]
    fn broken_tool_problem() {
        let tree = Tree::new();
        for file in [
            tree.beside().join("ffmpeg"),
            tree.beside().join("ffprobe"),
            tree.beside().join("exiftool").join("exiftool"),
        ] {
            write(&file);
        }
        let probe = |command: &ToolCommand, _: &str| {
            let program = command.program.to_string_lossy().into_owned();
            if program.ends_with("ffprobe") {
                Err("-version exited with exit status: 1".to_string())
            } else if program.ends_with("exiftool") {
                Ok("   ".to_string())
            } else {
                Ok("ffmpeg version fake".to_string())
            }
        };
        let tools = resolve(&tree, &Env::default(), "linux", "x86_64", &probe);

        assert!(tools.ffmpeg.info.available);
        assert_eq!(tools.ffmpeg.info.version, "ffmpeg version fake");

        let ffprobe = &tools.ffprobe;
        assert!(!ffprobe.info.available);
        assert_eq!(ffprobe.command, None);
        assert_eq!(ffprobe.info.path, Some(tree.beside().join("ffprobe")));
        let problem = ffprobe.info.problem.as_ref().expect("a problem");
        assert_eq!(problem.id(), PROBLEM_TOOL_BROKEN);
        assert_eq!(
            problem,
            &ToolProblem::Broken {
                path: tree.beside().join("ffprobe"),
                detail: "-version exited with exit status: 1".to_string(),
            }
        );

        let exiftool = &tools.exiftool;
        assert!(!exiftool.info.available);
        assert_eq!(
            exiftool.info.problem.as_ref().map(ToolProblem::id),
            Some(PROBLEM_TOOL_BROKEN)
        );
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        write_bytes(path, format!("#!/bin/sh\n{body}\n").as_bytes());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_probe_reads_the_first_line() {
        let tree = Tree::new();
        let script = tree.root().join("fake-tool");
        write_script(&script, "echo ''\necho 'ffmpeg version 7.1'\necho 'more'");
        let command = ToolCommand::direct(&script);
        assert_eq!(
            run_version_command(&command, "-version"),
            Ok("ffmpeg version 7.1".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_probe_reports_a_failed_command() {
        let tree = Tree::new();
        let script = tree.root().join("fake-tool");
        write_script(&script, "echo 'no such codec' >&2\nexit 3");
        let command = ToolCommand::direct(&script);
        let err = run_version_command(&command, "-version").unwrap_err();
        assert!(err.contains("-version exited with"), "{err}");
        assert!(err.contains("no such codec"), "{err}");
    }

    #[test]
    fn real_probe_reports_a_missing_program() {
        let tree = Tree::new();
        let command = ToolCommand::direct(&tree.root().join("does-not-exist"));
        let err = run_version_command(&command, "-version").unwrap_err();
        assert!(err.starts_with("cannot start"), "{err}");
    }
}
