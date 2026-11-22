// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Database schema and migrations (§6.13).
//!
//! Migration 1 creates every table and index of §6.13. Later migrations append to [`migrations`];
//! an existing migration is never edited. `foreign` is quoted because it is an SQL keyword.

use rusqlite_migration::{M, Migrations};

/// Migration 1: every table and index of §6.13. All timestamps are milliseconds since the Unix
/// epoch.
pub const MIGRATION_1: &str = "
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE render_jobs (
    id              TEXT PRIMARY KEY,
    sequence_name   TEXT NOT NULL,
    export_path     TEXT NOT NULL,
    variant_key     TEXT NOT NULL,
    output_filename TEXT NOT NULL,
    output_path     TEXT NOT NULL,
    partial_path    TEXT NOT NULL,
    plan_json       TEXT NOT NULL,
    status          TEXT NOT NULL,
    suspended_by    TEXT,
    pid             INTEGER,
    pid_start_time  INTEGER,
    queued_at       INTEGER NOT NULL,
    started_at      INTEGER,
    finished_at     INTEGER,
    progress_path   TEXT,
    log_path        TEXT,
    adopted         INTEGER NOT NULL,
    \"foreign\"       INTEGER NOT NULL
);

CREATE TABLE rendered_videos (
    id                TEXT PRIMARY KEY,
    rendered_at       INTEGER NOT NULL,
    sequence_name     TEXT NOT NULL,
    export_path       TEXT NOT NULL,
    output_filename   TEXT NOT NULL,
    output_path       TEXT NOT NULL,
    variant_key       TEXT NOT NULL,
    codec             TEXT NOT NULL,
    resolution_label  TEXT NOT NULL,
    frame_rate        INTEGER NOT NULL,
    frame_count       INTEGER NOT NULL,
    size_bytes        INTEGER NOT NULL,
    width             INTEGER NOT NULL,
    height            INTEGER NOT NULL,
    duration_seconds  REAL NOT NULL,
    render_seconds    REAL NOT NULL,
    thumbnail_path    TEXT,
    xmp_embedded      INTEGER NOT NULL,
    source            TEXT NOT NULL
);
CREATE INDEX rendered_videos_rendered_at ON rendered_videos (rendered_at DESC);

CREATE TABLE failures (
    id                  TEXT PRIMARY KEY,
    output_path         TEXT NOT NULL,
    kind                TEXT NOT NULL,
    first_seen          INTEGER NOT NULL,
    last_seen           INTEGER NOT NULL,
    occurrences         INTEGER NOT NULL,
    sequence_name       TEXT NOT NULL,
    export_path        TEXT NOT NULL,
    output_filename     TEXT NOT NULL,
    variant_key         TEXT NOT NULL,
    summary             TEXT NOT NULL,
    detail              TEXT NOT NULL,
    exit_code           INTEGER,
    command_line        TEXT NOT NULL,
    tool_output_json    TEXT NOT NULL,
    variant_auto_paused INTEGER NOT NULL,
    render_id           TEXT,
    UNIQUE (output_path, kind)
);

CREATE TABLE variant_strikes (
    sequence_name   TEXT NOT NULL,
    variant_key     TEXT NOT NULL,
    corrupt_strikes INTEGER NOT NULL,
    PRIMARY KEY (sequence_name, variant_key)
);

CREATE TABLE exif_cache (
    raw_dir        TEXT PRIMARY KEY,
    histogram_json TEXT NOT NULL,
    collected_at   INTEGER NOT NULL
);

CREATE TABLE devices (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    platform      TEXT NOT NULL,
    token_sha256  TEXT NOT NULL UNIQUE,
    paired_at     INTEGER NOT NULL,
    last_seen     INTEGER,
    revoked       INTEGER NOT NULL
);

CREATE TABLE log (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    at              INTEGER NOT NULL,
    level           TEXT NOT NULL,
    subject         TEXT NOT NULL,
    message         TEXT NOT NULL,
    event           TEXT,
    sequence_name   TEXT,
    output_filename TEXT,
    rendered_video_id TEXT,
    failure_id      TEXT,
    video_count     INTEGER,
    total_bytes     INTEGER
);
";

/// Every migration, in order. The schema version is the number of migrations applied.
pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(MIGRATION_1)])
}
