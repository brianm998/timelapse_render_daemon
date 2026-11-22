# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The trd component Makefile (overview §9). Recipes run only when a target is built, so
# `make -n <target>` works on every OS. `ci` runs scripts/ci.sh, which the gate's trd job runs
# on Windows too, so CI never needs make.

CARGO ?= cargo

.PHONY: run develop develop-daemon release clean test check ci coverage

# Debug build against the real setup. Until TRD-01's successors add the daemon, this runs the
# daemon command, which prints "not implemented yet".
run:
	$(CARGO) run -- daemon

# The sandbox arrives with TRD-28; until then both targets refuse to run.
develop develop-daemon:
	@echo "sandbox arrives with TRD-28"
	@exit 1

release:
	$(CARGO) build --release --locked

clean:
	$(CARGO) clean

test:
	$(CARGO) build --workspace --locked
	$(CARGO) test --workspace --locked

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
	bash scripts/test-check-system-tests.sh
	bash scripts/check-system-tests.sh

ci:
	bash scripts/ci.sh

coverage:
	bash scripts/coverage.sh
