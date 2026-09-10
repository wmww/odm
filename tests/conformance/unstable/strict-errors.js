//! odm unstable
// Fail-loud contract: unknown options, wrong arities, and removed forms all
// throw with pointed messages instead of silently doing nothing. `mode`
// selects which bad call to make so one file pins many errors.
export const meta = {
  inputs: {
    mode: {
      enum: [
        'ok',
        'unknown-opt',
        'scale-arity',
        'translate-args',
        'object-form',
        'raw-three',
        'free-fn',
        'opacity-high',
        'opacity-low',
        'extrude-opt',
        'revolve-opt',
        'sphere-opt',
        'box-shape',
        'cylinder-segments',
        'missing-invoke',
        'undeclared-input',
      ],
      default: 'ok',
    },
  },
};

export default function build(ctx) {
  const mode = ctx.input('mode');
  if (mode === 'unknown-opt') return odm.cylinder(1, 2, { centre: false });
  if (mode === 'scale-arity') return odm.box(1).scale(2);
  if (mode === 'translate-args') return odm.box(1).translate(1);
  if (mode === 'object-form') return odm.cylinder({ r: 1, h: 2 });
  if (mode === 'raw-three') return odm.group(new THREE.BoxGeometry(1, 1, 1));
  if (mode === 'free-fn') return odm.union(odm.box(1), odm.box(1));
  // Opacity is a fraction, and out-of-range is a mistake worth catching:
  // clamping it silently would hide a units bug (percent vs fraction).
  if (mode === 'opacity-high') return odm.box(1).opacity(2);
  if (mode === 'opacity-low') return odm.box(1).opacity(-1);
  // Every constructor's option list is closed, and the message lists it.
  if (mode === 'extrude-opt') return odm.extrude([[0, 0], [1, 0], [1, 1]], 1, { bevel: 1 });
  if (mode === 'revolve-opt') return odm.revolve([[1, 0], [2, 0], [2, 1]], { turns: 1 });
  if (mode === 'sphere-opt') return odm.sphere(1, { rings: 4 });
  // A size array is exactly three numbers.
  if (mode === 'box-shape') return odm.box([1, 2]);
  // A polygon needs three sides; segments are never inferred.
  if (mode === 'cylinder-segments') return odm.cylinder(1, 1, { segments: 2 });
  // The two ctx errors an agent hits most: each lists what *is* available.
  if (mode === 'missing-invoke') return ctx.invoke('nope.js');
  if (mode === 'undeclared-input') return odm.box(ctx.input('nope'));
  return odm.box(1);
}

export const checks = [
  { volume: [1, 1e-9] },
  { set: { mode: 'unknown-opt' }, error: "unknown cylinder option 'centre'" },
  { set: { mode: 'scale-arity' }, error: 'scale(x, y, z) takes all three' },
  { set: { mode: 'translate-args' }, error: 'translate y must be a finite number' },
  // The removed object call form fails as a plain bad argument.
  { set: { mode: 'object-form' }, error: 'cylinder radius must be a finite number' },
  { set: { mode: 'raw-three' }, error: 'odm.fromThreeGeometry' },
  // The free CSG functions are gone: methods are the only vocabulary.
  { set: { mode: 'free-fn' }, error: 'not a function' },
  { set: { mode: 'opacity-high' }, error: 'opacity must be in 0..1' },
  { set: { mode: 'opacity-low' }, error: 'opacity must be in 0..1' },
  // The valid keys are listed, so a typo is one read away from a fix.
  { set: { mode: 'extrude-opt' }, error: "unknown extrude option 'bevel' (valid: twist, scale, slices, curveSegments)" },
  { set: { mode: 'revolve-opt' }, error: "unknown revolve option 'turns' (valid: angle, segments, curveSegments)" },
  { set: { mode: 'sphere-opt' }, error: "unknown sphere option 'rings' (valid: segments)" },
  { set: { mode: 'box-shape' }, error: 'box size must be a number or [x, y, z]' },
  { set: { mode: 'cylinder-segments' }, error: 'segments' },
  // A missing path lists the files that do exist...
  { set: { mode: 'missing-invoke' }, error: 'no doohickey at "nope.js"; project has: root.js' },
  // ...and an undeclared input lists the declared names.
  { set: { mode: 'undeclared-input' }, error: 'not declared in meta.inputs (declared: mode)' },
];

// Note: calling odm.* at module top level makes the module fail to evaluate,
// so `checks` cannot be read and no check here could ever see it. That one is
// pinned as a ```js error= doctest in docs/api/errors.md instead.
