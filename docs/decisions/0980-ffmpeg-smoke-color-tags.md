# 0980: what the ffmpeg smoke test asserts about colour tags (TRD-22)

## Decision

The exact §6.5 argument vector (`-color_range pc -color_primaries bt709 -colorspace bt709
-color_trc bt709` with `-movflags +write_colr`) does not produce `color_primaries=bt709` for any
codec, nor `color_range=pc` for prores_ks, on FFmpeg 8.1 or 9.0.2. The owner decided the smoke
test asserts what the vector really produces:

- colorspace `bt709` on every output;
- color range `pc` for libx264 and libx265, `tv` for prores_ks;
- primaries and transfer are printed (`info` lines), not asserted.

The vector itself is unchanged; whether trd should tag primaries and transfer differently is a
spec question, not part of this ticket.

## Other smoke-test choices

- H.264/HEVC decoders report full-range 4:2:0 as `yuvj420p`; the test reads it as `yuv420p` and
  the range check confirms full range.
- The `tmcd` track has no decoder, so it is found by its codec tag, not its codec name.
- `lipo` is called as `/usr/bin/lipo` (MacPorts' `lipo` lacks `-archs`), and macOS builds put
  Apple's tools first on PATH.
