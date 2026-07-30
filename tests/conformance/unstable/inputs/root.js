//! odm unstable
// Unified inputs: plain inputs come from the immediate caller (the view
// here) with declared defaults; cascade inputs resolve up the invoke chain,
// view outermost, nearest provider winning; a declaration auto-provides its
// default for its own subtree.
export const meta = {
  inputs: {
    width: { type: 'number', default: 10 },
    depth: { type: 'number', default: 4 },
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 4 },
  },
};

export default function build(ctx) {
  const w = ctx.input('width');
  const d = ctx.input('depth');
  const h = 1 + ctx.input('t');
  return odm.group(
    odm.box([w, d, h]).name('slab'),
    // The explicit provide wins over pillar.js's own default (1)...
    ctx.invoke('pillar.js', {}, { lift: 2 }).translate(50, 0, 0).name('provided'),
    // ...and this one falls through to the view, else the declared default.
    ctx.invoke('pillar.js').translate(100, 0, 0).name('fallthrough'),
  );
}

export const checks = [
  // Defaults: 10×4×1 slab + pillars of volume 2 and 1.
  { volume: [43, 1e-9] },
  // t reaches the slab height as a view-level provide.
  { t: 2, volume: [123, 1e-9] },
  // A plain input set at the view (becomes a view arg).
  { set: { width: 3 }, volume: [15, 1e-9] },
  // A cascade set at the view reaches only the fall-through pillar; the
  // explicit provide still wins for the other (nearest provider).
  { set: { lift: 5 }, volume: [47, 1e-9] },
  // Values are validated against the declared schema at the boundary.
  { set: { width: 'wide' }, error: 'input "width"' },
];
