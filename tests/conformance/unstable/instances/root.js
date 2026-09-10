//! odm unstable
// Instances: what ctx.invoke returns, and how they compose. The headline
// property is sharing — one built subtree, placed many times, is interned
// once, so `meshes` counts distinct geometry rather than placements.
export default function build(ctx) {
  // One invoke, two placements: both wrap the same built subtree, so the
  // geometry is stored once however many times it is placed.
  const block = ctx.invoke('block.js');
  const twice = odm.group(block.translate(0, 0, 0).name('left'), block.translate(10, 0, 0).name('right'));

  // A Solid built here, passed to two different invokes: the geometry
  // crosses as a content hash, so both sides see the same mesh.
  const bar = odm.box([8, 2, 2]);
  const shared = odm.group(
    ctx.invoke('passthrough.js', { part: bar }).translate(0, 20, 0).name('pass-a'),
    ctx.invoke('passthrough.js', { part: bar }).translate(10, 20, 0).name('pass-b'),
  );

  // An instance of an instance: root -> outer.js -> block.js. The
  // transforms compose down the chain like any group.
  const nested = ctx.invoke('outer.js').translate(0, 40, 0).name('nested');

  // A colour on the Instance wrapper is a default for everything inside it
  // that has no colour of its own — like a Group's.
  const tinted = ctx.invoke('two-tone.js').translate(0, 60, 0).color('#ff0000').name('tinted');

  // A cascade value set on the outer invoke reaches the inner file, two
  // levels down: outer.js does not declare `size`, block.js does.
  const cascaded = ctx.invoke('outer.js', {}, { size: 6 }).translate(0, 80, 0).name('cascaded');

  return odm.group(twice, shared, nested, tinted, cascaded);
}

export const checks = [
  // Two placements of the 4-cube.
  { node: 'left', volume: [64, 1e-9] },
  { node: 'right', volume: [64, 1e-9], bounds: { min: [8, -2, -2], max: [12, 2, 2], eps: 1e-9 } },

  // outer.js offsets the block by (0, 0, 5) before root moves it to y=40.
  { node: 'nested', volume: [64, 1e-9], bounds: { min: [-2, 38, 3], max: [2, 42, 7], eps: 1e-9 } },

  // The cascade value crossed two invoke boundaries: a 6-cube, not a 4.
  { node: 'cascaded', volume: [216, 1e-9] },

  // Four distinct meshes behind eight instances: the 4-cube (`twice`'s two
  // placements and `nested`), the 8×2×2 bar (both passthroughs), the
  // 2-cube (two-tone's two halves), and the 6-cube the cascade produced.
  { meshes: 4 },

  // The Instance's colour reaches the uncoloured half of two-tone.js and
  // leaves the coloured half alone.
  {
    flat: [
      [null, 1], [null, 1], // twice: the two uncoloured blocks
      [null, 1], [null, 1], // shared: the two passthrough bars
      [null, 1], // nested
      ['#ff0000', 1], // tinted: inherits the Instance colour
      ['#00ff00', 1], // tinted: its own colour wins
      [null, 1], // cascaded
    ],
  },
];
