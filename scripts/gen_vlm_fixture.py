#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI: generate test-data/vlm/shapes-512.png.

A 512x512 RGB PNG with a red circle, a blue square, a green triangle, and the
word BENCH in black capitals on a white background, so a real VLM has
something describable. Standard library only (zlib + struct), and the output
is byte-identical on every run.

Usage: scripts/gen_vlm_fixture.py [OUTPUT]   (default test-data/vlm/shapes-512.png)
"""

import struct
import sys
import zlib
from pathlib import Path

SIZE = 512
WHITE = (255, 255, 255)
BLACK = (20, 20, 20)
RED = (220, 40, 40)
BLUE = (40, 80, 220)
GREEN = (30, 160, 70)

# 5x7 bitmap glyphs, one string per row, '#' = ink.
GLYPHS = {
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
    "E": ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "C": [".####", "#....", "#....", "#....", "#....", "#....", ".####"],
    "H": ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
}


def render():
    px = [[WHITE] * SIZE for _ in range(SIZE)]

    # Red circle, upper left.
    cx, cy, r = 128, 140, 80
    for y in range(cy - r, cy + r + 1):
        for x in range(cx - r, cx + r + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px[y][x] = RED

    # Blue square, upper right.
    for y in range(64, 224):
        for x in range(304, 464):
            px[y][x] = BLUE

    # Green triangle, lower middle (apex up).
    top, base, half = 250, 400, 110
    for y in range(top, base + 1):
        w = (y - top) * half // (base - top)
        for x in range(256 - w, 256 + w + 1):
            px[y][x] = GREEN

    # The word BENCH along the bottom, 8 px per glyph cell.
    scale, word = 8, "BENCH"
    width = len(word) * 6 * scale - scale
    x0, y0 = (SIZE - width) // 2, 424
    for i, ch in enumerate(word):
        for gy, row in enumerate(GLYPHS[ch]):
            for gx, cell in enumerate(row):
                if cell != "#":
                    continue
                for dy in range(scale):
                    for dx in range(scale):
                        px[y0 + gy * scale + dy][x0 + (i * 6 + gx) * scale + dx] = BLACK
    return px


def png_bytes(px):
    raw = b"".join(b"\x00" + bytes(c for p in row for c in p) for row in px)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def main():
    root = Path(__file__).resolve().parent.parent
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "test-data" / "vlm" / "shapes-512.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(png_bytes(render()))
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
