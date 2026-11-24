// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Events emitted to clients (§6.12).
//!
//! An event is a `log` row whose `event` field holds one of these names (§6.14). The names are the
//! `EventKind` names of `trd.proto`. This module holds the six that the render queue and completion
//! pipeline emit (TRD-12); the rest of §6.12 is added with the event stream (TRD-16).

/// A sequence became complete and its variants are considered for rendering.
pub const EVENT_SEQUENCE_READY: &str = "EVENT_SEQUENCE_READY";
/// A render's ffmpeg process was started.
pub const EVENT_RENDER_STARTED: &str = "EVENT_RENDER_STARTED";
/// A render finished and its video was recorded.
pub const EVENT_RENDER_FINISHED: &str = "EVENT_RENDER_FINISHED";
/// The last missing variant of a sequence finished; carries `video_count` and `total_bytes`.
pub const EVENT_SEQUENCE_FINISHED: &str = "EVENT_SEQUENCE_FINISHED";
/// A render failed; carries `failure_id`.
pub const EVENT_RENDER_FAILED: &str = "EVENT_RENDER_FAILED";
/// The export directory of a sequence was deleted after its renders (§6.6 step 8).
pub const EVENT_SEQUENCE_DELETED: &str = "EVENT_SEQUENCE_DELETED";
