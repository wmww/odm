//! odm unstable
// A non-uniform scale is the case every query has to get right, because it
// is the one where "measure the mesh then multiply" is wrong. Queries bake
// the pending transform (docs/api/queries.md), so all four answer in the
// solid's current frame.
export default function build() {
  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  // A 10-cube stretched 2× on x: a 20×10×10 block centred on the origin.
  const s = odm.box(10).scale(2, 1, 1).name('stretched');

  near(s.volume(), 2000, 1e-9, 'volume');
  // area does NOT compose under a non-uniform scale, so it has to be taken
  // from the baked geometry: 2·(20·10 + 20·10 + 10·10).
  near(s.area(), 2 * (200 + 200 + 100), 1e-9, 'area');

  const b = s.bounds();
  near(b.min.x, -10, 1e-9, 'bounds.min.x');
  near(b.max.x, 10, 1e-9, 'bounds.max.x');
  near(b.min.y, -5, 1e-9, 'bounds.min.y');
  near(b.max.z, 5, 1e-9, 'bounds.max.z');

  // The stretched face is at x = 10, not at the unscaled 5.
  const hit = s.raycast([100, 0, 0], [-1, 0, 0]);
  near(hit.distance, 90, 1e-9, 'raycast distance');
  near(hit.point.x, 10, 1e-9, 'raycast point.x');
  near(hit.normal.x, 1, 1e-9, 'raycast normal.x');

  // Two scaled solids: the gap is between the *scaled* faces. The second
  // block spans x ∈ [25, 35], so the gap is 15.
  const other = odm.box(10).scale(1, 1, 1).translate(30, 0, 0);
  const c = s.clearance(other);
  near(c.distance, 15, 1e-9, 'clearance');
  near(c.closest[0].x, 10, 1e-9, 'closest on the scaled solid');
  near(c.closest[1].x, 25, 1e-9, 'closest on the other');

  // Scaling about a point keeps that point fixed: the min face stays put.
  const anchored = odm.box(10).scale(2, 1, 1, { about: [-5, 0, 0] }).name('anchored');
  near(anchored.bounds().min.x, -5, 1e-9, 'anchored min.x');
  near(anchored.bounds().max.x, 15, 1e-9, 'anchored max.x');

  return odm.group(s, other.name('other'), anchored.translate(0, 30, 0));
}

export const checks = [
  // The runner's own `volume` folds |det| of the world transform in, so it
  // agrees with JS volume(): 2000 + 1000 + 2000.
  { volume: [5000, 1e-9] },
  { node: 'stretched', volume: [2000, 1e-9], bounds: { min: [-10, -5, -5], max: [10, 5, 5], eps: 1e-9 } },
  { node: 'anchored', bounds: { min: [-5, 25, -5], max: [15, 35, 5], eps: 1e-9 } },
  // The runner's `area` check deliberately ignores instance transforms
  // (see tests/conformance/README.md), so it sees three *unscaled* 10-cubes:
  // 3 × 600. JS area() above is the one that bakes the scale.
  { area: [1800, 1e-9] },
];
