//! ODM API unstable
//! The box's stacking lip. A separate part so editing root.js alone
//! shows partial rebuilds (this one stays memoized).
export const meta = {
  inputs: {
    width: { type: 'number', default: 60 },
    depth: { type: 'number', default: 40 },
    wall: { type: 'number', default: 3 },
  },
};

export default function build(ctx) {
  const width = ctx.input('width');
  const depth = ctx.input('depth');
  const wall = ctx.input('wall');
  const outline = [
    [0, 0],
    [width, 0],
    [width, depth],
    [0, depth],
  ];
  const inner = [
    [wall / 2, wall / 2],
    [wall / 2, depth - wall / 2],
    [width - wall / 2, depth - wall / 2],
    [width - wall / 2, wall / 2],
  ];
  return odm
    .extrude([outline, inner], wall / 2)
    .color('#aa6622')
    .name('lip');
}
