# 1159 — the CLI usage name is compared with EXE_SUFFIX, not hard-coded

Status: accepted (s1169)

Decision: the `service` usage-error test in `crates/trd/tests/cli.rs` builds its expected text as
`"Usage: trd{EXE_SUFFIX} service"`, so it reads `trd` on unix and `trd.exe` on Windows.

Decision: the exit-code-2 assertion and the `Usage: trd` prefix check are unchanged. No assertion
is weakened.

Reason: clap takes the usage name from the file name of argv[0] (clap_builder 4.6.x,
`command.rs:902-905`). On Windows the binary is `trd.exe`, so the usage line reads
`Usage: trd.exe service <COMMAND>`. A hard-coded `Usage: trd service` fails there.

Rejected: hard-coding `trd.exe` for Windows with `cfg!` branches. The suffix constant already
states the platform rule and matches what clap prints.
