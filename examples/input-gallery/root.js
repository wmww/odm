//! ODM API unstable
//! One exhibit per kind of input control the viewer's panel can show.
//! Every input below is declared with a different schema, and every one
//! changes something you can see; `parts/` adds a cascade input that
//! reaches the panel without root.js mentioning it.
export const meta = {
  inputs: {
    // --- Numbers with both ends declared: field + slider ---
    thickness: { type: 'number', default: 4, minimum: 1, maximum: 20, description: 'plate thickness' },
    columns: { type: 'integer', default: 4, minimum: 1, maximum: 8, description: 'how many pins' },

    // --- Numbers with no range: field + the /2 - + 2x adjusters ---
    beam: { type: 'number', default: 3.5, minimum: 0.5, description: 'beacon radius' },

    // --- Component rows (vectors, quaternion) and the matrix grid ---
    plate: { type: 'vector2', default: [180, 90], description: 'base plate footprint (X, Y)' },
    beacon: { type: 'vector3', default: [0, 0, 40], description: 'where the beacon floats' },
    tilt: { type: 'quaternion', default: [0, 0, 0, 1], description: 'gizmo orientation' },
    placement: {
      type: 'matrix4',
      default: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, -28, 8, 1],
      description: 'plaque placement, column-major',
    },

    // --- Text fields: whatever is left ---
    tint: { type: 'color', default: '#3b6ea5', description: 'plate color' },
    bars: {
      type: 'array',
      items: { type: 'number', minimum: 0 },
      default: [6, 14, 10, 20, 12],
      description: 'chart bar heights, one bar each',
    },
    hole: {
      type: 'object',
      properties: {
        r: { type: 'number', minimum: 0.5 },
        depth: { type: 'number', minimum: 0.5 },
        // A nested default: an absent `at` is filled in at normalization,
        // and a vector3 hydrates inside the object.
        at: { type: 'vector3', default: [68, -28, 0] },
      },
      required: ['r', 'depth'],
      default: { r: 6, depth: 6 },
      description: 'bore in the plate; a depth past the thickness cuts through',
    },
    label: { type: 'string', default: 'gallery', description: 'name of the root node' },
    note: { default: null, description: 'no type declared: any JSON at all, logged at build' },

    // --- Structure: a growable object list (array of union-shaped objects,
    // one parts/marker.js invoke per element) and a string-keyed map ---
    objects: {
      type: 'array',
      default: [
        { position: [30, -8, 4] },
        { position: [52, -8, 5], shape: { kind: 'sphere' } },
      ],
      items: {
        type: 'object',
        properties: {
          position: { type: 'vector3', default: [0, -8, 4] },
          shape: {
            variants: {
              box: { properties: { size: { type: 'vector3', default: [8, 8, 8] } } },
              sphere: { properties: { radius: { type: 'number', default: 5, minimum: 0.5 } } },
            },
            default: { kind: 'box' },
          },
        },
      },
      description: 'editable object list; each element is one marker invoke',
    },
    anchors: {
      type: 'object',
      additionalProperties: { type: 'vector3' },
      default: { ne: [85, 40, 3], nw: [-85, 40, 3] },
      description: 'named positions: a string-keyed map, one ball each',
    },

    // --- Check box ---
    windows: { type: 'boolean', default: true, description: 'cut windows in the tower' },

    // --- Radios: anything with an enum, whatever the type ---
    roof: { enum: ['flat', 'gabled', 'domed'], default: 'gabled', description: 'tower roof shape' },
    spacing: { type: 'number', enum: [6, 10, 14], default: 10, description: 'pin pitch' },

    // --- Transport: a ranged cascade number named t (scrub + play) ---
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2, description: 'one rotor turn per 2s' },

    // `detail` is declared in parts/ and shows up in the panel too.
  },
  presets: {
    showy: { roof: 'domed', windows: true, tint: '#c05746', bars: [4, 18, 9, 22, 13, 7], columns: 7, spacing: 6, beam: 5, t: 0.5 },
    bare: { roof: 'flat', windows: false, columns: 1, bars: [8], thickness: 2, hole: { r: 2, depth: 12 }, objects: [], anchors: {} },
    askew: {
      tilt: [0.3827, 0, 0, 0.9239],
      placement: [0.7071, 0.7071, 0, 0, -0.7071, 0.7071, 0, 0, 0, 0, 1, 0, 0, -28, 14, 1],
      beacon: [40, -24, 26],
    },
  },
};

export default function build(ctx) {
  const plate = ctx.input('plate'); // THREE.Vector2
  const thickness = ctx.input('thickness');
  const hole = ctx.input('hole');
  console.log('note:', JSON.stringify(ctx.input('note')));

  // The plate's top face is z=0, so every exhibit stands at z=0.
  const slab = odm
    .box([plate.x, plate.y, thickness], { center: false })
    .translate(-plate.x / 2, -plate.y / 2, -thickness);
  const bore = odm
    .cylinder(hole.r, hole.depth, { center: false })
    .translate(hole.at.x, hole.at.y, -hole.depth); // hole.at: a THREE.Vector3 at depth
  const base = slab.subtract(bore).color(ctx.input('tint')).name('plate');

  const tower = ctx
    .invoke('parts/tower.js', { roof: ctx.input('roof'), windows: ctx.input('windows') })
    .translate(-68, 12, 0)
    .name('tower');
  const chart = ctx
    .invoke('parts/chart.js', { bars: ctx.input('bars') })
    .translate(-26, 12, 0)
    .name('chart');

  // Integer count × an enum pitch.
  const columns = ctx.input('columns');
  const spacing = ctx.input('spacing');
  const pins = odm
    .group(
      Array.from({ length: columns }, (_, i) =>
        odm
          .cylinder(2, 10, { center: false, segments: 16 })
          .translate((i - (columns - 1) / 2) * spacing, 0, 0)
          .name(`pin-${i}`),
      ),
    )
    .translate(16, 12, 0)
    .color('#8d99ae')
    .name('pins');

  // The one moving exhibit: half a turn per unit of t.
  const arm = odm.box([22, 3, 2]).translate(0, 0, 13).rotateZ(ctx.input('t') * Math.PI).name('arm');
  const rotor = odm
    .group(odm.cylinder(2, 12, { center: false }).name('post'), arm)
    .translate(64, 12, 0)
    .color('#e0a458')
    .name('rotor');

  // A quaternion is an orientation: turn it into a matrix and apply it.
  const tilt = new THREE.Matrix4().makeRotationFromQuaternion(ctx.input('tilt'));
  const gizmo = odm
    .group(
      odm.cylinder(1.5, 10, { center: false }).name('stalk'),
      odm.box([14, 9, 3]).applyMatrix4(tilt).translate(0, 0, 12).name('vane'),
    )
    .translate(-62, -28, 0)
    .color('#6a994e')
    .name('gizmo');

  // A matrix4 is a whole placement — the plaque has no transform of its own.
  const plaque = odm
    .box([18, 10, 2])
    .applyMatrix4(ctx.input('placement'))
    .color('#bc4749')
    .name('plaque');

  const beaconAt = ctx.input('beacon'); // THREE.Vector3
  const beacon = odm
    .sphere(ctx.input('beam'), { segments: 32 })
    .translate(beaconAt.x, beaconAt.y, beaconAt.z)
    .color('#f2e8cf')
    .name('beacon');

  // The object list: one marker invoke per element, so editing one element
  // rebuilds one marker and memo-hits the rest.
  const objects = odm
    .group(ctx.input('objects').map((o, i) => ctx.invoke('parts/marker.js', o).name(`object-${i}`)))
    .color('#7f9c96')
    .name('objects');

  // The anchor map: one ball per named position.
  const anchors = odm
    .group(
      Object.entries(ctx.input('anchors')).map(([key, at]) =>
        odm.sphere(2.5, { segments: 24 }).translate(at.x, at.y, at.z).name(key),
      ),
    )
    .color('#d8b4e2')
    .name('anchors');

  return odm
    .group(base, tower, chart, pins, rotor, gizmo, plaque, beacon, objects, anchors)
    .name(ctx.input('label'));
}
