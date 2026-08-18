//! odm unstable
//! One bar per entry of the `bars` array input: the array's *length* is
//! part of what the input sets, so editing it adds and removes geometry.
export const meta = {
  inputs: {
    bars: { type: 'array', items: { type: 'number', minimum: 0 }, default: [5, 10, 5] },
    detail: {
      type: 'integer',
      cascade: true,
      default: 24,
      minimum: 3,
      maximum: 96,
      description: 'segments on round surfaces',
    },
  },
};

const PITCH = 7;

export default function build(ctx) {
  const bars = ctx.input('bars');
  const detail = ctx.input('detail');
  const cols = bars.map((h, i) =>
    odm
      .cylinder(2.5, Math.max(h, 0.5), { center: false, segments: detail })
      .translate(i * PITCH, 0, 0)
      .name(`bar-${i}`),
  );
  return odm
    .group(cols)
    .translate((-(bars.length - 1) * PITCH) / 2, 0, 0)
    .color('#7fb069'); // named by whoever places it
}
