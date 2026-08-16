# Doohickeys and build()

Every `.js` file in the project (dot-directories like `.odm` excluded)
is a doohickey: one composable piece, like a React component. Any file
can be viewed, queried, or invoked; `root.js` is pure convention — the
entry file the CLI and viewer try when no path is given, like
`index.html`. Projects are free to name entry files meaningfully
instead.

The project is marked by `odm.toml` at its root — what the CLI and the
engine require of the directory they are pointed at:

```toml
name = "flange-demo"   # shown in the window title / status
engine = 0             # last-used engine version, engine-maintained
```

Unknown keys are errors. The engine rewrites the `engine` value when
it differs from its own — the one project file it ever writes — and
warns when the project was last touched by a newer engine.

```js
//! odm unstable
//! A steel plate, for the doohickeys chapter.
export default function build(ctx) {
  return odm.box([40, 20, 5]).color('#4682b4');
}
```

The default export must be a function; it is called with a context
object carrying `ctx.input()` (declared inputs — see
[inputs.md](inputs.md)) and `ctx.invoke()`
([composition.md](composition.md)).

## Description and metadata

The leading `//!` comment block doubles as the file's prose
description (the `odm <version>` pragma line is excluded): first line =
one-sentence summary, the rest is the body. It is parsed without
running the file, so it survives broken builds and is greppable.
Structured metadata — input declarations, presets — lives in
`export const meta` ([inputs.md](inputs.md)).
`odm inspect '{"path": "<p>", "fields": ["description", "inputs", "presets"]}'`
reports both.

## The API version pragma

`//! odm <version>` in the leading comments names the JS API version the
file targets — `unstable` (the current, freely-breaking dev channel) or,
once stamped versions exist, `v1`, `v2`, …. It is per file: versions
coexist in one project and `ctx.invoke` crosses them freely. A missing
pragma currently means `unstable`; that default goes away when v1 is
cut, so write the pragma. The whole story: `odm docs versioning`.

## Return values

`build()` may return:

- a `Solid`, `Group`, or `Instance`;
- an **array** of any of these, nested arbitrarily — each array becomes
  an anonymous group node; `null`/`undefined` entries are dropped;
- `null` for an empty scene.

Anything else (a number, a plain object, a `THREE.Shape`, a raw
`THREE.BufferGeometry` — wrap that with `odm.fromThreeGeometry`, …) is
a build error.

## Sandbox

Each doohickey runs in its own V8 isolate with the framework preloaded.
There are no `import`s, no file or network access, no timers, and no
shared state with other doohickeys — communication happens only through
declared inputs and `ctx.invoke`. `Date` is frozen and `Math.random`
is a seeded PRNG ([determinism.md](determinism.md)).

`build()` must be pure: same file + same inputs → same output. The
engine relies on this to memoize and to rebuild only what changed.

## Console

`console.log/info/debug/warn/error` are captured (arguments are
stringified, objects via JSON) and returned with build results through
any CLI command — a usable debugging tool. Nothing is printed live.

## Errors

A thrown exception (or invalid return value) fails the build for that
generation; the error, its JS stack, and the console output come back
through the CLI, and the viewer keeps showing the last good build until
the code is fixed.
