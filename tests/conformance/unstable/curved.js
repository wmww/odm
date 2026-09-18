//! ODM API unstable
// Curved solids: values are discretization-dependent, and the default
// segment counts (cylinder 64, sphere 48, revolve 64) are API surface.
// Epsilons are sanity bands around the analytic value — wide enough for an
// inscribed polyhedron, tight enough to catch a radius/height/axis bug.
export default function build() {
  const cyl = odm.cylinder(5, 10).name('cyl');
  const ball = odm.sphere(5).name('ball').translate(100, 0, 0);
  // Square 4×4 profile revolved into a full torus-like ring at radius 10
  // (profile x ∈ [8, 12]): exact volume by Pappus = area × 2π·centroid
  // = 16 × 2π×10, discretized like the cylinder.
  const ring = odm
    .revolve(
      [[8, -2], [12, -2], [12, 2], [8, 2]],
      { segments: 64 },
    )
    .name('ring')
    .translate(200, 0, 0);

  return odm.group(cyl, ball, ring);
}

const CYL_V = Math.PI * 25 * 10; // 785.4
const BALL_V = (4 / 3) * Math.PI * 125; // 523.6
const RING_V = 16 * 2 * Math.PI * 10; // 1005.3 (Pappus)

export const checks = [
  // Inscribed discretizations sit just under analytic; 1.5% catches real bugs.
  { volume: [CYL_V + BALL_V + RING_V, 0.015 * (CYL_V + BALL_V + RING_V)] },
  // Cylinder is z-aligned and centered; sphere centered at (100,0,0);
  // ring outer radius 12. Circle vertices lie ON the radius (inscribed),
  // so the AABB is exact in the axis-aligned directions that hit a vertex —
  // keep eps loose to stay layout-agnostic.
  { bounds: { min: [-5.01, -12.01, -5.01], max: [212.01, 12.01, 5.01], eps: 0.15 } },
  // Down the cylinder's axis: its flat top is exactly z = +5.
  { raycast: { origin: [0, 0, 50], dir: [0, 0, -1], distance: [45, 1e-6], normal: [0, 0, 1], name: 'cyl' } },
];
