//! odm unstable
// odm.fromThreeGeometry: the one door raw three.js geometry comes in
// through. Every closed generator must survive the weld, three's Y-up axes
// must be preserved (a silent axis swap would be worse than the documented
// one you fix with rotateX), and open surfaces must be rejected.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'shape-geometry', 'plane', 'not-geometry'], default: 'ok' },
  },
};

export default function build(ctx) {
  const mode = ctx.input('mode');
  const shape = new THREE.Shape().absarc(0, 0, 5, 0, Math.PI * 2);
  // A flat sheet encloses no volume; the diagnosis names the boundary.
  if (mode === 'shape-geometry') return odm.fromThreeGeometry(new THREE.ShapeGeometry(shape));
  if (mode === 'plane') return odm.fromThreeGeometry(new THREE.BufferGeometry());
  if (mode === 'not-geometry') return odm.fromThreeGeometry({ attributes: {} });

  // The axis exhibit: BoxGeometry(1, 2, 3) is x=1, y=2, z=3 in *three's*
  // frame, and it arrives that way — the Y-up→Z-up fix is the caller's
  // rotateX, never a silent swap (docs/api/three.md).
  const oriented = odm.fromThreeGeometry(new THREE.BoxGeometry(1, 2, 3)).name('oriented');

  // A torus. Analytic volume 2π²Rr², discretized inward. Three's torus
  // rings around *z* already, so this one needs no axis fix — it lies flat
  // on the ground plane as ODM would draw it.
  const torus = odm
    .fromThreeGeometry(new THREE.TorusGeometry(10, 3, 24, 96))
    .translate(0, 40, 0)
    .name('torus');

  // A lathe closed by profile points on the axis at both ends.
  const lathe = odm
    .fromThreeGeometry(
      new THREE.LatheGeometry([new THREE.Vector2(0, -5), new THREE.Vector2(4, -5), new THREE.Vector2(4, 5), new THREE.Vector2(0, 5)], 48),
    )
    .translate(0, 80, 0)
    .name('lathe');

  // Cylinder and sphere generators, and a beveled ExtrudeGeometry — all
  // closed, all welded.
  const cyl = odm.fromThreeGeometry(new THREE.CylinderGeometry(3, 3, 8, 32)).translate(0, 120, 0).name('cyl');
  const ball = odm.fromThreeGeometry(new THREE.SphereGeometry(4, 32, 16)).translate(0, 160, 0).name('ball');
  const bevel = odm
    .fromThreeGeometry(new THREE.ExtrudeGeometry(shape, { depth: 4, bevelEnabled: true, bevelSize: 1, bevelThickness: 1, curveSegments: 24 }))
    .translate(0, 200, 0)
    .name('bevel');

  return odm.group(oriented, torus, lathe, cyl, ball, bevel);
}

const TORUS_V = 2 * Math.PI ** 2 * 10 * 9; // 2π²Rr² = 1776
const LATHE_V = Math.PI * 16 * 10; // r=4, h=10 (48-gon, inscribed)
const CYL_V = Math.PI * 9 * 8; // r=3, h=8 (32-gon)
const BALL_V = (4 / 3) * Math.PI * 64; // r=4

export const checks = [
  // No axis swap: the 2 is on y and the 3 on z, three's own frame.
  { node: 'oriented', volume: [6, 1e-9], bounds: { min: [-0.5, -1, -1.5], max: [0.5, 1, 1.5], eps: 1e-9 } },

  // Inscribed discretizations run a little under analytic; 3% catches a
  // radius or minor-radius bug without pinning the tessellation.
  { node: 'torus', volume: [TORUS_V, 0.03 * TORUS_V] },
  { node: 'lathe', volume: [LATHE_V, 0.03 * LATHE_V] },
  { node: 'cyl', volume: [CYL_V, 0.03 * CYL_V] },
  { node: 'ball', volume: [BALL_V, 0.05 * BALL_V] },
  // Outer radius 13 in the xy plane, tube half-thickness 3 on z.
  { node: 'torus', bounds: { min: [-13, 27, -3], max: [13, 53, 3], eps: 0.2 } },
  // The lathe keeps three's Y-up too: its axis is y, height 10.
  { node: 'lathe', bounds: { min: [-4, 75, -4], max: [4, 85, 4], eps: 0.02 } },
  // The bevel makes the disc wider than the shape's own radius of 5, and
  // pushes it a bevelThickness past each end of the depth-4 extrusion.
  { node: 'bevel', bounds: { min: [-6, 194, -1], max: [6, 206, 5], eps: 0.15 } },

  { set: { mode: 'shape-geometry' }, error: 'open surface' },
  { set: { mode: 'plane' }, error: 'fromThreeGeometry' },
  { set: { mode: 'not-geometry' }, error: 'fromThreeGeometry' },
];
