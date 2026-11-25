#!/usr/bin/env python3
"""Render VectorCraft's Nightly art from the committed app icon.

The field (the VectorCraft colour) goes a few shades deeper; the figure (ink and paper) is left
exactly as it is. The palette keeps the figure neutral (r = g = b), so the field's contribution to
each pixel is solved exactly instead of guessed per channel:

    w = ((r-g)(Br-Bg) + (g-b)(Bg-Bb)) / |(Br-Bg, Bg-Bb)|^2
    out = pixel + w * (B' - B)

Writes assets/app-icon/vectorcraft-nightly-1024.png (the padded macOS master, Apple's 824/1024 tile),
vectorcraft-nightly.png (the full tile), vectorcraft-nightly.ico and vectorcraft-nightly.icns.
Needs ffmpeg for scaling. Run from any folder: python3 packaging/nightly-icons.py
"""
import os
from pathlib import Path
import struct
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
ART = ROOT / "assets/app-icon"
FIELD = (0xE8, 0x57, 0x3F)
NIGHTLY_FIELD = (0xA8, 0x3A, 0x24)
# Apple's margin: the tile is 824 of the 1024 canvas, centred.
MAC_TILE = 824
MAC_CANVAS = 1024


def read_png(path):
    """(width, height, RGBA bytes) of an 8-bit, non-interlaced PNG."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    at, chunks, width, height, depth, colour, _, interlace = 8, [], 0, 0, 8, 6, 0, 0
    while at < len(data):
        (length,) = struct.unpack(">I", data[at:at + 4])
        kind = data[at + 4:at + 8]
        body = data[at + 8:at + 8 + length]
        at += 12 + length
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            chunks.append(body)
        elif kind == b"IEND":
            break
    if depth != 8 or colour != 6 or interlace != 0:
        raise ValueError(f"{path}: expected an 8-bit RGBA non-interlaced PNG")
    raw = zlib.decompress(b"".join(chunks))
    stride = width * 4
    out = bytearray(height * stride)
    previous = bytearray(stride)
    at = 0
    for row in range(height):
        filter_kind = raw[at]
        at += 1
        line = bytearray(raw[at:at + stride])
        at += stride
        if filter_kind == 1:
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif filter_kind == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_kind == 3:
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_kind == 4:
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = previous[i]
                c = previous[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                predictor = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + predictor) & 0xFF
        elif filter_kind != 0:
            raise ValueError(f"{path}: unsupported PNG filter {filter_kind}")
        out[row * stride:(row + 1) * stride] = line
        previous = line
    return width, height, bytes(out)


def write_png(path, width, height, rgba):
    stride = width * 4
    raw = bytearray()
    for row in range(height):
        raw.append(0)
        raw += rgba[row * stride:(row + 1) * stride]
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def deepen(width, height, rgba):
    """The field a deeper shade, the figure (ink and paper) untouched, pixel for pixel."""
    (br, bg, bb), (nr, ng, nb) = FIELD, NIGHTLY_FIELD
    dr, dg = br - bg, bg - bb
    norm = dr * dr + dg * dg
    out = bytearray(len(rgba))
    for i in range(0, len(rgba), 4):
        r, g, b, a = rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]
        if a == 0:
            out[i:i + 4] = bytes((r, g, b, 0))
            continue
        w = ((r - g) * dr + (g - b) * dg) / norm
        if w <= 0.0:
            out[i:i + 4] = bytes((r, g, b, a))
            continue
        w = min(w, 1.0)
        out[i:i + 4] = bytes((
            min(255, max(0, round(r + w * (nr - br)))),
            min(255, max(0, round(g + w * (ng - bg)))),
            min(255, max(0, round(b + w * (nb - bb)))),
            a,
        ))
    return bytes(out)


def ffmpeg(*args):
    result = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *map(str, args)], capture_output=True)
    if result.returncode != 0:
        raise SystemExit(f"ffmpeg failed: {result.stderr.decode(errors='replace')}")


def scaled(size, source, out):
    ffmpeg("-i", source, "-vf", f"scale={size}:{size}:flags=lanczos", out)


def padded_master(tile_1024, out):
    """Apple's master: the 1024 tile at 824 px, centred on a transparent 1024 canvas (no upscale)."""
    work = ART / ".nightly-work"
    work.mkdir(exist_ok=True)
    try:
        tile = work / "tile-824.png"
        scaled(MAC_TILE, tile_1024, tile)
        ffmpeg("-f", "lavfi", "-i", f"color=c=black@0.0:s={MAC_CANVAS}x{MAC_CANVAS}:d=1,format=rgba",
               "-i", tile, "-filter_complex", "[0][1]overlay=(W-w)/2:(H-h)/2:format=auto", "-frames:v", "1", out)
    finally:
        for leftover in work.iterdir():
            leftover.unlink()
        work.rmdir()


def icns(master, out):
    """A PNG-based .icns: every frame a native render of the master."""
    frames = {16: b"icp4", 32: b"icp5", 64: b"icp6", 128: b"ic07", 256: b"ic08", 512: b"ic09", 1024: b"ic10"}
    work = ART / ".nightly-work"
    work.mkdir(exist_ok=True)
    body = bytearray()
    try:
        for size, kind in frames.items():
            frame = work / f"icns-{size}.png"
            scaled(size, master, frame)
            data = frame.read_bytes()
            body += kind + struct.pack(">I", len(data) + 8) + data
    finally:
        for leftover in work.iterdir():
            leftover.unlink()
        work.rmdir()
    out.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + bytes(body))


def ico(tile_1024, out):
    """Windows frames: 32bpp BGRA DIB with AND masks up to 128, a PNG frame only for 256."""
    work = ART / ".nightly-work"
    work.mkdir(exist_ok=True)
    images = []
    try:
        for size in (16, 24, 32, 48, 64, 128, 256):
            frame = work / f"ico-{size}.png"
            scaled(size, tile_1024, frame)
            if size == 256:
                images.append((size, frame.read_bytes()))
                continue
            raw = work / f"ico-{size}.bgra"
            ffmpeg("-i", frame, "-f", "rawvideo", "-pix_fmt", "bgra", raw)
            pixels = raw.read_bytes()
            mask_row = ((size + 31) // 32) * 4
            header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
            mask = bytearray()
            for row in range(size - 1, -1, -1):  # bottom-up, like the colour rows
                bits = bytearray(mask_row)
                for column in range(size):
                    alpha = pixels[(row * size + column) * 4 + 3]
                    if alpha == 0:
                        bits[column // 8] |= 0x80 >> (column % 8)
                mask += bits
            colour = bytearray()
            for row in range(size - 1, -1, -1):
                colour += pixels[row * size * 4:(row + 1) * size * 4]
            images.append((size, header + bytes(colour) + bytes(mask)))
    finally:
        for leftover in work.iterdir():
            leftover.unlink()
        work.rmdir()
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, bodies = bytearray(), bytearray()
    for size, data in images:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        bodies += data
        offset += len(data)
    out.write_bytes(header + bytes(entries) + bytes(bodies))


def main() -> int:
    width, height, rgba = read_png(ART / "vectorcraft-1024.png")
    if (width, height) != (1024, 1024):
        raise SystemExit("the mastered tile must be 1024 square")
    nightly = deepen(width, height, rgba)
    tile = ART / "vectorcraft-nightly.png"
    write_png(tile, width, height, nightly)
    master = ART / "vectorcraft-nightly-1024.png"
    padded_master(tile, master)
    icns(master, ART / "vectorcraft-nightly.icns")
    ico(tile, ART / "vectorcraft-nightly.ico")
    for name in ("vectorcraft-nightly.png", "vectorcraft-nightly-1024.png", "vectorcraft-nightly.ico", "vectorcraft-nightly.icns"):
        print(f"{name}: {(ART / name).stat().st_size} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
