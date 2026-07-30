# Composition: groups and invoke

## odm.group(...children)

```js
const bolt = odm.cylinder(1, 8);
const nuts = [odm.sphere(2), odm.sphere(2).translate(0, 0, 8)];
odm.group(bolt, nuts).name('assembly').translate(0, 0, 10); // arrays flatten
```

Returns a `Group`: a pure grouping under one transform/color/name.
Children may be Solids, Groups, Instances, closed
`THREE.BufferGeometry`s, or arrays of those; one level of arrays is
flattened and `null`/`undefined` children are dropped (handy for
conditional parts). `g.children` returns a copy of the child list.

A Group can be transformed, colored, and named, but **not used in
CSG**. Its color is a default: it applies to descendants that don't
have a color of their own.

## ctx.invoke(path, args?)

```js
const wheel = ctx.invoke('parts/wheel.js', { radius: 8 });
return odm.group(wheel.translate(-20, 0, 0), wheel.translate(20, 0, 0));
```

Builds another doohickey and returns its output as an `Instance`.
`path` is project-relative. The invoked file's `build(ctx)` sees the
args as `ctx.args` (the root doohickey gets `{}`).

- **Args are JSON values plus Solids.** A Solid crosses the boundary as
  a content-hash handle (its pending transform, color, and name travel
  with it) and arrives as a real Solid. Groups and Instances are
  rejected; `undefined` becomes `null`.
- **Memoized**: same file content + same args (same context reads) →
  the cached result, free. Invoking one file many times with different
  args is the intended pattern for repeated parts.
- The invoked file runs in its own isolate; there is no other way to
  share values between doohickeys.
- Invokes can nest (a invokes b invokes c). A dependency cycle is a
  build error.

## Instance

The output of `ctx.invoke`: an opaque handle to the built subtree. It
can be transformed, colored (a default for descendants without one,
like a Group), and named — each copy independently — but it can be
neither queried nor used in CSG.

Structure around that limit: a doohickey that needs to *cut or measure*
a shape from elsewhere should receive it as a Solid through its args,
not invoke it. Data can also flow upward only as geometry — if a parent
needs numbers from a child (say, a mounting-hole position), pass the
parameters down and compute in both places, or query the returned
Instance's geometry via the CLI while developing.
