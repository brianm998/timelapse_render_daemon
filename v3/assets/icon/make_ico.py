# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Timelapse Render Daemon contributors
#
# Packs four PNG files into a PNG-compressed Windows .ico (TRD-24): 16, 32, 48 and 256 px. The
# standard library only, and no resampling: make_icon.swift draws every size itself.
#
# Usage: python3 -I make_ico.py OUT.ico 16.png 32.png 48.png 256.png
#
# Each input must be an 8-bit RGBA PNG (colour type 6) whose IHDR size is one of the four sizes,
# and the four sizes must be distinct. The ICO stores each PNG unchanged; Windows reads PNG entries
# from Vista on. Exits 2 with a message on bad input.

import os
import struct
import sys

SIZES = (16, 32, 48, 256)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def die(message):
    sys.stderr.write("make_ico.py: %s\n" % message)
    sys.exit(2)


def png_header(path, data):
    """Returns (width, height) of an 8-bit RGBA PNG, or exits."""
    if data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR" or len(data) < 33:
        die("%s is not a PNG file" % path)
    width, height = struct.unpack(">II", data[16:24])
    bit_depth, colour_type = data[24], data[25]
    if bit_depth != 8 or colour_type != 6:
        die("%s must be 8-bit RGBA (bit depth %d, colour type %d found)" % (path, bit_depth, colour_type))
    if width != height:
        die("%s is %dx%d; icons are square" % (path, width, height))
    return width, height


def build_ico(entries):
    """entries: list of (size, png_bytes). Returns the .ico bytes, entries sorted by size."""
    entries = sorted(entries, key=lambda entry: entry[0])
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = len(header) + 16 * len(entries)
    directory = b""
    payload = b""
    for size, data in entries:
        dimension = size if size < 256 else 0  # 0 means 256 in an ICO directory entry
        # width, height, colour count, reserved, colour planes, bits per pixel, data size, offset
        directory += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        payload += data
    return header + directory + payload


def main(argv):
    if len(argv) != 6:
        die("usage: python3 -I make_ico.py OUT.ico 16.png 32.png 48.png 256.png")
    out_path = argv[1]
    entries = []
    for path in argv[2:]:
        with open(path, "rb") as handle:
            data = handle.read()
        width, _ = png_header(path, data)
        entries.append((width, data))
    sizes = sorted(size for size, _ in entries)
    if tuple(sizes) != SIZES:
        die("the inputs must be %s px, got %s" % (", ".join(map(str, SIZES)), ", ".join(map(str, sizes))))

    temporary = out_path + ".tmp"
    with open(temporary, "wb") as handle:
        handle.write(build_ico(entries))
    os.replace(temporary, out_path)
    print("make_ico.py: wrote %s with %s px" % (out_path, ", ".join(map(str, sizes))))


if __name__ == "__main__":
    main(sys.argv)
