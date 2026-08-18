//! odm unstable
//! The enum and check-box exhibit: `roof` picks a shape, `windows` cuts
//! openings. It also declares the cascade input `detail` — root.js never
//! mentions it, yet it falls through and the view can set it.
export const meta = {
  inputs: {
    roof: { enum: ['flat', 'gabled', 'domed'], default: 'flat' },
    windows: { type: 'boolean', default: true },
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

const W = 18; // footprint
const H = 20; // wall height

export default function build(ctx) {
  const detail = ctx.input('detail');

  let body = odm.box([W, W, H], { center: false }).translate(-W / 2, -W / 2, 0);
  if (ctx.input('windows')) {
    for (const x of [-5, 5]) {
      body = body.subtract(odm.box([4, W + 2, 5]).translate(x, 0, 12));
    }
  }
  body = body.color('#c8b18a').name('tower-body');

  // Unnamed: root.js names the instance where it places it.
  return odm.group(body, roofOf(ctx.input('roof'), detail));
}

function roofOf(kind, detail) {
  if (kind === 'flat') {
    const w = W + 4;
    return odm
      .box([w, w, 2], { center: false })
      .translate(-w / 2, -w / 2, H)
      .color('#7a6b52')
      .name('roof');
  }
  if (kind === 'domed') {
    return odm
      .sphere(W / 2, { segments: detail })
      .scale(1, 1, 0.8)
      .translate(0, 0, H)
      .color('#7a6b52')
      .name('roof');
  }
  // Gabled: a triangular prism, extruded along Z then laid along -Y.
  const w = W / 2 + 1;
  return odm
    .extrude(
      [
        [-w, 0],
        [w, 0],
        [0, 9],
      ],
      W + 2,
    )
    .rotateX(odm.deg(90))
    .translate(0, (W + 2) / 2, H)
    .color('#7a6b52')
    .name('roof');
}
