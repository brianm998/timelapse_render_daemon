// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Raw, backup and export sequence scanning (§6.3).
//!
//! Every configured path is listed on a blocking thread under a timeout. A listing that hangs is
//! reported as `PROBLEM_PATH_UNREACHABLE` and the scan returns without waiting for it; the blocked
//! thread keeps its path marked in flight, so that path is not listed again until its listing
//! returns. No lock is held across a filesystem call, so one hung path never blocks another.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use regex::Regex;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::time::{timeout, timeout_at};

use crate::config::Config;
use crate::paths::{Clock, ExpansionCache, expand_years, has_year_placeholder};

/// The problem id raised for a path that cannot be listed (§6.3 step 4).
pub const PROBLEM_PATH_UNREACHABLE: &str = "PROBLEM_PATH_UNREACHABLE";

/// Timeout for one source or export path listing in a scan (§6.3 step 4).
pub const PATH_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for loading one backup directory in the background.
pub const BACKUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for one export size measurement.
pub const SIZE_TIMEOUT: Duration = Duration::from_secs(60);

/// A raw sequence directory is re-listed at least this often (§6.3 step 1).
pub const RAW_RELIST_AFTER: Duration = Duration::from_secs(600);

/// An export sequence directory is re-counted at least this often (§6.3 step 3).
pub const EXPORT_RECOUNT_AFTER: Duration = Duration::from_secs(60);

/// Backup sidecars are loaded again this long after the last attempt (§6.3 step 2).
pub const BACKUP_RELOAD_AFTER: Duration = Duration::from_secs(600);

/// Frame-count samples older than this are dropped for the export rate (§6.3 step 6).
pub const RATE_WINDOW: Duration = Duration::from_secs(300);

/// An export size is measured at most this often per sequence (§6.3 step 7).
pub const SIZE_INTERVAL: Duration = Duration::from_secs(60);

/// At most this many export size measurements run at once (§6.3 step 7).
pub const SIZE_CONCURRENCY: usize = 2;

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub name: String,
    /// A directory, following symbolic links.
    pub is_dir: bool,
    /// A regular file, following symbolic links.
    pub is_regular_file: bool,
}

/// The filesystem operations the scanner needs. Every method may block; the scanner only calls
/// them on blocking threads.
pub trait ScanFs: Send + Sync {
    /// The entries of a directory.
    fn list(&self, dir: &Path) -> io::Result<Vec<DirItem>>;
    /// The modification time of a directory.
    fn mtime(&self, dir: &Path) -> io::Result<SystemTime>;
    /// The recursive size in bytes of a directory's regular files.
    fn size(&self, dir: &Path) -> io::Result<u64>;
    /// The contents of a file.
    fn read(&self, file: &Path) -> io::Result<Vec<u8>>;
}

/// The real filesystem. Symbolic links are followed when listing; the size walk does not
/// descend into symbolic links, so a link cycle cannot loop.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealFs;

impl ScanFs for RealFs {
    fn list(&self, dir: &Path) -> io::Result<Vec<DirItem>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let (is_dir, is_regular_file) = match fs::metadata(entry.path()) {
                Ok(meta) => (meta.is_dir(), meta.is_file()),
                Err(_) => (false, false),
            };
            out.push(DirItem {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                is_regular_file,
            });
        }
        Ok(out)
    }

    fn mtime(&self, dir: &Path) -> io::Result<SystemTime> {
        fs::metadata(dir)?.modified()
    }

    fn size(&self, dir: &Path) -> io::Result<u64> {
        let mut total = 0u64;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in fs::read_dir(&current)? {
                let entry = entry?;
                let meta = fs::symlink_metadata(entry.path())?;
                if meta.is_dir() {
                    stack.push(entry.path());
                } else if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }

    fn read(&self, file: &Path) -> io::Result<Vec<u8>> {
        fs::read(file)
    }
}

/// A raw sequence found by a scan: a source directory or a backup sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSequence {
    pub name: String,
    /// The source path holding the directory, or the backup path holding the sidecar.
    pub parent: String,
    pub frames: u64,
    pub from_backup: bool,
}

/// An export sequence directory matched to a raw sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportSequence {
    /// The full path of the export directory.
    pub path: String,
    /// The name of the export directory.
    pub name: String,
    /// The raw sequence it matched.
    pub raw_name: String,
    pub raw_frames: u64,
    /// Image files in the export directory.
    pub frames: u64,
    /// Growth over the last 5 minutes per minute; 0 when unknown.
    pub export_frames_per_minute: f64,
    /// Seconds until the export is complete at that rate; -1 when unknown.
    pub export_eta_seconds: i64,
    /// Recursive size in bytes, measured in the background; -1 until measured.
    pub export_size: i64,
    /// Whether fewer frames are exported than the raw sequence has.
    pub in_progress: bool,
}

/// A path that could not be scanned this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathProblem {
    pub path: String,
    pub code: &'static str,
    pub reason: String,
}

impl PathProblem {
    fn unreachable(path: &str, reason: impl Into<String>) -> Self {
        PathProblem {
            path: path.to_owned(),
            code: PROBLEM_PATH_UNREACHABLE,
            reason: reason.into(),
        }
    }
}

/// What one scan found.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScanResult {
    pub raw: Vec<RawSequence>,
    pub exports: Vec<ExportSequence>,
    pub unreachable: Vec<PathProblem>,
}

/// A raw sequence candidate for export matching.
#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    parent: String,
    frames: u64,
    from_backup: bool,
}

/// A cached directory count.
#[derive(Debug, Clone, Copy)]
struct Counted {
    mtime: Option<SystemTime>,
    count: u64,
    counted_at: Instant,
}

/// The size of an export directory and when it was last scheduled.
#[derive(Debug, Default)]
struct SizeEntry {
    bytes: Option<i64>,
    scheduled_at: Option<Instant>,
    in_flight: bool,
}

#[derive(Default)]
struct State {
    /// Raw subdirectory counts, keyed by the subdirectory path.
    raw_dirs: HashMap<String, Counted>,
    /// Export directory counts, keyed by the directory path.
    export_dirs: HashMap<String, Counted>,
    /// Backup sidecar entries, keyed by backup directory.
    backups: HashMap<String, Vec<Candidate>>,
    backup_attempt: Option<Instant>,
    /// `$YEAR` expansion caches, one per pattern, taken out while their job runs.
    expansions: HashMap<String, ExpansionCache>,
    /// Frame-count samples per export directory.
    samples: HashMap<String, Vec<(Instant, u64)>>,
    sizes: HashMap<String, SizeEntry>,
    /// Export directories with fewer frames than their raw sequence, from the last scan.
    in_progress: Vec<String>,
    /// Paths whose listing has not returned yet.
    in_flight: HashSet<String>,
}

struct Inner {
    fs: Arc<dyn ScanFs>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    backup_running: AtomicBool,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks a path in flight for as long as its blocking job exists.
struct Claim {
    inner: Arc<Inner>,
    key: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.inner.state).in_flight.remove(&self.key);
    }
}

/// Clears the backup-running flag when a backup load ends, even if it is cancelled.
struct BackupRun(Arc<Inner>);

impl Drop for BackupRun {
    fn drop(&mut self) {
        self.0.backup_running.store(false, Ordering::SeqCst);
    }
}

/// The sequence scanner. Cheap to clone; clones share all state.
#[derive(Clone)]
pub struct Scanner {
    inner: Arc<Inner>,
    path_timeout: Duration,
    size_timeout: Duration,
}

impl Scanner {
    /// A scanner over `fs`, timing with `clock`, with the 10 s path and 60 s size timeouts.
    pub fn new(fs: Arc<dyn ScanFs>, clock: Arc<dyn Clock>) -> Self {
        Scanner {
            inner: Arc::new(Inner {
                fs,
                clock,
                state: Mutex::new(State::default()),
                backup_running: AtomicBool::new(false),
            }),
            path_timeout: PATH_TIMEOUT,
            size_timeout: SIZE_TIMEOUT,
        }
    }

    /// Replaces the per-path listing timeout.
    pub fn with_path_timeout(mut self, timeout: Duration) -> Self {
        self.path_timeout = timeout;
        self
    }

    /// Replaces the per-measurement size timeout.
    pub fn with_size_timeout(mut self, timeout: Duration) -> Self {
        self.size_timeout = timeout;
        self
    }

    /// The number of paths whose listing has not returned yet.
    pub fn in_flight_count(&self) -> usize {
        lock(&self.inner.state).in_flight.len()
    }

    /// Whether the backup sidecars should be loaded now: never loaded, or the last attempt is
    /// at least 10 minutes old, and no load is running.
    pub fn backups_due(&self) -> bool {
        if self.inner.backup_running.load(Ordering::SeqCst) {
            return false;
        }
        let attempt = lock(&self.inner.state).backup_attempt;
        match attempt {
            None => true,
            Some(at) => {
                self.inner.clock.instant().saturating_duration_since(at) >= BACKUP_RELOAD_AFTER
            }
        }
    }

    /// Loads the backup sidecars of every backup path (§6.3 step 2). Meant to run in the
    /// background; a call while another load runs returns at once. A directory whose load fails
    /// keeps its previous entries.
    pub async fn refresh_backups(&self, backup_paths: &[String]) {
        if self.inner.backup_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let _run = BackupRun(self.inner.clone());
        let now = self.inner.clock.instant();
        lock(&self.inner.state).backup_attempt = Some(now);

        let (dirs, expand_problems) = self.expand_patterns(backup_paths).await;
        for problem in &expand_problems {
            tracing::warn!(path = %problem.path, reason = %problem.reason, "backup path unreachable");
        }
        let (loaded, load_problems) = self
            .run_jobs(dirs.clone(), BACKUP_TIMEOUT, |inner, dir| {
                load_backup_dir(inner, dir)
            })
            .await;
        for problem in &load_problems {
            tracing::warn!(path = %problem.path, reason = %problem.reason, "backup path unreachable");
        }

        let mut st = lock(&self.inner.state);
        st.backups.retain(|dir, _| dirs.contains(dir));
        for (dir, entries) in loaded {
            st.backups.insert(dir, entries);
        }
    }

    /// Scans every configured path once (§6.3 steps 1 to 6). Never waits longer than the path
    /// timeout for any one path.
    pub async fn scan(&self, config: &Config) -> ScanResult {
        let mut unreachable = Vec::new();

        let (sources, problems) = self.expand_patterns(&config.source_paths).await;
        unreachable.extend(problems);
        let (export_roots, problems) = self.expand_patterns(&config.export_paths).await;
        unreachable.extend(problems);

        // Raw sequences: every source directory, then backups not overridden by a source.
        let raw_types = Arc::new(config.raw_types.clone());
        let (raw_found, problems) = self
            .run_jobs(sources, self.path_timeout, move |inner, root| {
                raw_job(inner, root, raw_types.as_slice())
            })
            .await;
        unreachable.extend(problems);

        let mut candidates: Vec<Candidate> = Vec::new();
        for (root, subs) in raw_found {
            for (name, frames) in subs {
                candidates.push(Candidate {
                    name,
                    parent: root.clone(),
                    frames,
                    from_backup: false,
                });
            }
        }
        let source_names: HashSet<String> = candidates.iter().map(|c| c.name.clone()).collect();
        {
            let st = lock(&self.inner.state);
            for entries in st.backups.values() {
                for entry in entries {
                    if !source_names.contains(&entry.name) {
                        candidates.push(entry.clone());
                    }
                }
            }
        }
        let mut raw: Vec<RawSequence> = candidates
            .iter()
            .map(|c| RawSequence {
                name: c.name.clone(),
                parent: c.parent.clone(),
                frames: c.frames,
                from_backup: c.from_backup,
            })
            .collect();
        raw.sort_by(|a, b| a.name.cmp(&b.name).then(a.parent.cmp(&b.parent)));

        // Export sequences, matched against the raw candidates.
        let ctx = Arc::new(ExportCtx {
            candidates,
            prefix: config.image_sequence_dirname_prefix.clone(),
            image_exts: config
                .image_extensions
                .iter()
                .map(|e| e.to_ascii_lowercase())
                .collect(),
            ignore: config
                .ignore_sequence_regexes
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect(),
        });
        let (export_found, problems) = self
            .run_jobs(export_roots, self.path_timeout, move |inner, root| {
                export_job(inner, root, &ctx)
            })
            .await;
        unreachable.extend(problems);

        // Samples, rates, ETAs and sizes.
        let now = self.inner.clock.instant();
        let mut exports = Vec::new();
        let mut in_progress = Vec::new();
        {
            let mut guard = lock(&self.inner.state);
            let st = &mut *guard;
            st.samples.retain(|_, samples| {
                samples.retain(|(at, _)| now.saturating_duration_since(*at) <= RATE_WINDOW);
                !samples.is_empty()
            });
            for (_root, found) in export_found {
                for item in found {
                    if item.frames == 0 {
                        continue;
                    }
                    let samples = st.samples.entry(item.path.clone()).or_default();
                    samples.push((now, item.frames));
                    let rate = rate_per_minute(samples);
                    let remaining = item.raw_frames.saturating_sub(item.frames);
                    let eta = if rate > 0.0 {
                        (remaining as f64 * 60.0 / rate).round() as i64
                    } else {
                        -1
                    };
                    let export_size = st.sizes.get(&item.path).and_then(|s| s.bytes).unwrap_or(-1);
                    let in_prog = item.frames < item.raw_frames;
                    if in_prog {
                        in_progress.push(item.path.clone());
                    }
                    exports.push(ExportSequence {
                        path: item.path,
                        name: item.name,
                        raw_name: item.raw_name,
                        raw_frames: item.raw_frames,
                        frames: item.frames,
                        export_frames_per_minute: rate,
                        export_eta_seconds: eta,
                        export_size,
                        in_progress: in_prog,
                    });
                }
            }
            st.in_progress = in_progress;
        }
        exports.sort_by(|a, b| a.path.cmp(&b.path));

        for problem in &unreachable {
            tracing::warn!(path = %problem.path, reason = %problem.reason, "path unreachable");
        }
        ScanResult {
            raw,
            exports,
            unreachable,
        }
    }

    /// Measures the size of every in-progress export directory that is due (§6.3 step 7): at
    /// most one measurement per sequence per 60 s, at most two running at once. A measurement
    /// that hangs keeps its slot until it returns.
    pub async fn measure_sizes(&self) {
        let now = self.inner.clock.instant();
        let due: Vec<String> = {
            let mut guard = lock(&self.inner.state);
            let st = &mut *guard;
            let mut due = Vec::new();
            for path in st.in_progress.iter() {
                let entry = st.sizes.entry(path.clone()).or_default();
                if entry.in_flight {
                    continue;
                }
                if entry
                    .scheduled_at
                    .is_some_and(|at| now.saturating_duration_since(at) < SIZE_INTERVAL)
                {
                    continue;
                }
                entry.scheduled_at = Some(now);
                entry.in_flight = true;
                due.push(path.clone());
            }
            due
        };

        let permits = Arc::new(Semaphore::new(SIZE_CONCURRENCY));
        let limit = self.size_timeout;
        let mut tasks = Vec::new();
        for path in due {
            let inner = self.inner.clone();
            let permits = permits.clone();
            tasks.push(tokio::spawn(async move {
                // The slot is held here on the async side, never by the blocking thread. A
                // measurement that outlives `limit` gives its slot back; its thread stays blocked
                // until `size` returns, holding nothing but its own captured path.
                let permit = permits
                    .acquire_owned()
                    .await
                    .expect("size semaphore is never closed");
                let job_inner = inner.clone();
                let job_path = path.clone();
                let handle = tokio::task::spawn_blocking(move || {
                    let bytes = job_inner
                        .fs
                        .size(Path::new(&job_path))
                        .ok()
                        .map(|b| b as i64);
                    let mut guard = lock(&job_inner.state);
                    if let Some(entry) = guard.sizes.get_mut(&job_path) {
                        if bytes.is_some() {
                            entry.bytes = bytes;
                        }
                        entry.in_flight = false;
                    }
                });
                let _ = timeout(limit, handle).await;
                drop(permit);
            }));
        }
        for task in tasks {
            let _ = task.await;
        }
    }

    /// Marks `key` in flight, or returns `None` if its previous listing has not returned.
    fn claim(&self, key: &str) -> Option<Claim> {
        let mut st = lock(&self.inner.state);
        if !st.in_flight.insert(key.to_owned()) {
            return None;
        }
        Some(Claim {
            inner: self.inner.clone(),
            key: key.to_owned(),
        })
    }

    /// Runs `job` for every key on its own blocking thread, all sharing one deadline. Returns the
    /// results of the jobs that finished in time, and a problem for each key that did not.
    async fn run_jobs<T, F>(
        &self,
        keys: Vec<String>,
        limit: Duration,
        job: F,
    ) -> (Vec<(String, T)>, Vec<PathProblem>)
    where
        T: Send + 'static,
        F: Fn(&Inner, &str) -> Result<T, String> + Send + Sync + 'static,
    {
        let job = Arc::new(job);
        let mut problems = Vec::new();
        let mut handles = Vec::new();
        let mut seen = HashSet::new();
        for key in keys {
            if !seen.insert(key.clone()) {
                continue;
            }
            match self.claim(&key) {
                None => problems.push(PathProblem::unreachable(
                    &key,
                    "a previous listing of this path has not returned",
                )),
                Some(claim) => {
                    let job = job.clone();
                    let handle = tokio::task::spawn_blocking(move || {
                        let claim = claim;
                        (*job)(&claim.inner, claim.key.as_str())
                    });
                    handles.push((key, handle));
                }
            }
        }

        let deadline = tokio::time::Instant::now() + limit;
        let mut done = Vec::new();
        for (key, handle) in handles {
            match timeout_at(deadline, handle).await {
                Ok(Ok(Ok(value))) => done.push((key, value)),
                Ok(Ok(Err(reason))) => problems.push(PathProblem::unreachable(&key, reason)),
                Ok(Err(err)) => problems.push(PathProblem::unreachable(
                    &key,
                    format!("listing failed: {err}"),
                )),
                Err(_) => problems.push(PathProblem::unreachable(
                    &key,
                    format!("listing did not return within {} s", limit.as_secs()),
                )),
            }
        }
        (done, problems)
    }

    /// Expands `$YEAR` entries (§6.1) in their own jobs, so a hung directory check cannot block
    /// the others. Returns the paths in configuration order.
    async fn expand_patterns(&self, patterns: &[String]) -> (Vec<String>, Vec<PathProblem>) {
        let mut year: Vec<String> = Vec::new();
        for pattern in patterns {
            if has_year_placeholder(pattern) && !year.contains(pattern) {
                year.push(pattern.clone());
            }
        }
        if year.is_empty() {
            return (patterns.to_vec(), Vec::new());
        }
        let (found, problems) = self
            .run_jobs(year, self.path_timeout, |inner, pattern| {
                let mut cache = lock(&inner.state)
                    .expansions
                    .remove(pattern)
                    .unwrap_or_default();
                let expanded =
                    expand_years(&[pattern.to_owned()], inner.clock.as_ref(), &mut cache);
                lock(&inner.state)
                    .expansions
                    .insert(pattern.to_owned(), cache);
                Ok::<Vec<String>, String>(expanded)
            })
            .await;
        let found: HashMap<String, Vec<String>> = found.into_iter().collect();
        let mut out = Vec::new();
        for pattern in patterns {
            if has_year_placeholder(pattern) {
                if let Some(list) = found.get(pattern) {
                    out.extend(list.iter().cloned());
                }
            } else {
                out.push(pattern.clone());
            }
        }
        (out, problems)
    }
}

/// What the export listing needs from the raw candidates and the configuration.
struct ExportCtx {
    candidates: Vec<Candidate>,
    prefix: String,
    /// Lower-case image extensions.
    image_exts: Vec<String>,
    ignore: Vec<Regex>,
}

/// An export directory matched to its raw sequence, with its frame count.
struct ExportFound {
    path: String,
    name: String,
    raw_name: String,
    raw_frames: u64,
    frames: u64,
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Whether the file name's extension is one of `exts`, ignoring ASCII case.
fn has_extension(name: &str, exts: &[String]) -> bool {
    match Path::new(name).extension().and_then(|e| e.to_str()) {
        Some(ext) => exts.iter().any(|x| x.eq_ignore_ascii_case(ext)),
        None => false,
    }
}

/// A cached count is usable when the mtime is the same and the count is younger than `max_age`.
fn is_fresh(c: &Counted, mtime: Option<SystemTime>, now: Instant, max_age: Duration) -> bool {
    c.mtime.is_some() && c.mtime == mtime && now.saturating_duration_since(c.counted_at) < max_age
}

/// Growth per minute from the oldest sample to the newest; 0 when unknown.
fn rate_per_minute(samples: &[(Instant, u64)]) -> f64 {
    let (Some(&(t0, c0)), Some(&(t1, c1))) = (samples.first(), samples.last()) else {
        return 0.0;
    };
    if samples.len() < 2 || c1 <= c0 {
        return 0.0;
    }
    let secs = t1.saturating_duration_since(t0).as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    (c1 - c0) as f64 * 60.0 / secs
}

/// The raw candidate an export directory name belongs to (§6.3 step 3): the candidate whose
/// `prefix + name` is a substring of `dir`; the longest name wins, then a source entry over a
/// backup entry, then the smallest parent path.
fn best_match<'a>(candidates: &'a [Candidate], prefix: &str, dir: &str) -> Option<&'a Candidate> {
    candidates
        .iter()
        .filter(|c| dir.contains(&format!("{prefix}{}", c.name)))
        .min_by(|a, b| {
            b.name
                .len()
                .cmp(&a.name.len())
                .then(a.from_backup.cmp(&b.from_backup))
                .then(a.parent.cmp(&b.parent))
        })
}

/// The raw sequences in one source directory: `(name, frames)` for each non-hidden subdirectory
/// (§6.3 step 1).
fn raw_job(inner: &Inner, root: &str, raw_types: &[String]) -> Result<Vec<(String, u64)>, String> {
    let root_path = Path::new(root);
    let items = inner.fs.list(root_path).map_err(|e| e.to_string())?;
    let now = inner.clock.instant();
    let mut out = Vec::new();
    for item in items.iter().filter(|i| i.is_dir && !is_hidden(&i.name)) {
        let sub = root_path.join(&item.name);
        let key = sub.to_string_lossy().into_owned();
        let mtime = inner.fs.mtime(&sub).ok();
        let cached = lock(&inner.state).raw_dirs.get(&key).copied();
        let count = match cached {
            Some(c) if is_fresh(&c, mtime, now, RAW_RELIST_AFTER) => c.count,
            _ => match inner.fs.list(&sub) {
                Ok(children) => {
                    let count = children
                        .iter()
                        .filter(|i| i.is_regular_file && has_extension(&i.name, raw_types))
                        .count() as u64;
                    lock(&inner.state).raw_dirs.insert(
                        key,
                        Counted {
                            mtime,
                            count,
                            counted_at: now,
                        },
                    );
                    count
                }
                Err(_) => continue,
            },
        };
        out.push((item.name.clone(), count));
    }
    Ok(out)
}

/// The export sequences in one export directory (§6.3 step 3).
fn export_job(inner: &Inner, root: &str, ctx: &ExportCtx) -> Result<Vec<ExportFound>, String> {
    let root_path = Path::new(root);
    let items = inner.fs.list(root_path).map_err(|e| e.to_string())?;
    let now = inner.clock.instant();
    let mut out = Vec::new();
    for item in items.iter().filter(|i| i.is_dir && !is_hidden(&i.name)) {
        if ctx.ignore.iter().any(|re| re.is_match(&item.name)) {
            continue;
        }
        let Some(raw) = best_match(&ctx.candidates, &ctx.prefix, &item.name) else {
            continue;
        };
        let sub = root_path.join(&item.name);
        let key = sub.to_string_lossy().into_owned();
        let mtime = inner.fs.mtime(&sub).ok();
        let cached = lock(&inner.state).export_dirs.get(&key).copied();
        let frames = match cached {
            Some(c) if is_fresh(&c, mtime, now, EXPORT_RECOUNT_AFTER) => c.count,
            _ => match inner.fs.list(&sub) {
                Ok(children) => {
                    let count = children
                        .iter()
                        .filter(|i| {
                            i.is_regular_file
                                && !is_hidden(&i.name)
                                && has_extension(&i.name, &ctx.image_exts)
                        })
                        .count() as u64;
                    lock(&inner.state).export_dirs.insert(
                        key.clone(),
                        Counted {
                            mtime,
                            count,
                            counted_at: now,
                        },
                    );
                    count
                }
                Err(_) => continue,
            },
        };
        out.push(ExportFound {
            path: key,
            name: item.name.clone(),
            raw_name: raw.name.clone(),
            raw_frames: raw.frames,
            frames,
        });
    }
    Ok(out)
}

/// The `(capture, count)` of a backup sidecar file name `^([^.]+)\.json$`.
fn sidecar_name(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".json")?;
    if stem.is_empty() || stem.contains('.') {
        None
    } else {
        Some(stem)
    }
}

/// A count from a JSON number, or from a string holding one.
fn as_count(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The frame count of a sidecar: the `ImageSize` object's first key in sorted order, else
/// `number_source_images` (§6.3 step 2).
fn backup_count(value: &Value) -> Option<u64> {
    if let Some(Value::Object(sizes)) = value.get("ImageSize") {
        let mut keys: Vec<&String> = sizes.keys().collect();
        keys.sort();
        if let Some(first) = keys.first()
            && let Some(count) = sizes.get(first.as_str()).and_then(as_count)
        {
            return Some(count);
        }
    }
    value.get("number_source_images").and_then(as_count)
}

/// The backup sidecars in one backup directory. Zero counts and unreadable files are skipped.
fn load_backup_dir(inner: &Inner, dir: &str) -> Result<Vec<Candidate>, String> {
    let dir_path = Path::new(dir);
    let items = inner.fs.list(dir_path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for item in items.iter().filter(|i| i.is_regular_file) {
        let Some(name) = sidecar_name(&item.name) else {
            continue;
        };
        let Ok(bytes) = inner.fs.read(&dir_path.join(&item.name)) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(frames) = backup_count(&value) else {
            continue;
        };
        if frames == 0 {
            continue;
        }
        out.push(Candidate {
            name: name.to_owned(),
            parent: dir.to_owned(),
            frames,
            from_backup: true,
        });
    }
    Ok(out)
}
