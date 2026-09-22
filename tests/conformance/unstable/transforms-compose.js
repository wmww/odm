//! ODM API unstable
// How transforms compose. Transforms are world-frame and applied in call
// order — each call left-multiplies onto the pending matrix — so order
// matters and the identities below are the ones a part can rely on.
//
// "Same solid" is asserted by symmetric difference: if a ⊖ b and b ⊖ a are
// both empty, the two solids are the same set of points. That is stronger
// than comparing bounds, and it is exact CSG arithmetic, not a tolerance on
// matrix entries.
export default function build() {
  const same = (a, b, what) => {
    const left = a.subtract(b).volume();
    const right = b.subtract(a).volume();
    // Slivers from two ways of arriving at the same matrix, not overlap.
    if (left > 1e-6 || right > 1e-6) {
      throw new Error(`${what}: symmetric difference ${left} / ${right}`);
    }
  };
  const differs = (a, b, what) => {
    if (a.subtract(b).volume() <= 1e-6 && b.subtract(a).volume() <= 1e-6) {
      throw new Error(`${what}: expected these to differ, they are the same solid`);
    }
  };

  const a = odm.box(10);
  const b = odm.box(4);

  // Moving the tool the other way is the same cut: a is at the origin, so
  // subtracting b at -5 equals moving a to +5 and subtracting b at 0 —
  // *relative* placement is what a boolean sees. But the results sit in
  // different places, so they are different solids.
  const movedFirst = a.translate(5, 0, 0).subtract(b);
  const movedTool = a.subtract(b.translate(-5, 0, 0));
  differs(movedFirst, movedTool, 'cut placement');
  same(movedFirst, movedTool.translate(5, 0, 0), 'cut equivalence after realigning');

  // { about: p } is exactly the conjugation you would write by hand.
  const arm = odm.box([20, 4, 4]).translate(30, 0, 0);
  const p = [20, 0, 0];
  same(
    arm.rotateZ(odm.deg(90), { about: p }),
    arm.translate(-p[0], -p[1], -p[2]).rotateZ(odm.deg(90)).translate(p[0], p[1], p[2]),
    'about-pivot conjugation',
  );

  // applyMatrix4 takes the composed matrix the chained calls would build.
  // Call order left-multiplies, so `.rotateZ(r).translate(t)` is T·R.
  const r = odm.deg(30);
  const M = new THREE.Matrix4()
    .makeTranslation(10, 0, 0)
    .multiply(new THREE.Matrix4().makeRotationZ(r));
  same(arm.rotateZ(r).translate(10, 0, 0), arm.applyMatrix4(M), 'applyMatrix4 composition');
  // A column-major array of 16 is the same argument.
  same(arm.applyMatrix4(M), arm.applyMatrix4(M.elements.slice()), 'applyMatrix4 array form');

  // The three axis shorthands are the general form with a unit axis, and
  // they turn the right way: a right-handed rotation about each axis.
  same(arm.rotateX(r), arm.rotate([1, 0, 0], r), 'rotateX = rotate([1,0,0])');
  same(arm.rotateY(r), arm.rotate([0, 1, 0], r), 'rotateY = rotate([0,1,0])');
  same(arm.rotateZ(r), arm.rotate([0, 0, 1], r), 'rotateZ = rotate([0,0,1])');
  // Right-handed: +x rotated 90° about z lands on +y, about y lands on -z.
  const probe = odm.box(2).translate(10, 0, 0);
  const q = odm.deg(90);
  same(probe.rotateZ(q), odm.box(2).translate(0, 10, 0), 'rotateZ is right-handed');
  same(probe.rotateY(q), odm.box(2).translate(0, 0, -10), 'rotateY is right-handed');
  same(odm.box(2).translate(0, 10, 0).rotateX(q), odm.box(2).translate(0, 0, 10), 'rotateX is right-handed');

  // Rotate-then-scale is not scale-then-rotate: the scale is world-axis
  // aligned either way, so it stretches a different direction of the solid.
  const bar = odm.box([20, 2, 2]);
  differs(bar.rotateZ(odm.deg(90)).scale(3, 1, 1), bar.scale(3, 1, 1).rotateZ(odm.deg(90)), 'rotate∘scale order');

  // A mirror is a legal transform, not a broken one: volume stays positive
  // and the surface still faces outward (the raycast normal below).
  const mirrored = odm.box(10).scale(-1, 1, 1).name('mirrored');
  if (mirrored.volume() <= 0) throw new Error(`mirror volume ${mirrored.volume()}`);

  // A group's transform composes onto its children's, in the same order.
  const inGroup = odm.group(odm.box(4).translate(2, 0, 0)).translate(0, 6, 0).name('in-group');

  return odm.group(mirrored.translate(0, 0, 0), inGroup);
}

export const checks = [
  // The mirror is a whole box again, in the same place (it is symmetric).
  { node: 'mirrored', volume: [1000, 1e-9], bounds: { min: [-5, -5, -5], max: [5, 5, 5], eps: 1e-9 } },
  // ...and its faces point out: the +x face's normal is +x, not -x.
  { raycast: { origin: [100, 0, 0], dir: [-1, 0, 0], distance: [95, 1e-9], normal: [1, 0, 0], name: 'mirrored' } },

  // Group transform + child transform add up: 2 + 0 on x, 0 + 6 on y.
  { node: 'in-group', bounds: { min: [0, 4, -2], max: [4, 8, 2], eps: 1e-9 } },

  { volume: [1064, 1e-9] }, // 1000 + 4³
];
