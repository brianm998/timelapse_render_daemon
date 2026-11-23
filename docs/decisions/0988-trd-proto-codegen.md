# 0988: trd.v1 proto and Rust codegen (TRD-02)

## Decisions

- `trd.proto` is the §7 block copied verbatim. `proto_matches_spec` extracts the block from the spec and compares it byte for byte (CR stripped on both sides for Windows checkouts), so the spec and the file cannot drift apart silently.
- protoc comes from `protoc-bin-vendored`. `build.rs` sets `PROTOC` to that binary before `tonic_prost_build` runs, so `cargo build -p trd-proto` works with no protoc on PATH.
- Both client and server code are generated (`build_client(true)`, `build_server(true)`), because the daemon uses the server and the tests use the client.
- `trd-proto` depends on `tonic`, `tonic-prost` and `prost` only; `tonic-prost-build` and `protoc-bin-vendored` are build dependencies.
- `Cargo.lock` is updated in the same change, because every CI command runs with `--locked`.
- The generated code is not checked in. It is produced in `OUT_DIR` at build time. Clippy runs over it with `-D warnings`.

## Known limit

The README's freeze line in spec 1 step 7 asks for a date. The freeze happens at TRD-GATE, so the README names the gate rather than a date that does not yet exist.
