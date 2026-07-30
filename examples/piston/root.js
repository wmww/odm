//! odm unstable
//! A crank–rod–piston assembly; motion is a pure function of the cascade
//! input `t`. Declaring t with range 0–2 makes the viewer loop it every
//! two seconds (one crank revolution at the default rpm).
export const meta = {
  inputs: {
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 },
    rpm: { type: 'number', default: 30, minimum: 1, description: 'crank speed' },
  },
};

export default function build(ctx) {
  const rpm = ctx.input('rpm');
  const a = ctx.input('t') * 2 * Math.PI * (rpm / 60); // crank angle
  const R = 10; // crank radius
  const L = 30; // connecting rod length

  const pin = [R * Math.sin(a), 0, R * Math.cos(a)];
  const pistonZ = R * Math.cos(a) + Math.sqrt(L * L - pin[0] * pin[0]);

  // Crank disc + pin rotate about the Y axis at the origin.
  const crank = odm
    .cylinder(14, 6)
    .rotateX(odm.deg(90))
    .color('#555566')
    .name('crank');
  const crankPin = odm
    .cylinder(3, 12)
    .rotateX(odm.deg(90))
    .translate(pin[0], 0, pin[2])
    .color('#c0c0c0')
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
    .cylinder(9, 16)
    .translate(0, 0, pistonZ + 6)
    .color('#aa6633')
    .name('piston');

  return odm.group(crank, crankPin, rod, piston).name('piston-assembly');
}
