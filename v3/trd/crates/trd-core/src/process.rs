// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Process table and ffmpeg process control (§6.6, §6.8).
//!
//! [`spawn_detached`] starts a process with its output in a log file, in a process group of its own,
//! and records its pid and start time so the daemon can find it again after a restart. [`suspend`],
//! [`resume`] and [`terminate`] act on a pid. [`terminate`] takes the process handle so that a child
//! reports its exit status. [`ProcessTable`] lists processes for re-adoption (§6.11): the real table
//! is [`SysinfoProcessTable`]; tests use [`FakeProcessTable`].

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid as UnixPid;

#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;

/// How long [`terminate`] waits after SIGTERM before SIGKILL (§6.8).
pub const TERMINATE_GRACE: Duration = Duration::from_secs(10);

/// How often a wait looks at the process again.
const POLL: Duration = Duration::from_millis(20);

/// A hang guard for the wait after SIGKILL: SIGKILL ends a process at once, so this only stops a
/// wait that would never end.
const KILL_WAIT: Duration = Duration::from_secs(30);

/// Why a process could not be started or its log opened.
#[derive(Debug)]
pub enum ProcessError {
    /// The program could not be started: a missing binary, a permission error.
    LaunchFailed { program: String, source: io::Error },
    /// The log file could not be created or opened for appending.
    Log { path: PathBuf, source: io::Error },
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessError::LaunchFailed { program, source } => {
                write!(f, "cannot launch {program}: {source}")
            }
            ProcessError::Log { path, source } => {
                write!(f, "cannot open the log {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProcessError::LaunchFailed { source, .. } | ProcessError::Log { source, .. } => {
                Some(source)
            }
        }
    }
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// The exit code; `None` when the process ended by a signal.
    pub code: Option<i32>,
    /// The signal that ended the process (unix only).
    pub by_signal: Option<i32>,
}

/// Whether a process is running, stopped (suspended) or gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Stopped,
    Gone,
}

/// A process the daemon knows by pid and start time. A child started by [`spawn_detached`] keeps its
/// handle, so [`RunningProcess::wait`] reports its exit status; an adopted process has no handle and
/// is only checked with [`is_alive`].
#[derive(Debug)]
pub struct RunningProcess {
    pub pid: u32,
    /// Seconds since the Unix epoch, as `sysinfo` reports it; 0 when the process had already gone.
    pub start_time: u64,
    child: Option<Child>,
}

impl RunningProcess {
    /// A process this daemon did not start: no exit handle.
    pub fn adopted(pid: u32, start_time: u64) -> RunningProcess {
        RunningProcess {
            pid,
            start_time,
            child: None,
        }
    }

    /// Whether this process is a child of the daemon (so its exit status can be read).
    pub fn is_child(&self) -> bool {
        self.child.is_some()
    }

    /// The exit status if the child has ended, without waiting. An adopted process has no exit
    /// status to read, so this is an error for it.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitInfo>> {
        Ok(self.child_mut()?.try_wait()?.map(exit_info))
    }

    /// Waits for a child to end. An adopted process has no exit status to read: an error.
    pub fn wait(&mut self) -> io::Result<ExitInfo> {
        Ok(exit_info(self.child_mut()?.wait()?))
    }

    fn child_mut(&mut self) -> io::Result<&mut Child> {
        self.child.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "an adopted process has no exit handle",
            )
        })
    }
}

/// Starts `argv` detached: stdin is null, stdout and stderr are appended to `log_path`, and the
/// process has a process group of its own (§6.6).
pub fn spawn_detached(argv: &[OsString], log_path: &Path) -> Result<RunningProcess, ProcessError> {
    spawn_detached_with_env(argv, &[], log_path)
}

/// [`spawn_detached`], with `env` added to the environment the process inherits.
pub fn spawn_detached_with_env(
    argv: &[OsString],
    env: &[(&str, &str)],
    log_path: &Path,
) -> Result<RunningProcess, ProcessError> {
    let Some((program, args)) = argv.split_first() else {
        return Err(ProcessError::LaunchFailed {
            program: String::new(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "no program given"),
        });
    };
    let name = program.to_string_lossy().into_owned();
    let log = open_log(log_path)?;
    let child = match launch(program, args, env, &log, true) {
        Ok(child) => child,
        // On Windows a job may refuse to let the child break away: start it inside the job then.
        Err(err) if is_access_denied(&err) => {
            launch(program, args, env, &log, false).map_err(|source| {
                ProcessError::LaunchFailed {
                    program: name.clone(),
                    source,
                }
            })?
        }
        Err(source) => {
            return Err(ProcessError::LaunchFailed {
                program: name,
                source,
            });
        }
    };
    let pid = child.id();
    let start_time = sample_pid(pid).map(|(start, _)| start).unwrap_or(0);
    Ok(RunningProcess {
        pid,
        start_time,
        child: Some(child),
    })
}

fn open_log(path: &Path) -> Result<File, ProcessError> {
    let wrap = |source: io::Error| ProcessError::Log {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(wrap)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(wrap)
}

fn launch(
    program: &OsString,
    args: &[OsString],
    env: &[(&str, &str)],
    log: &File,
    breakaway: bool,
) -> io::Result<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log.try_clone()?));
    for (key, value) in env {
        command.env(key, value);
    }
    detach(&mut command, breakaway);
    command.spawn()
}

#[cfg(unix)]
fn detach(command: &mut Command, _breakaway: bool) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command, breakaway: bool) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let mut flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    if breakaway {
        flags |= CREATE_BREAKAWAY_FROM_JOB;
    }
    command.creation_flags(flags);
}

#[cfg(windows)]
fn is_access_denied(err: &io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    err.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

#[cfg(not(windows))]
fn is_access_denied(_err: &io::Error) -> bool {
    false
}

fn exit_info(status: ExitStatus) -> ExitInfo {
    ExitInfo {
        code: status.code(),
        by_signal: signal_of(&status),
    }
}

#[cfg(unix)]
fn signal_of(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal_of(_status: &ExitStatus) -> Option<i32> {
    None
}

/// The start time and status of one pid, looked up by itself: a fresh `System` that refreshes only
/// that pid never lists other processes (§6.11).
fn sample_pid(pid: u32) -> Option<(u64, ProcessStatus)> {
    let mut system = System::new();
    let target = Pid::from_u32(pid);
    let _ = system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(target)
        .map(|process| (process.start_time(), process.status()))
}

/// Whether `pid` is a live process started at `start_time`. A zombie is not alive. A recycled pid
/// has another start time, so it is not alive either.
pub fn is_alive(pid: u32, start_time: u64) -> bool {
    match sample_pid(pid) {
        Some((start, status)) => start == start_time && !is_zombie(&status),
        None => false,
    }
}

fn is_zombie(status: &ProcessStatus) -> bool {
    matches!(status, ProcessStatus::Zombie)
}

/// Whether the process `pid` is running, stopped (suspended) or gone.
pub fn process_state(pid: u32) -> ProcessState {
    match sample_pid(pid) {
        None => ProcessState::Gone,
        Some((_, status)) if is_zombie(&status) => ProcessState::Gone,
        Some((_, status)) => {
            if is_stopped(pid, &status) {
                ProcessState::Stopped
            } else {
                ProcessState::Running
            }
        }
    }
}

#[cfg(unix)]
fn is_stopped(_pid: u32, status: &ProcessStatus) -> bool {
    matches!(status, ProcessStatus::Stop)
}

#[cfg(windows)]
fn is_stopped(pid: u32, _status: &ProcessStatus) -> bool {
    all_threads_suspended(pid)
}

/// Suspends the process (SIGSTOP; Windows `NtSuspendProcess`).
pub fn suspend(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        signal_pid(pid, Signal::SIGSTOP)
    }
    #[cfg(windows)]
    {
        nt_process_op(pid, b"NtSuspendProcess\0", "NtSuspendProcess")
    }
}

/// Resumes a suspended process (SIGCONT; Windows `NtResumeProcess`).
pub fn resume(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        signal_pid(pid, Signal::SIGCONT)
    }
    #[cfg(windows)]
    {
        nt_process_op(pid, b"NtResumeProcess\0", "NtResumeProcess")
    }
}

/// Ends the process: a suspended process is resumed first, then SIGTERM (Windows `TerminateProcess`)
/// is sent, and after `grace` a process that has not ended gets SIGKILL (§6.8).
///
/// Returns the exit status of a child. An adopted process has no status to read, so `None` is
/// returned for it once it is gone.
pub fn terminate(process: &mut RunningProcess, grace: Duration) -> io::Result<Option<ExitInfo>> {
    if let Check::Ended(exit) = check(process)? {
        return Ok(exit);
    }
    let pid = process.pid;
    if process_state(pid) == ProcessState::Stopped {
        resume(pid)?;
    }
    send_terminate(pid)?;
    if let Check::Ended(exit) = wait_for_end(process, Instant::now() + grace)? {
        return Ok(exit);
    }
    send_kill(pid)?;
    match wait_for_end(process, Instant::now() + KILL_WAIT)? {
        Check::Ended(exit) => Ok(exit),
        Check::Alive => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("process {pid} did not end after SIGKILL"),
        )),
    }
}

enum Check {
    Alive,
    Ended(Option<ExitInfo>),
}

fn check(process: &mut RunningProcess) -> io::Result<Check> {
    match process.child.as_mut() {
        Some(child) => Ok(match child.try_wait()? {
            Some(status) => Check::Ended(Some(exit_info(status))),
            None => Check::Alive,
        }),
        None if is_alive(process.pid, process.start_time) => Ok(Check::Alive),
        None => Ok(Check::Ended(None)),
    }
}

fn wait_for_end(process: &mut RunningProcess, deadline: Instant) -> io::Result<Check> {
    loop {
        match check(process)? {
            Check::Alive if Instant::now() < deadline => thread::sleep(POLL),
            other => return Ok(other),
        }
    }
}

#[cfg(unix)]
fn signal_pid(pid: u32, signal: Signal) -> io::Result<()> {
    match signal::kill(UnixPid::from_raw(pid as i32), signal) {
        // A process that has already gone needs no signal.
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(err) => Err(io::Error::from(err)),
    }
}

#[cfg(unix)]
fn send_terminate(pid: u32) -> io::Result<()> {
    signal_pid(pid, Signal::SIGTERM)
}

#[cfg(unix)]
fn send_kill(pid: u32) -> io::Result<()> {
    signal_pid(pid, Signal::SIGKILL)
}

#[cfg(windows)]
fn send_terminate(pid: u32) -> io::Result<()> {
    terminate_windows(pid)
}

#[cfg(windows)]
fn send_kill(pid: u32) -> io::Result<()> {
    terminate_windows(pid)
}

#[cfg(windows)]
const PROCESS_TERMINATE: u32 = 0x0001;
#[cfg(windows)]
const PROCESS_SUSPEND_RESUME: u32 = 0x0800;
#[cfg(windows)]
const THREAD_QUERY_INFORMATION: u32 = 0x0040;
/// `ThreadSuspendCount` in `NtQueryInformationThread`.
#[cfg(windows)]
const THREAD_SUSPEND_COUNT_CLASS: u32 = 35;

#[cfg(windows)]
fn open_process(pid: u32, access: u32) -> io::Result<HANDLE> {
    use windows_sys::Win32::System::Threading::OpenProcess;
    // SAFETY: OpenProcess has no pointer arguments; the handle is checked below.
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn terminate_windows(pid: u32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::TerminateProcess;
    let handle = match open_process(pid, PROCESS_TERMINATE) {
        Ok(handle) => handle,
        Err(_) if process_state(pid) == ProcessState::Gone => return Ok(()),
        Err(err) => return Err(err),
    };
    // SAFETY: `handle` is a process handle opened above and closed below.
    let ok = unsafe { TerminateProcess(handle, 1) };
    // SAFETY: closing the handle opened above.
    unsafe { CloseHandle(handle) };
    if ok != 0 || process_state(pid) == ProcessState::Gone {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The address of an export of ntdll.dll, looked up with `GetProcAddress`. `name` ends in NUL.
#[cfg(windows)]
fn ntdll_export(name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    let module_name: Vec<u16> = "ntdll.dll"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the module name is NUL-terminated and `name` is a NUL-terminated ASCII name.
    unsafe {
        let module = GetModuleHandleW(module_name.as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, name.as_ptr())
    }
}

#[cfg(windows)]
fn nt_process_op(pid: u32, export: &[u8], name: &str) -> io::Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    type NtProcessFn = unsafe extern "system" fn(HANDLE) -> i32;
    let Some(raw) = ntdll_export(export) else {
        return Err(io::Error::other(format!("{name} is not exported by ntdll")));
    };
    // SAFETY: NtSuspendProcess and NtResumeProcess take one HANDLE and return an NTSTATUS.
    let call =
        unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, NtProcessFn>(raw) };
    let handle = open_process(pid, PROCESS_SUSPEND_RESUME)?;
    // SAFETY: `handle` is a process handle opened above.
    let status = unsafe { call(handle) };
    // SAFETY: closing the handle opened above.
    unsafe { CloseHandle(handle) };
    if status >= 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{name} failed with status {status:#x}"
        )))
    }
}

/// Whether every thread of `pid` has a suspend count above zero (and there is at least one).
#[cfg(windows)]
fn all_threads_suspended(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::OpenThread;

    // SAFETY: the snapshot handle is checked, then closed; each THREADENTRY32 is zeroed with its
    // size set as the API requires; every opened thread handle is closed.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut threads = 0usize;
        let mut all_suspended = true;
        let mut more = Thread32First(snapshot, &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                threads += 1;
                let thread = OpenThread(THREAD_QUERY_INFORMATION, 0, entry.th32ThreadID);
                let suspended =
                    !thread.is_null() && thread_suspend_count(thread).is_some_and(|c| c > 0);
                if !thread.is_null() {
                    CloseHandle(thread);
                }
                if !suspended {
                    all_suspended = false;
                    break;
                }
            }
            more = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        threads > 0 && all_suspended
    }
}

/// The suspend count of an open thread handle, from `NtQueryInformationThread`.
#[cfg(windows)]
fn thread_suspend_count(thread: HANDLE) -> Option<u32> {
    type NtQueryThreadFn = unsafe extern "system" fn(HANDLE, u32, *mut u32, u32, *mut u32) -> i32;
    let raw = ntdll_export(b"NtQueryInformationThread\0")?;
    // SAFETY: the export has this signature (ThreadHandle, ThreadInformationClass,
    // ThreadInformation, ThreadInformationLength, ReturnLength). The output is one u32.
    unsafe {
        let call =
            std::mem::transmute::<unsafe extern "system" fn() -> isize, NtQueryThreadFn>(raw);
        let mut count: u32 = 0;
        let status = call(
            thread,
            THREAD_SUSPEND_COUNT_CLASS,
            &mut count,
            std::mem::size_of::<u32>() as u32,
            std::ptr::null_mut(),
        );
        (status >= 0).then_some(count)
    }
}

/// One process as a [`ProcessTable`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    /// The executable's name.
    pub name: String,
    /// The command line, after the program name.
    pub args: Vec<String>,
    /// Seconds since the Unix epoch.
    pub start_time: u64,
}

/// A list of processes, for re-adopting renders (§6.11).
pub trait ProcessTable {
    fn list(&mut self) -> Vec<ProcessEntry>;
}

/// The operating system's processes, through `sysinfo`.
pub struct SysinfoProcessTable {
    system: System,
}

// `new` takes no arguments but asserts when built for tests, so `Default` would be wrong.
#[allow(clippy::new_without_default)]
impl SysinfoProcessTable {
    // The assertion is constant per build on purpose (spec §6.8): it must panic at run time inside
    // unit tests (`sysinfo_table_refuses_in_unit_tests` is `#[should_panic]`); a `const` block
    // would instead stop the test build from compiling. Clippy's assertions_on_constants lint is
    // allowed here for that reason.
    #[allow(clippy::assertions_on_constants)]
    pub fn new() -> SysinfoProcessTable {
        assert!(
            !cfg!(test),
            "SysinfoProcessTable lists the owner's real processes; tests use FakeProcessTable"
        );
        SysinfoProcessTable {
            system: System::new(),
        }
    }
}

impl ProcessTable for SysinfoProcessTable {
    fn list(&mut self) -> Vec<ProcessEntry> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );
        self.system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessEntry {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                args: process
                    .cmd()
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                start_time: process.start_time(),
            })
            .collect()
    }
}

/// A process table that lists the entries it was given.
pub struct FakeProcessTable {
    entries: Vec<ProcessEntry>,
}

impl FakeProcessTable {
    pub fn new(entries: Vec<ProcessEntry>) -> FakeProcessTable {
        FakeProcessTable { entries }
    }
}

impl ProcessTable for FakeProcessTable {
    fn list(&mut self) -> Vec<ProcessEntry> {
        self.entries.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            name: name.to_owned(),
            args: vec!["-i".to_owned(), "in%04d.png".to_owned()],
            start_time: 1_700_000_000 + u64::from(pid),
        }
    }

    #[test]
    fn fake_table_lists_given_processes() {
        let given = vec![entry(10, "ffmpeg"), entry(11, "exiftool")];
        let mut table = FakeProcessTable::new(given.clone());
        assert_eq!(table.list(), given);
        assert_eq!(table.list(), given, "listing does not consume the entries");
    }

    #[test]
    #[should_panic]
    fn sysinfo_table_refuses_in_unit_tests() {
        let _ = SysinfoProcessTable::new();
    }
}
