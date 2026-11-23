# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The exiftool component (spec 1 §9.3, TRD-23). Plain recipes, so `make -n <target>` works on every
# OS. The host bundle is `windows` under Git Bash (MINGW*, MSYS*, CYGWIN*) and `unix` elsewhere.

HOST := $(if $(filter MINGW% MSYS% CYGWIN%,$(shell uname -s 2>/dev/null)),windows,unix)
BUNDLE_EXE := $(if $(filter windows,$(HOST)),exiftool.exe,exiftool)

.DEFAULT_GOAL := help
.PHONY: help run develop release clean test check ci

help:
	@echo "make run|develop   fetch the $(HOST) ExifTool bundle into dist/$(HOST) and smoke-test it"
	@echo "make release       fetch both bundles into dist/unix and dist/windows"
	@echo "make test          fetch the $(HOST) bundle when missing, then smoke-test it"
	@echo "make check         bash -n on every script"
	@echo "make clean         remove .work and dist"
	@echo "make ci            built by its own workflow (.github/workflows/v3-exiftool.yml)"

run develop:
	bash fetch.sh $(HOST) dist/$(HOST)
	bash smoke_test.sh dist/$(HOST)

release:
	bash fetch.sh unix dist/unix
	bash fetch.sh windows dist/windows

clean:
	rm -rf .work dist

test:
	@if [ ! -f dist/$(HOST)/$(BUNDLE_EXE) ]; then bash fetch.sh $(HOST) dist/$(HOST); fi
	bash smoke_test.sh dist/$(HOST)

check:
	@set -e; for script in *.sh; do bash -n "$$script"; done; echo "bash -n OK"

ci:
	@echo "built by its own workflow"
