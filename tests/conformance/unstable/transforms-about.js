//! odm unstable
// The {about} pivot on rotations and scale, applyMatrix4, and THREE-typed
// query results (Box3 from bounds(), Vector3 point from raycast()) used to
// position geometry inside a build.
export default function build() {
  // 10-box spun 90° about Z around (10, 0, 0): its center lands at (10, -10, 0).
  const spun = odm.box(10).rotateZ(odm.deg(90), { about: [10, 0, 0] }).name('spun');

  // The same pivot rotation via the general axis form, with a Vector3 pivot.
  const spun2 = odm
    .box(10)
    .rotate([0, 0, 1], odm.deg(90), { about: new THREE.Vector3(10, 0, 0) })
    .name('spun2')
    .translate(100, 0, 0);

  // Scale ×2 in x about the box's own +x face: x ∈ [-5, 5] → [-15, 5].
  const stretched = odm
    .box(10)
    .scale(2, 1, 1, { about: [5, 0, 0] })
    .name('stretched')
    .translate(200, 0, 0);

  // applyMatrix4 with a raw translation matrix.
  const moved = odm.box(10).applyMatrix4(new THREE.Matrix4().makeTranslation(300, 0, 7)).name('moved');

  // bounds() is a THREE.Box3: stack a lid exactly on moved's top face.
  const b = moved.bounds();
  const lid = odm.box([10, 10, 2]).translate(300, 0, b.max.z + 1).name('lid');

  // raycast() hits carry Vector3 point/normal.
  const hit = lid.raycast([300, 0, 50], [0, 0, -1]);
  if (Math.abs(hit.point.z - (b.max.z + 2)) > 1e-9) throw new Error('raycast point mismatch');
  if (Math.abs(hit.normal.z - 1) > 1e-9) throw new Error('raycast normal mismatch');

  return odm.group(spun, spun2, stretched, moved, lid);
}

export const checks = [
  // 1000 + 1000 + 2000 + 1000 + 200
  { volume: [5200, 1e-6] },
  // spun x ∈ [5,15], y ∈ [-15,-5]; spun2 the same + 100 in x;
  // stretched x ∈ [185,205]; moved z ∈ [2,12]; lid z ∈ [12,14].
  { bounds: { min: [5, -15, -5], max: [305, 5, 14], eps: 1e-9 } },
  { raycast: { origin: [10, -10, 50], dir: [0, 0, -1], distance: [45, 1e-6], name: 'spun' } },
  // The pivoted scale pushed the -x face out to x = 185.
  { raycast: { origin: [180, 0, 0], dir: [1, 0, 0], distance: [5, 1e-6], normal: [-1, 0, 0], name: 'stretched' } },
  { raycast: { origin: [300, 0, 50], dir: [0, 0, -1], distance: [36, 1e-6], name: 'lid' } },
  // Nothing remains where the un-pivoted box would have been.
  { raycast: { origin: [0, 0, 50], dir: [0, 0, -1], miss: true } },
];
