//! odm unstable
// a.hull(...b): the convex hull of every operand together. Values here are
// exact — a hull of axis-aligned boxes is a box, so no discretization.
export const meta = {
  inputs: {
    mode: {
      enum: ['ok', 'group-operand', 'array-operand'],
      default: 'ok',
    },
  },
};

export default function build(ctx) {
  const mode = ctx.input('mode');
  // Groups can never enter CSG, hull included.
  if (mode === 'group-operand') return odm.box(1).hull(odm.group(odm.box(1)));
  if (mode === 'array-operand') return odm.box(1).hull([odm.box(1).translate(3, 0, 0)]);

  const unit = odm.box(1, { center: false }); // [0,1]³

  // Two unit cubes 10 apart on x: the hull is the box [0,11]×[0,1]×[0,1].
  const span = unit.hull(unit.translate(10, 0, 0)).color('#ff0000').name('span');

  // A hull of one operand is that operand: a box is already convex.
  const alone = odm.box(2).hull().translate(0, 20, 0).name('alone');

  // Pending transforms are baked in like any CSG operand: box(2) spans
  // [-1,1]³, its copy z ∈ [3,5], and the hull is 2×2×6.
  const a = odm.box(2);
  const stretched = a.hull(a.translate(0, 0, 4)).translate(0, 40, 0).name('stretched');

  // Eight small cubes at the corners of a 10-cube: the hull is that cube.
  const corners = [];
  for (const x of [0, 9]) {
    for (const y of [0, 9]) {
      for (const z of [0, 9]) corners.push(unit.translate(x, y, z));
    }
  }
  const boxed = corners[0].hull(corners.slice(1)).translate(0, 60, 0).name('boxed');

  // Colour/name follow the CSG rule: the *first* operand's survive
  // (docs/api/csg.md). The second's blue is lost.
  const kept = odm
    .box(1)
    .color('#ff0000')
    .name('kept')
    .hull(odm.box(1).translate(4, 0, 0).color('#0000ff').name('lost'))
    .translate(0, 80, 0);

  return odm.group(span, alone, stretched, boxed, kept);
}

export const checks = [
  { node: 'span', volume: [11, 1e-9], bounds: { min: [0, 0, 0], max: [11, 1, 1], eps: 1e-9 } },
  { node: 'alone', volume: [8, 1e-9] },
  { node: 'stretched', volume: [24, 1e-9] },
  // The hull of the eight corner cubes fills [0,10]³ exactly.
  { node: 'boxed', volume: [1000, 1e-9], bounds: { min: [0, 60, 0], max: [10, 70, 10], eps: 1e-9 } },
  // 1 + 4 + 1 = 6 along x, cross-section 1×1.
  { node: 'kept', volume: [5, 1e-9], color: '#ff0000' },
  // The name comes from the first operand too, so the hull answers to it.
  { raycast: { origin: [0.5, 80.5, 50], dir: [0, 0, -1], distance: [49.5, 1e-9], name: 'kept' } },

  { set: { mode: 'group-operand' }, error: 'hull operands must be Solids' },
  // Variadic like the other CSG methods: one level of arrays flattens.
  { set: { mode: 'array-operand' }, volume: [4, 1e-9] },
];
