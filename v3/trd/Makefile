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

# On macOS `release` writes the installer of spec 1 §10.1 to dist/trd. It needs the ffmpeg and exiftool
# bundles (make release in v3/ffmpeg and v3/exiftool first). It is signed and notarized only when the
# variables are set: SIGN_APP and SIGN_PKG (the Developer ID identities) and APPLE_API_KEY_PATH (the
# notary key; the key id and issuer come from the environment too). CI passes these itself
# (v3-trd-build.yml, TRD-27). Elsewhere `release` is the plain cargo build.
PKG_VERSION ?= $(shell cat ../VERSION)
NATIVE_FFMPEG = $(shell ../ffmpeg/build.sh --print-native)
PKG_FLAGS = $(if $(SIGN_APP),--sign-app "$(SIGN_APP)") $(if $(SIGN_PKG),--sign-installer "$(SIGN_PKG)") $(if $(APPLE_API_KEY_PATH),--notarize)

# Linux packages (overview §10.2) are built on Linux for the host architecture.
ifeq ($(shell uname -s),Linux)
ifeq ($(shell uname -m),x86_64)
LINUX_DEB_ARCH := amd64
LINUX_DIST := linux-x64
endif
ifeq ($(shell uname -m),aarch64)
LINUX_DEB_ARCH := arm64
LINUX_DIST := linux-arm64
endif
endif

release:
	if [ "$$(uname -s)" = Darwin ]; then \
		bash packaging/macos/build-pkg.sh --version $(PKG_VERSION) \
			--ffmpeg-dir ../ffmpeg/dist/$(NATIVE_FFMPEG) --exiftool-dir ../exiftool/dist/unix \
			--out dist/trd $(PKG_FLAGS); \
	else \
		$(CARGO) build --release --locked; \
	fi
ifeq ($(shell uname -s),Linux)
	bash packaging/linux/build-deb.sh --arch $(LINUX_DEB_ARCH) \
		--trd-bin target/release/trd \
		--ffmpeg-dir ../ffmpeg/dist/$(LINUX_DIST) \
		--exiftool-dir ../exiftool/dist/unix \
		--out dist/trd
endif

clean:
	$(CARGO) clean

test:
	$(CARGO) build --workspace --locked
	$(CARGO) test --workspace --locked
	bash packaging/linux/test-maintainer-scripts.sh

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
	bash scripts/test-check-system-tests.sh
	bash scripts/check-system-tests.sh

ci:
	bash scripts/ci.sh

coverage:
	bash scripts/coverage.sh
