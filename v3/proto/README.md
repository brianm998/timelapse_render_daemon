<!--
SPDX-License-Identifier: GPL-3.0-or-later
Copyright (C) 2026 Timelapse Render Daemon contributors
-->

# trd.v1 API

`trd/v1/trd.proto` is the gRPC API between the `trd` daemon and its clients (spec 1 §7). It is
the single source of truth: the daemon and every client generate their code from this file. Do not
hand-edit generated code.

## API version rule

- `api_version = 1` in the proto is the version the daemon and the clients in specs 2–4 build
  against.
- API version 1 is frozen as of 2026-11-27; changes need `api_version` 2 and a new proto package `trd.v2`.
- After the freeze, a change that is not purely additive needs `api_version` 2 and a new proto
  package `trd.v2` in `trd/v2/trd.proto`. The `trd.v1` file stays unchanged so older clients keep
  working.

## How each client generates code

All generation reads `v3/proto/trd/v1/trd.proto` directly. No copy of the file is kept in any
client.

- **Rust (daemon and `trd-proto`)**: `v3/trd/crates/trd-proto/build.rs` runs `tonic-prost-build`
  with a protoc from `protoc-bin-vendored`, so no system `protoc` is needed. The code is generated
  into `OUT_DIR` at build time and included with `tonic::include_proto!("trd.v1")`.
- **Kotlin (desktop TRM and Android)**: the Gradle `proto` source set points at
  `v3/proto/` and uses the `protoc` artifact at version 3.25.5 (no system protoc). Package
  `com.timelapserender.trd.v1`.
- **Swift (iOS TRM)**: `v3/ios/scripts/gen-proto.sh` runs `protoc` with the `swift-protobuf` and
  `grpc-swift-protobuf` plugins on `trd/v1/trd.proto` and writes the output to `Generated/`, plus
  `Generated/PROTO_SHA256`, the SHA-256 of the proto the code came from.

Regenerate after any change to `trd.proto`, and commit the generated Swift code with the proto
change.
