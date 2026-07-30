# Doohickeys and build()

Every `.js` file in the project (dot-directories like `.odm` excluded)
is a doohickey: one composable piece, like a React component. `main.js`
is the root; its output is the scene.

```js
export default function build(ctx) {
  return odm.box([40, 20, 5]).color('steelblue');
}
```

The default export must be a function; it is called with a context
object (`ctx.args`, `ctx.t`, `ctx.param()`, `ctx.invoke()` — see
[composition.md](composition.md) and
[params-and-animation.md](params-and-animation.md)).

## Return values

`build()` may return:

- a `Solid`, `Group`, or `Instance`;
- a closed `THREE.BufferGeometry` (converted as if by
  `odm.fromThreeGeometry`);
- an **array** of any of these, nested arbitrarily — each array becomes
  an anonymous group node; `null`/`undefined` entries are dropped;
- `null` for an empty scene.

Anything else (a number, a plain object, a `THREE.Shape`, …) is a
build error.

## Sandbox

Each doohickey runs in its own V8 isolate with the framework preloaded.
There are no `import`s, no file or network access, no timers, and no
shared state with other doohickeys — communication happens only through
`ctx.args` / `ctx.invoke`. `Date` is frozen and `Math.random` is a
seeded PRNG ([determinism.md](determinism.md)).

`build()` must be pure: same file + same args + same context reads →
same output. The engine relies on this to memoize and to rebuild only
what changed.

## Console

`console.log/info/debug/warn/error` are captured (arguments are
stringified, objects via JSON) and returned with build results through
any CLI command — a usable debugging tool. Nothing is printed live.

## Errors

A thrown exception (or invalid return value) fails the build for that
generation; the error, its JS stack, and the console output come back
through the CLI, and the viewer keeps showing the last good build until
the code is fixed.
