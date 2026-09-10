//! odm unstable
// extrude(profile, height, opts): positional height; linear taper via the
// scale option is exact polyhedral arithmetic. Also `twist`/`slices` (the
// twisted solid is a stack of lofts, so refining slices converges *down* to
// the true prism volume), Shape profiles with curves, and holes.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'unknown-opt', 'zero-height'], default: 'ok' },
  },
};

const SQUARE = [[-2, -2], [2, -2], [2, 2], [-2, 2]]; // area 16

export default function build(ctx) {
  const mode = ctx.input('mode');
  if (mode === 'unknown-opt') return odm.extrude(SQUARE, 4, { depth: 2 });
  if (mode === 'zero-height') return odm.extrude(SQUARE, 0);

  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  // 20×10 rectangle, height 4 → 800, spanning z ∈ [0, 4].
  const slab = odm.extrude([[0, 0], [20, 0], [20, 10], [0, 10]], 4).name('slab');

  // 4×4 square extruded 10 with scale 0 → a pyramid: V = a²·h/3 = 160/3.
  const spike = odm.extrude(SQUARE, 10, { scale: 0 }).name('spike').translate(50, 0, 0);

  // `slices` subdivides the height. With no twist the slices are identical,
  // so the solid is byte-for-byte the same prism: 16 × 10.
  const plain = odm.extrude(SQUARE, 10);
  for (const slices of [1, 2, 7]) {
    near(odm.extrude(SQUARE, 10, { slices }).volume(), plain.volume(), 1e-9, `slices=${slices} untwisted`);
  }

  // A twist keeps every horizontal cross-section congruent to the profile,
  // so the true volume is the untwisted one (Cavalieri). The built solid
  // lofts linearly between slices, which bulges outward — so it is always
  // *larger*, and refining slices shrinks it monotonically toward 160.
  let prev = Infinity;
  for (const slices of [2, 9, 36, 180]) {
    const v = odm.extrude(SQUARE, 10, { twist: odm.deg(90), slices }).volume();
    if (!(v > 160)) throw new Error(`twist slices=${slices}: ${v} is not above the true 160`);
    if (!(v < prev)) throw new Error(`twist slices=${slices}: ${v} did not shrink from ${prev}`);
    prev = v;
  }
  near(prev, 160, 1, 'twist converges to the prism volume');

  // A THREE.Shape circle, flattened by curveSegments. Shape splits a full
  // absarc into two half-arcs, so `curveSegments: n` gives 2n edges — the
  // volume is then the inscribed 2n-gon's, exactly.
  // (The eps is relative, not 1e-9: profile coordinates pass through an
  // f32 cross-section stage — issues/2d-profiles-quantize-to-f32.md.)
  const r = 5;
  const h = 3;
  for (const n of [4, 8, 16]) {
    const disc = odm.extrude(new THREE.Shape().absarc(0, 0, r, 0, Math.PI * 2), h, { curveSegments: n });
    const edges = 2 * n;
    const want = (edges / 2) * r * r * Math.sin((2 * Math.PI) / edges) * h;
    near(disc.volume(), want, want * 1e-6, `disc n=${n}`);
  }

  // A hole: the outer loop counterclockwise, the hole clockwise (the fill
  // rule is winding, not even-odd — see docs/api/solids.md "2D profiles").
  const outer = [[0, 0], [10, 0], [10, 10], [0, 10]];
  const hole = [[3, 3], [3, 7], [7, 7], [7, 3]];
  const framed = odm.extrude([outer, hole], 2).translate(0, 30, 0).name('framed');

  // A THREE.Shape carries its holes through the same way.
  const shape = new THREE.Shape([[0, 0], [10, 0], [10, 10], [0, 10]].map((p) => new THREE.Vector2(...p)));
  shape.holes.push(new THREE.Path([[3, 3], [3, 7], [7, 7], [7, 3]].map((p) => new THREE.Vector2(...p))));
  near(odm.extrude(shape, 2).volume(), framed.volume(), 1e-9, 'Shape holes match the array form');

  return odm.group(slab, spike, framed);
}

export const checks = [
  { volume: [800 + 160 / 3 + 168, 1e-6] },
  { node: 'slab', volume: [800, 1e-9] },
  { node: 'spike', volume: [160 / 3, 1e-9] },
  // 10×10 minus the 4×4 hole, height 2.
  { node: 'framed', volume: [(100 - 16) * 2, 1e-9] },
  { bounds: { min: [0, -2, 0], max: [52, 40, 10], eps: 1e-9 } },
  // Extrusion starts at z = 0: the slab's top face is z = +4.
  { raycast: { origin: [10, 5, 50], dir: [0, 0, -1], distance: [46, 1e-6], normal: [0, 0, 1], name: 'slab' } },
  // Straight down the hole in `framed`: nothing there.
  { raycast: { origin: [5, 35, 50], dir: [0, 0, -1], miss: true } },

  { set: { mode: 'unknown-opt' }, error: "unknown extrude option 'depth'" },
  { set: { mode: 'zero-height' }, error: 'extrude height must be positive' },
];
