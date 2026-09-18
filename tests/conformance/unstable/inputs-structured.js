//! ODM API unstable
// Structured inputs: extension types at any depth, nested defaults applied
// at normalization, tagged unions (`variants`) selected by tag, and
// string-keyed maps (`additionalProperties`). ctx.input hydrates THREE
// instances at every extension-typed position.
export const meta = {
  inputs: {
    objects: {
      type: 'array',
      default: [{ kind: 'box' }],
      items: {
        type: 'object',
        properties: {
          position: { type: 'vector3', default: [0, 0, 0] },
          kind: { enum: ['box', 'sphere'], default: 'box' },
          size: { type: 'number', default: 2, minimum: 0.1 },
        },
      },
    },
    anchors: { type: 'object', default: {}, additionalProperties: { type: 'vector3' } },
    shape: {
      variants: {
        box: { properties: { size: { type: 'vector3', default: [2, 2, 2] } } },
        sphere: { properties: { radius: { type: 'number', minimum: 0.1 } }, required: ['radius'] },
      },
      default: { kind: 'box' },
    },
  },
};

export default function build(ctx) {
  const objects = ctx.input('objects');
  const parts = objects.map((o, i) => {
    if (!(o.position instanceof THREE.Vector3)) throw new Error('position not hydrated at depth');
    const s = o.kind === 'sphere' ? odm.sphere(o.size / 2) : odm.box(o.size);
    return s.translate(o.position.x, o.position.y, o.position.z).name(`obj-${i}`);
  });
  const anchors = ctx.input('anchors');
  const pins = Object.entries(anchors).map(([name, at]) => {
    if (!(at instanceof THREE.Vector3)) throw new Error('anchor value not hydrated');
    return odm.box(1).translate(at.x, at.y, at.z).name(name);
  });
  const sh = ctx.input('shape');
  let shape;
  if (sh.kind === 'box') {
    if (!(sh.size instanceof THREE.Vector3)) throw new Error('union member not hydrated');
    shape = odm.box([sh.size.x, sh.size.y, sh.size.z]);
  } else {
    shape = odm.sphere(sh.radius, { segments: 64 });
  }
  console.log('objects:', JSON.stringify(objects.length), 'anchors:', Object.keys(anchors).length);
  return odm.group(parts, pins, shape.translate(0, 0, 20).name('shape'));
}

export const checks = [
  // Defaults through the tree: one all-default box element (kind/size/
  // position filled from nested defaults) + the union default's branch
  // defaults (box [2, 2, 2]). 8 + 8.
  { volume: [16, 1e-9] },
  // Deep normalize: a THREE-form position inside an array element
  // canonicalizes; absent element properties fill from their defaults.
  { set: { objects: [{ size: 3, position: { x: 10, y: 0, z: 0 } }] }, volume: [35, 1e-9] },
  // An empty element is legal: every property fills from its default.
  { set: { objects: [{}, {}] }, volume: [24, 1e-9] },
  // Map values normalize and hydrate; entries are just entries.
  { set: { anchors: { a: [5, 5, 0], b: { x: -5, y: 0, z: 0 } } }, volume: [18, 1e-9] },
  // Union: switching the tag selects the other branch's schema.
  { set: { shape: { kind: 'sphere', radius: 2 } }, bounds: { min: [-2, -2, -1], max: [2, 2, 22], eps: 0.01 } },
  // Union errors name the tag; nested validation errors name the path.
  { set: { shape: { kind: 'cone' } }, error: 'unknown variant "cone"' },
  { set: { shape: { kind: 'sphere' } }, error: 'radius' },
  { set: { objects: [{ position: [1, 2] }] }, error: '/0/position' },
];
