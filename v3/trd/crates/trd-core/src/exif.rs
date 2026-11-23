// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! EXIF histogram collection with exiftool and sidecars (§6.4).
//!
//! A raw sequence's histogram counts, for every EXIF column, how often each value occurs across
//! its raw files. It is collected with batched `exiftool -csv -@ <argfile>` runs, stored as the
//! sidecar `<raw parent>/<raw name>.json` (or in the `exif_cache` table when that cannot be
//! written), and folded into the ffmpeg metadata: every tag with exactly one observed value.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::store::{ExifCacheEntry, Store, StoreError};
use crate::tools::{ResolvedTool, ToolCommand};

/// Column → value → number of raw files holding that value.
pub type Histogram = BTreeMap<String, BTreeMap<String, u64>>;

/// The most raw files one exiftool run reads (§6.4).
pub const BATCH_SIZE: usize = 64;

/// The binary columns, by their `-csv` names and by their spaced names (§6.4).
const SKIPPED_COLUMNS: [&str; 6] = [
    "ThumbnailImage",
    "PreviewImage",
    "TiffMeteringImage",
    "Thumbnail Image",
    "Preview Image",
    "Tiff Metering Image",
];

/// Values that exiftool writes for binary data.
const BINARY_PREFIX: &str = "(Binary data";

/// Why an EXIF collection, sidecar or cache operation failed.
#[derive(Debug)]
pub enum ExifError {
    Io(io::Error),
    /// The tool has no command: it was not found or is broken.
    ToolUnavailable,
    /// The tool could not be started.
    Launch {
        program: String,
        source: io::Error,
    },
    /// The tool exited unsuccessfully and printed nothing to read.
    Failed {
        status: String,
        stderr: String,
    },
    Csv(csv::Error),
    Json(serde_json::Error),
    Store(StoreError),
}

impl fmt::Display for ExifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExifError::Io(err) => write!(f, "EXIF file error: {err}"),
            ExifError::ToolUnavailable => write!(f, "exiftool is not available"),
            ExifError::Launch { program, source } => {
                write!(f, "cannot run exiftool {program}: {source}")
            }
            ExifError::Failed { status, stderr } => {
                write!(f, "exiftool failed ({status}): {stderr}")
            }
            ExifError::Csv(err) => write!(f, "exiftool printed bad CSV: {err}"),
            ExifError::Json(err) => write!(f, "EXIF histogram JSON: {err}"),
            ExifError::Store(err) => write!(f, "EXIF cache: {err}"),
        }
    }
}

impl std::error::Error for ExifError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExifError::Io(err) => Some(err),
            ExifError::Launch { source, .. } => Some(source),
            ExifError::Csv(err) => Some(err),
            ExifError::Json(err) => Some(err),
            ExifError::Store(err) => Some(err),
            ExifError::ToolUnavailable | ExifError::Failed { .. } => None,
        }
    }
}

impl From<io::Error> for ExifError {
    fn from(err: io::Error) -> Self {
        ExifError::Io(err)
    }
}

impl From<csv::Error> for ExifError {
    fn from(err: csv::Error) -> Self {
        ExifError::Csv(err)
    }
}

impl From<serde_json::Error> for ExifError {
    fn from(err: serde_json::Error) -> Self {
        ExifError::Json(err)
    }
}

impl From<StoreError> for ExifError {
    fn from(err: StoreError) -> Self {
        ExifError::Store(err)
    }
}

/// Where a histogram went when it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarOutcome {
    /// Written to the sidecar at this path.
    Sidecar(PathBuf),
    /// The sidecar could not be written; the histogram is in `exif_cache`.
    Cached,
}

/// The sidecar of a raw directory: `<raw parent>/<raw name>.json`.
pub fn sidecar_path(raw_dir: &Path) -> PathBuf {
    let name = raw_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    raw_dir
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{name}.json"))
}

/// The `exif_cache` key of a raw directory.
fn cache_key(raw_dir: &Path) -> String {
    raw_dir.to_string_lossy().into_owned()
}

/// The raw files of `raw_dir` whose extension is in `raw_types` (case-insensitive), sorted by
/// name.
pub fn raw_files(raw_dir: &Path, raw_types: &[String]) -> io::Result<Vec<PathBuf>> {
    let types: Vec<String> = raw_types
        .iter()
        .map(|kind| kind.trim_start_matches('.').to_lowercase())
        .collect();
    let mut files = Vec::new();
    for entry in fs::read_dir(raw_dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if types.contains(&extension.to_lowercase()) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Splits files into batches of at most `BATCH_SIZE`, keeping their order.
pub fn batches(files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    files.chunks(BATCH_SIZE).map(<[PathBuf]>::to_vec).collect()
}

/// The histogram of the raw files of `raw_dir`: lists them, runs exiftool on the batches and
/// merges the results.
pub fn collect(
    raw_dir: &Path,
    raw_types: &[String],
    max_processes: usize,
    tool: &ResolvedTool,
) -> Result<Histogram, ExifError> {
    let command = tool.command.as_ref().ok_or(ExifError::ToolUnavailable)?;
    let files = raw_files(raw_dir, raw_types)?;
    collect_with(&batches(&files), max_processes, |batch| {
        run_exiftool(command, batch)
    })
}

/// Runs `run_batch` on every batch, with at most `max_processes` (at least one) at once, and
/// merges the CSV each returns into one histogram. `run_batch` returns exiftool's stdout.
pub fn collect_with<F>(
    batches: &[Vec<PathBuf>],
    max_processes: usize,
    run_batch: F,
) -> Result<Histogram, ExifError>
where
    F: Fn(&[PathBuf]) -> Result<String, ExifError> + Sync,
{
    let mut total = Histogram::new();
    if batches.is_empty() {
        return Ok(total);
    }
    let workers = max_processes.max(1).min(batches.len());
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let parts: Vec<Result<Histogram, ExifError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut part = Histogram::new();
                    while !failed.load(Ordering::SeqCst) {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        let Some(batch) = batches.get(index) else {
                            break;
                        };
                        let step = run_batch(batch).and_then(|out| parse_csv(&out, &mut part));
                        if let Err(err) = step {
                            failed.store(true, Ordering::SeqCst);
                            return Err(err);
                        }
                    }
                    Ok(part)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    for part in parts {
        merge_into(&mut total, part?);
    }
    Ok(total)
}

/// Adds every count of `part` to `total`.
fn merge_into(total: &mut Histogram, part: Histogram) {
    for (column, values) in part {
        let counts = total.entry(column).or_default();
        for (value, count) in values {
            *counts.entry(value).or_default() += count;
        }
    }
}

/// Whether a cell of `column` is skipped: a binary column or a binary value.
fn is_skipped(column: &str, value: &str) -> bool {
    SKIPPED_COLUMNS.contains(&column) || value.starts_with(BINARY_PREFIX)
}

/// Adds one `exiftool -csv` table to `histogram`. The first row is the header; every later row
/// is a file. Empty and skipped cells are not counted.
pub fn parse_csv(text: &str, histogram: &mut Histogram) -> Result<(), ExifError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = reader.records();
    let Some(header) = records.next().transpose()? else {
        return Ok(());
    };
    let columns: Vec<String> = header.iter().map(str::to_owned).collect();
    for record in records {
        let record = record?;
        for (column, cell) in columns.iter().zip(record.iter()) {
            if cell.is_empty() || is_skipped(column, cell) {
                continue;
            }
            *histogram
                .entry(column.clone())
                .or_default()
                .entry(cell.to_owned())
                .or_default() += 1;
        }
    }
    Ok(())
}

/// Runs `command -csv -@ <argfile>` on a batch and returns its stdout. The argfile holds one path
/// per line and lives in the system temporary directory until the run ends. Exiftool exits 1 on
/// warnings, so a non-zero exit counts as a failure only when it printed nothing.
fn run_exiftool(command: &ToolCommand, files: &[PathBuf]) -> Result<String, ExifError> {
    let mut argfile = tempfile::Builder::new()
        .prefix("trd-exif-")
        .suffix(".args")
        .tempfile()?;
    for file in files {
        argfile.write_all(file.to_string_lossy().as_bytes())?;
        argfile.write_all(b"\n")?;
    }
    argfile.flush()?;
    let output = command
        .command()
        .args(["-csv", "-@"])
        .arg(argfile.path())
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ExifError::Launch {
            program: command.program.to_string_lossy().into_owned(),
            source,
        })?;
    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ExifError::Failed {
            status: output.status.to_string(),
            stderr: stderr.lines().next().unwrap_or_default().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Writes the histogram as the sidecar: pretty JSON with sorted keys, written to a hidden sibling
/// temporary file that is then renamed over `sidecar`. When that fails, the compact JSON goes to
/// `exif_cache` under `raw_dir`, a warning is logged, and the outcome is `Cached`.
pub fn write_sidecar(
    histogram: &Histogram,
    sidecar: &Path,
    raw_dir: &Path,
    store: &Store,
    now_ms: i64,
) -> Result<SidecarOutcome, ExifError> {
    match write_atomic(histogram, sidecar) {
        Ok(()) => Ok(SidecarOutcome::Sidecar(sidecar.to_path_buf())),
        Err(err) => {
            tracing::warn!(
                raw_dir = %raw_dir.display(),
                sidecar = %sidecar.display(),
                error = %err,
                "cannot write the EXIF sidecar; the histogram is kept in the database"
            );
            let entry = ExifCacheEntry {
                raw_dir: cache_key(raw_dir),
                histogram_json: serde_json::to_string(histogram)?,
                collected_at: now_ms,
            };
            store.put_exif_cache(&entry)?;
            Ok(SidecarOutcome::Cached)
        }
    }
}

/// Writes the pretty JSON to a temporary file beside `sidecar`, syncs it and renames it into
/// place. The temporary file is removed when any step fails.
fn write_atomic(histogram: &Histogram, sidecar: &Path) -> Result<(), ExifError> {
    let parent = sidecar
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let text = serde_json::to_string_pretty(histogram)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".trd-exif-")
        .suffix(".json.tmp")
        .tempfile_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(sidecar)
        .map_err(|err| ExifError::Io(err.error))?;
    Ok(())
}

/// The histogram of a raw sequence: the sidecar when it reads, else the `exif_cache` row of
/// `raw_dir`. A sidecar that cannot be read or parsed is logged and skipped. `None` when neither
/// has a usable histogram.
pub fn read_histogram(
    sidecar: &Path,
    raw_dir: &Path,
    store: &Store,
) -> Result<Option<Histogram>, ExifError> {
    match fs::read_to_string(sidecar) {
        Ok(text) => match serde_json::from_str::<Histogram>(&text) {
            Ok(histogram) => return Ok(Some(histogram)),
            Err(err) => tracing::warn!(
                sidecar = %sidecar.display(),
                error = %err,
                "the EXIF sidecar is corrupt; using the cached histogram"
            ),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!(
            sidecar = %sidecar.display(),
            error = %err,
            "cannot read the EXIF sidecar; using the cached histogram"
        ),
    }
    let Some(entry) = store.exif_cache(&cache_key(raw_dir))? else {
        return Ok(None);
    };
    match serde_json::from_str::<Histogram>(&entry.histogram_json) {
        Ok(histogram) => Ok(Some(histogram)),
        Err(err) => {
            tracing::warn!(
                raw_dir = %raw_dir.display(),
                error = %err,
                "the cached EXIF histogram is corrupt"
            );
            Ok(None)
        }
    }
}

/// The ffmpeg metadata of a histogram: each tag with exactly one observed value, sorted by tag.
pub fn metadata(histogram: &Histogram) -> BTreeMap<String, String> {
    histogram
        .iter()
        .filter(|(_, values)| values.len() == 1)
        .filter_map(|(tag, values)| {
            values
                .keys()
                .next()
                .map(|value| (tag.clone(), value.clone()))
        })
        .collect()
}
