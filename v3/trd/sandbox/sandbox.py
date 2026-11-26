#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
"""The `make develop` sandbox (spec 1 §12, TRD-28).

Usage: python3 -I v3/trd/sandbox/sandbox.py init [--dir PATH] [--reset] [--check]
       python3 -I v3/trd/sandbox/sandbox.py drip [--dir PATH] [--fps N]
       python3 -I v3/trd/sandbox/sandbox.py path [--dir PATH]

It uses the Python standard library only. The tree holds a config.json and video_codecs.json
(the built-in table), raw, backup, export, master, data and logs folders, and a `.trd-sandbox`
marker. The marker is written last; `init --reset` removes a tree only when the marker is there.

Tools are resolved in the order of spec 1 §6.15, steps 1, 3 and 4: TRD_FFMPEG_DIR or
TRD_EXIFTOOL_DIR, then the development tree (v3/ffmpeg/dist/<target>, v3/exiftool/dist/unix or
windows), then PATH. A Unix exiftool script (a file starting with #!) runs as `perl <script>`.
Exit status: 0 on success, 1 on a problem (named on stderr), 130 when drip is interrupted.
"""

import argparse
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time

MARKER_NAME = ".trd-sandbox"
MARKER_TEXT = "trd sandbox: created by v3/trd/sandbox/sandbox.py; init --reset removes this tree\n"
PREFIX = "LRT_"
RAW_NAME_PREFIX = "2026_01_01-sandbox-"
RAW_FRAME_PATTERN = "IMG_%05d.tif"
EXPORT_FRAME_PATTERN = PREFIX + "%05d.tif"
FRAME_WIDTH = 320
FRAME_HEIGHT = 213
FILE_RE_RAW = r"IMG_\d{5}\.tif"
FILE_RE_EXPORT = r"LRT_\d{5}\.tif"
A_FRAMES = 60
C_TRUNCATED_FRAME = 12
C_TRUNCATED_BYTES = 100
F_FRAMES = 16
PLACEHOLDER = b"trd sandbox placeholder\n"

# (letter, raw sequence name, raw frame count). F has no raw folder and is listed separately.
RAW_SEQUENCES = [
    ("A", RAW_NAME_PREFIX + "a", A_FRAMES),
    ("B", RAW_NAME_PREFIX + "b", 30),
    ("C", RAW_NAME_PREFIX + "c", 24),
    ("D", RAW_NAME_PREFIX + "d", 20),
    ("E", RAW_NAME_PREFIX + "e-aligned", 12),
    ("G", RAW_NAME_PREFIX + "g-merge", 12),
]
F_NAME = RAW_NAME_PREFIX + "f"

# The three outputs of sequence D, already in master/ so the daemon never queues it.
D_NAME = RAW_NAME_PREFIX + "d"
D_OUTPUTS = [
    D_NAME + "_ProRes-422_Rec.709F_720p_30_MQ.mov",
    D_NAME + "_H264-420_Rec.709F_720w_30_HQ.mov",
    D_NAME + "_ProRes-444_Rec.709F_OriRes_30_UHQ.mov",
]

RENDERING = [
    {"codec": "ProResMQ", "frame_rate": 30, "resolution": [1080, 720]},
    {"codec": "h264", "frame_rate": 30, "resolution": [720]},
    {"codec": "ProResUHQ", "frame_rate": 30},
]

# The built-in codec table, a copy of trd-core's codecs.rs `builtin` in JSON form.
BUILTIN_CODECS = {
    "ProResUHQ": {
        "ffmpeg": {
            "codec": "prores_ks",
            "pixel_format": "yuv422p10le",
            "color_range": "pc",
            "profile": "1",
        },
        "filename": {"pixel_format": "444", "codec": "ProRes", "quality": "UHQ"},
    },
    "ProResMQ": {
        "ffmpeg": {
            "codec": "prores_ks",
            "pixel_format": "yuv444p10le",
            "color_range": "pc",
            "profile": "1",
        },
        "filename": {"pixel_format": "422", "codec": "ProRes", "quality": "MQ"},
    },
    "h264": {
        "ffmpeg": {
            "codec": "libx264",
            "pixel_format": "yuv420p",
            "color_range": "pc",
            "profile": "high",
            "extra_args": ["-crf", "18"],
        },
        "filename": {"pixel_format": "420", "codec": "H264", "quality": "HQ"},
    },
}

# repo root: the script is v3/trd/sandbox/sandbox.py, four directory levels below it.
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", "..", ".."))
V3_ROOT = os.path.join(REPO_ROOT, "v3")


class SandboxError(Exception):
    """A problem to report on stderr; the command exits 1."""


def default_dir():
    digest = hashlib.sha1(REPO_ROOT.encode("utf-8")).hexdigest()[:8]
    return os.path.join(tempfile.gettempdir(), "trd-sandbox-" + digest)


def sandbox_dir(arg):
    return os.path.abspath(arg) if arg else default_dir()


def layout(root):
    """Every path of the tree, by role."""
    return {
        "root": root,
        "config": os.path.join(root, "config.json"),
        "codecs": os.path.join(root, "video_codecs.json"),
        "marker": os.path.join(root, MARKER_NAME),
        "raw": os.path.join(root, "raw"),
        "backup": os.path.join(root, "backup"),
        "export": os.path.join(root, "export"),
        "master": os.path.join(root, "master"),
        "data": os.path.join(root, "data"),
        "logs": os.path.join(root, "logs"),
    }


def export_name(raw_name):
    return PREFIX + raw_name


def expected_config(paths):
    return {
        "source_paths": [paths["raw"]],
        "export_paths": [paths["export"]],
        "backup_paths": [paths["backup"]],
        "master_videos": [paths["master"]],
        "image_name_prefix": PREFIX,
        "image_sequence_dirname_prefix": PREFIX,
        "raw_types": ["tif"],
        "image_extensions": ["tif", "tiff"],
        "rendering": RENDERING,
        "scan_interval_seconds": 1,
        "max_processes": {"ffmpeg": 2, "exiftool": 4},
        "ignore_sequence_regexes": ["-aligned$"],
        "delete_sequence_after_render_regexes": ["-merge$"],
    }


# ---------------------------------------------------------------- tools


def is_windows():
    return os.name == "nt"


def exe_name(base):
    return base + ".exe" if is_windows() else base


def dev_target():
    """The v3/ffmpeg/dist folder for this host, as trd-core's dev_target names it."""
    machine = platform.machine()
    arch = {"AMD64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(
        machine
    )
    system = platform.system()
    if system == "Darwin":
        return "macos-universal"
    if system == "Linux" and arch == "x86_64":
        return "linux-x64"
    if system == "Linux" and arch == "aarch64":
        return "linux-arm64"
    if system == "Windows" and arch == "x86_64":
        return "windows-x64"
    return None


def tool_candidates(name):
    """The paths to try for one tool, in §6.15 order."""
    file = exe_name(name)
    out = []
    if name == "exiftool":
        env_dir = os.environ.get("TRD_EXIFTOOL_DIR")
        if env_dir:
            out.append(os.path.join(env_dir, file))
        out.append(os.path.join(V3_ROOT, "exiftool", "dist", "windows" if is_windows() else "unix", file))
    else:
        env_dir = os.environ.get("TRD_FFMPEG_DIR")
        if env_dir:
            out.append(os.path.join(env_dir, file))
        target = dev_target()
        if target:
            out.append(os.path.join(V3_ROOT, "ffmpeg", "dist", target, file))
    for folder in os.environ.get("PATH", "").split(os.pathsep):
        if folder:
            out.append(os.path.join(folder, file))
    return out


def resolve_tool(name):
    """The command prefix that runs `name`; raises SandboxError naming every path tried."""
    tried = tool_candidates(name)
    for path in tried:
        if os.path.isfile(path):
            if name == "exiftool" and starts_with_shebang(path):
                perl = "/usr/bin/perl" if os.path.exists("/usr/bin/perl") else "perl"
                return [perl, path]
            return [path]
    raise SandboxError(
        "cannot find %s; tried: %s (set TRD_FFMPEG_DIR or TRD_EXIFTOOL_DIR)"
        % (name, ", ".join(tried))
    )


def starts_with_shebang(path):
    with open(path, "rb") as handle:
        return handle.read(2) == b"#!"


def run_tool(command, stdin=None):
    """Runs a tool with no shell. `stdin` is bytes or None (the null device)."""
    if stdin is None:
        proc = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    else:
        proc = subprocess.run(command, input=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        lines = proc.stderr.decode("utf-8", "replace").strip().splitlines()
        detail = lines[-1] if lines else "no output"
        raise SandboxError("%s exited %d: %s" % (os.path.basename(command[0]), proc.returncode, detail))


# ---------------------------------------------------------------- building


def write_json(path, value):
    """Writes sorted, two-space JSON atomically: a sibling temp file, then a rename."""
    text = json.dumps(value, indent=2, sort_keys=True) + "\n"
    temp = path + ".tmp"
    with open(temp, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.replace(temp, path)


def make_folders(paths):
    for key in ("raw", "backup", "export", "master", "data", "logs"):
        os.makedirs(paths[key], exist_ok=True)


def frames_command(ffmpeg, directory, frames, pattern):
    """One ffmpeg call that writes `frames` numbered TIFF frames of a test pattern."""
    return ffmpeg + [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=%dx%d:rate=30" % (FRAME_WIDTH, FRAME_HEIGHT),
        "-frames:v",
        str(frames),
        "-start_number",
        "1",
        os.path.join(directory, pattern),
    ]


def exif_script(directory, frames):
    """One exiftool -@ argfile: the same tags for each frame, one second apart."""
    start = datetime.datetime(2026, 1, 1, 0, 0, 0)
    lines = []
    for index in range(1, frames + 1):
        when = start + datetime.timedelta(seconds=index - 1)
        lines += [
            "-overwrite_original",
            "-q",
            "-Make=SandboxCam",
            "-Model=SC-1",
            "-LensModel=Sandbox 14mm F1.8",
            "-ISO=1600",
            "-FNumber=1.8",
            "-ExposureTime=1/8",
            "-FocalLength=14.0",
            "-DateTimeOriginal=" + when.strftime("%Y:%m:%d %H:%M:%S"),
            os.path.join(directory, RAW_FRAME_PATTERN % index),
            "-execute",
        ]
    return ("\n".join(lines) + "\n").encode("utf-8")


def copy_frames(source_dir, target_dir, frames):
    os.makedirs(target_dir, exist_ok=True)
    for index in range(1, frames + 1):
        shutil.copyfile(
            os.path.join(source_dir, RAW_FRAME_PATTERN % index),
            os.path.join(target_dir, EXPORT_FRAME_PATTERN % index),
        )


def truncate_frame(path, size):
    with open(path, "rb") as handle:
        data = handle.read(size)
    # A source frame shorter than `size` is padded, so the truncated frame is always `size` bytes.
    with open(path, "wb") as handle:
        handle.write(data.ljust(size, b"\0"))


def build(paths, tools):
    ffmpeg = tools["ffmpeg"]
    exiftool = tools["exiftool"]
    make_folders(paths)

    for letter, raw_name, frames in RAW_SEQUENCES:
        raw_dir = os.path.join(paths["raw"], raw_name)
        os.makedirs(raw_dir, exist_ok=True)
        run_tool(frames_command(ffmpeg, raw_dir, frames, RAW_FRAME_PATTERN))
        run_tool(exiftool + ["-@", "-"], stdin=exif_script(raw_dir, frames))

    # A starts with an empty export folder; drip fills it.
    os.makedirs(os.path.join(paths["export"], export_name(RAW_SEQUENCES[0][1])), exist_ok=True)
    # B, C, D, E and G are fully exported, as full copies of their raw frames.
    for letter, raw_name, frames in RAW_SEQUENCES[1:]:
        export_dir = os.path.join(paths["export"], export_name(raw_name))
        copy_frames(os.path.join(paths["raw"], raw_name), export_dir, frames)
        if letter == "C":
            truncate_frame(
                os.path.join(export_dir, EXPORT_FRAME_PATTERN % C_TRUNCATED_FRAME),
                C_TRUNCATED_BYTES,
            )

    # F has no raw folder: its export is generated directly, and its backup sidecar is the raw.
    f_export = os.path.join(paths["export"], export_name(F_NAME))
    os.makedirs(f_export, exist_ok=True)
    run_tool(frames_command(ffmpeg, f_export, F_FRAMES, EXPORT_FRAME_PATTERN))
    write_json(
        os.path.join(paths["backup"], F_NAME + ".json"),
        {
            "ImageSize": {"%dx%d" % (FRAME_WIDTH, FRAME_HEIGHT): F_FRAMES},
            "Make": {"SandboxCam": F_FRAMES},
            "Model": {"SC-1": F_FRAMES},
        },
    )

    for output in D_OUTPUTS:
        with open(os.path.join(paths["master"], output), "wb") as handle:
            handle.write(PLACEHOLDER)

    write_json(paths["codecs"], BUILTIN_CODECS)
    write_json(paths["config"], expected_config(paths))
    with open(paths["marker"], "w", encoding="utf-8") as handle:
        handle.write(MARKER_TEXT)


# ---------------------------------------------------------------- checking


def count_matching(directory, pattern_re):
    if not os.path.isdir(directory):
        return None
    regex = re.compile("^" + pattern_re + "$")
    return sum(1 for name in os.listdir(directory) if regex.match(name))


def check_tree(paths):
    """Every problem with the tree, as text. An empty list means the tree is valid."""
    problems = []
    if not os.path.isfile(paths["marker"]):
        problems.append("no %s marker in %s" % (MARKER_NAME, paths["root"]))
    for key in ("raw", "backup", "export", "master", "data", "logs"):
        if not os.path.isdir(paths[key]):
            problems.append("missing folder %s" % paths[key])

    try:
        with open(paths["config"], encoding="utf-8") as handle:
            config = json.load(handle)
        for key, want in expected_config(paths).items():
            if config.get(key) != want:
                problems.append("config.json: %s is %r, want %r" % (key, config.get(key), want))
    except (OSError, ValueError) as err:
        problems.append("config.json: %s" % err)

    try:
        with open(paths["codecs"], encoding="utf-8") as handle:
            if json.load(handle) != BUILTIN_CODECS:
                problems.append("video_codecs.json is not the built-in table")
    except (OSError, ValueError) as err:
        problems.append("video_codecs.json: %s" % err)

    for letter, raw_name, frames in RAW_SEQUENCES:
        raw_dir = os.path.join(paths["raw"], raw_name)
        found = count_matching(raw_dir, FILE_RE_RAW)
        if found != frames:
            problems.append("raw %s (%s): %s frames, want %d" % (letter, raw_name, found, frames))

    if os.path.exists(os.path.join(paths["raw"], F_NAME)):
        problems.append("sequence F has a raw folder; it must have none")

    a_export = os.path.join(paths["export"], export_name(RAW_SEQUENCES[0][1]))
    found = count_matching(a_export, FILE_RE_EXPORT)
    if found is None or not 0 <= found <= A_FRAMES:
        problems.append("export A: %s frames, want 0 to %d" % (found, A_FRAMES))

    for letter, raw_name, frames in RAW_SEQUENCES[1:]:
        export_dir = os.path.join(paths["export"], export_name(raw_name))
        found = count_matching(export_dir, FILE_RE_EXPORT)
        if found != frames:
            problems.append("export %s (%s): %s frames, want %d" % (letter, raw_name, found, frames))
    c_frame = os.path.join(paths["export"], export_name(RAW_SEQUENCES[2][1]), EXPORT_FRAME_PATTERN % C_TRUNCATED_FRAME)
    if os.path.isfile(c_frame) and os.path.getsize(c_frame) != C_TRUNCATED_BYTES:
        problems.append("export C frame %d is %d bytes, want %d" % (C_TRUNCATED_FRAME, os.path.getsize(c_frame), C_TRUNCATED_BYTES))

    f_export = os.path.join(paths["export"], export_name(F_NAME))
    found = count_matching(f_export, FILE_RE_EXPORT)
    if found != F_FRAMES:
        problems.append("export F: %s frames, want %d" % (found, F_FRAMES))

    backup_path = os.path.join(paths["backup"], F_NAME + ".json")
    try:
        with open(backup_path, encoding="utf-8") as handle:
            sidecar = json.load(handle)
        sizes = sidecar.get("ImageSize", {})
        first = sorted(sizes)[0] if sizes else None
        if first is None or sizes.get(first) != F_FRAMES:
            problems.append("backup sidecar for F: ImageSize count is not %d" % F_FRAMES)
    except (OSError, ValueError) as err:
        problems.append("backup sidecar for F: %s" % err)

    for output in D_OUTPUTS:
        if not os.path.isfile(os.path.join(paths["master"], output)):
            problems.append("master output missing for D: %s" % output)
    return problems


# ---------------------------------------------------------------- commands


def cmd_init(args):
    root = sandbox_dir(args.dir)
    paths = layout(root)

    if args.check:
        if not os.path.isdir(root):
            raise SandboxError("no sandbox at %s; run init first" % root)
        problems = check_tree(paths)
        if problems:
            raise SandboxError("sandbox at %s is not valid:\n  %s" % (root, "\n  ".join(problems)))
        print("sandbox OK: %s" % root)
        return 0

    # Decide what to do before anything is removed or written.
    remove_old_tree = False
    if os.path.lexists(root):
        if os.path.isfile(paths["marker"]):
            if not args.reset:
                problems = check_tree(paths)
                if problems:
                    raise SandboxError(
                        "the sandbox at %s is damaged; run init --reset:\n  %s"
                        % (root, "\n  ".join(problems))
                    )
                print("sandbox already present: %s" % root)
                return 0
            remove_old_tree = True
        elif args.reset:
            raise SandboxError("%s has no %s marker; refusing to remove it" % (root, MARKER_NAME))
        elif os.listdir(root):
            raise SandboxError(
                "%s is not empty and has no %s marker; refusing to use it" % (root, MARKER_NAME)
            )
    created_root = remove_old_tree or not os.path.lexists(root)

    # Resolve every tool before the old tree is removed, so a missing tool leaves it intact.
    tools = {
        "ffmpeg": resolve_tool("ffmpeg"),
        "ffprobe": resolve_tool("ffprobe"),
        "exiftool": resolve_tool("exiftool"),
    }
    if remove_old_tree:
        shutil.rmtree(root)
    try:
        build(paths, tools)
        problems = check_tree(paths)
        if problems:
            raise SandboxError("the new sandbox is not valid:\n  %s" % "\n  ".join(problems))
    except BaseException:
        cleanup(root, created_root)
        raise
    print("sandbox ready: %s" % root)
    return 0


def cleanup(root, created_root):
    """Removes a half-built tree: the whole folder if this run made it, else its contents."""
    if created_root:
        shutil.rmtree(root, ignore_errors=True)
        return
    for name in os.listdir(root):
        target = os.path.join(root, name)
        if os.path.isdir(target) and not os.path.islink(target):
            shutil.rmtree(target, ignore_errors=True)
        else:
            os.remove(target)


def cmd_drip(args):
    paths = layout(sandbox_dir(args.dir))
    a_raw = os.path.join(paths["raw"], RAW_SEQUENCES[0][1])
    if not os.path.isdir(a_raw):
        raise SandboxError("no sequence A at %s; run init first" % a_raw)
    a_export = os.path.join(paths["export"], export_name(RAW_SEQUENCES[0][1]))
    os.makedirs(a_export, exist_ok=True)
    frames = count_matching(a_raw, FILE_RE_RAW) or 0
    pause = 1.0 / args.fps
    for index in range(1, frames + 1):
        name = EXPORT_FRAME_PATTERN % index
        target = os.path.join(a_export, name)
        if os.path.exists(target):
            continue
        # Copy under a hidden name, then rename, so the daemon never sees a half-written frame.
        temp = os.path.join(a_export, "." + name + ".partial")
        shutil.copyfile(os.path.join(a_raw, RAW_FRAME_PATTERN % index), temp)
        os.replace(temp, target)
        time.sleep(pause)
    print("sequence A exported: %d frames in %s" % (frames, a_export))
    return 0


def cmd_path(args):
    print(sandbox_dir(args.dir))
    return 0


def build_parser():
    parser = argparse.ArgumentParser(prog="sandbox.py", description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)

    init = sub.add_parser("init", help="create the sandbox tree")
    init.add_argument("--dir", help="the sandbox folder (default: the temp folder for this worktree)")
    init.add_argument("--reset", action="store_true", help="recreate a tree that has the marker")
    init.add_argument("--check", action="store_true", help="validate the tree only")
    init.set_defaults(func=cmd_init)

    drip = sub.add_parser("drip", help="export sequence A's frames at a steady rate")
    drip.add_argument("--dir", help="the sandbox folder")
    drip.add_argument("--fps", type=float, default=2.0, help="frames copied per second (default 2)")
    drip.set_defaults(func=cmd_drip)

    path = sub.add_parser("path", help="print the sandbox folder")
    path.add_argument("--dir", help="the sandbox folder")
    path.set_defaults(func=cmd_path)
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    if args.command == "drip" and args.fps <= 0:
        print("sandbox.py: --fps must be above zero", file=sys.stderr)
        return 1
    try:
        return args.func(args)
    except SandboxError as err:
        print("sandbox.py: %s" % err, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
