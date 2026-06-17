#!/usr/bin/env python3
"""Restore transparent rounded corners on the rasterised app icon.

QuickLook (`qlmanage`) renders the SVG thumbnail onto a solid white
background, so the four corners outside the brand squircle ship as opaque
white. macOS app icons are expected to have transparent corners, so this
step rewrites the icon's alpha channel: pixels in the corner regions that
fall outside the rounded-rectangle are made fully transparent (with 1px
anti-aliasing), while the straight, full-bleed edges and the white ZF
monogram are left untouched.

Uses only the Python standard library (zlib/struct) so it needs no extra
tooling. Operates in place on the PNG given as the single argument.
"""

import math
import struct
import sys
import zlib

# Corner radius in output pixels: the source SVG rounds a 460px tile by
# rx=104, rasterised to 512px (104 * 512 / 460 ≈ 115.8).
RADIUS = 116


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    i = 8
    idat = b""
    width = height = bit_depth = color_type = 0
    while i < len(data):
        length = struct.unpack(">I", data[i : i + 4])[0]
        ctype = data[i + 4 : i + 8]
        chunk = data[i + 8 : i + 8 + length]
        if ctype == b"IHDR":
            width, height, bit_depth, color_type = struct.unpack(">IIBB", chunk[:10])
        elif ctype == b"IDAT":
            idat += chunk
        elif ctype == b"IEND":
            break
        i += 12 + length
    assert bit_depth == 8 and color_type == 6, "expected 8-bit RGBA"
    raw = zlib.decompress(idat)
    return width, height, _unfilter(raw, width, height)


def _unfilter(raw, width, height):
    ch = 4
    stride = width * ch
    out = bytearray()
    prev = bytearray(stride)
    pos = 0
    for _ in range(height):
        f = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + stride])
        pos += stride
        for x in range(stride):
            a = line[x - ch] if x >= ch else 0
            b = prev[x]
            c = prev[x - ch] if x >= ch else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        out += line
        prev = line
    return out


def write_png(path, width, height, px):
    ch = 4
    stride = width * ch
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter type 0 (none)
        raw += px[y * stride : (y + 1) * stride]
    comp = zlib.compress(bytes(raw), 9)

    def chunk(tag, body):
        return (
            struct.pack(">I", len(body))
            + tag
            + body
            + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
        )

    with open(path, "wb") as fh:
        fh.write(b"\x89PNG\r\n\x1a\n")
        fh.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)))
        fh.write(chunk(b"IDAT", comp))
        fh.write(chunk(b"IEND", b""))


def corner_coverage(x, y, width, height, r):
    """1.0 inside the rounded shape, 0.0 outside, anti-aliased at the edge.

    Only the four corner squares are rounded; straight, full-bleed edges keep
    full coverage so the tile still bleeds to the image border.
    """
    cx = r if x < r else (width - r if x >= width - r else None)
    cy = r if y < r else (height - r if y >= height - r else None)
    if cx is None or cy is None:
        return 1.0
    dist = math.hypot((x + 0.5) - cx, (y + 0.5) - cy)
    return max(0.0, min(1.0, r + 0.5 - dist))


def main():
    path = sys.argv[1]
    width, height, px = read_png(path)
    for y in range(height):
        for x in range(width):
            cov = corner_coverage(x, y, width, height, RADIUS)
            if cov < 1.0:
                o = (y * width + x) * 4 + 3
                px[o] = round(px[o] * cov)
    write_png(path, width, height, px)
    print(f"round-icon-corners: transparent corners applied to {path}")


if __name__ == "__main__":
    main()
