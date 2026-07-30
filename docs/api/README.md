# JS API reference

Full reference for the doohickey API. The short version is
`../prompts/js.md`; these files add every function, default, and edge
case.

| Topic | Covers |
| --- | --- |
| [doohickeys.md](doohickeys.md) | The file model, `build(ctx)`, return values, sandbox, console |
| [solids.md](solids.md) | `box`, `cylinder`, `sphere`, `extrude`, `revolve`, `fromThreeGeometry`, 2D profiles |
| [transforms.md](transforms.md) | `translate`/`rotate*`/`scale`/`applyMatrix4`, the `about` pivot, `color`, `name` |
| [csg.md](csg.md) | `union`/`subtract`/`intersect`/`hull` and their semantics |
| [queries.md](queries.md) | `volume`, `area`, `bounds`, `raycast` |
| [composition.md](composition.md) | `group`, `Group`, `ctx.invoke`, `Instance`, passing Solids as args |
| [inputs.md](inputs.md) | `meta.inputs`, `ctx.input`, cascade values, presets, the `t` convention |
| [colors.md](colors.md) | Accepted color formats, inheritance |
| [three.md](three.md) | The vendored THREE subset and how to use it with ODM |
| [determinism.md](determinism.md) | Conventions (Z-up, radians, units), purity, memoization |
| [errors.md](errors.md) | Common errors and what they actually mean |

## The globals

Doohickeys see three globals. (`import`ing `'odm'` or `'three'` also
works and resolves to the same surface; any other import is an error —
use `ctx.invoke` to reach other doohickeys.)

- **`odm`** — the framework:
  classes `Solid`, `Group`, `Instance` (usable for `instanceof`);
  constructors `box`, `cylinder`, `sphere`, `extrude`, `revolve`,
  `fromThreeGeometry`; `group`; and `deg` (degrees → radians). CSG
  (`union`/`subtract`/`intersect`/`hull`) lives as methods on `Solid`.
- **`THREE`** — a vendored subset of three.js ([three.md](three.md)).
- **`console`** — `log`/`info`/`debug`/`warn`/`error`, captured into
  build results ([doohickeys.md](doohickeys.md)).

The build function's `ctx` argument carries `input()` (read a declared
input) and `invoke()` — see [inputs.md](inputs.md) and
[composition.md](composition.md).
