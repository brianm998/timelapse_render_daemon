# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The FFmpeg component (overview §9). `make release` builds this machine's native target with
# build.sh; `run` and `develop` build it and smoke-test it; `test` smoke-tests it, building it
# first when it is missing. CI builds every target in .github/workflows/v3-ffmpeg.yml, so `ci`
# only says so.

NATIVE  = $(shell ./build.sh --print-native)
SCRIPTS = build.sh smoke_test.sh $(wildcard lib/*.sh)

.PHONY: release run develop test clean check ci

release:
	./build.sh

run develop: release
	./smoke_test.sh --target $(NATIVE) dist/$(NATIVE)

test:
	@if [ ! -x dist/$(NATIVE)/ffmpeg ]; then ./build.sh; fi
	./smoke_test.sh --target $(NATIVE) dist/$(NATIVE)

clean:
	rm -rf .work dist

check:
	@for f in $(SCRIPTS); do bash -n $$f || exit 1; done
	@echo "bash -n OK ($(words $(SCRIPTS)) scripts)"

ci:
	@echo "built by its own workflow"
