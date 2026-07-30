# JS API reference

Full reference for the doohickey API. The short version is
`../prompts/js.md`; these files add every function, default, and edge
case.

| Topic | Covers |
| --- | --- |
| [doohickeys.md](doohickeys.md) | The file model, `build(ctx)`, return values, sandbox, console |
| [solids.md](solids.md) | `box`, `cylinder`, `sphere`, `extrude`, `revolve`, `fromThreeGeometry`, 2D profiles |
| [transforms.md](transforms.md) | `translate`/`rotate*`/`scale`/`transform`, `color`, `name`, `bake` |
| [csg.md](csg.md) | `union`/`subtract`/`intersect`/`hull` and their semantics |
| [queries.md](queries.md) | `volume`, `area`, `bounds`, `raycast` |
| [composition.md](composition.md) | `group`, `Group`, `ctx.invoke`, `Instance`, passing Solids as args |
| [params-and-animation.md](params-and-animation.md) | `ctx.param`, `ctx.t`, `ctx.args`, `odm.json` |
| [colors.md](colors.md) | Accepted color formats, the named-color list, inheritance |
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
  `fromThreeGeometry`; `group`; CSG functions `union`, `difference`,
  `intersection`, `hull`; helpers `deg` (degrees → radians) and
  `parseColor`.
- **`THREE`** — a vendored subset of three.js ([three.md](three.md)).
- **`console`** — `log`/`info`/`debug`/`warn`/`error`, captured into
  build results ([doohickeys.md](doohickeys.md)).

The build function's `ctx` argument carries `args`, `t`, `param()`, and
`invoke()` — see [composition.md](composition.md) and
[params-and-animation.md](params-and-animation.md).
