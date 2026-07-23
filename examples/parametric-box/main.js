// parametric-box: sizes come from odm.json params — edit them (or the
// defaults here) and watch the rebuild.
export default function build(ctx) {
  const width = ctx.param('width', 60);
  const depth = ctx.param('depth', 40);
  const height = ctx.param('height', 30);
  const wall = ctx.param('wall', 3);

  const outer = odm.box({ size: [width, depth, height], center: false });
  const cavity = odm
    .box({ size: [width - 2 * wall, depth - 2 * wall, height], center: false })
    .translate(wall, wall, wall);

  const lip = ctx.invoke('lip.js', { width, depth, wall });

  return odm.group(outer.subtract(cavity).color('#cc8844').name('body'), lip);
}
