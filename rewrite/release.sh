#!/bin/bash

# Multi-platform release orchestrator.  Mirrors nighttime/release.sh.
#
# 1. Runs the local platform release script.
# 2. Reads release_config.json (gitignored) for remote hosts; SSHes to each,
#    checks out the current git commit, runs its platform release script, and
#    collects the produced artifacts back here.
# All builds run in parallel.  The git tag is created only after every build passes.
#
# Requires: jq, git, ssh, scp, a running ssh-agent with your key loaded.
# Remote hosts must have this repo checked out at the configured path with the
# Swift toolchain and any per-platform build prerequisites installed.
#
# release_config.json format: see release_config.json.example

set -e
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ── Preflight checks ──────────────────────────────────────────────────────────
need_cmd() {
    command -v "$1" >/dev/null 2>&1 || { echo "ERROR: '$1' not found in PATH"; exit 1; }
}
need_cmd git
need_cmd ssh
need_cmd scp

# jq is only required if a release_config.json exists.
if [ -f release_config.json ]; then
    need_cmd jq
fi

if [ -f release_config.json ] && [ -z "${SSH_AGENT_PID:-}" ]; then
    echo "ERROR: SSH_AGENT_PID is not set."
    echo "       Start the agent:  eval \$(ssh-agent -s)"
    echo "       Load your key:    ssh-add ~/.ssh/id_ed25519"
    exit 1
fi

# ── Version and commit ────────────────────────────────────────────────────────
VERSION="$(./TimelapseRenderCore/version.sh)"
GIT_HASH=$(git rev-parse HEAD)
LOCAL_PLATFORM="$(uname -s | tr '[:upper:]' '[:lower:]')"   # darwin or linux

echo "==> timelapse-render v${VERSION}  commit ${GIT_HASH}"
echo "==> Local platform: ${LOCAL_PLATFORM}"

if ! git diff --quiet HEAD 2>/dev/null; then
    echo "WARNING: working tree has uncommitted changes; remotes will build from ${GIT_HASH} without them."
fi

# Push so remote hosts can fetch this commit.
if [ -f release_config.json ]; then
    echo "==> Pushing current branch to origin..."
    git push
fi

# ── Output directories ────────────────────────────────────────────────────────
RELEASE_DIR="releases/${VERSION}"
mkdir -p "$RELEASE_DIR"
LOG_DIR="releases/logs/${VERSION}"
mkdir -p "$LOG_DIR"

# ── Helpers ───────────────────────────────────────────────────────────────────
collect_local_artifacts() {
    local found=0
    for f in cli/.build/timelapse-render*.pkg \
             cli/.build/timelapse-render*.deb \
             cli/.build/timelapse-render*.zip \
             gui/.build/timelapse-render*.pkg; do
        [ -f "$f" ] || continue
        cp "$f" "$RELEASE_DIR"/
        echo "==> [local] collected: $(basename "$f")"
        found=$((found + 1))
    done
    if [ "$found" -eq 0 ]; then echo "==> [local] (no artifacts found)"; fi
}

# ── Parse remote config before launching any background jobs ──────────────────
REMOTE_PLATFORMS=()
REMOTE_HOSTS=()
REMOTE_USERS=()
REMOTE_PATHS=()

CONFIG="release_config.json"
if [ -f "$CONFIG" ]; then
    COUNT=$(jq '.platforms | length' "$CONFIG")
    echo "==> Found ${COUNT} remote platform(s) in ${CONFIG}"
    for i in $(seq 0 $((COUNT - 1))); do
        REMOTE_PLATFORMS+=("$(jq -r ".platforms[$i].platform" "$CONFIG" | tr '[:upper:]' '[:lower:]')")
        REMOTE_HOSTS+=("$(jq    -r ".platforms[$i].hostname"  "$CONFIG")")
        REMOTE_USERS+=("$(jq    -r ".platforms[$i].username"  "$CONFIG")")
        REMOTE_PATHS+=("$(jq    -r ".platforms[$i].path"      "$CONFIG")")
    done
else
    echo "==> No ${CONFIG} found; building local platform only."
    echo "    (create one from release_config.json.example to add remote platforms)"
fi

# ── Launch all builds in parallel ─────────────────────────────────────────────
BUILD_PIDS=()
BUILD_NAMES=()

# Local build
(
    set -e
    set -o pipefail
    echo "==> [local/${LOCAL_PLATFORM}] Starting..."
    "./release_${LOCAL_PLATFORM}.sh" 2>&1 | tee "${LOG_DIR}/${LOCAL_PLATFORM}_local.log"
    collect_local_artifacts
    echo "==> [local/${LOCAL_PLATFORM}] Done."
) &
BUILD_PIDS+=($!)
BUILD_NAMES+=("local/${LOCAL_PLATFORM}")

# Remote builds
for i in "${!REMOTE_HOSTS[@]}"; do
    _platform="${REMOTE_PLATFORMS[$i]}"
    _host="${REMOTE_HOSTS[$i]}"
    _user="${REMOTE_USERS[$i]}"
    _path="${REMOTE_PATHS[$i]}"
    _log="${LOG_DIR}/${_platform}_${_host}.log"
    _release_dir="$RELEASE_DIR"
    _git_hash="$GIT_HASH"

    (
        set -e
        set -o pipefail
        echo "==> [remote/${_platform} @ ${_user}@${_host}] Starting..."

        # Send the build script via stdin; positional args go to bash via --.
        # Single-quoted heredoc: no local expansion — all variables run remotely.
        ssh -A "${_user}@${_host}" bash -s -- "$_path" "$_git_hash" "$_platform" <<'REMOTE_SCRIPT' 2>&1 | tee "$_log"
set -e
set -o pipefail
REPO="$1"
GIT_HASH="$2"
PLATFORM="$3"
cd "$REPO"
git fetch origin
git checkout "$GIT_HASH"
"./release_${PLATFORM}.sh"
REMOTE_SCRIPT

        echo "==> [remote/${_platform} @ ${_host}] Collecting artifacts..."
        scp "${_user}@${_host}:${_path}/cli/.build/timelapse-render*.deb" "$_release_dir"/ \
            2>/dev/null && echo "==> [remote/${_platform} @ ${_host}] collected .deb" || true
        scp "${_user}@${_host}:${_path}/cli/.build/timelapse-render*.pkg" "$_release_dir"/ \
            2>/dev/null && echo "==> [remote/${_platform} @ ${_host}] collected .pkg" || true
        scp "${_user}@${_host}:${_path}/cli/.build/timelapse-render*.zip" "$_release_dir"/ \
            2>/dev/null && echo "==> [remote/${_platform} @ ${_host}] collected .zip" || true
        scp "${_user}@${_host}:${_path}/gui/.build/timelapse-render*.pkg" "$_release_dir"/ \
            2>/dev/null && echo "==> [remote/${_platform} @ ${_host}] collected gui .pkg" || true
        echo "==> [remote/${_platform} @ ${_host}] Done."
    ) &
    BUILD_PIDS+=($!)
    BUILD_NAMES+=("remote/${_platform}@${_host}")
done

# ── Wait for all builds ───────────────────────────────────────────────────────
echo ""
echo "==> Waiting for ${#BUILD_PIDS[@]} build(s) to complete..."
FAILED=0
for i in "${!BUILD_PIDS[@]}"; do
    pid="${BUILD_PIDS[$i]}"
    name="${BUILD_NAMES[$i]}"
    if wait "$pid"; then
        echo "==> [${name}] PASSED"
    else
        echo "==> [${name}] FAILED"
        FAILED=$((FAILED + 1))
    fi
done

if [ "$FAILED" -gt 0 ]; then
    echo ""
    echo "ERROR: ${FAILED} build(s) failed. Skipping git tag."
    exit 1
fi

# ── Tag (only after all builds pass) ─────────────────────────────────────────
TAG="release/${VERSION}"
echo ""
echo "==> Tagging ${GIT_HASH} as '${TAG}'..."

# Commented out by default; uncomment after verifying the release.
#git tag -a "$TAG" "$GIT_HASH" -m "Release ${VERSION}"
#git push origin "$TAG"

echo ""
echo "==> Release complete: v${VERSION}"
echo "    Tag (not pushed): ${TAG}"
echo "    Artifacts:        ${RELEASE_DIR}/"
ls "$RELEASE_DIR"/ 2>/dev/null | sed 's/^/    /' || true
