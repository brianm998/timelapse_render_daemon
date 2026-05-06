.PHONY: build release clean install

PREFIX ?= /usr/local

# non-release build
build:
	swift build

# release build (calls release.sh on macOS for full pkg+notarize, plain swift build otherwise)
release:
	@case "$$(uname -s)" in \
	  Darwin) ./release.sh ;; \
	  Linux)  swift build -c release -Xswiftc -static-stdlib ;; \
	  *)      swift build -c release ;; \
	esac

install:
	install -m 755 .build/release/timelapse-render $(PREFIX)/bin/timelapse-render

clean:
	rm -rf .build
