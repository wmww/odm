#!/usr/bin/env python3
"""Convert an X11 BDF bitmap font to a TTF whose outlines are pixel rectangles.

One pixel becomes a UPP x UPP square, so `upem = PIXEL_SIZE * UPP` and rendering
the result at exactly PIXEL_SIZE px reproduces the original bitmap. Any other
size is a blurry mess — the viewer pins the sizes in `theme.rs` to match.

Used to build the viewer's UI fonts; see crates/odm-viewer-core/assets/fonts/README.md
for the sources and the exact commands.

    scripts/bdf2ttf.py in.bdf out.ttf "Family Name"

Needs fonttools (`pip install fonttools`).
"""

import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

# Font units per bitmap pixel.
UPP = 100

# Codepoints worth carrying. The source fonts cover far more (misc-fixed has
# 4500+ glyphs), and everything dropped here still renders via egui's built-in
# fallback fonts — antialiased, but legible.
KEEP = [
    (0x0020, 0x024F),  # Latin + supplement + extended A/B
    (0x0370, 0x04FF),  # Greek, Cyrillic
    (0x2000, 0x206F),  # general punctuation
    (0x20A0, 0x20BF),  # currency
    (0x2190, 0x21FF),  # arrows
    (0x2500, 0x25FF),  # box drawing, blocks, geometric shapes
]


def keep(cp):
    return any(lo <= cp <= hi for lo, hi in KEEP)


def parse_bdf(path):
    """-> (properties, [glyph]) where a glyph has enc/dwidth/bbx/rows."""
    props, glyphs, cur, rows = {}, [], None, None
    with open(path, "rb") as fh:
        for raw in fh:
            line = raw.decode("latin-1").rstrip("\r\n")
            if rows is not None:
                if line.strip() == "ENDCHAR":
                    cur["rows"], rows = rows, None
                    glyphs.append(cur)
                    cur = None
                else:
                    rows.append(line.strip())
                continue
            parts = line.split()
            if not parts:
                continue
            kw = parts[0]
            if kw == "STARTCHAR":
                cur = {}
            elif kw == "ENCODING":
                cur["enc"] = int(parts[1])
            elif kw == "DWIDTH":
                cur["dwidth"] = int(parts[1])
            elif kw == "BBX":
                cur["bbx"] = tuple(int(p) for p in parts[1:5])
            elif kw == "BITMAP":
                rows = []
            elif kw in ("FONT_ASCENT", "FONT_DESCENT", "PIXEL_SIZE", "CAP_HEIGHT", "X_HEIGHT"):
                props[kw] = int(parts[1])
    return props, glyphs


def runs(rows, w, h):
    """Half-open horizontal runs of set pixels as (x0, x1, y), y up from bottom."""
    for i, row in enumerate(rows[:h]):
        # A BDF row is hex, padded to whole bytes on the right.
        width = len(row) * 4
        bits = int(row or "0", 16)
        on = lambda x: bits >> (width - 1 - x) & 1  # noqa: E731
        x = 0
        while x < w:
            if on(x):
                x1 = x + 1
                while x1 < w and on(x1):
                    x1 += 1
                yield x, x1, h - 1 - i
                x = x1
            else:
                x += 1


def build(bdf, out, family, copyright=""):
    props, glyphs = parse_bdf(bdf)
    px, asc, desc = props["PIXEL_SIZE"], props["FONT_ASCENT"], props["FONT_DESCENT"]

    order, outlines, advances, cmap = [".notdef"], {}, {}, {}
    outlines[".notdef"] = TTGlyphPen(None).glyph()
    advances[".notdef"] = px // 2 * UPP
    for g in glyphs:
        cp = g.get("enc", -1)
        name = "uni%04X" % cp
        if not keep(cp) or name in outlines:
            continue
        w, h, xoff, yoff = g["bbx"]
        pen = TTGlyphPen(None)
        for x0, x1, y in runs(g["rows"], w, h):
            # Squares, all wound the same way: abutting ones union cleanly
            # under the nonzero fill rule.
            left, right = (x0 + xoff) * UPP, (x1 + xoff) * UPP
            bot, top = (y + yoff) * UPP, (y + yoff + 1) * UPP
            pen.moveTo((left, bot))
            pen.lineTo((right, bot))
            pen.lineTo((right, top))
            pen.lineTo((left, top))
            pen.closePath()
        outlines[name] = pen.glyph()
        advances[name] = g["dwidth"] * UPP
        cmap[cp] = name
        order.append(name)

    fb = FontBuilder(px * UPP, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(outlines)
    fb.setupHorizontalMetrics({n: (advances[n], 0) for n in order})
    fb.setupHorizontalHeader(ascent=asc * UPP, descent=-desc * UPP, lineGap=0)
    ps_name = family.replace(" ", "")
    fb.setupNameTable(
        {
            "familyName": family,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"{ps_name}-Regular-1.000",
            "fullName": family,
            "psName": f"{ps_name}-Regular",
            "version": "Version 1.000",
            "copyright": copyright,
        }
    )
    fb.setupOS2(
        sTypoAscender=asc * UPP,
        sTypoDescender=-desc * UPP,
        sTypoLineGap=0,
        usWinAscent=asc * UPP,
        usWinDescent=desc * UPP,
        sCapHeight=props.get("CAP_HEIGHT", 0) * UPP,
        sxHeight=props.get("X_HEIGHT", 0) * UPP,
    )
    fb.setupPost()
    fb.save(out)
    print(f"{out}: {len(order)} glyphs, design size {px}px, ascent {asc}, descent {desc}")


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    build(*sys.argv[1:])
