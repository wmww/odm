//! odm unstable
// Fail-loud contract: unknown options, wrong arities, and removed forms all
// throw with pointed messages instead of silently doing nothing. `mode`
// selects which bad call to make so one file pins many errors.
export const meta = {
  inputs: {
    mode: {
      enum: ['ok', 'unknown-opt', 'scale-arity', 'translate-args', 'object-form', 'raw-three', 'free-fn'],
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
];
