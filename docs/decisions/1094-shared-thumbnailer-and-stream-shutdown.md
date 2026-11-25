# 1094 — One thumbnailer for the engine and the server; streams stop without stalling

Status: accepted (TRD-17)

Decision: the daemon builds one `Thumbnailer` and hands the engine a `share()` handle. The gate
and runner are shared behind `Arc`, so the concurrency cap is one cap for the process.

Decision: every stream send races the shutdown watch. A stopped stream ends with UNAVAILABLE,
with the notice wait bounded at one second, so a client that does not read cannot keep the
daemon's graceful shutdown waiting.

Why: two separate thumbnailers each got their own gate, doubling the cap; a blocked send
ignored shutdown.
