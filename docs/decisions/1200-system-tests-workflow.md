# 1200: system tests workflow decisions (TRD-29)

Decisions taken while adding the daemon's system tests of TRD-29 that spec 1 §9.5 and §9.7 leave open.

- **The build gate is an output of the version job, not an `if:` on the event.** `github.event_name`
  inside a called workflow is the caller's event, so a tag push that calls with `build: false` would
  look like a push and build again. The version job computes `build` from the event and `inputs.build`
  of this workflow's own call or dispatch, and every build job tests `needs.version.outputs.build == 'true'`.
  Push and schedule always build.
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
