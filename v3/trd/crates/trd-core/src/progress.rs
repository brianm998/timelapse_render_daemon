// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Parsing ffmpeg's -progress file (§6.6).
//!
//! ffmpeg appends `key=value` lines to the `-progress` file and ends each block with
//! `progress=continue` or `progress=end`. [`ProgressReader`] reads only the bytes appended since its
//! last poll, keeps an incomplete last line for the next poll, and reports the latest complete block.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One complete block of the `-progress` file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressBlock {
    /// The frame count so far (0 when the block did not say).
    pub frame: u64,
    /// The encoding rate in frames per second (0.0 when the block did not say).
    pub fps: f64,
    pub out_time_us: Option<u64>,
    pub total_size: Option<u64>,
    /// True for a block that ends with `progress=end`.
    pub ended: bool,
}

/// The values of the block being read, before its `progress=` line completes it.
#[derive(Debug, Default)]
struct Pending {
    frame: Option<u64>,
    fps: Option<f64>,
    out_time_us: Option<u64>,
    total_size: Option<u64>,
}

/// Follows a `-progress` file as ffmpeg writes it.
#[derive(Debug)]
pub struct ProgressReader {
    path: PathBuf,
    /// How many bytes of the file have been read.
    offset: u64,
    /// An incomplete last line, kept until its newline arrives.
    partial: Vec<u8>,
    pending: Pending,
    latest: Option<ProgressBlock>,
}

impl ProgressReader {
    pub fn new(path: impl Into<PathBuf>) -> ProgressReader {
        ProgressReader {
            path: path.into(),
            offset: 0,
            partial: Vec::new(),
            pending: Pending::default(),
            latest: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The latest complete block read so far.
    pub fn latest(&self) -> Option<ProgressBlock> {
        self.latest
    }

    /// Reads what ffmpeg appended since the last poll and returns the latest complete block. A
    /// missing file (ffmpeg has not started writing) gives the block read so far, `None` at first.
    pub fn poll(&mut self) -> io::Result<Option<ProgressBlock>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(self.latest),
            Err(err) => return Err(err),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            // The file was replaced or truncated: start again from its beginning.
            self.offset = 0;
            self.partial.clear();
            self.pending = Pending::default();
            self.latest = None;
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut appended = Vec::new();
        file.read_to_end(&mut appended)?;
        self.offset += appended.len() as u64;
        self.feed(&appended);
        Ok(self.latest)
    }

    /// Splits the bytes into lines. The text after the last newline waits for the next read.
    fn feed(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(bytes);
        let mut start = 0;
        for (index, byte) in data.iter().enumerate() {
            if *byte == b'\n' {
                self.feed_line(&data[start..index]);
                start = index + 1;
            }
        }
        self.partial = data[start..].to_vec();
    }

    /// Takes one `key=value` line. Other lines (ffmpeg's own messages) are ignored.
    fn feed_line(&mut self, raw: &[u8]) {
        let text = String::from_utf8_lossy(raw);
        let Some((key, value)) = text.trim().split_once('=') else {
            return;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "frame" => self.pending.frame = value.parse().ok(),
            "fps" => self.pending.fps = value.parse().ok(),
            "out_time_us" => self.pending.out_time_us = value.parse().ok(),
            "total_size" => self.pending.total_size = value.parse().ok(),
            "progress" => {
                let pending = std::mem::take(&mut self.pending);
                self.latest = Some(ProgressBlock {
                    frame: pending.frame.unwrap_or(0),
                    fps: pending.fps.unwrap_or(0.0),
                    out_time_us: pending.out_time_us,
                    total_size: pending.total_size,
                    ended: value == "end",
                });
            }
            _ => {}
        }
    }
}

/// Seconds left to encode: (total − frame) ÷ fps when fps is above zero, else -1 (§6.6).
pub fn eta_seconds(total_frames: u64, block: &ProgressBlock) -> i64 {
    if !block.fps.is_finite() || block.fps <= 0.0 {
        return -1;
    }
    let remaining = total_frames.saturating_sub(block.frame) as f64;
    (remaining / block.fps) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `text` to a reader and returns the latest block.
    fn read(text: &str) -> Option<ProgressBlock> {
        let mut reader = ProgressReader::new("unused");
        reader.feed(text.as_bytes());
        reader.latest()
    }

    #[test]
    fn parses_a_progress_block() {
        // The key=value form of a block ffmpeg writes for `frame=1569 fps=0.9`.
        let block = read(
            "frame=1569\nfps=0.9\ntotal_size=286475\nout_time_us=52260000\nspeed=0.0292x\nprogress=continue\n",
        )
        .expect("a complete block");
        assert_eq!(block.frame, 1569);
        assert_eq!(block.fps, 0.9);
        assert_eq!(block.out_time_us, Some(52_260_000));
        assert_eq!(block.total_size, Some(286_475));
        assert!(!block.ended);
    }

    #[test]
    fn parses_padded_values() {
        // ffmpeg may pad values after the `=`, as the old fixed-width format did.
        let block = read("frame=   30\nfps=0.00\nprogress=continue\n").expect("a complete block");
        assert_eq!(block.frame, 30);
        assert_eq!(block.fps, 0.0);
    }

    #[test]
    fn ignores_lines_that_are_not_progress() {
        assert_eq!(read("Stream mapping:\n  Press [q] to stop\n\n"), None);
    }

    #[test]
    fn end_block_is_marked_ended() {
        let block = read("frame=9\nfps=25.00\nprogress=end\n").expect("a complete block");
        assert!(block.ended);
        assert_eq!(block.frame, 9);
    }

    #[test]
    fn partial_line_waits_for_its_newline() {
        let mut reader = ProgressReader::new("unused");
        reader.feed(b"frame=5\nfps=1.0\nprogr");
        assert_eq!(reader.latest(), None, "the block is not complete yet");
        reader.feed(b"ess=end\n");
        let block = reader.latest().expect("the block completes");
        assert_eq!(block.frame, 5);
        assert!(block.ended);
    }

    #[test]
    fn eta_is_minus_one_without_fps() {
        let stalled = ProgressBlock {
            frame: 10,
            fps: 0.0,
            out_time_us: None,
            total_size: None,
            ended: false,
        };
        assert_eq!(eta_seconds(100, &stalled), -1);
        let moving = ProgressBlock {
            fps: 30.0,
            ..stalled
        };
        assert_eq!(eta_seconds(100, &moving), 3);
    }
}
