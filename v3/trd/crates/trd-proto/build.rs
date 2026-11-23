// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Compiles the trd.v1 API (v3/proto/trd/v1/trd.proto) into Rust client and server code.
//! protoc comes from protoc-bin-vendored, so the build never looks for a system protoc.

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: build scripts are single-threaded, so nothing reads the environment concurrently.
    unsafe { std::env::set_var("PROTOC", protoc) };

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let proto_root = manifest.join("../../../proto");
    let proto = proto_root.join("trd/v1/trd.proto");
    println!("cargo:rerun-if-changed={}", proto.display());

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&[proto], &[proto_root])?;
    Ok(())
}
