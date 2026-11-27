#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Works out the trd version of a workflow run and whether the run is a release (spec 1 §9.5,
# TRD-27). v3-release.yml's version job runs it and appends its stdout to $GITHUB_OUTPUT.
#
#   release-version.sh <github-ref> <VERSION-file>
#
# Stdout is exactly three lines: version=<v>, is_release=<true|false>, prerelease=<true|false>.
# A ref of refs/tags/v3.* is a release: the tag without its leading v must equal the VERSION file,
# or the script exits 1. Any other ref takes the version from the file and is not a release. A
# version with a -suffix is a pre-release. Exit 2 on bad arguments; exit 1 when the file holds no
# valid version or the tag does not match it. Messages go to stderr.
set -euo pipefail

usage() {
  echo "usage: release-version.sh <github-ref> <VERSION-file>" >&2
  exit 2
}

[ $# -eq 2 ] || usage
ref="$1"
file="$2"
[ -n "$ref" ] || usage

[ -f "$file" ] || { echo "release-version.sh: no such file: $file" >&2; exit 1; }

# The first line, without a CR and without leading or trailing blanks.
version="$(head -n 1 "$file" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
version_re='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
if ! [[ "$version" =~ $version_re ]]; then
  echo "release-version.sh: $file must hold a version like 3.0.0 or 3.0.0-beta.1, got '$version'" >&2
  exit 1
fi

case "$ref" in
  refs/tags/v3.*)
    tag="${ref#refs/tags/v}"
    if [ "$tag" != "$version" ]; then
      echo "release-version.sh: tag v$tag does not match $file ($version)" >&2
      exit 1
    fi
    is_release=true
    ;;
  *)
    is_release=false
    ;;
esac

case "$version" in
  *-*) prerelease=true ;;
  *) prerelease=false ;;
esac

printf 'version=%s\nis_release=%s\nprerelease=%s\n' "$version" "$is_release" "$prerelease"
