# 1200: system tests workflow decisions (TRD-29)

Decisions taken while adding the daemon's system tests of TRD-29 that spec 1 §9.5 and §9.7 leave open.

- **The build gate is an output of the version job, computed from `inputs.build` alone.** The version
  job sets `build` to `'false'` only when `toJSON(inputs.build) == 'false'`, otherwise `'true'`. Every
  build job tests `needs.version.outputs.build == 'true'`.
- **`github.event_name` is not used for the gate.** Inside a called workflow it is the caller's event,
  so a v3.* tag push that calls with `build: false` shows up as `push`. An event test cannot tell that
  release call from a push, so the release would rebuild the packages and upload duplicate
  `release-asset-*` artifacts. Push and schedule have no inputs, so `toJSON(inputs.build)` is `'null'`
  there and they build. A call or dispatch with `build: false` skips the build, whatever the caller's event.
- **`inputs.build == false` is not used.** In GitHub expressions `null == false` is true, so it would
  also match push and schedule runs, where `inputs` is empty.
- **`inputs.build != false` is not used.** On push and schedule `inputs` is empty, and the comparison
  is false there, so the build jobs would be skipped on every push.
- **A dispatch with `build: false` is not guarded.** It has no caller artifacts, so the test jobs fail at
  download. The input description says so.
- **The Linux matrix maps each runner to its own artifact.** The artifacts are `release-asset-trd-linux-x64`
  and `release-asset-trd-linux-arm64`, the names `v3-trd-build.yml` uploads.
- **The systemd user manager wait skips settling states.** `systemctl --user is-system-running` exits
  non-zero when degraded, so only its output is read. Empty, `offline`, `initializing` and `starting`
  keep the wait going, up to 60 s, so `is-enabled` and `start` do not race the user manager.
- **`wait-for-trd.sh` checks its deadline between attempts only.** A `trd status` call that hangs is not
  bounded by `--timeout`; the job limit is the backstop. Known limit, not fixed.

Stage note: `docs/stages/s1210.md`.
