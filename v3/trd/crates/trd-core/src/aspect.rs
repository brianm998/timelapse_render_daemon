// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Aspect ratio strings for output files (§6.5).

/// The `-aspect` value for a `width`×`height` frame, ported from the Swift `AspectRatio.string`.
///
/// A height of zero or less gives `w/h`. A width that is a whole multiple of the height gives
/// `n/1`. Otherwise the smallest multiplier `m` in 2..=1000 that makes `w/h × m` a whole number
/// gives `n/m`. Failing that, `w/h`.
pub fn aspect_string(width: i64, height: i64) -> String {
    if height <= 0 {
        return format!("{width}/{height}");
    }
    if width % height == 0 {
        return format!("{}/1", width / height);
    }
    let ratio = width as f64 / height as f64;
    for m in 2..=1000_i64 {
        let lhs = ratio * m as f64;
        if (lhs - lhs.round()).abs() < 1e-9 {
            return format!("{}/{m}", lhs.round() as i64);
        }
    }
    format!("{width}/{height}")
}
