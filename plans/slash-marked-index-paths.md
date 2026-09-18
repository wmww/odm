# Self-marking index paths

Resolves issues/numeric-names-collide-with-index-paths.md: a node named
`12` is unreachable because `scene::locate` reads any all-digit address as
an index path. Fix: index paths start with `/` (`/0`, `/1/0/2`); anything
else is a name. The heuristic goes away and the two address kinds become
disjoint.

## Why both lookups stay

- Names are the agent-facing model ("parts have names" is in the core
  prompt) and survive structural edits.
- Index paths are the only *total* addressing: unnamed intermediates and
  repeated names (a leg doohickey invoked four times) have nothing else.
  They are also what the engine hands back as `id` everywhere.

Rejected alternatives: name-first with index fallback (a pasted `id` could
silently resolve to a same-looking name — the same bug mirrored, plus a
tree walk before every path lookup); separate `node`/`id` request fields
(every address-taking field — `focus`, `pairs` — would need the split).

## Id format

| node | today | after |
| --- | --- | --- |
| root | `""` | `""` (unchanged; `/` accepted as an address alias) |
| root's child 0 | `0` | `/0` |
| grandchild | `0/2` | `/0/2` |

The root id stays empty rather than becoming `/`: `node_id` then needs no
special case (`"" + "/" + i`), `selection_covers` and the tree's ancestor
walk work unchanged, and every existing `id.is_empty()` root check stays
right. Cost: `"id": ""` in inspect output for the root, which is already
the case.

Ids are minted in exactly one place, `odm_render::node_id` (flatten.rs),
and consumed by: render instances (`Instance::id`), `inspect` entries,
raycast hits, clearance `between`/`overlapping`, `locate`'s duplicate-name
error, viewer selection + tree state, `status`'s selection JSON, and the
web viewer's pick. All of them get the new format for free; the items
below are the places that *parse* ids or hard-code the old spelling.

## Steps

1. **odm-render `flatten.rs`** — `node_id`: `format!("{prefix}/{index}")`
   unconditionally; update its doc comment. Fix the test at ~207
   (`i.id == "1"` → `"/1"`).
2. **odm-engine `scene.rs`** — replace `is_index_path` with
   `addr.starts_with('/')`. `find_by_path` strips the leading `/` and treats
   an empty remainder as the root (so `/` resolves). `locate`: `""` and `/`
   → root; `/...` → path (keep the "no node at index path" error); else →
   name. Update the module doc and the comment that claimed numeric names
   are unreachable. Tests: `kids[0]["id"] == "/0"`, the ambiguity error
   lists `/0, /1, /2`, tiebreaker lookups use `/2` and `/9`. Add a test
   that `locate(.., "12")` finds a node named `12` and that `/` is the root.
3. **odm-viewer-core `tree.rs`** — `node_depth`: `""` → 0, else count of
   `/` (was count + 1). `reveal`'s ancestor prefixes via `match_indices('/')`
   already yield `""`, `/0`, ... — verify with the existing tests, drop the
   now-redundant `needed.insert(String::new())` if it is. The `#N` label for
   unnamed rows uses `rsplit('/')` — unchanged. `selection_covers` —
   unchanged; update its tests' literals.
4. **Request field descriptions, `requests.rs`** — `node` (~271), `focus`
   (~347), `pairs` (~398): show `"/1/0/2"`. Regenerate the GENERATED block
   in docs/cli.md (the drift test in requests.rs tells you how).
5. **docs/cli.md prose** — "Addressing" (~197): paths start with `/`, `""`
   or `/` is the root, duplicate-name error lists ids which are paths; add
   one sentence that a name beginning with `/` is unreachable by name.
   Check the bare-path mentions at ~316, ~342, ~371.
6. **Conformance `tests/conformance/unstable/names.js`** — the `name(123)`
   pin becomes a direct `{ node: '123', ... }` check (keep the raycast
   check or drop it; it no longer needs to stand in). No other conformance
   file asserts ids.
7. **Notes/plans** — notes/architecture.md ~939 (`node` scope line);
   plans/chat-links.md mentions locate accepting "name or index path"
   (still true, wording fine). Delete the issue.
8. `cargo test --workspace`, then run the conformance suite headless and
   eyeball `odm inspect` / a raycast for the new ids.

## Not affected

- Viewer selection is not persisted across restarts (tabs.rs saves no
  ids), so no migration.
- Web export: `odm-web/src/host.rs` pick returns `inst.id`; static JS does
  not parse ids.
- Stability: index-path addressing is docs-only and its tests are in
  `unstable/`, so the id format change is not a stability break. Mention
  it in the commit message anyway since agents may have old ids in
  transcripts.
