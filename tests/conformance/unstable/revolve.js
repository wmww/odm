//! ODM API unstable
// odm.revolve(profile, { angle, segments, curveSegments }): the profile's
// (x, y) is (radius, z), turned around the Z axis. Partial angles leave flat
// end caps; a negative radius is an error, not quietly folded geometry.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'negative-x', 'unknown-opt'], default: 'ok' },
  },
};

// A 4×4 square section at radius 8..12: Pappus gives 16 × 2π × 10 exactly,
// and the 64-gon discretization lands just inside it.
const PROFILE = [[8, -2], [12, -2], [12, 2], [8, 2]];
const FULL = 16 * 2 * Math.PI * 10;

export default function build(ctx) {
  const mode = ctx.input('mode');
  if (mode === 'negative-x') return odm.revolve([[-1, 0], [2, 0], [2, 2]]);
  if (mode === 'unknown-opt') return odm.revolve(PROFILE, { turns: 2 });

  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  const full = odm.revolve(PROFILE).name('full');

  // Half a turn is half the volume — the caps are flat and add nothing.
  const half = odm.revolve(PROFILE, { angle: Math.PI }).name('half');
  near(half.volume() / full.volume(), 0.5, 0.005, 'half-turn ratio');

  // A quarter likewise. `segments` is the count for a *full* turn, so the
  // partial arc keeps the same facet size.
  const quarter = odm.revolve(PROFILE, { angle: Math.PI / 2 });
  near(quarter.volume() / full.volume(), 0.25, 0.005, 'quarter-turn ratio');

  // A THREE.Shape profile: an arc, flattened by curveSegments. Refining it
  // can only add volume (the polygon is inscribed in the arc) and never
  // reach the analytic torus, 2π²Rr² with R=10, r=3.
  const arc = () => new THREE.Shape().absarc(10, 0, 3, 0, Math.PI * 2);
  const TORUS = 2 * Math.PI ** 2 * 10 * 9;
  let prev = 0;
  for (const curveSegments of [4, 8, 32]) {
    const v = odm.revolve(arc(), { curveSegments }).volume();
    if (!(v > prev)) throw new Error(`curveSegments=${curveSegments}: ${v} did not grow from ${prev}`);
    if (!(v < TORUS)) throw new Error(`curveSegments=${curveSegments}: ${v} exceeds the true ${TORUS}`);
    prev = v;
  }
  const ring = odm.revolve(arc(), { curveSegments: 32 }).translate(60, 0, 0).name('ring');

  return odm.group(full, half.translate(0, 0, 20), ring);
}

export const checks = [
  // Discretized inward from Pappus; 1% is a sanity band, not a pin on the
  // facet count.
  { node: 'full', volume: [FULL, 0.01 * FULL] },
  { node: 'half', volume: [FULL / 2, 0.01 * FULL] },
  // A full turn's AABB is the outer radius each way and the profile's own
  // z extent — the 64-gon's vertices sit on the radius.
  { node: 'full', bounds: { min: [-12, -12, -2], max: [12, 12, 2], eps: 0.02 } },
  // Half a turn keeps only one side: y >= 0 (the arc starts on +x).
  { node: 'half', bounds: { min: [-12, 0, 18], max: [12, 12, 22], eps: 0.02 } },
  { node: 'ring', volume: [2 * Math.PI ** 2 * 10 * 9, 0.1 * 2 * Math.PI ** 2 * 10 * 9] },

  { set: { mode: 'negative-x' }, error: 'revolve profile x must be >= 0' },
  { set: { mode: 'unknown-opt' }, error: "unknown revolve option 'turns'" },
];
