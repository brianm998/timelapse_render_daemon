# 1205: Windows uninstall retries the directory removal (TRD-1231)

Decisions taken for the windows-x64 failure in `test-installer.sh` on develop (run 45991373147,
commit ef871f8). Spec 1 §10.3 still holds; no spec change.

- **The cause is a hypothesis.** The log shows the scheduled task deleted (PASS) and the install
  directory still present 60 s after the silent uninstall (FAIL). The uninstall section calls
  `RMDir /r "$INSTDIR"` once, straight after `trd.exe service uninstall`. If a file in the directory is
  still open at that moment, NSIS leaves it and the directory, and nothing tries again. The likely
  holders are `trdw.exe`, which `schtasks /End` only asks to stop and which may not have exited when
  `service uninstall` returns, and a transient antivirus handle. Neither was observed on a Windows
  machine; makensis and Windows are not available locally.
- **The fix is a bounded retry in `trd.nsi`.** The uninstall section retries `RMDir /r "$INSTDIR"` up
  to 30 times, 1 s apart, and stops once `"$INSTDIR\*.*"` no longer exists. The `$INSTDIR == ""` guard
  and the `service uninstall` call stay as they were.
- **`/REBOOTOK` is not used.** It would mark the leftover files for deletion at reboot and report
  success, which hides the failure instead of removing the directory.
- **The rejected Rust-side wait.** Making `trd.exe service uninstall` wait for `trdw.exe` to exit was
  not done. It would change the schtasks calls that `service/windows.rs` unit tests record, and it
  does nothing for antivirus locks, which can hold a file after the process has exited.
- **`test-installer.sh` is unchanged.** Its 60 s wait and its PASS/FAIL checks stay as they are. The
  60 s window is a bound on the uninstall, not the cause.

Stage note: `docs/stages/s1215.md`.
