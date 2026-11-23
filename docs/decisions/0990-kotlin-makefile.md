# 0990: Kotlin client Makefile (task 1026)

## Decisions

- The check was not relaxed. `check-makefiles.sh` reports a component directory without a Makefile as an error, and `test-scripts.sh` asserts that case (`v3/ios: no Makefile`). Spec 1 covers every component whose directory exists, so the fix is the missing Makefile, not a looser check.
- `v3/clients/kotlin/Makefile` implements spec 2 §7 verbatim: each target is a plain delegation to `./gradlew` or `scripts/`, with no `$(shell ...)` and no parse-time work, so `make -n` runs nothing.
- `develop` runs `make -C ../../trd develop-daemon` in the background, waits for `TRD_LISTENING` in its log, runs the desktop client against `127.0.0.1:50081`, and kills the daemon's make process on exit. The trd `develop-daemon` target exits 1 until TRD-28, so `develop` fails when run for real; `make -n` passes.
- `release` leaves the signing variables (`TRM_SIGN_IDENTITY`, `APPLE_API_*`, `TRM_JPACKAGE_JDK`) to the Gradle build, which reads them from the environment.
- The Gradle wrapper, settings and `scripts/` are DSK-01 work and are not added here. Until they land, the targets that call them fail when run.

## Known limit

`develop` kills only the `make` process it started. If `trd` does not exit when its make parent is signalled, the daemon can outlive the recipe. This needs a check once TRD-28 makes `develop-daemon` real.
