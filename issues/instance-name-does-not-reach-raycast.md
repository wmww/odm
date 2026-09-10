# A name on an Instance wrapper never reaches a raycast hit

`ctx.invoke(...)` returns an `Instance`; naming it labels the *wrapper*
node, while `raycast` (and the CLI's `hits[].name`) reports the name of
the node that owns the mesh — one or more levels below. So:

```js
const part = ctx.invoke('parts/wheel.js');
return part.name('wheel-fl');       // odm inspect finds it...
```

`odm inspect '{"node": "wheel-fl"}'` works (it addresses nodes), but
`odm raycast` on that wheel comes back with whatever name the invoked
file gave the mesh, or none — never `wheel-fl`. An agent placing four
identical parts and raycasting to tell them apart gets no answer.

Same for a name on `odm.group(...)`: pinned in
`tests/conformance/unstable/names.js` (a group's name does not reach the
solid inside it), which is arguably right for groups and clearly wrong
for the invoke case, where the Instance is the only handle the caller
has.

Seen in `tests/conformance/unstable/invoke/root.js`, whose raycast check
carries a comment saying exactly this.

Fix shape: `scene::raycast` (crates/odm-engine/src/scene.rs) reports the
instance's own `name`; the flattener could carry the nearest *named*
ancestor instead when the hit node is unnamed, or report the whole name
path. Decide, document in docs/api/queries.md, then pin it.
