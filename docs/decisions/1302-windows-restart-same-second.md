# 1302 — The live restart check waits out the Windows last run time (TRD #1344)

Status: accepted (TRD #1344)

Decision: `service_install_status_restart_uninstall` sleeps two seconds (`LAST_RUN_STEP`) between reading the pre-restart status and calling `trd service restart`, on every OS. The restart check itself is unchanged: state `running` and a changed pid or last run time.

Evidence: the failed `Live tests (windows-2022)` run at commit 319c1c4 took 60.97 s in total, and the restart wait alone is `WAIT` = 60 s. So install, the first status wait, the first answer and the restart all fitted in about 0.97 s. The last status was `running`, with no pid and a last run time printed to the second (`11/28/2026 9:29:08 AM`).

Cause (inferred from that log and the code, not observed or reproduced on Windows): Windows `status` has no pid, so the test sees a restart only as a changed `Last Run Time`. When both `schtasks /Run` calls fall in the same second, the two values are equal and the poll times out.

Why the earlier fix (#1255, commit d9355fd) was not enough: it set `MultipleInstancesPolicy` to `StopExisting`, so `/Run` replaces an instance that is still running. That does nothing about a new run stamped with the same second as the old one. The setting stays.

Why the pause is in the test: `schtasks` offers no pid and no finer time, and spec 1 §10.4 fixes the commands. The pause makes the second start at least two seconds after the first. It is not a timing assertion (overview §10 rule 5).

Note on decision 1114: its sentence that "the task ignores a second start" no longer holds with `StopExisting`. The NSIS installer's own `/Run` now replaces the running instance.

Also decided (found when this change was first landed): `make_tools_dir` copies the fake tools with `/bin/cp` on Unix. `embed_succeeds_when_exiftool_exits_zero` failed in `v3-gate` with "Text file busy": the test process copied the tools itself, so a child forked by a parallel test could still hold a copy open for writing when it was run. With the copy made by another process, no such descriptor exists in the test process. Retrying the run or serializing the tests would have had to be repeated in every test that uses the fakes.
