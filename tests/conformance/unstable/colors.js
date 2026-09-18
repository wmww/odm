//! ODM API unstable
// Color contract: hex strings ('#rrggbb'/'#rgb') and [r,g,b]/[r,g,b,1]
// sRGB arrays, nothing else. Named colors and 0xRRGGBB numbers are
// deliberately rejected with pointed errors.
export const meta = {
  inputs: {
    c: { type: 'color', default: '#4682b4' },
  },
};

export default function build(ctx) {
  // .color() also takes a 4-array (alpha renders translucent — see
  // colors-alpha.js; the *wire* form stays canonical: 3 numbers).
  odm.box(1).color([1, 0, 0, 1]);
  return odm.box(10).color(ctx.input('c'));
}

export const checks = [
  { volume: [1000, 1e-9] }, // '#rrggbb' default
  { set: { c: '#f00' }, volume: [1000, 1e-9] }, // '#rgb' shorthand
  { set: { c: [1, 0, 0] }, volume: [1000, 1e-9] },
  { set: { c: 'steelblue' }, error: 'named colors are not supported' },
  // The wire form is exactly a string or 3 numbers — no alpha slot.
  { set: { c: [1, 0, 0, 1] }, error: 'declared type: color' },
];
