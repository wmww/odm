//! odm unstable
//! One gallery object: a shape at a position. root.js invokes it once per
//! `objects` element, so editing one element rebuilds one marker and
//! memo-hits the rest (`"stats": true` shows it).
export const meta = {
  inputs: {
    position: { type: 'vector3', default: [0, 0, 0] },
    shape: {
      variants: {
        box: { properties: { size: { type: 'vector3', default: [8, 8, 8] } } },
        sphere: { properties: { radius: { type: 'number', default: 5, minimum: 0.5 } } },
      },
      default: { kind: 'box' },
    },
  },
};

export default function build(ctx) {
  const at = ctx.input('position');
  const shape = ctx.input('shape');
  const solid =
    shape.kind === 'sphere'
      ? odm.sphere(shape.radius, { segments: 32 })
      : odm.box([shape.size.x, shape.size.y, shape.size.z]);
  return solid.translate(at.x, at.y, at.z);
}
