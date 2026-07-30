//! odm unstable
//! A wheel: tire + hub + spokes, built along the Z axis.
export const meta = {
  inputs: {
    radius: { type: 'number', default: 8, minimum: 1, description: 'tire radius' },
  },
};

export default function build(ctx) {
  const r = ctx.get('radius');
  const tire = odm
    .cylinder({ r, h: 6 })
    .subtract(odm.cylinder({ r: r * 0.55, h: 8 }))
    .color('#222222')
    .name('tire');
  const hub = odm.cylinder({ r: r * 0.2, h: 7 }).color('silver').name('hub');

  const spokes = [];
  for (let i = 0; i < 5; i++) {
    spokes.push(
      odm
        .box([r * 0.55, 1.6, 4])
        .translate(r * 0.3, 0, 0)
        .rotateZ((i / 5) * 2 * Math.PI),
    );
  }
  return odm.group(tire, hub, odm.group(spokes).color('#999999').name('spokes')).name('wheel');
}
