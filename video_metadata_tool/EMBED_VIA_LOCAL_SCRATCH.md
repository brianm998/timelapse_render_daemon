# Task: make `embed_xmp` work through local scratch instead of rewriting in place on the SMB share

## Summary

Modify `inspect_video_metadata.py` so that `embed_xmp()` copies the target video to a
**local** scratch directory, runs `exiftool` there, and copies the finished file
**back** to the share atomically — instead of running `exiftool -overwrite_original`
directly on the file on the SMB-mounted share. This removes the I/O pattern that
saturates/wedges the SMB link.

## Target

- File: `inspect_video_metadata.py` (in `video_metadata_tool/`)
- Function: **`embed_xmp(video, json_path, max_attempts=3)`** — around line 436
- Its only call site: the `--apply` loop in `main()` — around line 719:
  `if embed_xmp(video, json_path):`
- Helpers it already uses and must keep using unchanged: `build_exiftool_assignments()`
  (~line 389), `_CORRUPTION_NEEDLES` (~line 425), `EmbedError` (~line 419).

## Why (do not skip — it explains every requirement below)

`embed_xmp` runs `exiftool -overwrite_original` on `video`, which lives on an SMB
share (`/mammoth`, a Synology reached over a direct 10GbE link). `-overwrite_original`
rewrites the **entire** file: exiftool reads the whole video, writes a complete new
copy to a sidecar temp (`<name>_exiftool_tmp`), then renames it over the original.

Performed in place over `smbfs`, that read + full-rewrite + rename pattern **stalls the
SMB connection**. Symptoms observed in the field: `stat()` on the mountpoint starts
timing out, the `smbmounter` daemon decides the mount is wedged and force-unmounts to
recover, and a wedged `smbfs` unmount blocks for the kernel "hard dead timer"
(`net.smb.fs.kern_hard_deadtimer`, ~600 s). Net result: `/mammoth` disappears for
~10 minutes on every `--apply` run, and the embed fails. (This is reproducible every
time with a large video.)

The fix is to keep exiftool's heavy random-access rewrite **on local disk**, leaving
only two **sequential, streaming** SMB operations: one read (copy the video down) and
one write (copy the finished video back). Sequential streaming is the access pattern
`smbfs` handles well; the in-place rewrite is the one that kills it.

## Required changes

1. Add a `scratch_dir: Path` parameter to `embed_xmp`.
   `main()` already computes the scratch root (~line 604:
   `scratch_root = cfg.scratch_dir or Path(tempfile.gettempdir())`, then
   `scratch_root.mkdir(parents=True, exist_ok=True)`), and `scratch_root` is in scope at
   the call site, so just thread it through.

2. Update the call site (~line 719) to pass it:
   `if embed_xmp(video, json_path, scratch_root):`

3. Rewrite the body of `embed_xmp` so it:
   a. Keeps the existing early `return False` when `build_exiftool_assignments()` yields
      nothing — **before** any copy (never copy a multi-GB file just to discover there's
      nothing to embed).
   b. Copies `video` → a **unique** file in `scratch_dir` (local disk).
   c. Runs `exiftool -overwrite_original` on the **local** copy, keeping the existing
      retry loop, the `_CORRUPTION_NEEDLES` early-break, and the `EmbedError` on
      exhaustion exactly as the caller expects them.
   d. On success, copies the finished local file back to the share via a temp name in the
      **same directory** as `video`, then `os.replace(tmp, video)` for an atomic swap.
   e. In a `finally`, always removes the local copy, exiftool's local sidecar temp, and
      the share temp.

## Reference implementation

Adapt names to the file; `os`, `shutil`, `tempfile`, `subprocess`, and `json` are already
imported/used elsewhere in the module (verify before relying on them).

```python
def embed_xmp(video: Path, json_path: Path, scratch_dir: Path, max_attempts: int = 3) -> bool:
    """Embed XMP via exiftool, doing the rewrite on LOCAL disk to avoid wedging the
    SMB link.

    exiftool -overwrite_original rewrites the whole file; doing that in place on an
    smbfs share stalls the connection (read + full-rewrite + rename). Instead we copy
    the video to local `scratch_dir`, embed there, then stream the result back to the
    share via a temp name + atomic os.replace(). The only SMB I/O is one sequential
    read (down) and one sequential write (back).

    Returns True if embedded, False if the sidecar has no assignable tags, and raises
    EmbedError if every exiftool attempt fails.
    """
    with json_path.open() as f:
        histogram = json.load(f)
    assignments = build_exiftool_assignments(histogram)
    if not assignments:
        return False

    scratch_dir.mkdir(parents=True, exist_ok=True)
    # Unique local working copy (safe even if embeds are ever parallelized).
    fd, local_name = tempfile.mkstemp(prefix=f"{video.stem}_", suffix=video.suffix, dir=scratch_dir)
    os.close(fd)
    local_copy = Path(local_name)
    local_tmp  = local_copy.with_name(local_copy.name + "_exiftool_tmp")   # exiftool's sidecar
    share_tmp  = video.with_name(video.name + ".embed_upload_tmp")          # beside the original

    try:
        shutil.copyfile(video, local_copy)             # SMB read  -> local disk

        cmd = ["exiftool", "-overwrite_original", "-q"] + assignments + [str(local_copy)]
        last_output, last_code, ok = "", 0, False
        for _ in range(max_attempts):
            if local_tmp.exists():
                try:
                    local_tmp.unlink()
                except OSError:
                    pass
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode == 0:
                ok = True
                break
            last_code = result.returncode
            last_output = ((result.stdout or "") + (result.stderr or "")).strip()
            if any(n in last_output for n in _CORRUPTION_NEEDLES):
                break   # definitive corruption in the source file; stop retrying
        if not ok:
            raise EmbedError(f"exit {last_code} after {max_attempts} attempt(s)", last_output)

        shutil.copyfile(local_copy, share_tmp)         # local -> SMB write (sequential)
        os.replace(share_tmp, video)                   # atomic swap within the share
        return True
    finally:
        for p in (local_copy, local_tmp, share_tmp):
            try:
                if p.exists():
                    p.unlink()
            except OSError:
                pass
```

Call site (~line 719):

```python
            if embed_xmp(video, json_path, scratch_root):
```

## Requirements / things to get right

1. **Never corrupt the original.** Do not let exiftool or a partial copy touch `video`
   directly. Write finished bytes to `share_tmp` (same directory as `video`), then
   `os.replace(share_tmp, video)`. `os.replace` is atomic only when source and dest are
   on the same filesystem — keeping `share_tmp` beside `video` guarantees that. If the
   copy-back stalls or fails, the original `video` is left intact and the run can be
   retried.

2. **Always clean up.** The `finally` must remove `local_copy`, exiftool's local sidecar
   temp, and `share_tmp` (the last only survives if the copy-back failed before the
   replace). These are full-size video copies — leaking them will fill the scratch volume.

3. **Disk space.** `scratch_dir` needs room for a full copy of the largest video (can be
   many GB). If that's a real risk, check free space before copying and raise a clear
   error instead of failing mid-copy. Note that `scratch_dir` defaults to the system temp
   dir when `cfg.scratch_dir` is unset — make sure that volume is large enough, or require
   `scratch_dir` in the config and document it.

4. **Preserve the caller's contract.** `embed_xmp` must still: return `True` (embedded),
   return `False` (no assignable tags — the caller treats this as a soft skip), or raise
   `EmbedError(msg, output)` whose `.output` is matched against `_CORRUPTION_NEEDLES` at
   the call site (~line 729). Do not change these return/raise semantics.

5. **Don't pre-copy when there's nothing to do.** Keep `if not assignments: return False`
   before any file copy.

6. **(Optional) Skip the round-trip for already-local videos.** If some `video_paths`
   point at local disk, copying local→local is wasted work. You may guard the round-trip
   with: if `os.stat(video).st_dev == os.stat(scratch_dir).st_dev`, the video is already
   on the same (local) filesystem as scratch — just run exiftool in place. Keep it simple
   if all inputs are on the share.

7. **(Optional) mtime.** The copy-back gives `video` a fresh modification time. The old
   in-place exiftool (no `-P`) also updated it, so this matches prior behavior. If you want
   to preserve the original's mtime, capture `st = video.stat()` before and
   `os.utime(video, (st.st_atime, st.st_mtime))` after the replace.

## How to verify

- **Local sanity:** point a test config's `video_paths` at a local dir with a sample
  video, run `--apply`, and confirm the tags are embedded (`exiftool -G1 <file>` shows the
  new XMP) and the video still plays / is byte-valid.
- **On the share:** run `./inspect_video_metadata.py --config example_config.json -v --apply`
  and watch `smbmounter status` in another terminal. It should stay `Mounted` the whole
  time (no `Recovering`), and exiftool should no longer stall. Confirm the embedded videos
  on `/mammoth` are intact afterward.
- **Interrupt safety:** kill the script mid-copy-back and confirm the original video on the
  share is still intact — only a leftover `*.embed_upload_tmp` should remain, which the
  next run overwrites/cleans.

## Notes / context (not part of the change)

- This is the robust workflow fix. Separately, the `smbmounter` daemon that manages
  `/mammoth` is being tuned to recover faster from any SMB stall (reducing
  `net.smb.fs.kern_hard_deadtimer`), but that only shortens the outage — it doesn't prevent
  it. Moving exiftool's rewrite off the share is what prevents the stall in the first place.
- Scope is internal to `embed_xmp` and its one call site. No CLI/flag changes. Mirror the
  module's existing scratch convention if you prefer (see `extract_raw_frames` /
  `scratch = scratch_root / f"video_metadata_tool_{base}"` around line 539).
