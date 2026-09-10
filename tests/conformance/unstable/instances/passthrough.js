//! odm unstable
// Takes a Solid across the invoke boundary and hands it straight back: the
// geometry travels as a content hash, so it is never rebuilt or re-interned.
export const meta = {
  inputs: {
    part: { type: 'solid' },
  },
};

export default function build(ctx) {
  return ctx.input('part');
}
