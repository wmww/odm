# Viewer icons

One PNG per icon, 11×11 RGBA. `icons.rs` `include_bytes!`s them, decodes and
uploads each once, and draws it as a single textured quad sampled `NEAREST` at a
whole-number scale — so the file is what appears on screen, pixel for pixel
(verified: a screenshot of the tree matches `mesh.png` composited over the panel
background to within 1/255). Full color and alpha work; art is drawn untinted.

Currently `empty` (a node with no geometry of its own — gray axes gizmo) and
`mesh` (a node carrying one — solid isometric cube in cyan), drawn left of each
scene-tree name; plus `folder` (a plain directory) and `project` (the same
folder with `mesh`'s cyan cube on it), drawn left of each row of the File ▸ Open
dialog.

## Editing

Any pixel editor will do, but eleven pixels is easier as text, so
`scripts/icon-png.py` converts both ways:

```sh
scripts/icon-png.py dump crates/odm-engine/assets/icons/mesh.png > /tmp/mesh.txt
$EDITOR /tmp/mesh.txt
scripts/icon-png.py build /tmp/mesh.txt crates/odm-engine/assets/icons/mesh.png
```

The grid is `.` for transparent, one character per color, named by palette lines
(`rgb`, `rrggbb` or `rrggbbaa`):

```
// mesh — a node that carries geometry
# = 7fdbe8
+ = 3f9cb4
- = 27657f
.....#.....
...#####...
```

Any character works, so art with distinct parts rather than shaded faces can
name them (`x`/`y`/`z` for an axes gizmo, say). `dump` doesn't know that and
assigns `#`/`+`/`-` by brightness.

The text form is a convenience, not a source of truth — the PNG is the asset, so
there is nothing to keep in sync. `dump` regenerates a grid whenever you want
one.

## Adding an icon

1. Put `<name>.png` here.
2. Add an `Icon` variant in `icons.rs` plus arms in `name()` and `png()`, and
   list it in `ALL` so the unit test covers it.
3. Draw it with `theme::tree_row` (nesting gutter + icon + label) or
   `theme::list_row` (icon + label), or `icons::paint` for a bare icon at a
   position.

Keep to 11×11 (the unit test allows 4–16): a whole number of pixels, about the
cap height of the 14px UI font, so it sits level with a line of text. Colors must
read on both the window background (`#1a1a1a`) and the blue selection fill
(`#3060c0`) — blues and dark tones are the ones to watch, since the fill eats
them. `icons::paint` takes a tint, but tinting only multiplies, so it can darken
art and never brighten it.

## Rules worth knowing before changing how this is drawn

- **Whole pixels only.** `icons::SCALE` is an integer and positions are snapped
  (`theme::snap`). Fractional `pixels_per_point` blurs icons exactly like it
  blurs the bitmap fonts.
- **Don't paint art as rects.** egui's tessellator replaces any rect thinner
  than two pixels with a feathered line segment, so a grid painted with
  `rect_filled` smears every 1px row and drops single pixels entirely. That is
  what the texture avoids.
- Each icon is its own texture, which costs a draw call per row. Fine for a
  handful of icons; if there are ever dozens, pack them into one atlas and
  offset the UV instead.

To see changes, screenshot (gui-testing skill; see AGENTS.md) and zoom — pixel
work is unreadable at 1:1:

```sh
DIR=$(.../guibox start -- ./target/debug/odm run examples/piston) && . $DIR/env
grim $DIR/shot.png
python3 -c "from PIL import Image; Image.open('$DIR/shot.png').crop((0,36,240,160)).resize((720,372), Image.NEAREST).save('$DIR/zoom.png')"
```
