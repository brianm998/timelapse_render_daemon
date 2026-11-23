// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! A Snapshot encodes with prost and decodes back to an equal value.

use prost::Message;
use trd_proto::trd::v1::{Counts, Phase, Snapshot};

#[test]
fn snapshot_round_trips() {
    let snapshot = Snapshot {
        revision: 42,
        generated_at_ms: 1_700_000_000_123,
        phase: Phase::Rendering as i32,
        global_pause: true,
        counts: Some(Counts {
            rendering: 2,
            suspended: 1,
            queued: 5,
            exporting: 0,
            paused_sequences: 3,
            failures: 1,
            rendered_last_24h: 7,
        }),
        config_path: "/Users/example/trd/config.toml".to_string(),
        ..Default::default()
    };

    let bytes = snapshot.encode_to_vec();
    let decoded = Snapshot::decode(bytes.as_slice()).expect("decode Snapshot");
    assert_eq!(decoded, snapshot);
}
