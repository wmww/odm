//! ODM API unstable
//! An open box with a stacking lip; every dimension is an input.
export const meta = {
  inputs: {
    width: { type: 'number', default: 60, minimum: 1, description: 'outer width (X)' },
    depth: { type: 'number', default: 40, minimum: 1, description: 'outer depth (Y)' },
    height: { type: 'number', default: 30, minimum: 1, description: 'outer height (Z)' },
    wall: { type: 'number', default: 3, minimum: 0.4, description: 'wall thickness' },
  },
  presets: {
    shallow: { height: 12 },
    chunky: { wall: 6, height: 40 },
  },
};

export default function build(ctx) {
  const width = ctx.input('width');
  const depth = ctx.input('depth');
  const height = ctx.input('height');
  const wall = ctx.input('wall');

  const outer = odm.box([width, depth, height], { center: false });
  const cavity = odm
    .box([width - 2 * wall, depth - 2 * wall, height], { center: false })
    .translate(wall, wall, wall);

  const lip = ctx.invoke('lip.js', { width, depth, wall });

  return odm.group(outer.subtract(cavity).color('#cc8844').name('body'), lip);
}
