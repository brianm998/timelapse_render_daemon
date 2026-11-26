// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Per-user autostart: `trd service install|uninstall|status|restart` (spec 1 §10.4).
//!
//! One module per OS implements [`Service`]. Each works through a [`Target`], which holds the home
//! directory, the user and the [`CommandRunner`] that talks to the service manager. `--dry-run`
//! builds the same `Target` with a [`RecordingRunner`] and `dry_run` set: every step is printed
//! and recorded, and nothing is written or run.

pub mod linux;
pub mod macos;
pub mod windows;

use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard};

use crate::cli::ServiceArgs;

/// The four `trd service` actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Install,
    Uninstall,
    Status,
    Restart,
}

/// The result of one command: whether it succeeded, and what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// A successful command that printed `stdout`.
    pub fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }
}

/// Runs a service-manager command (`launchctl`, `systemctl`, `schtasks`).
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput>;
}

/// Runs the real commands. Unit tests must never reach it: `run` panics under `cfg(test)`.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    #[allow(clippy::assertions_on_constants)]
    fn run(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        assert!(
            !cfg!(test),
            "SystemRunner runs real service-manager commands; tests use RecordingRunner"
        );
        let output = std::process::Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Records every command as one line (`program arg …`) and answers from a script. A command
/// without a scripted answer succeeds with no output. `--dry-run` uses it, so nothing real runs.
#[derive(Default)]
pub struct RecordingRunner {
    calls: Mutex<Vec<String>>,
    script: Mutex<VecDeque<CommandOutput>>,
}

impl RecordingRunner {
    pub fn new() -> RecordingRunner {
        RecordingRunner::default()
    }

    /// A runner whose next commands answer with `answers`, in order.
    #[cfg(test)]
    pub fn scripted(answers: Vec<CommandOutput>) -> RecordingRunner {
        RecordingRunner {
            calls: Mutex::new(Vec::new()),
            script: Mutex::new(VecDeque::from(answers)),
        }
    }

    /// The commands run so far, one line each.
    #[cfg(test)]
    pub fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        lock(&self.calls).push(command_line(program, args));
        Ok(lock(&self.script)
            .pop_front()
            .unwrap_or_else(|| CommandOutput::ok("")))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Everything a service action needs: where its files go, which user it is for, and how the
/// service manager is reached.
pub struct Target<'a> {
    /// The home directory the per-user files go under.
    pub home: PathBuf,
    /// The user id that `gui/<uid>` names (macOS).
    pub uid: u32,
    /// The account the task runs for: `DOMAIN\user` on Windows.
    pub user: String,
    /// The trd executable the service starts.
    pub binary: PathBuf,
    /// The config file the service starts with, as an absolute path.
    pub config: Option<PathBuf>,
    /// The agent the macOS .pkg installs (§10.1). When it exists, install does nothing.
    pub system_plist: PathBuf,
    /// The unit the Linux package installs (§10.2). When it exists, no per-user unit is written.
    pub system_unit: PathBuf,
    /// Print every step and change nothing.
    pub dry_run: bool,
    /// Runs the service-manager commands.
    pub runner: &'a dyn CommandRunner,
}

impl Target<'_> {
    /// Writes a file and creates its folder. Under dry-run it prints the file instead.
    pub fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if self.dry_run {
            println!("would write {} ({} bytes):", path.display(), bytes.len());
            if let Ok(text) = std::str::from_utf8(bytes) {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Removes a file, which may already be gone. Under dry-run it prints the removal instead.
    pub fn remove_file(&self, path: &Path) -> Result<(), String> {
        if self.dry_run {
            if path.exists() {
                println!("would remove {}", path.display());
            }
            return Ok(());
        }
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
        }
    }

    /// Runs one command and returns its output. Under dry-run it prints the command first.
    pub fn exec(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
        if self.dry_run {
            println!("would run: {}", command_line(program, args));
        }
        self.runner
            .run(program, args)
            .map_err(|e| format!("cannot run {program}: {e}"))
    }

    /// Prints the outcome of an action. A dry run only prints the steps, so it prints nothing here.
    pub fn report(&self, message: &str) {
        if !self.dry_run {
            println!("{message}");
        }
    }

    /// Runs one command that must succeed.
    pub fn exec_ok(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
        let output = self.exec(program, args)?;
        if output.success {
            return Ok(output);
        }
        Err(format!(
            "{} failed: {}",
            command_line(program, args),
            detail(&output)
        ))
    }

    /// Runs one command whose failure is only a warning, such as stopping a service that is not
    /// running.
    pub fn exec_warn(&self, program: &str, args: &[&str]) {
        match self.exec(program, args) {
            Ok(output) if output.success => {}
            Ok(output) => eprintln!(
                "warning: {} failed: {}",
                command_line(program, args),
                detail(&output)
            ),
            Err(message) => eprintln!("warning: {message}"),
        }
    }
}

fn command_line(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The stderr of a failed command, or its stdout when stderr is empty.
fn detail(output: &CommandOutput) -> String {
    let text = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if text.is_empty() {
        "no output".to_string()
    } else {
        text.to_string()
    }
}

/// What `status` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub last_run: Option<String>,
}

/// The four actions of one OS (§10.4).
pub trait Service {
    fn install(&self, target: &Target) -> Result<(), String>;
    fn uninstall(&self, target: &Target) -> Result<(), String>;
    fn status(&self, target: &Target) -> Result<(), String>;
    fn restart(&self, target: &Target) -> Result<(), String>;
}

/// Prints a [`State`] as the `status:` line and the fields that are known.
pub fn print_state(state: &State) {
    if !state.installed {
        println!("status: not installed");
        return;
    }
    let word = if state.running { "running" } else { "stopped" };
    println!("status: {word}");
    if let Some(pid) = state.pid {
        println!("pid: {pid}");
    }
    if let Some(last_run) = &state.last_run {
        println!("last run: {last_run}");
    }
}

/// Prints a state read from the service manager. A dry run reads nothing, so it says that instead.
pub fn show_status(t: &Target, state: &State) {
    if t.dry_run {
        println!("dry run: the state is not read");
    } else {
        print_state(state);
    }
}

/// The implementation for the OS this binary was built for.
pub fn for_current_os() -> &'static dyn Service {
    if cfg!(target_os = "macos") {
        &macos::LaunchAgent
    } else if cfg!(windows) {
        &windows::LogonTask
    } else {
        &linux::SystemdUser
    }
}

/// Escapes the text of an XML element: `&`, `<` and `>`.
pub fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Replaces each `@KEY@` in `template` by its value. A value is inserted as it is and is not
/// scanned again, so a path that contains `@X@` stays as written.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('@') {
        let after = &rest[start + 1..];
        let found: Option<(usize, &str)> = after.find('@').and_then(|end| {
            let key = &after[..end];
            values
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (end, *value))
        });
        out.push_str(&rest[..start]);
        match found {
            Some((end, value)) => {
                out.push_str(value);
                rest = &after[end + 1..];
            }
            None => {
                out.push('@');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Runs one `trd service` action and returns the process exit code.
pub fn run(verb: Verb, args: &ServiceArgs) -> ExitCode {
    match execute(verb, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trd service: {message}");
            ExitCode::from(1)
        }
    }
}

fn execute(verb: Verb, args: &ServiceArgs) -> Result<(), String> {
    let system = SystemRunner;
    let recording = RecordingRunner::new();
    let runner: &dyn CommandRunner = if args.dry_run { &recording } else { &system };
    let target = target(args, runner)?;
    if args.dry_run {
        println!("dry run: the steps are printed and nothing is changed");
    }
    let service = for_current_os();
    match verb {
        Verb::Install => service.install(&target),
        Verb::Uninstall => service.uninstall(&target),
        Verb::Status => service.status(&target),
        Verb::Restart => service.restart(&target),
    }
}

/// The target of this process: the real home, user and executable, with the given options.
fn target<'a>(args: &ServiceArgs, runner: &'a dyn CommandRunner) -> Result<Target<'a>, String> {
    let home = trd_core::dirs::Env::system().home;
    if home.as_os_str().is_empty() {
        return Err("no home directory: HOME and USERPROFILE are unset".to_string());
    }
    let binary = match args.binary.as_deref() {
        Some(path) => absolute(path)?,
        None => env::current_exe().map_err(|e| format!("cannot find the trd executable: {e}"))?,
    };
    let config = args.config.as_deref().map(absolute).transpose()?;
    Ok(Target {
        home,
        uid: current_uid(),
        user: user_name(),
        binary,
        config,
        system_plist: PathBuf::from(macos::PACKAGE_PLIST),
        system_unit: PathBuf::from(linux::PACKAGE_UNIT),
        dry_run: args.dry_run,
        runner,
    })
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))
}

/// The account name; on Windows it is prefixed by the user domain.
fn user_name() -> String {
    let user = env::var("USERNAME")
        .or_else(|_| env::var("USER"))
        .unwrap_or_default();
    match env::var("USERDOMAIN") {
        Ok(domain) if cfg!(windows) => format!("{domain}\\{user}"),
        _ => user,
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid takes no arguments, has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Shared fixtures for the unit tests of the three OS modules.
#[cfg(test)]
pub(crate) mod testing {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{CommandRunner, Target};

    /// A target under `home` with user id 501, no package copies, and the given runner. The
    /// package copies sit under `home` too, so no test reads the real `/Library` or `/usr`.
    pub fn target<'a>(home: &Path, runner: &'a dyn CommandRunner) -> Target<'a> {
        Target {
            home: home.to_path_buf(),
            uid: 501,
            user: "EXAMPLE\\example".to_string(),
            binary: PathBuf::from("/opt/trd/trd"),
            config: None,
            system_plist: home.join("package/com.timelapserender.trd.plist"),
            system_unit: home.join("package/trd.service"),
            dry_run: false,
            runner,
        }
    }

    /// Creates `path` (and its folder) with empty contents.
    pub fn put_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("a test folder");
        }
        fs::write(path, b"").expect("a test file");
    }

    /// Every file under `dir`, found recursively.
    pub fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{files_under, target};
    use super::*;

    #[test]
    #[should_panic(expected = "SystemRunner runs real service-manager commands")]
    fn system_runner_refuses_in_unit_tests() {
        let _ = SystemRunner.run("true", &[]);
    }

    #[test]
    fn dry_run_changes_nothing() {
        let home = tempfile::tempdir().expect("a temporary home");
        let recording = RecordingRunner::new();
        let mut t = target(home.path(), &recording);
        t.dry_run = true;
        let services: [&dyn Service; 3] = [
            &macos::LaunchAgent,
            &linux::SystemdUser,
            &windows::LogonTask,
        ];
        for service in services {
            service.install(&t).expect("dry-run install");
            service.status(&t).expect("dry-run status");
            service.restart(&t).expect("dry-run restart");
            service.uninstall(&t).expect("dry-run uninstall");
        }
        assert_eq!(
            files_under(home.path()),
            Vec::<PathBuf>::new(),
            "dry-run wrote files"
        );
        assert!(!recording.calls().is_empty(), "dry-run printed no commands");
    }

    #[test]
    fn fill_substitutes_each_key_once() {
        assert_eq!(
            fill("a @X@ b @Y@ @Z@", &[("X", "@Y@"), ("Y", "y")]),
            "a @Y@ b y @Z@"
        );
    }

    #[test]
    fn xml_text_escapes_markup_only() {
        assert_eq!(xml_text("a&b <c> \"d\""), "a&amp;b &lt;c&gt; \"d\"");
    }
}
