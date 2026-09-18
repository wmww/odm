//! ODM API unstable
// Translucency: color alpha and node opacity are accepted and validated.
// Alpha renders translucent (depth-peeled); geometry queries are unaffected.
export const meta = {
  inputs: {
    a: { type: 'number', default: 0.5 },
    o: { type: 'number', default: 1 },
  },
};

export default function build(ctx) {
  const a = ctx.input('a');
  odm.box(1).color('#ff000080'); // '#rrggbbaa' hex parses
  const outer = odm.box(10).color([1, 0, 0, a]);
  // .opacity() multiplies down the tree; chaining multiplies too.
  return odm.group(outer, odm.box(2).name('inner')).opacity(ctx.input('o'));
}

export const checks = [
  { volume: [1008, 1e-9] },
  { set: { o: 0.25 }, volume: [1008, 1e-9] },
  { set: { a: 2 }, error: 'alpha must be in 0..1' },
  { set: { o: 1.5 }, error: 'opacity must be in 0..1' },
  { set: { o: -0.1 }, error: 'opacity must be in 0..1' },
];
