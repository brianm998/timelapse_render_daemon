# 1114 — Service install, uninstall, status and restart (TRD-21)

Status: accepted (TRD-21)

Decision: `install` on Windows also runs `schtasks /Run`, so all three OSes start the service when it is installed. The §10.4 table lists only `/Create`; the NSIS installer's own `/Run` then does nothing extra, since the task ignores a second start.

Decision: Windows `uninstall` ends the task (`/End`, failure only a warning) before `/Delete /F`, and then removes the task's XML file. Deleting a task does not stop a running trdw.

Decision: Linux `uninstall` runs `daemon-reload` after it removes the per-user unit, so systemd forgets the unit at once.

Decision: when the package's unit exists, Linux `install` writes no per-user unit but still runs `daemon-reload` and `enable --now`. §10.4 is read literally: the write is skipped, the enable still happens. macOS differs: when the package's agent exists, `install` does nothing, as §10.4 says. Uninstall never removes a package copy; it prints the command that would.

Decision: the package copies (`/Library/LaunchAgents/…plist`, `/usr/lib/systemd/user/trd.service`) are fields of `Target`, so tests point them under a temporary home and never read the real paths.

Decision: `status` prints a `status:` line (`running`, `stopped` or `not installed`) and then `pid:` or `last run:` when known. The live test polls that line. Windows has no pid in `schtasks` output, so the restart check uses the Last Run Time.

Decision: `--dry-run` runs the same code with a `RecordingRunner`. It prints each file's contents and each command, and changes nothing. Action summaries are not printed, and `status` says the state is not read, since a dry run has nothing real to read.

Decision: `ExecStart` words follow systemd's quoting: a plain word stays bare; anything else is double-quoted with `\`, `"`, `$` and `%` escaped.

Why: the spec's table is the contract, and the package copies must win where the spec says so. Tests must never touch the real service managers, so every command goes through `CommandRunner`, and `SystemRunner` panics under `cfg(test)`.
