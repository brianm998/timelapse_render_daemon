# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The Kotlin desktop client Makefile (spec 2 §7, component `desktop`). Every recipe is a plain
# delegation to ./gradlew or scripts/, and no line runs at parse time, so `make -n <target>`
# works on every OS. The Gradle wrapper, settings and scripts arrive with DSK-01; until then the
# targets that call them fail when run, and `make -n` still passes.

.PHONY: run develop release clean test check ci coverage contract

# Runs the desktop app against the installed trd on 50071.
run:
	./gradlew :desktop:run

# Runs the desktop app against a sandbox trd on 50081. The sandbox daemon is started in the
# background, the recipe waits for its TRD_LISTENING line, and the daemon is stopped on exit.
develop:
	@mkdir -p build
	@rm -f build/develop-daemon.log
	@set -e; \
	make -C ../../trd develop-daemon > build/develop-daemon.log 2>&1 & pid=$$!; \
	trap 'kill $$pid 2>/dev/null || true' EXIT INT TERM; \
	until grep -q TRD_LISTENING build/develop-daemon.log 2>/dev/null; do \
	  kill -0 $$pid 2>/dev/null || { cat build/develop-daemon.log; exit 1; }; \
	  sleep 1; \
	done; \
	./gradlew :desktop:run --args="--server 127.0.0.1:50081"

# Signing and notarization are switched on by the environment: TRM_SIGN_IDENTITY,
# APPLE_API_KEY_PATH, APPLE_API_KEY_ID and APPLE_API_ISSUER_ID. TRM_JPACKAGE_JDK points packaging
# at a JDK that has jpackage. The build reads them; the recipe does not.
release:
	./gradlew :desktop:packageReleaseDistributionForCurrentOS

clean:
	./gradlew clean

test:
	./gradlew :proto:build :core:test :desktop:test

# Warnings are errors.
check:
	./gradlew :proto:compileKotlin :core:compileKotlin :desktop:compileKotlin

ci:
	bash scripts/ci-desktop.sh

# HTML reports land under build/reports/kover.
coverage:
	./gradlew :core:koverVerify :desktop:koverVerify

contract:
	bash scripts/contract-test.sh
