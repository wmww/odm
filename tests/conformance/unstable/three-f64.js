//! ODM API unstable
// f32-regression tripwire, pinning three layers at once: the vendored three
// generators must emit f64 position attributes, fromThreeGeometry must not
// coerce to Float32Array, and op_solid_from_mesh must take f64. 0.1 is not an
// f32 value, so the eps-0 bounds check fails if any of them quantizes.
export default function build() {
  return odm.fromThreeGeometry(new THREE.BoxGeometry(0.2, 0.2, 0.2));
}

export const checks = [
  { bounds: { min: [-0.1, -0.1, -0.1], max: [0.1, 0.1, 0.1], eps: 0 } },
  { volume: [0.008, 1e-12] },
];
