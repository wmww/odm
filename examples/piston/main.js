//! odm unstable
// piston: build(t) animation. The crank angle is a pure function of ctx.t;
// scrub the timeline (or `odm render --t 1.25`) to see it move.
export default function build(ctx) {
  const rpm = ctx.param('rpm', 30);
  const a = ctx.t * 2 * Math.PI * (rpm / 60); // crank angle
  const R = 10; // crank radius
  const L = 30; // connecting rod length

  const pin = [R * Math.sin(a), 0, R * Math.cos(a)];
  const pistonZ = R * Math.cos(a) + Math.sqrt(L * L - pin[0] * pin[0]);

  // Crank disc + pin rotate about the Y axis at the origin.
  const crank = odm
    .cylinder({ r: 14, h: 6 })
    .rotateX(odm.deg(90))
    .color('#555566')
    .name('crank');
  const crankPin = odm
    .cylinder({ r: 3, h: 12 })
    .rotateX(odm.deg(90))
    .translate(pin[0], 0, pin[2])
    .color('silver')
    .name('crank-pin');

  // Rod from the crank pin up to the piston pin.
  const tilt = Math.asin(-pin[0] / L);
  const rod = odm
    .box([6, 4, L])
    .translate(0, 0, L / 2)
    .rotateY(tilt)
    .translate(pin[0], 0, pin[2])
    .color('#8899aa')
    .name('rod');

  const piston = odm
    .cylinder({ r: 9, h: 16 })
    .translate(0, 0, pistonZ + 6)
    .color('#aa6633')
    .name('piston');

  return odm.group(crank, crankPin, rod, piston).name('piston-assembly');
}
