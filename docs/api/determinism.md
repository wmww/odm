# Conventions and determinism

## Conventions

- **Z-up, right-handed.** The ground grid is the XY plane; "up" is +Z.
- **Radians everywhere** (like three.js); `odm.deg(90)` converts.
- **Units are yours**: pick one (mm, m, …) and stay consistent; the
  engine doesn't care.
- **Everything is immutable**: transforms, `color`, `name`, and CSG all
  return new values.
- Geometry lives engine-side, content-addressed; JS holds opaque
  handles. There is deliberately no API to read vertex data — use
  [queries](queries.md).

## Purity

`build(ctx)` must be a pure function of its file, its args, and the
input values it reads (`ctx.input`). The engine memoizes on
exactly those inputs and rebuilds only what a change touches; impure
builds break that silently.

To make accidental impurity harmless:

- `Date` is frozen (`Date.now()` and `new Date()` always return the
  same fixed instant).
- `Math.random()` is a seeded PRNG: every build of a doohickey gets the
  same sequence. Usable for stable "organic" jitter — but prefer an
  explicit seed parameter, since the sequence also restarts identically
  in every *other* doohickey, and call order changes results.

```js
// Frozen clock: two reads in one build are the same instant, so a
// timestamp can never leak into geometry and defeat memoization.
if (Date.now() !== Date.now()) throw new Error('the clock moved');
// Math.random still returns numbers — just the same ones every build.
const jitter = Math.random();
if (typeof jitter !== 'number' || jitter < 0 || jitter >= 1) {
  throw new Error(`Math.random gave ${jitter}`);
}
return odm.box(10).translate(jitter, 0, 0);
```

There is no way to reach the filesystem, network, or another
doohickey's state from build code.

## What determinism buys

Same project state → byte-identical scene, every time, on every
machine. That makes memoization sound (`ctx.invoke` results, CSG ops,
and whole builds are content-addressed caches) and makes renders
reproducible for comparison.
