//! ODM API unstable
// Box primitives and booleans: everything here is exact arithmetic, so the
// epsilons are tight. Curved solids (discretization-dependent) live in
// curved.js.
export default function build() {
  const plain = odm.box([10, 20, 30]).name('plain');

  // box(10) minus box(10) shifted +5 in x: x ∈ [-5, 0] remains → 500.
  const bitten = odm
    .box(10)
    .subtract(odm.box(10).translate(5, 0, 0))
    .name('bitten')
    .translate(100, 0, 0);

  // The union of that pair spans x ∈ [-5, 10] → 1000 + 1000 - 500 = 1500.
  const merged = odm
    .box(10)
    .union(odm.box(10).translate(5, 0, 0))
    .name('merged')
    .translate(200, 0, 0);

  // And their intersection is the 5×10×10 overlap → 500.
  const overlap = odm
    .box(10)
    .intersect(odm.box(10).translate(5, 0, 0))
    .name('overlap')
    .translate(300, 0, 0);

  return odm.group(plain, bitten, merged, overlap);
}

export const checks = [
  // 6000 + 500 + 1500 + 500
  { volume: [8500, 1e-6] },
  // plain 2200; bitten 5×10×10 box → 400; merged 15×10×10 box → 800;
  // overlap 5×10×10 → 400. (Instance translations are ignored for area.)
  { area: [3800, 1e-6] },
  { bounds: { min: [-5, -10, -15], max: [305, 10, 15], eps: 1e-9 } },
  // Straight down onto the plain box's top face (z = +15).
  { raycast: { origin: [0, 0, 100], dir: [0, 0, -1], distance: [85, 1e-6], normal: [0, 0, 1], name: 'plain' } },
  // Against -x into the bitten box: its cut face sits at x = 0 + 100.
  { raycast: { origin: [110, 0, 0], dir: [-1, 0, 0], distance: [10, 1e-6], normal: [1, 0, 0], name: 'bitten' } },
  { raycast: { origin: [0, 0, 100], dir: [0, 0, 1], miss: true } },
];
