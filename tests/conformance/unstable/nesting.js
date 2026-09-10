//! odm unstable
// odm.group: nesting, flattening, and the empty cases. A Group is pure
// structure — one transform/color/name over children — and the transforms
// accumulate down the tree.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'csg-group'], default: 'ok' },
  },
};

export default function build(ctx) {
  const mode = ctx.input('mode');
  // A Group can never be a CSG operand, however it was built.
  if (mode === 'csg-group') return odm.box(2).subtract(odm.group(odm.box(1)));

  // Transforms accumulate: the inner box's world position is the sum of
  // every translate on the way down — 1 + 10 + 100 = 111 on x.
  const deep = odm
    .group(odm.group(odm.box(2).translate(1, 0, 0)).translate(10, 0, 0))
    .translate(100, 0, 0)
    .name('deep');

  // One level of arrays flattens, and null/undefined children drop out —
  // the point being that `cond && part` is a usable idiom.
  const flat = odm
    .group([odm.box(2), odm.box(2).translate(4, 0, 0)], null, odm.box(2).translate(8, 0, 0), undefined)
    .translate(0, 20, 0)
    .name('flat');
  if (flat.children.length !== 3) throw new Error(`flattened to ${flat.children.length} children, want 3`);
  // children is a copy: mutating it cannot reach back into the Group.
  flat.children.push(odm.box(99));
  if (flat.children.length !== 3) throw new Error('children is not a copy');

  // A group with nothing in it is legal and contributes no geometry.
  const nothing = odm.group().name('nothing');
  if (nothing.children.length !== 0) throw new Error('odm.group() is not empty');

  // A group whose children all dropped out is the same thing.
  const alsoNothing = odm.group(null, undefined, []).name('also-nothing');

  // One level of arrays flattens; an array nested deeper survives as a
  // child of its own — an unnamed group with no transform. Two extra
  // levels here, so two extra nodes between the group and the box.
  const nested = odm.group([[[odm.box(2)]]]).translate(0, 40, 0).name('nested');

  // A subtract that removes everything is a legal empty Solid: volume 0,
  // bounds null (never an inverted Box3), and usable in further ops.
  const empty = odm.box(2).subtract(odm.box(4));
  if (empty.volume() !== 0) throw new Error(`empty volume ${empty.volume()}`);
  if (empty.bounds() !== null) throw new Error('an empty solid must have null bounds');
  if (empty.union(odm.box(2)).volume() !== 8) throw new Error('an empty solid is not a usable operand');

  return odm.group(deep, flat, nothing, alsoNothing, nested, empty.name('empty'));
}

export const checks = [
  // 1 + 10 + 100, the box being 2 wide.
  { node: 'deep', bounds: { min: [110, -1, -1], max: [112, 1, 1], eps: 1e-9 } },
  { node: 'flat', volume: [24, 1e-9], bounds: { min: [-1, 19, -1], max: [9, 21, 1], eps: 1e-9 } },
  { node: 'nothing', volume: [0, 1e-12] },
  { node: 'also-nothing', volume: [0, 1e-12] },
  { node: 'empty', volume: [0, 1e-12] },
  // The surviving arrays are nodes: 'nested' -> array -> array -> box.
  { node: 'nested', volume: [8, 1e-9], bounds: { min: [-1, 39, -1], max: [1, 41, 1], eps: 1e-9 } },
  { node: '4/0/0/0', volume: [8, 1e-9] },
  // Five boxes of side 2 (one in `deep`, three in `flat`, one in `nested`);
  // the empty solid and the two empty groups add nothing.
  { volume: [40, 1e-9] },
  // One 2-cube behind all five placements — and the empty solid, which the
  // flattener drops rather than interning a mesh for.
  { meshes: 1 },

  { set: { mode: 'csg-group' }, error: 'operands must be Solids' },
];
