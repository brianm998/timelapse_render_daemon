.PHONY: build test release clean core cli gui

# Convenience top-level targets that delegate to each sub-package.
#
# Most useful targets:
#   make build     — debug build of all three packages
#   make test      — run TimelapseRenderCore unit tests
#   make release   — call ./release.sh (multi-platform orchestrator)
#   make clean     — wipe all .build directories

build: core cli gui

core:
	$(MAKE) -C TimelapseRenderCore build

cli:
	$(MAKE) -C cli build

gui:
	$(MAKE) -C gui build

test:
	$(MAKE) -C TimelapseRenderCore test

release:
	./release.sh

clean:
	$(MAKE) -C TimelapseRenderCore clean
	$(MAKE) -C cli clean
	$(MAKE) -C gui clean
