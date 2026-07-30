//! odm unstable
//! Primitives + booleans: an L-bracket with four bolt holes.
export default function build(ctx) {
  // Base plate on the XY floor, upright wall along the -X edge. Units: mm.
  const base = odm.box({ size: [60, 40, 6], center: false });
  const wall = odm.box({ size: [6, 40, 40], center: false });

  // A reusable hole cutter: cylinder along Z, long enough to pierce the base.
  const hole = odm.cylinder({ r: 4, h: 20, center: false }).translate(0, 0, -5);

  const bracket = base
    .union(wall)
    .subtract(
      hole.translate(24, 12, 0),
      hole.translate(46, 12, 0),
      hole.translate(24, 28, 0),
      hole.translate(46, 28, 0),
    );

  return bracket.color('steelblue').name('bracket');
}
