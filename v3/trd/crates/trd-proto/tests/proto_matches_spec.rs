// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! A1: the trd.v1 proto in the repository is the §7 block of spec 1, byte for byte.
//! The test only reads files. CRLF is dropped from both sides so a Windows checkout with
//! autocrlf still compares equal.

use std::fs;
use std::path::PathBuf;

fn manifest_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn strip_cr(text: &str) -> String {
    text.replace('\r', "")
}

/// Lines between the first ```proto fence and the next ``` fence, each ending in a newline.
fn spec_proto_block(spec: &str) -> String {
    let mut block = String::new();
    let mut inside = false;
    for line in spec.lines() {
        if !inside {
            if line == "```proto" {
                inside = true;
            }
        } else if line == "```" {
            return block;
        } else {
            block.push_str(line);
            block.push('\n');
        }
    }
    panic!("no ```proto block found in the spec");
}

#[test]
fn proto_matches_spec() {
    let spec =
        fs::read_to_string(manifest_path("../../../1-trd-core-daemon.md")).expect("read spec 1");
    let proto = fs::read_to_string(manifest_path("../../../proto/trd/v1/trd.proto"))
        .expect("read trd.proto");

    let expected = strip_cr(&spec_proto_block(&strip_cr(&spec)));
    let actual = strip_cr(&proto);
    assert_eq!(
        actual, expected,
        "trd.proto differs from the §7 block of spec 1"
    );
}
