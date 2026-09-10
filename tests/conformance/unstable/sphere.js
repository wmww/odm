//! odm unstable
// odm.sphere(r, { segments }): default 48 segments, centered on the origin.
// A sphere's volume is discretization-dependent, so what is pinned is the
// shape of the dependence — monotone in `segments`, always inscribed — plus
// a band around the analytic value at the default.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'negative', 'zero', 'nan', 'unknown-opt'], default: 'ok' },
  },
};

const R = 5;
const ANALYTIC = (4 / 3) * Math.PI * R ** 3; // 523.60

export default function build(ctx) {
  const mode = ctx.input('mode');
  if (mode === 'negative') return odm.sphere(-1);
  if (mode === 'zero') return odm.sphere(0);
  if (mode === 'nan') return odm.sphere(NaN);
  if (mode === 'unknown-opt') return odm.sphere(1, { detail: 8 });

  // Refinement is monotone and always *inside* the true sphere: an
  // inscribed polyhedron can only gain volume as it gains faces, and can
  // never exceed the sphere it is inscribed in. Asserted here rather than
  // as three checks, because it is the relation between them that matters.
  let prev = 0;
  for (const segments of [8, 16, 48]) {
    const v = odm.sphere(R, { segments }).volume();
    if (!(v > prev)) throw new Error(`segments=${segments}: volume ${v} did not grow from ${prev}`);
    if (!(v < ANALYTIC)) throw new Error(`segments=${segments}: volume ${v} exceeds the true ${ANALYTIC}`);
    prev = v;
  }

  return odm.sphere(R).name('ball');
}

export const checks = [
  // 48 segments lands ~1% under the true volume; 3% catches a radius bug
  // without pinning the tessellation.
  { node: 'ball', volume: [ANALYTIC, 0.03 * ANALYTIC] },

  // Vertices lie *on* the sphere, and there is one at each of the six axis
  // poles, so the AABB is exactly ±r on every axis — not merely within it.
  { node: 'ball', bounds: { min: [-R, -R, -R], max: [R, R, R], eps: 1e-9 } },

  // ...which the ray down the z axis confirms: the north pole is a vertex.
  { raycast: { origin: [0, 0, 100], dir: [0, 0, -1], distance: [95, 1e-9], name: 'ball' } },

  // A sphere has no zero or negative form; a non-number is caught earlier.
  { set: { mode: 'negative' }, error: 'sphere radius must be positive' },
  { set: { mode: 'zero' }, error: 'sphere radius must be positive' },
  { set: { mode: 'nan' }, error: 'sphere radius must be a finite number' },
  { set: { mode: 'unknown-opt' }, error: "unknown sphere option 'detail'" },
];
