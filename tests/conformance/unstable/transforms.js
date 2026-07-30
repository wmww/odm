//! odm unstable
// Transform semantics: chaining order (world-frame, applied left-multiplied),
// volume under scale (×|det|), bounds under rotation, raycast through
// transformed instances.
export default function build() {
  // Scaled box: 10×10×10 → 20×10×10, volume ×2.
  const wide = odm.box(10).scale(2, 1, 1).name('wide');

  // Rotated 45° about Z: xy footprint grows to ±10/√2·2 = ±7.0711…
  const diamond = odm.box(10).rotateZ(odm.deg(45)).name('diamond').translate(100, 0, 0);

  // Chain order: translate then rotate happens in the world frame, so the
  // rotation swings the translated box around the origin. +5x then 90° about
  // Z lands the center on +5y.
  const swung = odm.box(2).translate(5, 0, 0).rotateZ(odm.deg(90)).name('swung').translate(200, 0, 0);

  return odm.group(wide, diamond, swung);
}

const S2 = Math.SQRT2;

export const checks = [
  { volume: [2000 + 1000 + 8, 1e-6] },
  {
    bounds: {
      // wide: x ±10; diamond: ±5√2 around (100,0); swung: 2-box at (200, 5).
      min: [-10, -5 * S2, -5],
      max: [201, 5 * S2, 5],
      eps: 1e-9,
    },
  },
  // Down onto the scaled box: top face still z = +5, but x = ±10 is solid.
  { raycast: { origin: [9, 0, 50], dir: [0, 0, -1], distance: [45, 1e-6], name: 'wide' } },
  // Rotation is about Z, so the diamond's top face stays flat at z = +5.
  { raycast: { origin: [100, 0, 50], dir: [0, 0, -1], distance: [45, 1e-6], normal: [0, 0, 1], name: 'diamond' } },
  // The swung box really is at (200, 5, 0): probe straight down through it.
  { raycast: { origin: [200, 5, 50], dir: [0, 0, -1], distance: [49, 1e-6], name: 'swung' } },
  // And nothing remains at its pre-rotation spot (205, 0).
  { raycast: { origin: [205, 0, 50], dir: [0, 0, -1], miss: true } },
];
