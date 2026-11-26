# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# The trd component Makefile (overview §9). Recipes run only when a target is built, so
# `make -n <target>` works on every OS. `ci` runs scripts/ci.sh, which the gate's trd job runs
# on Windows too, so CI never needs make.

CARGO ?= cargo
PYTHON ?= python3

# The sandbox (spec 1 §12): `SANDBOX_DIR=<path>` picks its folder, `RESET=1` recreates it.
SANDBOX_DIR ?=
SANDBOX_OPT = $(if $(SANDBOX_DIR),--dir $(SANDBOX_DIR))
SANDBOX_INIT_OPT = $(if $(filter 1,$(RESET)),--reset)
SANDBOX_PY = $(PYTHON) -I sandbox/sandbox.py

# Each target refuses to start when something already listens on its port. The check is a
# connect to loopback: exit 1 when the connect succeeds. The ports are the owner's real trd
# (50071), and the sandbox's (50081).
LISTEN_CHECK_RUN = $(PYTHON) -I -c "import socket,sys; s=socket.socket(); s.settimeout(1); r=s.connect_ex(('127.0.0.1',50071)); s.close(); sys.exit(1 if r==0 else 0)"
LISTEN_CHECK_SANDBOX = $(PYTHON) -I -c "import socket,sys; s=socket.socket(); s.settimeout(1); r=s.connect_ex(('127.0.0.1',50081)); s.close(); sys.exit(1 if r==0 else 0)"

.PHONY: run develop develop-daemon release clean test check ci coverage

# Debug build against the real setup: the daemon with no overrides. It refuses to start while
# something listens on 50071, the owner's installed trd.
run:
	@$(LISTEN_CHECK_RUN) || { echo "trd is already listening on 127.0.0.1:50071; not starting"; exit 1; }
	$(CARGO) run -- daemon

# The sandbox daemon alone (spec 1 §12): a complete, isolated trd on 50081 and LAN 50082. The
# clients' develop targets start it. It refuses while something listens on 50081.
develop-daemon:
	@$(LISTEN_CHECK_SANDBOX) || { echo "something already listens on 127.0.0.1:50081; not starting"; exit 1; }
	$(SANDBOX_PY) init $(SANDBOX_OPT) $(SANDBOX_INIT_OPT)
	SB=$$($(SANDBOX_PY) path $(SANDBOX_OPT)) && \
	$(CARGO) run -- daemon --sandbox --config "$$SB/config.json" --data-dir "$$SB/data" \
	  --log-dir "$$SB/logs" --port 50081 --lan-bind 0.0.0.0:50082 --advertise-name trd-sandbox

# The sandbox daemon with sequence A drip-fed in the background (spec 1 §12). Ctrl-C stops both:
# background jobs ignore SIGINT, so the trap kills drip on exit or on a signal.
develop:
	@$(LISTEN_CHECK_SANDBOX) || { echo "something already listens on 127.0.0.1:50081; not starting"; exit 1; }
	$(SANDBOX_PY) init $(SANDBOX_OPT) $(SANDBOX_INIT_OPT)
	SB=$$($(SANDBOX_PY) path $(SANDBOX_OPT)); \
	$(SANDBOX_PY) drip $(SANDBOX_OPT) --fps 2 & drip=$$!; \
	trap 'kill $$drip 2>/dev/null; exit' 0 HUP INT TERM; \
	$(CARGO) run -- daemon --sandbox --config "$$SB/config.json" --data-dir "$$SB/data" \
	  --log-dir "$$SB/logs" --port 50081 --lan-bind 0.0.0.0:50082 --advertise-name trd-sandbox

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
