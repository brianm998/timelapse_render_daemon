# 1011: process runner in trd-core

## Decisions

- `spawn_detached` runs the child with stdin null, appends stdout and stderr to the log file, and
  gives it its own process group (`process_group(0)` on unix). On Windows it uses
  `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB` and retries without the
  breakaway flag when the start fails with access denied.
- A child keeps its `Child` handle, so `wait` and `try_wait` report its exit status. An adopted
  process has no handle; `is_alive(pid, start_time)` checks it, and `terminate` reports no status
  for it.
- `terminate` resumes a suspended process, sends SIGTERM (Windows `TerminateProcess`), waits the
  grace period (10 s), then sends SIGKILL.
- Suspend and resume use SIGSTOP and SIGCONT on unix, and `NtSuspendProcess` and
  `NtResumeProcess` looked up on ntdll with `GetProcAddress` on Windows.
- `process_state` reports Stopped from the process status on unix. Windows has no such status in
  `sysinfo`, so a process counts as stopped when every one of its threads has a suspend count above
  zero (`NtQueryInformationThread`, class `ThreadSuspendCount`).
- Process listing and the start time of a pid go through `sysinfo`, always for one pid at a time
  (`ProcessesToUpdate::Some`), except the table, which lists everything.
- `SysinfoProcessTable::new` asserts `!cfg!(test)`, so no unit test can list the owner's processes
  (overview §10 rule 2). `FakeProcessTable` is the test table.
- `ProgressReader` reads only the bytes appended since its last poll, keeps an incomplete last line
  for the next poll, and reports the latest complete block. `eta_seconds` is -1 when fps is 0 or
  less.
- `ProcessError` is a hand-written enum (`LaunchFailed`, `Log`) with `Display` and `Error`, not a
  `thiserror` derive, so `trd-core` takes no new crate for it.

## Consequences

- `trd-core` depends on `sysinfo` and `nix` (unix) as pinned in the workspace, and on the
  `trd-testtools` path crate as a dev-dependency. The Windows features gained the ToolHelp
  (thread snapshot), JobObjects, LibraryLoader and Threading modules.
- Windows code is verified only by the three-OS `trd` CI job; the macOS worktree cannot build it.
