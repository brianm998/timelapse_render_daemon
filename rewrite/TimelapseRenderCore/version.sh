#!/bin/bash
# Prints the project version. Mirrors the role of nighttime/StarCore/version.pl.
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cat "$SCRIPT_DIR/../VERSION" | tr -d '[:space:]'
