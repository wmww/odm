# Scene tree nodes are inlined, not content-addressed

`Node.children: Vec<Node>` (`odm-ir/src/types.rs`) — only *meshes* are shared
by hash. `examples/assembly` invokes the wheel once and places it four times,
producing four full copies of the wheel subtree in the IR, hashed and
serialized four times. (design-considerations.md's "one geometry blob + n
tiny IR nodes" holds for mesh buffers but not the tree.)

Invisible at MVP scale; becomes the scaling wall for arrays/patterns (a
20×20 grid of an assembly) and makes every root hash O(total tree).

Fix direction: `children: Vec<Hash>` or an `Instance(Hash)` node variant.
Either is a breaking IR change (FORMAT_VERSION bump, goldens regenerate,
store/flatten/tree walks all touch it) — decide **before real projects
exist**; cheap now, migration later.
