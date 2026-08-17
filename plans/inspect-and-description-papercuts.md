# Inspect + description papercuts

Replaces `issues/inspect-papercuts.md` and
`issues/description-not-in-short-prompt.md` (agent feedback, 2026-08).
Three independent fixes; each can land alone.

## 1. `full` + `fields`: fields narrows full

Today `commands.rs:755` rejects the combination ("`full` and `fields`
are alternatives"). The intuitive reading — `fields` narrows `full` —
is exactly what `fields` alone already produces, so the error saves
nothing and cost a real session a failed call.

- Delete the error. The match at `commands.rs:774` already has
  `(_, Some(_))` arms, so removal alone yields fields-wins semantics.
- Update the field help (`requests.rs:276`) and `docs/cli.md` (~85,
  ~212-214) to say `fields` overrides `full` when both are given.
- Test: request with both parses and behaves as `fields` alone.

## 2. `volume`/`area` as subtree totals

`volume`/`area` are only emitted for a node's *own* mesh
(`scene.rs:351`), so a named Instance wrapping a Solid has no `volume`
— you must recurse to the child. `bounds`/`tris`/`verts` are already
subtree aggregates; make `volume`/`area` match.

- Add `volume`/`area` to `Agg` (`scene.rs:237`); accumulate in
  `walk()` for every mesh node — children are already always visited —
  and emit from the aggregate instead of `node.mesh`.
- Only compute when requested (`f.volume || f.area`): the `Agg`
  "never touches the kernel" invariant relaxes to "only when
  measurements are asked for"; default summary/recursive stay cheap.
  Update the comment.
- Cache *local* volume/area per mesh hash next to `MeshStats`
  (lazily, requested-only) so repeated parts pay Manifold once;
  per instance scale by s³/s² under similarity, fall back to
  `transform_solid` per instance otherwise (existing `measure()`
  logic, `scene.rs:409`).
- Failure rule: if any mesh in the subtree fails to measure, omit the
  key for that entry — no quietly-wrong partial sums (matches
  `measure()`'s ethos).
- Semantics note for docs: totals are per-solid sums (like `tris`),
  so overlapping siblings double-count; `overlap` in `clearance` is
  the tool for shared volume. Say so in `docs/cli.md` **Detail**.
- Tests (`scene.rs`): mesh-less named wrapper reports summed child
  volume; root total = sum of leaves; scaled instance ×s³/s²; repeat
  collapsing unaffected.

## 3. Description: fix the prompt lie, hint in the meta error

The description is the leading `//!` comment block, but the short
prompt implies it's meta-adjacent, and its example doesn't even
return it: `docs/prompts/js.md:111` claims `fields: ["inputs",
"presets"]` "reports a file's description" — it doesn't;
`commands.rs:806` only emits view-level fields actually requested.

- `docs/prompts/js.md:111-113`: change the example to
  `{"fields": ["description", "inputs", "presets"]}` and add the one
  clause: the description is the file's leading `//!` comment block,
  not a meta key. (AGENTS.md marker blocks regenerate from these
  prompt files on engine open — verify after editing.)
- `crates/odm-build/src/meta.rs:170`: when the unknown meta key is
  exactly `"description"`, extend the error with "the file's
  description is its leading `//!` comment block". Test next to the
  existing unknown-key cases (~meta.rs:431).
- Decision, recorded: `//!` stays the sole source; no
  `meta.description` key (two sources of truth, and per-input
  `description` inside schemas is a different, standard-JSON-Schema
  thing). Revisit only if the hint + prompt clause don't stop
  repeats.
- `docs/cli.md` and `docs/prompts/cli.md` were checked: their
  "what can I set here" lines don't make the false claim; no edits
  needed there.
