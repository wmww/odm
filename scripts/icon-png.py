#!/usr/bin/env python3
"""Text grid <-> viewer icon PNG.

Viewer icons are small PNGs (see crates/odm-engine/assets/icons/README.md).
Editing eleven pixels in a paint program is no fun, so this builds one from a
text grid, and dumps an existing PNG back to a grid to edit or review:

    scripts/icon-png.py build mesh.txt crates/odm-engine/assets/icons/mesh.png
    scripts/icon-png.py dump crates/odm-engine/assets/icons/mesh.png

Grid format: `.` and space are transparent, every other character is a pixel.
Palette lines name their colors (`rgb`, `rrggbb` or `rrggbbaa`, default white);
`//` starts a comment. All rows must be the same width.

    // mesh — a node that carries geometry
    # = ccc
    + = 8a8a8a
    .....#.....
    ...#####...

Needs Pillow.
"""

import re
import sys

from PIL import Image

PALETTE_LINE = re.compile(r"^(\S)\s*=\s*#?([0-9a-fA-F]{3,8})$")
# Assigned brightest-first when dumping; `.` is reserved for transparent.
DUMP_CHARS = "#+-*=%o&$@~;:"


def parse_color(hex_digits):
    if len(hex_digits) == 3:
        hex_digits = "".join(c * 2 for c in hex_digits)
    if len(hex_digits) == 6:
        hex_digits += "ff"
    if len(hex_digits) != 8:
        sys.exit(f"bad color '{hex_digits}': want rgb, rrggbb or rrggbbaa")
    return tuple(int(hex_digits[i : i + 2], 16) for i in (0, 2, 4, 6))


def build(grid_path, png_path):
    palette, rows = {}, []
    for line in open(grid_path):
        line = line.rstrip("\n").rstrip()
        if not line or line.startswith("//"):
            continue
        match = PALETTE_LINE.match(line)
        if match:
            palette[match.group(1)] = parse_color(match.group(2))
        else:
            rows.append(line)
    if not rows:
        sys.exit(f"{grid_path}: no grid rows")
    width = len(rows[0])
    for y, row in enumerate(rows):
        if len(row) != width:
            sys.exit(f"{grid_path}: row {y} is {len(row)} wide, expected {width}")

    image = Image.new("RGBA", (width, len(rows)), (0, 0, 0, 0))
    for y, row in enumerate(rows):
        for x, char in enumerate(row):
            if char not in ". ":
                image.putpixel((x, y), palette.get(char, (255, 255, 255, 255)))
    image.save(png_path)
    print(f"{png_path}: {width}x{len(rows)}, {len(palette) or 1} colors")


def dump(png_path):
    image = Image.open(png_path).convert("RGBA")
    pixels = [image.getpixel((x, y)) for y in range(image.height) for x in range(image.width)]
    colors = {px for px in pixels if px[3] != 0}
    # Brightest first, so `#` reads as the lit tone in the usual monochrome art.
    order = sorted(colors, key=lambda c: -(c[0] + c[1] + c[2]) * c[3])
    if len(order) > len(DUMP_CHARS):
        sys.exit(f"{png_path}: {len(order)} colors, only {len(DUMP_CHARS)} chars to spend")
    chars = dict(zip(order, DUMP_CHARS))

    print(f"// {png_path}")
    for color, char in chars.items():
        hex_digits = "".join(f"{c:02x}" for c in color)
        print(f"{char} = {hex_digits[:-2] if color[3] == 255 else hex_digits}")
    for y in range(image.height):
        print("".join(chars.get(image.getpixel((x, y)), ".") for x in range(image.width)))


def main(argv):
    match argv:
        case [_, "build", grid_path, png_path]:
            build(grid_path, png_path)
        case [_, "dump", png_path]:
            dump(png_path)
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
