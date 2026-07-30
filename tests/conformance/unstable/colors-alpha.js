//! odm unstable
// The renderer has no blending; a translucent color errors rather than
// silently drawing opaque.
export default function build() {
  return odm.box(10).color([1, 0, 0, 0.5]);
}

export const checks = [
  { error: 'alpha must be 1' },
];
