# 0985: Tool resolution and the fake test tools (TRD-05)

## Decisions

- `Tools::resolve(env, exe)` takes the environment as a closure and calls `resolve_with`, which takes the OS, the architecture and the version probe. Unit tests inject the probe and fake directory trees, so they need no binaries and behave the same on every OS.
- Each tool's candidates are listed in §6.15 order and the first existing file wins. A missing tool's problem lists every candidate checked, including dev-tree and `PATH` entries. An empty `TRD_*_DIR` counts as unset.
- The dev tree is the nearest ancestor `v3` directory whose parent has `v3/VERSION`. The ffmpeg and ffprobe candidate there depends on the host target. Exiftool's dev candidate is `dist/unix` or `dist/windows`.
- An exiftool file that starts with `#!` runs as `<perl> <script>`, with `/usr/bin/perl` when it exists and `perl` from `PATH` otherwise. The shebang test applies on every OS, so a Perl script still works in a test on Windows. Binaries run directly.
- The version probe runs with a 30 s limit and reports the first non-empty line. Its failures and empty output make the tool broken.
- Fake binaries read JSON from their `TRD_FAKE_*` variable, inline when it starts with `{`, else from a file. A `rules` entry overlays the base fields for the first invocation whose joined arguments contain its `match`.
- The fake ffmpeg numbers image sequences from `-start_number`, default 0, which matches ffmpeg's image2 muxer. Its thumbnail JPEG is a header-only JPEG: it is recognised by its bytes but does not decode.
- `fake-tools-dir.sh` reads `TRD_FAKE_BIN_DIR` when set, so the Rust test that runs it does not start a nested cargo build.
- The system-test guard is split into `system_test_allowed` (pure) and `require_system_test` (reads the environment and panics). The `system_test!` macro expands to `::tokio::test` in the calling crate.
