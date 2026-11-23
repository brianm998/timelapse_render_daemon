// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Config path resolution and $YEAR path expansion (§6.1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local};

/// The placeholder that `$YEAR` path entries expand.
pub const YEAR_PLACEHOLDER: &str = "$YEAR";

/// How long an expansion stays cached (§6.1).
pub const EXPANSION_CACHE_TTL: Duration = Duration::from_secs(60);

/// First year `$YEAR` expands down to (§6.1).
pub const FIRST_YEAR: i32 = 1970;

/// Source of the time for `$YEAR` expansion and the cache, injectable for tests.
pub trait Clock: Send + Sync {
    /// Monotonic time, used for the expansion cache.
    fn instant(&self) -> Instant;
    /// The current calendar year, used for `$YEAR`.
    fn year(&self) -> i32;
}

/// The real clock: monotonic time and the local calendar year.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn instant(&self) -> Instant {
        Instant::now()
    }

    fn year(&self) -> i32 {
        Local::now().year()
    }
}

/// A clock that only moves when told to. Used by tests and by any caller that needs a fixed time.
#[derive(Debug)]
pub struct ManualClock {
    instant: Mutex<Instant>,
    year: AtomicI32,
}

impl ManualClock {
    /// A clock at year `year`, whose instant is the moment of the call.
    pub fn new(year: i32) -> Self {
        ManualClock {
            instant: Mutex::new(Instant::now()),
            year: AtomicI32::new(year),
        }
    }

    /// Moves the monotonic time forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut at = self.instant.lock().unwrap_or_else(|e| e.into_inner());
        *at += by;
    }

    /// Sets the calendar year.
    pub fn set_year(&self, year: i32) {
        self.year.store(year, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn instant(&self) -> Instant {
        *self.instant.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn year(&self) -> i32 {
        self.year.load(Ordering::SeqCst)
    }
}

/// Whether a path entry contains the `$YEAR` placeholder.
pub fn has_year_placeholder(pattern: &str) -> bool {
    pattern.contains(YEAR_PLACEHOLDER)
}

/// Expands one path entry for a given current year, without caching.
///
/// A path without `$YEAR` is returned unchanged, with no existence check. A path with `$YEAR`
/// becomes one path per year from `current_year` down to 1970, keeping only existing directories.
pub fn expand_year_path(pattern: &str, current_year: i32) -> Vec<String> {
    if !has_year_placeholder(pattern) {
        return vec![pattern.to_owned()];
    }
    let mut out = Vec::new();
    for year in (FIRST_YEAR..=current_year).rev() {
        let candidate = pattern.replace(YEAR_PLACEHOLDER, &year.to_string());
        if Path::new(&candidate).is_dir() {
            out.push(candidate);
        }
    }
    out
}

/// Cache of `$YEAR` expansions, keyed by the unexpanded entry.
#[derive(Debug, Default)]
pub struct ExpansionCache {
    entries: HashMap<String, (Instant, Vec<String>)>,
}

impl ExpansionCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Expands every entry of a path list (§6.1), using the 60 s cache for `$YEAR` entries.
///
/// The result keeps the order of `paths`; each `$YEAR` entry contributes its expansions newest
/// first. An entry is re-expanded when its cached result is 60 s old or more.
pub fn expand_years(
    paths: &[String],
    clock: &dyn Clock,
    cache: &mut ExpansionCache,
) -> Vec<String> {
    let now = clock.instant();
    let year = clock.year();
    let mut out = Vec::new();
    for pattern in paths {
        if !has_year_placeholder(pattern) {
            out.push(pattern.clone());
            continue;
        }
        let fresh = cache
            .entries
            .get(pattern)
            .filter(|(at, _)| now.saturating_duration_since(*at) < EXPANSION_CACHE_TTL)
            .map(|(_, expanded)| expanded.clone());
        let expanded = match fresh {
            Some(expanded) => expanded,
            None => {
                let expanded = expand_year_path(pattern, year);
                cache
                    .entries
                    .insert(pattern.clone(), (now, expanded.clone()));
                expanded
            }
        };
        out.extend(expanded);
    }
    out
}

/// The config file to use: the `--config` flag, else the stored `config_path` setting, else the
/// default path (§6.1, §6.13).
pub fn resolve_config_path(flag: Option<&Path>, stored: Option<&Path>, default: &Path) -> PathBuf {
    flag.or(stored).unwrap_or(default).to_path_buf()
}
