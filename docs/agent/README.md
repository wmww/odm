# ODM for agents

ODM is a CAD/3D framework where you build models by writing JavaScript
"doohickey" files and inspect them through the `odm` CLI. A long-running
engine (started by the user) watches the project, rebuilds on every CLI call,
and shows results in a viewer.

- `doohickeys.md` — how to write model code (start here)
- `api.md` — `odm` / `ctx` / three.js API reference
- `cli.md` — inspecting, rendering, and debugging with the CLI

## The 30-second version

A project is a directory. Every `.js` file is a doohickey; `main.js` is the
scene root. A doohickey default-exports a pure build function:

```js
export default function build(ctx) {
  const plate = odm.box([40, 20, 5]);
  const hole = odm.cylinder({ r: 3, h: 12 });
  return plate.subtract(hole.translate(10, 0, 0)).color('steelblue');
}
```

Check your work with structured queries first, renders second:

```
odm tree             # scene structure, node ids, bounds
odm inspect 0        # volume/bounds/transform of one node
odm raycast --origin 0,0,50 --dir 0,0,-1
odm render           # PNG; prints the file path
odm render --t 1.5   # animated scenes: pick a time
```

Errors from your code come back through the CLI with stacks and console
output — `console.log` in a doohickey is visible in build results.

## Conventions

- **Z-up**, right-handed. The ground grid is the XY plane.
- **Radians** everywhere (like three.js). `odm.deg(90)` converts.
- Solids are **immutable**: every method returns a new value.
- Geometry lives engine-side; JS holds content-hash handles. Don't try to
  read vertex data in JS — use queries (volume, bounds, raycast).
- `build(ctx)` must be **pure**: same inputs → same output. `Date` is frozen
  and `Math.random` is seeded (deterministic per build), so using them won't
  break rebuilds, but prefer explicit parameters.
