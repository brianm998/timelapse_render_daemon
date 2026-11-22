// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The daemon's logic without networking (spec 1 §6); its modules are declared here.

pub mod adopt;
pub mod aspect;
pub mod codecs;
pub mod config;
pub mod controls;
pub mod diagnosis;
pub mod dirs;
pub mod engine;
pub mod events;
pub mod exif;
pub mod fda;
pub mod history_import;
pub mod logging;
pub mod paths;
pub mod pattern;
pub mod pipeline;
pub mod plan;
pub mod problems;
pub mod process;
pub mod progress;
pub mod queue;
pub mod scan;
pub mod snapshot;
pub mod store;
pub mod thumbs;
pub mod tools;
pub mod xmp;
