# 1156: macOS installer and icon decisions (TRD-24)

Decisions taken while building the macOS side of TRD-24 that the spec leaves open.

- **Icons are drawn at every size.** `make_icon.swift` renders each iconset size, and the 16/32/48/256
  ICO sources, from the same vector description with CoreGraphics, instead of resampling a 1024 master
  with `sips`. `make_ico.py` only packs the PNGs it is given. The 48 px PNG is not committed.
- **Signing identities are values.** `--sign-app IDENTITY` and `--sign-installer IDENTITY` take the
  identity, so CI passes `"$SIGN_APP"` and `"$SIGN_PKG"`. Without them the bundle is ad hoc signed
  and the `.pkg` is unsigned.
- **Notarization skips locally, fails in CI.** With `--notarize` but without the three
  `APPLE_API_*` variables, the step prints that it was skipped. Under `GITHUB_ACTIONS` it fails, so a
  CI release can never ship unnotarized by accident.
- **`--require-universal` covers every Mach-O in `Contents/MacOS`.** `trd`, `ffmpeg` and `ffprobe`
  each need x86_64 and arm64 (arm64e counts as arm64). The default build only requires a valid Mach-O.
- **`make release` on macOS uses the native ffmpeg target.** It points at
  `../ffmpeg/dist/$(build.sh --print-native)`, so a local release needs only the native build. CI passes
  `macos-universal` itself. The `$(shell)` is recursive so other platforms never run it.
- **Self-check builds the real package.** `--self-check` runs the same assembly, signing, `pkgbuild`
  and `pkgutil --payload-files` path as a release, using copies of `/usr/bin/true` as stand-ins, so
  `ci.sh` on macOS exercises the packaging without any inputs.
- **Install scripts use bare command names.** `preinstall` and `postinstall` call `stat`, `id`,
  `launchctl`, `chown` and `chmod` without absolute paths, so `test-scripts.sh` can stub them on `PATH`.

Stage note: `docs/stages/s1166.md`.
