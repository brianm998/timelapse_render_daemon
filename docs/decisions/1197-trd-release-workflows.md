# 1197: trd build and release workflow decisions (TRD-27)

Decisions taken while building the release pipeline of TRD-27 that spec 1 §9.4 and §9.5 leave open.

- **A manual run never releases, even from a tag.** `v3-release.yml`'s version job passes
  `refs/heads/manual-dispatch` to `release-version.sh` for any event other than a tag push, so a
  dispatch started from a `v3.*` ref builds the version in `v3/VERSION` and `is_release` is false.
  The publish job also requires `github.event_name == 'push'`.
- **The signing check is the macOS job's first step.** It fails with the §9.4 `::error::` text
  before anything is downloaded or built, and it sets `signed` for the steps after it. Signing
  secrets are read from the environment, not from `if:` (the `secrets` context is not allowed there).
- **Secret names come from overview §8.** `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`,
  `KEYCHAIN_PASSWORD`, `APPLE_TEAM_ID`, `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID` and
  `APPLE_API_ISSUER_ID`. The ReelVault workflow was not available in this checkout, so the names
  are taken from the overview, which says they match ReelVault.
- **Downloaded bundles get their executable bits back.** The ffmpeg and exiftool bundles are
  downloaded as artifacts, which do not keep the mode bits, so the build jobs `chmod +x` the binaries
  they run (as spec 1 §9.6 does).
- **The keychain is deleted and the API key removed on every run.** The cleanup step is `if: always()`
  and skips what was never created.
- **The release is a draft.** `softprops/action-gh-release@v2` runs with `draft: true`, so the owner
  reviews and publishes it.
- **The zip gets a member check as well.** `build-installer.sh --self-check` on the zip runs after the
  build, in addition to the silent install check the spec names.
- **NSIS is installed only when `makensis` is missing**, through `choco install nsis`, and put on PATH
  through `GITHUB_PATH`, so the next step sees it.
- **No system-tests job yet.** TRD-29 adds it, with `system-tests.needs` and `publish.needs`.

Stage note: `docs/stages/s1207.md`.
