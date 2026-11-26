#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Installs a trd .deb in a clean ubuntu:24.04 container and checks it: apt-get install, then
# /usr/bin/trd version, then a sandboxed daemon that answers `trd status --plain` within 60 s.
# Exits non-zero when any step fails. The deb is mounted read-only; the container's steps come
# on stdin, so nothing is written to the host except the container's own temporary files.
#
#   test-deb-in-container.sh <deb>
#
# Needs docker. Runs in v3-trd-build.yml's Linux jobs (overview §9.4); the owner's Mac need not
# have docker.
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: test-deb-in-container.sh <deb>" >&2
  exit 2
fi
image="ubuntu:24.04"

deb_path="$1"
[ -f "$deb_path" ] || { echo "test-deb-in-container.sh: no such file: $deb_path" >&2; exit 1; }
deb_dir="$(cd "$(dirname "$deb_path")" && pwd)"
deb_name="$(basename "$deb_path")"
command -v docker >/dev/null 2>&1 || { echo "test-deb-in-container.sh: docker is needed" >&2; exit 1; }

docker run --rm -i -v "$deb_dir/$deb_name:/pkg/trd.deb:ro" "$image" bash -s <<'CONTAINER'
set -eu
export DEBIAN_FRONTEND=noninteractive

echo "== apt-get install /pkg/trd.deb"
apt-get update
apt-get install -y /pkg/trd.deb

echo "== /usr/bin/trd version"
/usr/bin/trd version

work="$(mktemp -d)"
mkdir -p "$work/data" "$work/logs"
echo "== trd daemon --sandbox (config, data and log directories under $work)"
/usr/bin/trd daemon --sandbox --no-mdns --lan-bind off \
  --config "$work/config.json" --data-dir "$work/data" --log-dir "$work/logs" \
  > "$work/daemon.out" 2>&1 &
daemon_pid=$!

answered=0
waited=0
while [ "$waited" -lt 60 ]; do
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    echo "daemon exited before it answered"
    break
  fi
  if /usr/bin/trd status --plain > "$work/status.out" 2>&1; then
    answered=1
    break
  fi
  sleep 1
  waited=$((waited + 1))
done

if [ "$answered" -eq 1 ]; then
  echo "== trd status --plain answered after ${waited} s"
  cat "$work/status.out"
else
  echo "== trd status --plain did not answer within 60 s"
  echo "-- daemon output:"
  tail -n 100 "$work/daemon.out" || true
  if [ -f "$work/logs/trd.log" ]; then
    echo "-- trd.log:"
    tail -n 100 "$work/logs/trd.log" || true
  fi
fi

kill "$daemon_pid" 2>/dev/null || true
wait "$daemon_pid" 2>/dev/null || true
[ "$answered" -eq 1 ]
CONTAINER
