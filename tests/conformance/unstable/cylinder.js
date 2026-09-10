//! odm unstable
// odm.cylinder(r, h, opts): axis along Z, centered by default, `segments`
// (default 64) explicit and >= 3, `r2` making a frustum.
//
// A cylinder is the prism over the regular n-gon *inscribed* in radius r,
// whose area is (n/2)·r²·sin(2π/n) — so low segment counts are exact, no
// discretization band needed.
export const meta = {
  inputs: {
    mode: { enum: ['ok', 'two-segments', 'one-segment', 'unknown-opt'], default: 'ok' },
  },
};

/** Area of the regular n-gon inscribed in a circle of radius r. */
const ngon = (n, r) => (n / 2) * r * r * Math.sin((2 * Math.PI) / n);

export default function build(ctx) {
  const mode = ctx.input('mode');
  if (mode === 'two-segments') return odm.cylinder(1, 1, { segments: 2 });
  if (mode === 'one-segment') return odm.cylinder(1, 1, { segments: 1 });
  if (mode === 'unknown-opt') return odm.cylinder(1, 1, { sides: 8 });

  // Square prism: the 4-gon inscribed in r=3 has area 2r² = 18, height 10.
  const square = odm.cylinder(3, 10, { segments: 4 }).name('square');

  // Hexagonal prism: (3√3/2)·r²·h.
  const hex = odm.cylinder(3, 10, { segments: 6 }).translate(30, 0, 0).name('hex');

  // Frustum of the 4-gon prism: V = (h/3)(A₁ + A₂ + √(A₁A₂)), and with
  // A = k·r² the k factors out to k·h·(r² + r·r2 + r2²)/3.
  const cone = odm
    .cylinder(6, 9, { r2: 2, segments: 4 })
    .translate(60, 0, 0)
    .name('cone');

  // center: false puts the base at z = 0 — the whole solid is z ∈ [0, h].
  const based = odm.cylinder(2, 7, { segments: 4, center: false }).translate(90, 0, 0).name('based');

  return odm.group(square, hex, cone, based);
}

const SQUARE_V = ngon(4, 3) * 10; // 180
const HEX_V = ngon(6, 3) * 10; // 233.8
const CONE_V = ((ngon(4, 1) * 9) / 3) * (6 * 6 + 6 * 2 + 2 * 2); // k=2: 312
const BASED_V = ngon(4, 2) * 7; // 56

export const checks = [
  { node: 'square', volume: [SQUARE_V, 1e-6] },
  { node: 'hex', volume: [HEX_V, 1e-6] },
  { node: 'cone', volume: [CONE_V, 1e-6] },
  { node: 'based', volume: [BASED_V, 1e-6] },

  // Centered on the origin: z ∈ [-h/2, h/2]. (x/y bounds depend on where
  // the n-gon's first vertex sits, so they are not pinned here.)
  { node: 'square', bounds: { min: [-3, -3, -5], max: [3, 3, 5], eps: 1e-9 } },
  // ...and `center: false` shifts it to z ∈ [0, h].
  { node: 'based', bounds: { min: [88, -2, 0], max: [92, 2, 7], eps: 1e-9 } },

  // The flat top of the centered cylinder is exactly z = +5.
  { raycast: { origin: [0, 0, 50], dir: [0, 0, -1], distance: [45, 1e-9], normal: [0, 0, 1], name: 'square' } },

  // Segments are always explicit and a polygon needs three sides.
  { set: { mode: 'two-segments' }, error: 'segments' },
  { set: { mode: 'one-segment' }, error: 'segments' },
  { set: { mode: 'unknown-opt' }, error: "unknown cylinder option 'sides'" },
];
