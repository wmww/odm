# Inputs

A doohickey declares everything it can be given in one place:

```js
//! odm unstable
//! A parametric flange.
export const meta = {
  inputs: {
    radius: { type: 'number', default: 20, minimum: 1, description: 'outer radius' },
    holes: { type: 'integer', default: 6, minimum: 0 },
    finish: { enum: ['raw', 'painted'], default: 'raw' },
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 },
  },
  presets: {
    heavy: { radius: 40, holes: 12 },
  },
};

export default function build(ctx) {
  const r = ctx.input('radius');
  const disc = odm.cylinder(r, 4).rotateZ(ctx.input('t') * Math.PI);
  return ctx.input('finish') === 'painted' ? disc.color('#b22222') : disc;
}
```

`meta.inputs` is **one map**, name → entry. Reading is uniform —
`ctx.input(name)` — and the *declaration* decides where the value comes
from:

- **Plain input** (no `cascade`): the value comes from the immediate
  caller only — the invoking doohickey's args, or the view (viewer
  panel / a CLI request's `inputs`) when this file is the view target.
  No `default` means required.
- **Cascade input** (`cascade: true`): an authoring tool — the input
  becomes settable from anywhere above the declaring file, without
  being threaded through every invoke in between. `default` is
  mandatory. See "Cascade inputs" below.

Reading an undeclared name is an error, and so is passing an undeclared
name in an invoke's args — typos fail loudly at the boundary, not
silently downstream.

## Input schemas

An input entry is a JSON Schema in a strict profile: `type`, `enum`,
`default`, `description`, `minimum`/`maximum`, `items` (arrays),
`properties`/`required` (objects) — plus the ODM key `cascade`. Unknown
keys are rejected. `type` may be omitted (any JSON value). `default`
has real semantics (it is applied, not just documented). The engine
validates every value at invoke and view boundaries against the
declared schema.

Beyond the JSON types, `type` can name an ODM extension type:

| `type` | wire form (JSON) | `ctx.input` returns |
| --- | --- | --- |
| `'solid'` | opaque handle | a real `Solid` |
| `'vector2'` | `[x, y]` | `THREE.Vector2` |
| `'vector3'` | `[x, y, z]` | `THREE.Vector3` |
| `'quaternion'` | `[x, y, z, w]` | `THREE.Quaternion` |
| `'matrix4'` | 16 numbers, column-major | `THREE.Matrix4` |
| `'color'` | hex string or `[r, g, b]` | as sent — exactly what `.color()` takes |

Senders may pass THREE instances or the JSON form; values are
normalized to the wire form at the boundary (so hashing and
memoization only ever see canonical JSON). A `solid` input cannot have
a `default` (and therefore cannot cascade). Extension types also drive
the viewer's typed controls (vector rows, color picker).

```js
//! odm unstable
export const meta = {
  inputs: {
    offset: { type: 'vector3', default: [0, 0, 10] },
  },
};
export default function build(ctx) {
  const off = ctx.input('offset'); // a real THREE.Vector3
  return odm.box(5).translate(off.x, off.y, off.z);
}
```

## What a view can set

From the outside, a view simply *has inputs*. After each build the
engine reports them as one flat list — the target's own inputs plus
every cascade input that reached the view level — each entry with its
current value, where it came from (`view` = explicitly set, `default`
otherwise), its schema (type, range, choices, default), and the
file(s) declaring it. That report is what the viewer's input panel
and the CLI's input-name validation are generated from;
`odm inspect '{"fields": ["inputs"]}'` prints it, and a set name
nothing reads is an error listing what *is* settable.

Lints ride along with the report: two unrelated subtrees falling
through with conflicting defaults get a warning (conflicting *types*
are an error); a cascade value that nothing in the invoked subtree
declares is a warning — a typo'd name (or a plain-input value sent
through the cascade channel) must not silently do nothing; and a plain
input on the target shadowing a same-named fall-through cascade input
is a warning, since a set value only reaches the plain one.

A *failed* build still reports the target's declared inputs next to
the error — the declared schema needs no successful pass.

## Cascade inputs

A cascade input makes something declared deep inside a model settable
at the top without threading it through every invoke in between. The
declaration lives at the *reader*; values are provided from above.

`ctx.invoke(path, args, cascade)` has two separate channels:

- **args** target the invoked file's plain inputs (validated, defaults
  merged; cascade inputs cannot be passed here);
- **cascade** values need no declaration on either side and scope over
  the whole subtree of the invoke — they may target descendants the
  invoker has never heard of.

A reader's cascade input resolves to, in order:

1. the **nearest explicitly provided value** above it — an invoke's
   `cascade` argument, or the view's set values (the view is the
   outermost layer);
2. otherwise, the default from the **shallowest declaration on the
   reader's own invoke path** (including the reader itself).

Rule 2 means declaring a cascade input auto-provides its default for
your whole subtree: any subtree under a common declaring ancestor
agrees on the value whether or not it was explicitly set. Resolution
depends only on the reader's invoke path, so it memoizes like any
other input.

### Time is a convention, not a feature

The worked example — there is no animation system; `t` is an ordinary
cascade number with a range:

```js
//! odm unstable
//! One revolution every 2 seconds, standalone or composed.
export const meta = {
  inputs: { t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 } },
};
export default (ctx) => odm.box([10, 2, 2]).rotateZ(Math.PI * ctx.input('t'));
```

Any doohickey that reads `t` animates; assemblies compose animated
parts without mentioning `t` at all, yet the view can still set it —
that is the cascade mechanism doing its job. The viewer renders a
ranged, fall-through numeric control named `t` as a transport (scrub,
plus play at 1 unit/second looping over the range); the CLI sets it
like any input (`odm render '{"inputs": {"t": 1.5}}'`). Declaring
`t` 0–2 *is*
"this loops every 2 seconds". Only doohickeys that read `t` rebuild
when it changes — keep static geometry in doohickeys that don't, and
animate at the assembly level with transforms, so scrubbing stays
cheap (`"stats": true` on any view command shows what actually
re-ran).

## Presets

`meta.presets` names input bundles (both kinds of input):

```js skip
export const meta = {
  inputs: { /* ... */ },
  presets: {
    heavy: { radius: 40, holes: 12 },
    demo: { radius: 25, t: 1 },
  },
};
```

One click in the viewer applies one; the CLI takes
`'{"preset": "heavy"}'` (explicit `inputs` values override the
preset). Use them as the "stories" of a doohickey: the configurations
worth looking at.

`odm inspect '{"fields": ["inputs", "presets"]}'` is the CLI's window
into all of this: the file's `//!` description, its settable inputs,
and its presets — even when the build fails.
