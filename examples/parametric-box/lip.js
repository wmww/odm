// A separate doohickey so editing main.js alone shows partial rebuilds
// (this one stays memoized).
export default function build(ctx) {
  const { width = 60, depth = 40, wall = 3 } = ctx.args;
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
    .extrude([outline, inner], { height: wall / 2 })
    .color('#aa6622')
    .name('lip');
}
