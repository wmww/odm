//! odm unstable
// Color and opacity inheritance, end to end: `node` reads what was
// *authored*, `flat` the effective per-instance colors the renderer draws.
//
// Contract (docs/api/colors.md): a color on a group is a default that any
// descendant's own color overrides; opacity is multiplicative down the tree;
// effective alpha = the color's own alpha × the ancestors' opacity product.
// And per docs/api/csg.md, a CSG result keeps the first operand's color.

export default function build() {
  // A colored group over one bare child and one that overrides.
  const group = odm
    .group(odm.box(2).name('inherits'), odm.box(2).translate(5, 0, 0).color('#00ff00').name('overrides'))
    .color('#ff0000')
    .name('group');

  // 0.5 over 0.5 = 0.25 effective; the child's own color carries alpha 1.
  const nested = odm
    .group(odm.box(2).translate(0, 5, 0).color('#0000ff').opacity(0.5).name('nested-child'))
    .opacity(0.5)
    .name('nested');

  // Chained opacity multiplies into one authored value.
  const chained = odm.box(2).translate(5, 5, 0).color('#ffffff').opacity(0.5).opacity(0.5).name('chained');

  // A color's own alpha times an ancestor opacity: 0.8 × 0.5 = 0.4.
  const alpha = odm
    .group(odm.box(2).translate(0, 0, 5).color('#ffff00cc').name('alpha-child'))
    .opacity(0.5)
    .name('alpha-group');

  // CSG keeps the first operand's color: red wins, and nothing is left
  // uncolored. Volume 2³ − the overlapping eighth = 8 − 1 = 7.
  const cut = odm
    .box(2)
    .color('#ff00ff')
    .subtract(odm.box(2).translate(1, 1, 1).color('#000000'))
    .translate(5, 0, 5)
    .name('cut');

  // No color anywhere up this one's tree: the flattener's neutral default.
  const bare = odm.box(2).translate(0, 5, 5).name('bare');

  return odm.group(group, nested, chained, alpha, cut, bare);
}

// 0xcc/255 = 0.8 exactly enough for the alpha arithmetic below.
const CC = 0xcc / 255;

export const checks = [
  // --- authored attributes, node by node ------------------------------
  { node: 'group', color: '#ff0000', opacity: null },
  { node: 'inherits', color: null, opacity: null },
  { node: 'overrides', color: '#00ff00' },
  { node: 'nested', opacity: 0.5, color: null },
  { node: 'nested-child', opacity: 0.5 },
  // Chained calls collapse into one authored 0.25 — not two 0.5 nodes.
  { node: 'chained', opacity: 0.25 },
  { node: 'alpha-child', color: [1, 1, 0, CC] },
  // CSG keeps the first operand's color, and the box's own volume: 8 - 1.
  { node: 'cut', color: '#ff00ff', volume: [7, 1e-9] },
  { node: 'bare', color: null, opacity: null },

  // --- effective colors, as the renderer will draw them ---------------
  {
    flat: [
      ['#ff0000', 1], // inherits: the group's color, no opacity anywhere
      ['#00ff00', 1], // overrides: its own color wins
      ['#0000ff', 0.25], // 0.5 (group) × 0.5 (child)
      ['#ffffff', 0.25], // .opacity(0.5).opacity(0.5)
      ['#ffff00', CC * 0.5], // color alpha × ancestor opacity
      ['#ff00ff', 1], // the CSG result: first operand's color
      [null, 1], // bare: the uncolored default
    ],
  },

  // Six whole boxes of side 2, plus the cut one (8 − 1): 6·8 + 7.
  { volume: [55, 1e-9] },
];
