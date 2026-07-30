//! odm unstable
// Context semantics: ctx.t drives geometry, ctx.param reads odm.json params
// (with defaults for missing keys).
export default function build(ctx) {
  const w = ctx.param('width', 1); // odm.json says 10
  const d = ctx.param('missing', 4); // not in odm.json → default
  const h = 1 + ctx.t;
  return odm.box([w, d, h]).name('slab');
}

export const checks = [
  { volume: [10 * 4 * 1, 1e-9] }, // t defaults to 0
  { t: 2, volume: [10 * 4 * 3, 1e-9] },
  { t: 2, bounds: { min: [-5, -2, -1.5], max: [5, 2, 1.5], eps: 1e-9 } },
];
