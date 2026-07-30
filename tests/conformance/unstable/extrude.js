//! odm unstable
// extrude(profile, height, opts): positional height; linear taper via the
// scale option is exact polyhedral arithmetic.
export default function build() {
  // 20×10 rectangle, height 4 → 800, spanning z ∈ [0, 4].
  const slab = odm.extrude([[0, 0], [20, 0], [20, 10], [0, 10]], 4).name('slab');

  // 4×4 square extruded 10 with scale 0 → a pyramid: V = a²·h/3 = 160/3.
  const spike = odm
    .extrude([[-2, -2], [2, -2], [2, 2], [-2, 2]], 10, { scale: 0 })
    .name('spike')
    .translate(50, 0, 0);

  return odm.group(slab, spike);
}

export const checks = [
  { volume: [800 + 160 / 3, 1e-6] },
  { bounds: { min: [0, -2, 0], max: [52, 10, 10], eps: 1e-9 } },
  // Extrusion starts at z = 0: the slab's top face is z = +4.
  { raycast: { origin: [10, 5, 50], dir: [0, 0, -1], distance: [46, 1e-6], normal: [0, 0, 1], name: 'slab' } },
];
