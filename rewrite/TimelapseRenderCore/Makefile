.PHONY: build test release clean

# non-release build
build:
	swift build

# run unit tests
test:
	swift test

# release build (no packaging — Core is a library)
release:
	swift build -c release

clean:
	rm -rf .build
