# The sandbox (`make develop`)

`sandbox.py` builds a complete, isolated `trd` tree with generated sequences in every state
(spec 1 §12). It uses only the Python standard library and runs with `python3 -I`.

## Commands

```
python3 -I v3/trd/sandbox/sandbox.py init [--dir PATH] [--reset] [--check]
python3 -I v3/trd/sandbox/sandbox.py drip [--dir PATH] [--fps N]
python3 -I v3/trd/sandbox/sandbox.py path [--dir PATH]
```

- `init` builds the tree. A valid tree that is already there is left alone. `--reset` recreates it,
  but only a tree holding the `.trd-sandbox` marker is removed. A non-empty folder without the marker is
  refused. `--check` validates the tree and changes nothing.
- `drip` copies sequence A's raw frames into its export folder at `--fps` frames a second (default 2),
  until all 60 are there. Each frame is copied under a hidden name and renamed, so the daemon never
  sees a half-written frame.
- `path` prints the folder.

Without `--dir` the folder is `$TMPDIR/trd-sandbox-<first 8 hex of the SHA-1 of the worktree path>`.

Tools are found as in spec 1 §6.15: `TRD_FFMPEG_DIR` or `TRD_EXIFTOOL_DIR`, then
`v3/ffmpeg/dist/<target>` and `v3/exiftool/dist/unix|windows`, then `PATH`. The sandbox needs ffmpeg,
ffprobe and exiftool.

## The tree

| Folder | Contents |
|---|---|
| `raw/` | TIFF frames (320×213, `testsrc2`) with EXIF from exiftool. Sequences A 60, B 30, C 24, D 20, E 12, G 12 |
| `backup/` | `2026_01_01-sandbox-f.json`: F has no raw folder; its `ImageSize` count is 16 |
| `export/` | `LRT_<name>` folders with `LRT_00001.tif` … |
| `master/` | D's three outputs, so D is already rendered and never shown |
| `data/`, `logs/` | the daemon's database and log |
| `config.json`, `video_codecs.json` | the config and the built-in codec table |

| Sequence | What the daemon does with it |
|---|---|
| A `2026_01_01-sandbox-a` | export starts empty; `drip` fills it gradually |
| B `2026_01_01-sandbox-b` | fully exported; renders its three variants |
| C `2026_01_01-sandbox-c` | fully exported; frame 12 is truncated to 100 bytes; fails |
| D `2026_01_01-sandbox-d` | every output is in `master/`; never shown |
| E `2026_01_01-sandbox-e-aligned` | ignored by `-aligned$` |
| F `2026_01_01-sandbox-f` | backup sidecar only; 16 frames exported |
| G `2026_01_01-sandbox-g-merge` | fully exported; renders, then its export folder is deleted (`-merge$`) |

## Running the daemon

```
make -C v3/trd develop            # the sandbox daemon, with drip in the background
make -C v3/trd develop-daemon     # the sandbox daemon alone (the clients' develop targets use this)
make -C v3/trd run                # the real daemon with no overrides; refuses if 50071 is taken
```

| Target | Port | Notes |
|---|---|---|
| `develop`, `develop-daemon` | `--port 50081`, LAN `0.0.0.0:50082`, mDNS name `trd-sandbox` | refuse to start when 50081 is taken |
| `run` | the defaults: 50071 and LAN 50072 | refuses to start when 50071 is taken |

`make develop` runs `init`, starts `drip --fps 2` in the background and runs `cargo run -- daemon
--sandbox` on the tree. Ctrl-C stops both. Set `RESET=1` to recreate the tree first, and
`SANDBOX_DIR=<path>` to use another folder.

The sandbox advertises over mDNS and binds the LAN, so macOS may ask for permission. That is expected:
only the owner runs it.

## Tests

`crates/trd/tests/sandbox.rs` runs `init`, `init --check`, `drip`, `path` and `init --reset` in a
temporary folder with the fake tools (TRD-28 A2). It starts the daemon on that tree with `--sandbox
--no-mdns --port 0 --lan-bind 127.0.0.1:0` and checks that B renders three variants, C fails, D and E
never appear, and G's export folder is gone after its renders (A3). No test touches 50071 or 50072.

Running `make develop` on a screen is the owner's manual check M-TRD-9 (spec 1 §16).
