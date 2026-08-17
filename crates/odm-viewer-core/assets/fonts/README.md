# Viewer fonts

Two bitmap faces from the X11 font distribution, converted to TTF (one square
outline per pixel) by `scripts/bdf2ttf.py`. They match the Windows 95 look of
`theme.rs`: `odm-sans-14` is an Adobe Helvetica bitmap, the same lineage as the
Helvetica clone Microsoft shipped as MS Sans Serif.

Both are pixel grids, not outlines — they are only crisp at their design size,
which is baked into the file name and pinned in `theme.rs`. At a fractional
`pixels_per_point` they blur like any bitmap font; whole-number scaling is fine.

| file | source | design size | license |
| --- | --- | --- | --- |
| `odm-sans-14.ttf` | `font-adobe-100dpi-1.0.4/helvR10.bdf` | 14px | `LICENSE-adobe-100dpi` (MIT-style) |
| `odm-mono-14.ttf` | `font-misc-misc-1.1.3/7x14.bdf` | 14px | `LICENSE-misc-misc` (public domain) |

To resize the UI, swap in a different strike rather than changing the point
size. Adobe Helvetica ships 8/10/12/14/18/24px at 75dpi and 11/14/17/20px at
100dpi (`helvR08` … `helvR24` in each pack — the file number is the point size,
the pixel size is in the BDF's `FONT` line); misc-fixed has `6x10`, `6x12`,
`6x13`, `7x13`, `7x14`, `8x13`, `9x15`, `10x20`.

Glyph coverage is trimmed to Latin/Greek/Cyrillic, punctuation, arrows and box
drawing (see `KEEP` in the script). Anything outside that falls back to egui's
built-in fonts — antialiased, but legible.

## Regenerating

```sh
curl -LO https://www.x.org/releases/individual/font/font-adobe-100dpi-1.0.4.tar.gz
curl -LO https://www.x.org/releases/individual/font/font-misc-misc-1.1.3.tar.gz
tar xzf font-adobe-100dpi-1.0.4.tar.gz && tar xzf font-misc-misc-1.1.3.tar.gz

scripts/bdf2ttf.py font-adobe-100dpi-1.0.4/helvR10.bdf \
  crates/odm-viewer-core/assets/fonts/odm-sans-14.ttf 'ODM Sans 14' \
  'Copyright 1984-1989, 1994 Adobe Systems Incorporated. Copyright 1988, 1994 Digital Equipment Corporation. See LICENSE.'

scripts/bdf2ttf.py font-misc-misc-1.1.3/7x14.bdf \
  crates/odm-viewer-core/assets/fonts/odm-mono-14.ttf 'ODM Mono 14' 'Public domain.'
```
