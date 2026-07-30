//! odm unstable
// Helper for invoke/root.js: bores a hole through whatever Solid it is given.
export const meta = {
  inputs: {
    blank: { type: 'solid' },
    r: { type: 'number', minimum: 0 },
  },
};

export default function build(ctx) {
  const r = ctx.input('r');
  console.log(`drilling r=${r}`);
  return ctx.input('blank').subtract(odm.cylinder(r, 100));
}
