# Parameters and animation

Both come from the optional `odm.json` at the project root:

```json
{
  "params": { "width": 60, "holes": 3 },
  "animation": { "duration": 4 }
}
```

The engine watches `odm.json` like any source file; editing it rebuilds
exactly the doohickeys that read the changed values.

## ctx.param(name, default)

Reads a project parameter — any JSON value — falling back to `default`
when `odm.json` has no such key (or no `params` at all). Prefer params
over hard-coded magic numbers for anything a user might want to tweak;
they are the project's public knobs.

```js
const width = ctx.param('width', 40);
```

## ctx.t

Animation time in seconds; `0` in static scenes. Model motion as a pure
function of it:

```js
const rpm = ctx.param('rpm', 30);
const angle = ctx.t * 2 * Math.PI * (rpm / 60);
```

Only doohickeys that actually read `ctx.t` rebuild when time changes —
keep static geometry in doohickeys that don't touch it, and animate at
the assembly level with transforms, so scrubbing stays cheap.

`animation.duration` (seconds) is what enables the viewer's timeline
and `--t` on CLI commands (`odm render --t 2.5`); without it the
project is static.

## ctx.args

The arguments this doohickey was invoked with (`{}` for the root) —
see [composition.md](composition.md).
