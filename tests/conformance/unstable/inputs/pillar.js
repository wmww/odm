//! odm unstable
// A 1×1×lift box; `lift` cascades (default 1).
export const meta = {
  inputs: {
    lift: { type: 'number', cascade: true, default: 1 },
  },
};

export default function build(ctx) {
  const lift = ctx.get('lift');
  return odm.box({ size: [1, 1, lift], center: false }).name('pillar');
}
