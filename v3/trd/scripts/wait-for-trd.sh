#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Polls `trd status --plain --address <a>` until it answers (spec 1 §9.7). The system-test jobs of
# v3-system-tests.yml use it after an install starts the daemon. It exits 0 at the first answer and
# 1 when nothing answers within --timeout seconds. It exits 2 on a usage error.
#
# Usage: wait-for-trd.sh [--trd PATH] [--address A] [--timeout S]
#   --trd PATH      the trd to run (default: trd, found on PATH)
#   --address A     the daemon's listener (default: 127.0.0.1:50071)
#   --timeout S     seconds to wait, a whole number of at least 1 (default: 60)
#
# Bash 3.2 compatible, so it runs on the macOS runners and in Git Bash on Windows. There is no
# `timeout` command here (macOS lacks one), so the deadline is kept with the SECONDS counter.
set -u

trd="trd"
address="127.0.0.1:50071"
timeout=60

usage() {
  echo "usage: wait-for-trd.sh [--trd PATH] [--address A] [--timeout S]" >&2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --trd|--address|--timeout)
      if [ $# -lt 2 ]; then
        echo "wait-for-trd.sh: $1 needs a value" >&2
        usage
        exit 2
      fi
      case "$1" in
        --trd) trd="$2" ;;
        --address) address="$2" ;;
        --timeout) timeout="$2" ;;
      esac
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "wait-for-trd.sh: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

case "$timeout" in
  ''|*[!0-9]*)
    echo "wait-for-trd.sh: --timeout needs a whole number of seconds, not '$timeout'" >&2
    exit 2
    ;;
esac
if [ "$timeout" -lt 1 ]; then
  echo "wait-for-trd.sh: --timeout must be at least 1" >&2
  exit 2
fi

SECONDS=0
while :; do
  if "$trd" status --plain --address "$address" >/dev/null 2>&1; then
    exit 0
  fi
  if [ "$SECONDS" -ge "$timeout" ]; then
    echo "wait-for-trd.sh: $trd did not answer on $address within $timeout s" >&2
    exit 1
  fi
  sleep 1
done
