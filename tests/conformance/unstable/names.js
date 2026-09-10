//! odm unstable
// .name(n): the label the viewer shows, `odm inspect <name>` addresses, and
// a raycast hit reports. Names are strings — anything else is stringified —
// and a second .name() replaces rather than appends.
export default function build() {
  // The name survives the transforms applied after it: the whole point is
  // that you can label a part once and place it afterwards.
  const moved = odm.box(2).name('moved').translate(0, 0, 10).rotateZ(odm.deg(45)).scale(1, 1, 2);

  // Non-strings are stringified, not rejected: name(123) is '123'.
  const numbered = odm.box(2).name(123).translate(10, 0, 0);

  // A second name replaces the first — one label per node, always the last.
  const renamed = odm.box(2).name('first').name('second').translate(20, 0, 0);

  // CSG keeps the *first* operand's name (docs/api/csg.md), and only that
  // one: the cutter's name goes with its geometry.
  const cut = odm.box(4).name('body').subtract(odm.box(2).translate(1, 1, 1).name('cutter')).translate(30, 0, 0);

  // A group's name is its own; it does not reach the solids inside.
  const boxed = odm.group(odm.box(2).translate(40, 0, 0)).name('crate');

  return odm.group(moved, numbered, renamed, cut, boxed);
}

export const checks = [
  // Addressable by name, with the post-name transforms folded in: the box
  // was scaled 2x on z about the origin, so its z half-extent is 2, centred
  // on z = 20. Rotation about z leaves an unrotated square's AABB alone
  // only at 45° if you account for the diagonal: 2·√2/2 = √2 each way.
  { node: 'moved', volume: [16, 1e-9], bounds: { min: [-Math.SQRT2, -Math.SQRT2, 18], max: [Math.SQRT2, Math.SQRT2, 22], eps: 1e-9 } },

  // name(123) stores the string '123' — checked through raycast below,
  // since a purely numeric name reads as an index path to node addressing.

  // The replaced name is gone, not shadowed.
  { node: 'second', volume: [8, 1e-9] },

  // 4³ minus the 2³ corner it overlaps: 64 − 8. The cutter is not a node.
  { node: 'body', volume: [56, 1e-9] },

  { node: 'crate', volume: [8, 1e-9] },

  // The name reaches raycast, which is how a CLI hit is attributed.
  { raycast: { origin: [10, 0, 50], dir: [0, 0, -1], distance: [49, 1e-9], name: '123' } },
  { raycast: { origin: [30, 0, 50], dir: [0, 0, -1], distance: [48, 1e-9], name: 'body' } },
  // A group name does not reach the solid inside it: the hit is unnamed.
  { raycast: { origin: [40, 0, 50], dir: [0, 0, -1], distance: [49, 1e-9], name: '' } },
];
