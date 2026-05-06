.PHONY: build release clean

# non-release build
build:
	swift build

# release build — wraps the binary in a .app bundle and (optionally) signs+notarizes
release:
	./release.sh

clean:
	rm -rf .build
