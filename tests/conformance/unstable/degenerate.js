//! odm unstable
// Edge cases at the boundaries of the API: sizes that cannot make a solid,
// rays that hit nothing, and the exact value of odm.deg.
export const meta = {
  inputs: {
    mode: {
      enum: ['ok', 'zero-box', 'negative-box', 'short-size', 'long-size', 'string-size', 'zero-dir', 'negative-maxdist'],
      default: 'ok',
    },
  },
};

export default function build(ctx) {
  const mode = ctx.input('mode');
  // There is no zero-volume primitive: an empty solid is derived, not built
  // (docs/api/solids.md).
  if (mode === 'zero-box') return odm.box(0);
  if (mode === 'negative-box') return odm.box([1, -2, 3]);
  // A size array is exactly three numbers — no 2D box, no implicit fill.
  if (mode === 'short-size') return odm.box([1, 2]);
  if (mode === 'long-size') return odm.box([1, 2, 3, 4]);
  if (mode === 'string-size') return odm.box('10');
  if (mode === 'zero-dir') return odm.box(1).raycast([0, 0, 10], [0, 0, 0]) ?? odm.box(1);
  if (mode === 'negative-maxdist') return odm.box(1).raycast([0, 0, 10], [0, 0, -1], -1) ?? odm.box(1);

  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  const box = odm.box(10); // [-5, 5]³

  // A ray that passes the solid entirely is null, not a hit at infinity.
  if (box.raycast([100, 100, 100], [0, 0, -1]) !== null) throw new Error('a ray past the solid must miss');
  // A ray pointing away from it is a miss too.
  if (box.raycast([0, 0, 10], [0, 0, 1]) !== null) throw new Error('a ray pointing away must miss');

  // maxDist shorter than the hit is a miss; longer is the hit. The surface
  // is 5 away from z = 10, so 4.9 misses and 5.1 hits.
  if (box.raycast([0, 0, 10], [0, 0, -1], 4.9) !== null) throw new Error('maxDist 4.9 must not reach');
  near(box.raycast([0, 0, 10], [0, 0, -1], 5.1).distance, 5, 1e-9, 'maxDist 5.1');

  // dir need not be normalized, and distance is still in world units.
  near(box.raycast([0, 0, 10], [0, 0, -7]).distance, 5, 1e-9, 'unnormalized dir');

  // deg is exact at the values that matter — no accumulated /180*π drift.
  if (odm.deg(180) !== Math.PI) throw new Error(`deg(180) = ${odm.deg(180)}, not Math.PI`);
  if (odm.deg(90) !== Math.PI / 2) throw new Error('deg(90) is not π/2');
  if (odm.deg(0) !== 0) throw new Error('deg(0) is not 0');
  if (odm.deg(-90) !== -Math.PI / 2) throw new Error('deg(-90) is not -π/2');

  return box.name('box');
}

export const checks = [
  { node: 'box', volume: [1000, 1e-9] },
  { raycast: { origin: [100, 100, 100], dir: [0, 0, -1], miss: true } },

  { set: { mode: 'zero-box' }, error: 'box size must be positive' },
  { set: { mode: 'negative-box' }, error: 'box size must be positive' },
  { set: { mode: 'short-size' }, error: 'box size must be a number or [x, y, z]' },
  { set: { mode: 'long-size' }, error: 'box size must be a number or [x, y, z]' },
  { set: { mode: 'string-size' }, error: 'box size must be a number or [x, y, z]' },
  { set: { mode: 'zero-dir' }, error: 'dir' },
  { set: { mode: 'negative-maxdist' }, error: 'positive max_dist' },
];
