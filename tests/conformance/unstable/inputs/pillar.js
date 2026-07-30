//! odm unstable
// A 1×1×lift box; `lift` cascades (default 1).
export const meta = {
  inputs: {
    lift: { type: 'number', cascade: true, default: 1 },
  },
};

export default function build(ctx) {
  const lift = ctx.input('lift');
  return odm.box([1, 1, lift], { center: false }).name('pillar');
}
