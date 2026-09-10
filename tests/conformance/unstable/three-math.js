//! odm unstable
// The THREE math types are accepted wherever the array form is, and the
// rotation constructions agree. Equalities are asserted here in JS (a wrong
// answer throws, which the runner reports as a failed build); one summary
// solid goes back to the runner.
export default function build() {
  const same = (a, b, what) => {
    const l = a.subtract(b).volume();
    const r = b.subtract(a).volume();
    if (l > 1e-6 || r > 1e-6) throw new Error(`${what}: symmetric difference ${l} / ${r}`);
  };
  const near = (x, want, eps, what) => {
    if (Math.abs(x - want) > eps) throw new Error(`${what}: ${x}, wanted ${want}`);
  };

  const bar = odm.box([20, 4, 4]);
  const r = odm.deg(37);

  // A Vector3 is a rotate axis, and a Vector3 is an { about } pivot.
  same(bar.rotate([0, 0, 1], r), bar.rotate(new THREE.Vector3(0, 0, 1), r), 'Vector3 axis');
  same(
    bar.rotateZ(r, { about: [3, 0, 0] }),
    bar.rotateZ(r, { about: new THREE.Vector3(3, 0, 0) }),
    'Vector3 pivot',
  );

  // ...and a Vector3 is a raycast origin and dir.
  const box = odm.box(10);
  const byArray = box.raycast([0, 0, 50], [0, 0, -1]);
  const byVector = box.raycast(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
  near(byVector.distance, byArray.distance, 0, 'Vector3 raycast distance');

  // The hit is three-shaped: named components, not indices.
  if (!(byArray.point instanceof THREE.Vector3)) throw new Error('raycast point is not a Vector3');
  if (!(byArray.normal instanceof THREE.Vector3)) throw new Error('raycast normal is not a Vector3');
  near(byArray.point.z, 5, 1e-9, 'hit point.z');

  // bounds() is a Box3, so getSize/getCenter work on it.
  const b = odm.box([2, 4, 6]).translate(1, 0, 0).bounds();
  if (!(b instanceof THREE.Box3)) throw new Error('bounds() is not a Box3');
  const size = b.getSize(new THREE.Vector3());
  near(size.x, 2, 1e-9, 'size.x');
  near(size.y, 4, 1e-9, 'size.y');
  near(size.z, 6, 1e-9, 'size.z');
  near(b.getCenter(new THREE.Vector3()).x, 1, 1e-9, 'center.x');

  // The four ways to say "rotate r about z" agree.
  const viaMatrix = bar.applyMatrix4(new THREE.Matrix4().makeRotationZ(r));
  same(bar.rotateZ(r), viaMatrix, 'makeRotationZ');
  const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), r);
  same(bar.rotateZ(r), bar.applyMatrix4(new THREE.Matrix4().makeRotationFromQuaternion(q)), 'Quaternion');
  const e = new THREE.Euler(0, 0, r);
  same(bar.rotateZ(r), bar.applyMatrix4(new THREE.Matrix4().makeRotationFromEuler(e)), 'Euler');
  same(bar.rotateZ(r), bar.rotate([0, 0, 1], r), 'axis-angle');

  // An unnormalized axis is normalized for you.
  same(bar.rotate([0, 0, 7], r), bar.rotateZ(r), 'unnormalized axis');

  // odm.deg is three's degToRad, bit for bit — no second conversion table.
  for (const d of [0, 1, 30, 45, 90, 180, -37.5]) {
    if (THREE.MathUtils.degToRad(d) !== odm.deg(d)) throw new Error(`deg(${d}) disagrees with degToRad`);
  }

  // Vector2s are profile points wherever [x, y] pairs are.
  const pts = [[0, 0], [10, 0], [10, 4], [0, 4]];
  same(
    odm.extrude(pts, 3),
    odm.extrude(pts.map((p) => new THREE.Vector2(p[0], p[1])), 3),
    'Vector2 profile points',
  );

  // One solid the runner can measure: the bar, rotated the long way round.
  return bar.rotateZ(odm.deg(90)).name('bar');
}

export const checks = [
  { node: 'bar', volume: [320, 1e-9], bounds: { min: [-2, -10, -2], max: [2, 10, 2], eps: 1e-9 } },
];
