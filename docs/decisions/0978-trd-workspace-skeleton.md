# 0978: trd workspace skeleton (TRD-01)

## Decisions

- `resolver = "3"` with `edition = "2024"`: edition 2024 implies resolver 3; the spec's "resolver 2024" names no Cargo value.
- Every §4 dependency is declared in `[workspace.dependencies]`, but a member declares only what its code uses (trd: clap and trd-core). Cargo locks only what members reach, so the lock holds 21 packages.
- `Cargo.lock` was generated offline (`CARGO_NET_OFFLINE=true`) from the local registry cache.
- `trd version` prints `macos-universal` on macOS, `linux-x64`, `linux-arm64`, `windows-x64`, otherwise `<os>-<arch>`.
- With no arguments `trd` prints help and exits 0. Unimplemented commands print `<name>: not implemented yet` and exit 1.
- `trdw` inserts `daemon` as the subcommand when the arguments name none; `cli.rs` is shared by both binaries through `#[path]`.
- `coverage.py` measures `trd-core` and `trd` only; trd-testtools and generated code are excluded.

## Known limit

The literal acceptance command `python3 -I -m unittest v3/trd/scripts/test_coverage.py` cannot import the test module: `-I` keeps the working directory off `sys.path`, so unittest cannot resolve the dotted name. The tests run and pass as `python3 -I scripts/test_coverage.py` from `v3/trd`, which `coverage.sh` uses.
