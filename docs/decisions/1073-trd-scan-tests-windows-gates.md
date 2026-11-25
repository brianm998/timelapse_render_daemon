# Decision 1073: scan test gates release on drop, with no timer

Ticket #1104 (TRD-06b).

- The fake filesystem keys every path through `norm()`, which turns `\` into `/`. `Path::join` on Windows
  therefore matches the fake's `/fake/...` entries. The scanner is unchanged.
- `Gate::wait` is a plain Condvar wait with no timer. The `GateHandle` returned by `block` and
  `block_size` releases the gate on drop, so a failing assertion before `release()` frees the blocked
  listing and the runtime can shut down.
- Why no timer: a `HANG_GUARD`-length timer inside `Gate::wait` would race the scanner's own 30 s
  `with_path_timeout` in `slow_path_does_not_block` and `in_flight_path_not_relisted`. `HANG_GUARD`
  bounds only the polling loops in the test bodies.
