// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! SQLite store: the database used by the daemon (§6.13), and the log events of §6.14.
//!
//! The database is opened in WAL mode and migrated to the latest schema by [`Store::open`]. Every
//! typed accessor takes the connection's lock for one statement or one transaction. Timestamps
//! are milliseconds since the Unix epoch and are passed in by the caller, never read from the
//! clock here, so tests control time.

pub mod schema;

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, named_params};
use tokio::sync::broadcast;
use tracing::Level;
use uuid::Uuid;

/// One day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;
/// The newest log rows kept by [`Store::prune`].
pub const DEFAULT_LOG_MAX_ROWS: i64 = 20_000;
/// The age beyond which log rows are deleted by [`Store::prune`].
pub const DEFAULT_LOG_MAX_AGE_MS: i64 = 30 * DAY_MS;
/// A failure card is removed when it was last seen longer ago than this (§6.9).
pub const FAILURE_RETENTION_MS: i64 = 30 * DAY_MS;
/// How many live log rows a slow subscriber may lag behind before it misses some.
const LOG_CHANNEL_CAPACITY: usize = 1024;

/// Errors of the store.
#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Migration(rusqlite_migration::Error),
    Io(io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "database error: {e}"),
            StoreError::Migration(e) => write!(f, "database migration failed: {e}"),
            StoreError::Io(e) => write!(f, "database directory: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

impl From<rusqlite_migration::Error> for StoreError {
    fn from(e: rusqlite_migration::Error) -> Self {
        StoreError::Migration(e)
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// The database of the daemon. Clones share one connection and one live log channel, so a clone
/// given to the engine and one kept by the server see the same rows and the same live log.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    live: broadcast::Sender<LogRow>,
}

impl Store {
    /// Opens (creating it and its parent directories when missing) the database at `path`, sets
    /// WAL mode and migrates it to the latest schema.
    pub fn open(path: &Path) -> StoreResult<Store> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        // Returns the new mode as a row; WAL is what the daemon relies on.
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        schema::migrations().to_latest(&mut conn)?;
        let (live, _) = broadcast::channel(LOG_CHANNEL_CAPACITY);
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
            live,
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves the connection usable; keep serving.
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The schema version: the number of migrations applied.
    pub fn schema_version(&self) -> StoreResult<i64> {
        Ok(self
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// A receiver of every log row written by [`log_event`] from now on.
    pub fn subscribe_logs(&self) -> broadcast::Receiver<LogRow> {
        self.live.subscribe()
    }

    // settings

    /// The value stored under `key`, if any.
    pub fn setting(&self, key: &str) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Stores `value` under `key`, replacing any earlier value.
    pub fn set_setting(&self, key: &str, value: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }

    /// Removes the setting `key`; a missing key is not an error.
    pub fn clear_setting(&self, key: &str) -> StoreResult<()> {
        self.conn()
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    /// The `config_path` setting: the config file chosen with `SetConfigPath`.
    pub fn config_path(&self) -> StoreResult<Option<PathBuf>> {
        Ok(self.setting("config_path")?.map(PathBuf::from))
    }

    /// Stores the `config_path` setting.
    pub fn set_config_path(&self, path: &str) -> StoreResult<()> {
        self.set_setting("config_path", path)
    }

    /// The `global_pause` setting; false when never set.
    pub fn global_pause(&self) -> StoreResult<bool> {
        self.bool_setting("global_pause")
    }

    /// Stores the `global_pause` setting.
    pub fn set_global_pause(&self, paused: bool) -> StoreResult<()> {
        self.set_bool_setting("global_pause", paused)
    }

    /// The `swift_history_imported` setting; false when never set.
    pub fn swift_history_imported(&self) -> StoreResult<bool> {
        self.bool_setting("swift_history_imported")
    }

    /// Stores the `swift_history_imported` setting.
    pub fn set_swift_history_imported(&self, imported: bool) -> StoreResult<()> {
        self.set_bool_setting("swift_history_imported", imported)
    }

    fn bool_setting(&self, key: &str) -> StoreResult<bool> {
        Ok(self.setting(key)?.as_deref() == Some("true"))
    }

    fn set_bool_setting(&self, key: &str, value: bool) -> StoreResult<()> {
        self.set_setting(key, if value { "true" } else { "false" })
    }

    /// The identity of this daemon: a UUID v4 generated the first time and kept afterwards.
    pub fn server_id(&self) -> StoreResult<String> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('server_id', ?1)",
            [Uuid::new_v4().to_string()],
        )?;
        let id = tx.query_row(
            "SELECT value FROM settings WHERE key = 'server_id'",
            [],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(id)
    }

    // render_jobs

    /// Inserts `job`, or replaces the row with the same id.
    pub fn put_render_job(&self, job: &RenderJob) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO render_jobs (
                id, sequence_name, export_path, variant_key, output_filename, output_path,
                partial_path, plan_json, status, suspended_by, pid, pid_start_time, queued_at,
                started_at, finished_at, progress_path, log_path, adopted, \"foreign\"
            ) VALUES (
                :id, :sequence_name, :export_path, :variant_key, :output_filename, :output_path,
                :partial_path, :plan_json, :status, :suspended_by, :pid, :pid_start_time,
                :queued_at, :started_at, :finished_at, :progress_path, :log_path, :adopted,
                :foreign
            )",
            named_params! {
                ":id": job.id,
                ":sequence_name": job.sequence_name,
                ":export_path": job.export_path,
                ":variant_key": job.variant_key,
                ":output_filename": job.output_filename,
                ":output_path": job.output_path,
                ":partial_path": job.partial_path,
                ":plan_json": job.plan_json,
                ":status": job.status.as_str(),
                ":suspended_by": job.suspended_by.map(SuspendedBy::as_str),
                ":pid": job.pid,
                ":pid_start_time": job.pid_start_time,
                ":queued_at": job.queued_at,
                ":started_at": job.started_at,
                ":finished_at": job.finished_at,
                ":progress_path": job.progress_path,
                ":log_path": job.log_path,
                ":adopted": job.adopted,
                ":foreign": job.foreign,
            },
        )?;
        Ok(())
    }

    /// The render job with `id`.
    pub fn render_job(&self, id: &str) -> StoreResult<Option<RenderJob>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{RENDER_JOB_SELECT} WHERE id = ?1"),
                [id],
                render_job_from_row,
            )
            .optional()?)
    }

    /// Render jobs in queue order, optionally only those with `status`.
    pub fn render_jobs(&self, status: Option<JobStatus>) -> StoreResult<Vec<RenderJob>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{RENDER_JOB_SELECT} WHERE (?1 IS NULL OR status = ?1) ORDER BY queued_at, id"
        ))?;
        let rows = stmt.query_map([status.map(JobStatus::as_str)], render_job_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // rendered_videos

    /// Inserts `video`, or replaces the row with the same id.
    pub fn put_rendered_video(&self, video: &RenderedVideo) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO rendered_videos (
                id, rendered_at, sequence_name, export_path, output_filename, output_path,
                variant_key, codec, resolution_label, frame_rate, frame_count, size_bytes, width,
                height, duration_seconds, render_seconds, thumbnail_path, xmp_embedded, source
            ) VALUES (
                :id, :rendered_at, :sequence_name, :export_path, :output_filename, :output_path,
                :variant_key, :codec, :resolution_label, :frame_rate, :frame_count, :size_bytes,
                :width, :height, :duration_seconds, :render_seconds, :thumbnail_path,
                :xmp_embedded, :source
            )",
            named_params! {
                ":id": video.id,
                ":rendered_at": video.rendered_at,
                ":sequence_name": video.sequence_name,
                ":export_path": video.export_path,
                ":output_filename": video.output_filename,
                ":output_path": video.output_path,
                ":variant_key": video.variant_key,
                ":codec": video.codec,
                ":resolution_label": video.resolution_label,
                ":frame_rate": video.frame_rate,
                ":frame_count": video.frame_count,
                ":size_bytes": video.size_bytes,
                ":width": video.width,
                ":height": video.height,
                ":duration_seconds": video.duration_seconds,
                ":render_seconds": video.render_seconds,
                ":thumbnail_path": video.thumbnail_path,
                ":xmp_embedded": video.xmp_embedded,
                ":source": video.source,
            },
        )?;
        Ok(())
    }

    /// The rendered video with `id`.
    pub fn rendered_video(&self, id: &str) -> StoreResult<Option<RenderedVideo>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{RENDERED_VIDEO_SELECT} WHERE id = ?1"),
                [id],
                rendered_video_from_row,
            )
            .optional()?)
    }

    /// Every rendered video, newest first.
    pub fn rendered_videos(&self) -> StoreResult<Vec<RenderedVideo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{RENDERED_VIDEO_SELECT} ORDER BY rendered_at DESC, id"
        ))?;
        let rows = stmt.query_map([], rendered_video_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // failures

    /// Records one occurrence of a failure. A stored failure with the same output path and kind
    /// is updated instead: its id and first-seen time stay, its occurrence count goes up by one,
    /// and the other fields take the new values. Returns the row as stored.
    pub fn record_failure(&self, failure: &Failure) -> StoreResult<Failure> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let stored = match tx
            .query_row(
                "SELECT id, occurrences, first_seen FROM failures WHERE output_path = ?1 AND kind = ?2",
                [&failure.output_path, &failure.kind],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()?
        {
            Some((id, occurrences, first_seen)) => Failure {
                id,
                occurrences: occurrences + 1,
                first_seen,
                ..failure.clone()
            },
            None => failure.clone(),
        };
        tx.execute(
            "INSERT OR REPLACE INTO failures (
                id, output_path, kind, first_seen, last_seen, occurrences, sequence_name,
                export_path, output_filename, variant_key, summary, detail, exit_code,
                command_line, tool_output_json, variant_auto_paused, render_id
            ) VALUES (
                :id, :output_path, :kind, :first_seen, :last_seen, :occurrences, :sequence_name,
                :export_path, :output_filename, :variant_key, :summary, :detail, :exit_code,
                :command_line, :tool_output_json, :variant_auto_paused, :render_id
            )",
            named_params! {
                ":id": stored.id,
                ":output_path": stored.output_path,
                ":kind": stored.kind,
                ":first_seen": stored.first_seen,
                ":last_seen": stored.last_seen,
                ":occurrences": stored.occurrences,
                ":sequence_name": stored.sequence_name,
                ":export_path": stored.export_path,
                ":output_filename": stored.output_filename,
                ":variant_key": stored.variant_key,
                ":summary": stored.summary,
                ":detail": stored.detail,
                ":exit_code": stored.exit_code,
                ":command_line": stored.command_line,
                ":tool_output_json": stored.tool_output_json,
                ":variant_auto_paused": stored.variant_auto_paused,
                ":render_id": stored.render_id,
            },
        )?;
        tx.commit()?;
        Ok(stored)
    }

    /// The failure with `id`.
    pub fn failure(&self, id: &str) -> StoreResult<Option<Failure>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{FAILURE_SELECT} WHERE id = ?1"),
                [id],
                failure_from_row,
            )
            .optional()?)
    }

    /// Every failure, most recently seen first.
    pub fn failures(&self) -> StoreResult<Vec<Failure>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{FAILURE_SELECT} ORDER BY last_seen DESC, id"))?;
        let rows = stmt.query_map([], failure_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Removes the failure with `id` (Dismiss). Returns whether a row was removed.
    pub fn delete_failure(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM failures WHERE id = ?1", [id])?
            > 0)
    }

    /// Removes every failure of `output_path` (a successful render of that output). Returns how
    /// many rows were removed.
    pub fn delete_failures_for_output(&self, output_path: &str) -> StoreResult<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM failures WHERE output_path = ?1", [output_path])?)
    }

    // variant_strikes

    /// The corrupt-strike count of a variant; 0 when none is stored.
    pub fn corrupt_strikes(&self, sequence_name: &str, variant_key: &str) -> StoreResult<i64> {
        Ok(self
            .conn()
            .query_row(
                "SELECT corrupt_strikes FROM variant_strikes WHERE sequence_name = ?1 AND variant_key = ?2",
                [sequence_name, variant_key],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// Stores the corrupt-strike count of a variant.
    pub fn set_corrupt_strikes(
        &self,
        sequence_name: &str,
        variant_key: &str,
        strikes: i64,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO variant_strikes (sequence_name, variant_key, corrupt_strikes)
             VALUES (?1, ?2, ?3)",
            (sequence_name, variant_key, strikes),
        )?;
        Ok(())
    }

    // exif_cache

    /// Inserts or replaces the histogram cached for a raw directory.
    pub fn put_exif_cache(&self, entry: &ExifCacheEntry) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO exif_cache (raw_dir, histogram_json, collected_at)
             VALUES (?1, ?2, ?3)",
            (&entry.raw_dir, &entry.histogram_json, entry.collected_at),
        )?;
        Ok(())
    }

    /// The cached histogram of a raw directory.
    pub fn exif_cache(&self, raw_dir: &str) -> StoreResult<Option<ExifCacheEntry>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT raw_dir, histogram_json, collected_at FROM exif_cache WHERE raw_dir = ?1",
                [raw_dir],
                |row| {
                    Ok(ExifCacheEntry {
                        raw_dir: row.get("raw_dir")?,
                        histogram_json: row.get("histogram_json")?,
                        collected_at: row.get("collected_at")?,
                    })
                },
            )
            .optional()?)
    }

    // devices

    /// Inserts `device`, or replaces the row with the same id.
    pub fn put_device(&self, device: &Device) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO devices (id, name, platform, token_sha256, paired_at, last_seen, revoked)
             VALUES (:id, :name, :platform, :token_sha256, :paired_at, :last_seen, :revoked)",
            named_params! {
                ":id": device.id,
                ":name": device.name,
                ":platform": device.platform,
                ":token_sha256": device.token_sha256,
                ":paired_at": device.paired_at,
                ":last_seen": device.last_seen,
                ":revoked": device.revoked,
            },
        )?;
        Ok(())
    }

    /// The device with `id`.
    pub fn device(&self, id: &str) -> StoreResult<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{DEVICE_SELECT} WHERE id = ?1"),
                [id],
                device_from_row,
            )
            .optional()?)
    }

    /// The device whose token hashes to `token_sha256`.
    pub fn device_by_token_hash(&self, token_sha256: &str) -> StoreResult<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{DEVICE_SELECT} WHERE token_sha256 = ?1"),
                [token_sha256],
                device_from_row,
            )
            .optional()?)
    }

    /// Every device, oldest pairing first.
    pub fn devices(&self) -> StoreResult<Vec<Device>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{DEVICE_SELECT} ORDER BY paired_at, id"))?;
        let rows = stmt.query_map([], device_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // log

    /// Inserts a log row stamped `at` and returns it with its sequence number. Use [`log_event`]
    /// to also write `tracing` and publish the row.
    pub fn insert_log(
        &self,
        at: i64,
        level: LogLevel,
        subject: &str,
        message: &str,
        fields: &LogFields,
    ) -> StoreResult<LogRow> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO log (at, level, subject, message, event, sequence_name, output_filename,
                rendered_video_id, failure_id, video_count, total_bytes)
             VALUES (:at, :level, :subject, :message, :event, :sequence_name, :output_filename,
                :rendered_video_id, :failure_id, :video_count, :total_bytes)",
            named_params! {
                ":at": at,
                ":level": level.as_str(),
                ":subject": subject,
                ":message": message,
                ":event": fields.event,
                ":sequence_name": fields.sequence_name,
                ":output_filename": fields.output_filename,
                ":rendered_video_id": fields.rendered_video_id,
                ":failure_id": fields.failure_id,
                ":video_count": fields.video_count,
                ":total_bytes": fields.total_bytes,
            },
        )?;
        Ok(LogRow {
            seq: conn.last_insert_rowid(),
            at,
            level,
            subject: subject.to_owned(),
            message: message.to_owned(),
            fields: fields.clone(),
        })
    }

    /// The stored log rows with `seq > after`, oldest first.
    pub fn log_rows_after(&self, after: i64) -> StoreResult<Vec<LogRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT seq, at, level, subject, message, event, sequence_name, output_filename,
                rendered_video_id, failure_id, video_count, total_bytes
             FROM log WHERE seq > ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([after], log_row_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Up to `limit` stored log rows at `min_level` or above, newest first. `before` is the
    /// sequence number to page from: only rows with `seq < before` are taken; `0` means the newest.
    pub fn log_rows_before(
        &self,
        before: i64,
        min_level: LogLevel,
        limit: i64,
    ) -> StoreResult<Vec<LogRow>> {
        // The level names are fixed strings from LogLevel, so they are written into the statement.
        let levels: Vec<String> = [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ]
        .into_iter()
        .filter(|level| *level >= min_level)
        .map(|level| format!("'{}'", level.as_str()))
        .collect();
        if levels.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT seq, at, level, subject, message, event, sequence_name, output_filename,
                rendered_video_id, failure_id, video_count, total_bytes
             FROM log WHERE (?1 <= 0 OR seq < ?1) AND level IN ({})
             ORDER BY seq DESC LIMIT ?2",
            levels.join(", ")
        ))?;
        let rows = stmt.query_map([before, limit], log_row_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // retention

    /// Applies the retention limits of §6.13 with the default log limits (20 000 rows, 30 days).
    /// Returns the thumbnail paths of the deleted rendered videos; the caller deletes the files.
    pub fn prune(&self, now_ms: i64, history_days: i64) -> StoreResult<Vec<String>> {
        self.prune_with(
            now_ms,
            history_days,
            DEFAULT_LOG_MAX_ROWS,
            DEFAULT_LOG_MAX_AGE_MS,
        )
    }

    /// Deletes `rendered_videos` rows older than `history_days` days, `failures` cards not seen for
    /// [`FAILURE_RETENTION_MS`], and `log` rows older than `log_max_age_ms` or beyond the newest
    /// `log_max_rows`. Returns the thumbnail paths of the deleted videos. The sequence numbers of
    /// `log` never go back, even after the newest row is deleted.
    pub fn prune_with(
        &self,
        now_ms: i64,
        history_days: i64,
        log_max_rows: i64,
        log_max_age_ms: i64,
    ) -> StoreResult<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let history_cutoff = now_ms - history_days * DAY_MS;
        let thumbnails = {
            let mut stmt = tx.prepare(
                "SELECT thumbnail_path FROM rendered_videos
                 WHERE rendered_at < ?1 AND thumbnail_path IS NOT NULL ORDER BY rendered_at, id",
            )?;
            let rows = stmt.query_map([history_cutoff], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()?
        };
        tx.execute(
            "DELETE FROM rendered_videos WHERE rendered_at < ?1",
            [history_cutoff],
        )?;
        tx.execute(
            "DELETE FROM failures WHERE last_seen < ?1",
            [now_ms - FAILURE_RETENTION_MS],
        )?;
        tx.execute("DELETE FROM log WHERE at < ?1", [now_ms - log_max_age_ms])?;
        // The row at offset `log_max_rows` is the oldest one kept: delete it and everything older.
        tx.execute(
            "DELETE FROM log WHERE seq <= (SELECT seq FROM log ORDER BY seq DESC LIMIT 1 OFFSET ?1)",
            [log_max_rows],
        )?;
        tx.commit()?;
        Ok(thumbnails)
    }
}

/// Writes one log event: a `tracing` record and a `log` row carrying the same fields, then
/// publishes the row to the live subscribers. Returns the row as stored.
pub fn log_event(
    store: &Store,
    now_ms: i64,
    level: LogLevel,
    subject: &str,
    message: &str,
    fields: &LogFields,
) -> StoreResult<LogRow> {
    let row = store.insert_log(now_ms, level, subject, message, fields)?;
    let seq = row.seq;
    let f = &row.fields;
    macro_rules! emit {
        ($level:expr) => {
            tracing::event!(
                target: "trd",
                $level,
                seq = seq,
                subject = subject,
                message = message,
                event = f.event.as_deref(),
                sequence_name = f.sequence_name.as_deref(),
                output_filename = f.output_filename.as_deref(),
                rendered_video_id = f.rendered_video_id.as_deref(),
                failure_id = f.failure_id.as_deref(),
                video_count = f.video_count,
                total_bytes = f.total_bytes,
            )
        };
    }
    match level {
        LogLevel::Debug => emit!(Level::DEBUG),
        LogLevel::Info => emit!(Level::INFO),
        LogLevel::Warn => emit!(Level::WARN),
        LogLevel::Error => emit!(Level::ERROR),
    }
    // No subscriber is not an error: the row is stored either way.
    let _ = store.live.send(row.clone());
    Ok(row)
}

// Records

/// Status of a render job (§6.13 `render_jobs.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Suspended,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Suspended => "suspended",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<JobStatus> {
        Some(match value {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "suspended" => JobStatus::Suspended,
            "done" => JobStatus::Done,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            _ => return None,
        })
    }
}

/// Who suspended a render job (§6.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendedBy {
    User,
    Global,
}

impl SuspendedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            SuspendedBy::User => "user",
            SuspendedBy::Global => "global",
        }
    }

    pub fn parse(value: &str) -> Option<SuspendedBy> {
        Some(match value {
            "user" => SuspendedBy::User,
            "global" => SuspendedBy::Global,
            _ => return None,
        })
    }
}

/// Level of a log row (§6.14), in increasing severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    pub fn parse(value: &str) -> Option<LogLevel> {
        Some(match value {
            "DEBUG" => LogLevel::Debug,
            "INFO" => LogLevel::Info,
            "WARN" => LogLevel::Warn,
            "ERROR" => LogLevel::Error,
            _ => return None,
        })
    }
}

/// A row of `render_jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderJob {
    pub id: String,
    pub sequence_name: String,
    pub export_path: String,
    pub variant_key: String,
    pub output_filename: String,
    pub output_path: String,
    pub partial_path: String,
    pub plan_json: String,
    pub status: JobStatus,
    pub suspended_by: Option<SuspendedBy>,
    pub pid: Option<i64>,
    pub pid_start_time: Option<i64>,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub progress_path: Option<String>,
    pub log_path: Option<String>,
    pub adopted: bool,
    pub foreign: bool,
}

const RENDER_JOB_SELECT: &str =
    "SELECT id, sequence_name, export_path, variant_key, output_filename,
    output_path, partial_path, plan_json, status, suspended_by, pid, pid_start_time, queued_at,
    started_at, finished_at, progress_path, log_path, adopted, \"foreign\" FROM render_jobs";

fn render_job_from_row(row: &Row<'_>) -> rusqlite::Result<RenderJob> {
    let status: String = row.get("status")?;
    let suspended_by: Option<String> = row.get("suspended_by")?;
    Ok(RenderJob {
        id: row.get("id")?,
        sequence_name: row.get("sequence_name")?,
        export_path: row.get("export_path")?,
        variant_key: row.get("variant_key")?,
        output_filename: row.get("output_filename")?,
        output_path: row.get("output_path")?,
        partial_path: row.get("partial_path")?,
        plan_json: row.get("plan_json")?,
        status: JobStatus::parse(&status).ok_or_else(|| bad_value("status", &status))?,
        suspended_by: suspended_by
            .map(|s| SuspendedBy::parse(&s).ok_or_else(|| bad_value("suspended_by", &s)))
            .transpose()?,
        pid: row.get("pid")?,
        pid_start_time: row.get("pid_start_time")?,
        queued_at: row.get("queued_at")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        progress_path: row.get("progress_path")?,
        log_path: row.get("log_path")?,
        adopted: row.get("adopted")?,
        foreign: row.get("foreign")?,
    })
}

/// A row of `rendered_videos`.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedVideo {
    pub id: String,
    pub rendered_at: i64,
    pub sequence_name: String,
    pub export_path: String,
    pub output_filename: String,
    pub output_path: String,
    pub variant_key: String,
    pub codec: String,
    pub resolution_label: String,
    pub frame_rate: i64,
    pub frame_count: i64,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
    pub duration_seconds: f64,
    pub render_seconds: f64,
    pub thumbnail_path: Option<String>,
    pub xmp_embedded: bool,
    pub source: String,
}

const RENDERED_VIDEO_SELECT: &str = "SELECT id, rendered_at, sequence_name, export_path,
    output_filename, output_path, variant_key, codec, resolution_label, frame_rate, frame_count,
    size_bytes, width, height, duration_seconds, render_seconds, thumbnail_path, xmp_embedded,
    source FROM rendered_videos";

fn rendered_video_from_row(row: &Row<'_>) -> rusqlite::Result<RenderedVideo> {
    Ok(RenderedVideo {
        id: row.get("id")?,
        rendered_at: row.get("rendered_at")?,
        sequence_name: row.get("sequence_name")?,
        export_path: row.get("export_path")?,
        output_filename: row.get("output_filename")?,
        output_path: row.get("output_path")?,
        variant_key: row.get("variant_key")?,
        codec: row.get("codec")?,
        resolution_label: row.get("resolution_label")?,
        frame_rate: row.get("frame_rate")?,
        frame_count: row.get("frame_count")?,
        size_bytes: row.get("size_bytes")?,
        width: row.get("width")?,
        height: row.get("height")?,
        duration_seconds: row.get("duration_seconds")?,
        render_seconds: row.get("render_seconds")?,
        thumbnail_path: row.get("thumbnail_path")?,
        xmp_embedded: row.get("xmp_embedded")?,
        source: row.get("source")?,
    })
}

/// A row of `failures`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub id: String,
    pub output_path: String,
    pub kind: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub occurrences: i64,
    pub sequence_name: String,
    pub export_path: String,
    pub output_filename: String,
    pub variant_key: String,
    pub summary: String,
    pub detail: String,
    pub exit_code: Option<i64>,
    pub command_line: String,
    pub tool_output_json: String,
    pub variant_auto_paused: bool,
    pub render_id: Option<String>,
}

const FAILURE_SELECT: &str = "SELECT id, output_path, kind, first_seen, last_seen, occurrences,
    sequence_name, export_path, output_filename, variant_key, summary, detail, exit_code,
    command_line, tool_output_json, variant_auto_paused, render_id FROM failures";

fn failure_from_row(row: &Row<'_>) -> rusqlite::Result<Failure> {
    Ok(Failure {
        id: row.get("id")?,
        output_path: row.get("output_path")?,
        kind: row.get("kind")?,
        first_seen: row.get("first_seen")?,
        last_seen: row.get("last_seen")?,
        occurrences: row.get("occurrences")?,
        sequence_name: row.get("sequence_name")?,
        export_path: row.get("export_path")?,
        output_filename: row.get("output_filename")?,
        variant_key: row.get("variant_key")?,
        summary: row.get("summary")?,
        detail: row.get("detail")?,
        exit_code: row.get("exit_code")?,
        command_line: row.get("command_line")?,
        tool_output_json: row.get("tool_output_json")?,
        variant_auto_paused: row.get("variant_auto_paused")?,
        render_id: row.get("render_id")?,
    })
}

/// A row of `exif_cache`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExifCacheEntry {
    pub raw_dir: String,
    pub histogram_json: String,
    pub collected_at: i64,
}

/// A row of `devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub token_sha256: String,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub revoked: bool,
}

const DEVICE_SELECT: &str =
    "SELECT id, name, platform, token_sha256, paired_at, last_seen, revoked FROM devices";

fn device_from_row(row: &Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get("id")?,
        name: row.get("name")?,
        platform: row.get("platform")?,
        token_sha256: row.get("token_sha256")?,
        paired_at: row.get("paired_at")?,
        last_seen: row.get("last_seen")?,
        revoked: row.get("revoked")?,
    })
}

/// The structured fields of a log row, besides its level, subject and message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFields {
    pub event: Option<String>,
    pub sequence_name: Option<String>,
    pub output_filename: Option<String>,
    pub rendered_video_id: Option<String>,
    pub failure_id: Option<String>,
    pub video_count: Option<i64>,
    pub total_bytes: Option<i64>,
}

/// A row of `log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRow {
    pub seq: i64,
    pub at: i64,
    pub level: LogLevel,
    pub subject: String,
    pub message: String,
    pub fields: LogFields,
}

fn log_row_from_row(row: &Row<'_>) -> rusqlite::Result<LogRow> {
    let level: String = row.get("level")?;
    Ok(LogRow {
        seq: row.get("seq")?,
        at: row.get("at")?,
        level: LogLevel::parse(&level).ok_or_else(|| bad_value("level", &level))?,
        subject: row.get("subject")?,
        message: row.get("message")?,
        fields: LogFields {
            event: row.get("event")?,
            sequence_name: row.get("sequence_name")?,
            output_filename: row.get("output_filename")?,
            rendered_video_id: row.get("rendered_video_id")?,
            failure_id: row.get("failure_id")?,
            video_count: row.get("video_count")?,
            total_bytes: row.get("total_bytes")?,
        },
    })
}

/// A conversion error for a stored value that no enum variant matches.
fn bad_value(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        Type::Text,
        Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{column}: unknown value {value:?}"),
        )),
    )
}
