# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# v3 top-level Makefile (overview §9). It delegates to every component directory that exists and
# discovers components with $(wildcard ...), so adding a component never requires editing it.
# It runs nothing at parse time, so `make -n <target>` works on every OS.

COMPONENT_ORDER := ffmpeg exiftool trd desktop android ios

dir_ffmpeg   := ffmpeg
dir_exiftool := exiftool
dir_trd      := trd
dir_desktop  := clients/kotlin
dir_android  := clients/kotlin/android
dir_ios      := ios

COMPONENTS     := $(foreach c,$(COMPONENT_ORDER),$(if $(wildcard $(dir_$(c))/Makefile),$(c)))
COMPONENT_DIRS := $(foreach c,$(COMPONENTS),$(dir_$(c)))

DELEGATED := release clean test check ci

.DEFAULT_GOAL := help
.PHONY: help run develop $(DELEGATED)

help:
	@echo "make run|develop                 trd against the real setup / in the sandbox"
	@echo "make release|clean|test|check|ci every component that exists: $(COMPONENTS)"
	@echo "make <component>-<target>        one target of one component, e.g. make desktop-develop"

$(DELEGATED):
	@set -e; for d in $(COMPONENT_DIRS); do echo "== $$d: $@"; $(MAKE) -C $$d $@; done

ifneq ($(filter trd,$(COMPONENTS)),)
run: trd-run
develop: trd-develop
else
run develop:
	@echo "trd does not exist yet" >&2; exit 1
endif

define component_rules
$(1)-%:
	@$$(MAKE) -C $(2) $$*
endef
$(foreach c,$(COMPONENTS),$(eval $(call component_rules,$(c),$(dir_$(c)))))
