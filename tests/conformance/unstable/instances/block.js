//! odm unstable
// A cube whose size is a cascade input, so a value set at any outer invoke
// (or at the view) reaches it without every level redeclaring it.
export const meta = {
  inputs: {
    size: { type: 'number', cascade: true, default: 4 },
  },
};

export default function build(ctx) {
  return odm.box(ctx.input('size')).name('block');
}
