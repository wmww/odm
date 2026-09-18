//! ODM API unstable
// Cross-doohickey invocation: JSON + Solid args cross the boundary, the
// result comes back as an Instance, and console output is captured from
// both files.
export default function build(ctx) {
  const blank = odm.box([10, 10, 2]);
  const drilled = ctx.invoke('drill.js', { blank, r: 2 });
  console.log('drilled one blank');
  return odm.group(drilled.name('plate'), drilled.translate(20, 0, 0).name('copy'));
}

export const checks = [
  // Plate 10×10×2 minus a through-hole r=2 (64-gon): 200 − π·4·2, twice.
  { volume: [2 * (200 - Math.PI * 4 * 2), 0.2] },
  { bounds: { min: [-5, -5, -1], max: [25, 5, 1], eps: 1e-9 } },
  // Down the hole axis: nothing there.
  { raycast: { origin: [0, 0, 50], dir: [0, 0, -1], miss: true } },
  // Next to the hole: the *copy*'s top face at z = +1. The name on the
  // Instance wrapper attributes the hit, which is how two copies of one
  // content-addressed subtree are told apart.
  { raycast: { origin: [24, 0, 50], dir: [0, 0, -1], distance: [49, 1e-6], name: 'copy' } },
  { raycast: { origin: [4, 0, 50], dir: [0, 0, -1], distance: [49, 1e-6], name: 'plate' } },
  { console: ['drilling r=2', 'drilled one blank'] },
];
