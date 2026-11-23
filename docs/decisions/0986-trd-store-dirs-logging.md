# 0986: directories, logging and the SQLite store (TRD-04)

## Decisions

- `Dirs::for_os(os, &Env)` takes the home directory and the variables as values; only `Env::system()` reads the process environment, and no test calls it. The XDG rule holds: a relative `XDG_*` value counts as unset and the home default applies. `APPDATA` and `LOCALAPPDATA` are used as given.
- The overview's `logs` paths name the `trd.log` file. Here `Dirs.logs` is the directory that holds it.
- Rolling log files are `trd.YYYY-MM-DD.log`, not a bare `trd.log`. `tracing-appender` always puts the date in the name; prefix `trd`, suffix `log`, daily rotation, 14 files kept.
- The stderr layer is added only when stderr was a terminal before any redirect and the process is not in daemon mode, so a log line is never written twice to the same stream.
- The daemon's stderr redirect uses `dup2` on unix and `SetStdHandle` on Windows. The Windows branch is compiled only on Windows; this worktree builds on macOS.
- `foreign` is quoted as `"foreign"` in the schema because it is an SQL keyword. The column names and types otherwise follow §6.13.
- `rusqlite_migration` 1.2.0 uses rusqlite 0.31, the workspace pin, so rusqlite is not bumped.
- Migration version is `PRAGMA user_version`, which `rusqlite_migration` manages; `schema_version()` returns it.
- Enums with stored text (`JobStatus`, `SuspendedBy`, `LogLevel`) convert with `as_str` and `parse`. Other text columns, such as failure `kind` and `source`, stay `String`, since their vocabularies belong to later tickets.
- `log_event` inserts the row first so the `tracing` record carries its `seq`, then publishes the row on a `tokio` broadcast channel of 1024 rows. Sending with no subscriber is not an error.
- `prune_with(now, history_days, log_max_rows, log_max_age_ms)` returns the deleted thumbnail paths and leaves the files to the caller. `prune` uses 20 000 rows and 30 days. Failures are not age-pruned here: §6.9's 30-day rule belongs to the engine ticket.
- `Store` holds a `Mutex<Connection>`; each method takes the lock for one statement or one transaction.

## Known limits

- Only the unix stderr redirect is tested. The Windows branch is untested until a Windows CI run.
