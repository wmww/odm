# Agent surface policy (prompt vs docs)

Adopted 2026-08 (from plans/cli-diet.md, after the 2026-08 surface audit).
The scarce resource is the agent's context: prompt is ~3K tokens paid once
per session, but output *shape* dominates everything (the un-dieted `tree`
printed 191 KB). Priority order for cuts: output fields ≫ concepts ≫
command count.

## The policy

- **The prompt teaches only the core loop**: edit → `inspect` → `render` →
  `poll`/`say`, plus "view commands take one JSON request", `inputs`/
  `preset`, and "parts have names". After basic work the concept load
  should be: files are doohickeys; queries target a view; `inputs` sets
  inputs; parts have names; poll/say to talk.
- **Everything else lives in `odm docs`**, paid for only when consulted.
  `docs/cli.md` (topic `cli`) is the full CLI reference: every request
  field of every command (a generated section printed from the engine's
  own spec table — `requests.rs` — so it can't drift), geometry queries,
  explicit camera placement, viewer view slots, index-path addressing,
  `--project`. Cascade mechanics, memoization, generations, color spaces,
  camera math — optional depth.
- **New features default to docs-only.** Promotion into the prompt needs a
  reason on the scale of the core loop.
- Cuts are about defaults and prompt space — full capability stays
  reachable via CLI + docs, and `odm --help` lists everything (one line
  per command; fields live in docs).

## One JSON grammar (2026-08, plans/cli-json-args.md)

`odm [--project <dir>] <cmd> ['{…json}']` — the argument *is* the socket
request body, minus `cmd`. No per-command flags, no other positionals;
bare = defaults. Exceptions, deliberately: `poll --timeout/--follow`
(configures the CLI's own waiting; the follow *loop* stays CLI-side, but
since 2026-08-17 `--follow` also sets the request's `events` flag so the
engine answers on build/health value changes), `say <free text>`, `docs`
(engineless), `--project` prefix (transport).
The engine's spec table validates field names (typos list siblings,
removed commands get redirect errors) — one error path, no CLI grammar
errors beyond "that wasn't a JSON object". Wire word is `inputs`
(renamed from `set` 2026-08: it's the word meta and the interface report
use; "set" read imperative). `say` grew one optional *leading* flag
(2026-08-17): `--task <text>` / `--done [<text>]` for the working
status; the rest stays free text. Responses of say/poll/status echo a
standing `task` (only when set — absent claims nothing); that echo is
the whole staleness story, deliberately no expiry.

## Geometry queries: flat toplevel, JS parity

Kernel geometry queries (`raycast`, `clearance` today; sections, mass
properties maybe later) are plain toplevel commands sharing the JSON
grammar. Standing decisions:

- **No `query` namespace** — a prefix is a classification the agent must
  remember, costs a word per call; command count is the cheapest surface
  there is. The set-ness lives in docs (`docs/cli.md`'s "Geometry
  queries" section + JS-twin table), not grammar.
- **JS parity rule**: every kind is defined once — name, parameters,
  result shape — and exists in both surfaces: JS as a method
  (`s.raycast(origin, dir, maxDist?)`), CLI as the same-named command
  with the same result shape plus what JS gets free from object
  references (view spec; hits carry `id`/`name`). No tagged union in
  either surface — per-kind serde structs with `deny_unknown_fields`;
  don't reintroduce one "for generality".
- **Plural-native**: where mapping is natural the request takes arrays
  (`rays: […]`), all against the request's one view, answered in order.
  No heterogeneous cross-kind arrays — that's two commands, and the
  memoized build makes the second nearly free.
- `inspect` is not in the parity set — no JS twin (in JS you hold the
  object graph); simply absent from the JS-twin table.
- **`clearance` is a signed distance** (2026-08-17, from
  plans/signed-distance.md; the 2026-08 field report killed the old
  `{overlap, gap_lower_bound}` tiers — the bound read 0 for every
  nesting/resting pair, groups couldn't say *what* collided, and exact
  tangency made the boolean a coin flip the agent "fixed" with 0.002'
  nudges). The contract, deliberately asymmetric:
  - **Positive = exact** minimum gap (triangle-BVH branch-and-bound in
    `odm-kernel/src/dist.rs`; per-mesh BVHs cached by content hash
    beside the Manifold cache, transforms applied at query time) plus
    `closest` points.
  - **Negative = upper bound**: `-s` where `s` is the best found
    separating translation (`separate`, applied to the pair's *second*
    node) — bisection on a fast overlap predicate (tri-tri intersection
    + containment ray parity) over candidate directions (harvested
    face normals, centroid line, axes), hill-climbed, padded ~1e-9
    relative past the boundary so applying `separate` robustly
    separates. A guarantee, not a minimum; exact MTD for non-convex
    meshes is intractable and NOT promised.
  - **Tangency**: the sign of a near-zero distance is float noise;
    docs everywhere say threshold `|distance|`, never nudge geometry.
    No `tolerance` request field — the continuous value subsumes it.
  - **CLI names offenders**: `between` (deciding leaf pair; argmin or
    first collider) and, when negative, `overlapping` (every colliding
    leaf pair, deduped by label). Labels = leaf's own name, else
    nearest named ancestor within the queried subtree, else index
    path. A raycast hit's `name` uses the same rule (2026-09-17,
    plans/raycast-name-inherits.md), resolved in odm-render's
    flattener so viewer picks and web-export picks inherit it too —
    its ancestors are whole-scene, clearance's stop at the queried
    node, which is why the two walks stay separate. Naming a group or
    an `Instance` is thus how copies of one invoked part are told
    apart. JS parity carve-out as with raycast: JS gets the numeric
    fields only (`closest`/`separate` hydrate to Vector3s).
  - manifold-csg's `min_gap` exists but is only a test cross-check:
    it goes quadratic when `search_length` is loose (measured: >60 s
    on two 125k-tri spheres, see spike-findings), and gives no points
    and no negative side.
  Result keys are snake_case in *both* surfaces — parity of result
  shape beats JS camelCase. CLI errors on a pair where one node
  contains the other, and on nodes with no geometry. A weaker
  complementary idea (build-report lint flagging subtrees whose bounds
  touch nothing — "floating part") was left unbuilt: heuristic,
  false-positives on grounded/intentionally-gapped parts.

## Inspect + description papercuts (landed 2026-08-17)

From the 2026-08 agent feedback (plans/inspect-and-description-papercuts.md):

- **`fields` overrides `full`**, never an error — narrowing is what the
  agent means, and it's what `fields` alone already does.
- **`volume`/`area` are subtree totals** like `bounds`/`tris`: per-solid
  sums, so overlapping siblings double-count (`clearance` is the overlap
  check). Computed only when requested — `Agg` touches the kernel only
  then, so default summaries stay cheap. If any mesh in a subtree fails
  to measure, the key is omitted (no quietly-wrong partial sums). Local
  measurements cache per mesh hash; instances scale by s³/s² under
  similarity, else `transform_solid` per instance.
- **The description stays the leading `//!` block only** — no
  `meta.description` key (two sources of truth; per-input `description`
  inside schemas is a different, standard-JSON-Schema thing). The
  unknown-meta-key error hints at the `//!` block when the key is
  `"description"`, and the short prompt's example asks for
  `"description"` explicitly. Revisit only if agents keep reaching for
  a meta key anyway.

## Render camera: one parameter set (landed 2026-08-17)

From plans/render-camera.md. A camera is target + gaze direction (or
eye) + distance + up + projection + fov/ortho_height; every request
field is independently given or defaulted, and the defaults are the
auto fit of the framed bounds. No `Auto | Explicit` modes —
`odm_render::Camera` is a struct of Options with one `resolve(bounds,
aspect)`; the viewer's orbit camera is just a fully-given one.

- `look` keywords (`top/bottom/front/back/left/right`) are the six
  drafting views and imply **ortho** by spelling; a `look` *vector* is
  perspective. `ortho` is tri-state and beats the implication both
  ways. Keyword→axis: `front` gazes along +y, `right` along −x,
  `top` along −z (Z-up; matches Blender/CAD drafting).
- `focus: "<node>"` fits that subtree's bounds (camera only — the whole
  scene still draws); `zoom` scales *fitted* values only.
- Over-determined combos are errors, not precedence: `eye`+`look`,
  `eye`+`zoom`, `zoom`+`ortho_height`, `eye`==`target`. A parameter of
  the other projection (`fov` when ortho, `ortho_height` when
  perspective) errors instead of being silently ignored — the old
  surface's sin.
- Every render response echoes the resolved `camera` —
  `eye`/`target`/`up` + `fov` or `ortho`+`ortho_height`, the request's
  own spelling, f32-shortest-rounded (fitted values come out of
  normalization/trig with 17-digit decimals). Paste-back re-renders
  byte-identically (verified).
- Poll messages each carry `view` (was one top-level `view` per poll):
  tab path + inputs + selection + `camera` in the same spelling,
  **stamped by the viewer at send time** (per design-decisions "sent,
  not sampled") — a poll can collect long after the send.
- Prompt teaches the `look` keywords only (it replaced the
  `direction`+`ortho` pair); `focus`/`zoom`/`eye`/`target`/`up`/`fov`/
  `ortho_height` are docs-only.
- `direction` is deleted; `requests.rs::removed_field` (new mechanism,
  parallel to removed commands) redirects it to `look`. The same hook
  teaches `camera` (the echo's wrapper, pasted back whole) to unwrap —
  the echo's *contents* are request fields; there is deliberately no
  nested-`camera` request spelling (one way only, and camera-nodes may
  later claim the key for `"camera": "<node>"`).
- The default direction stays the skewed `[-1,-1.4,-0.9]` — equal-angle
  `[-1,-1,-1]` degenerately stacks projected edges of axis-aligned
  models. No name for the default view.
- Camera nodes (plans/camera-nodes.md, shelved) slot in later as
  another *source of defaults* in the same overlay.

## Contact sheets: `frames` (landed 2026-08-17)

From plans/render-frames.md. `render` takes `frames`: an array of
partial requests, each merged over the base (shallow per field,
`inputs` by key — expansion lives in `requests.rs::expand_frames`, so
commands.rs sees ready per-frame `RenderReq`s) and rendered as one
captioned tile. Explicit only — no ranged sampling, no cartesian form;
writing the values costs tokens ~nothing vs reading tiles.

- **Shared framing**: every frame builds first, then tiles whose camera
  has no `fit` get `fit = union of all frames' bounds`. The auto-fit is
  a direction-dependent corner fit (same `fit_distance` as the viewer's
  F-frame), so equal scale across mixed per-frame `look`s is explicit:
  `odm_render::share_fitted_scale` resolves the participating tiles,
  takes the largest fitted world half-height at the target plane, and
  sets each tile's `zoom` to match it. A frame's `focus` (own fit) or
  `eye`/`zoom`/`ortho_height` opts out per parameter.
- Whole-sheet fields, rejected inside a frame: `frames` (no recursion),
  `width`/`height` (per-tile size; sheet default 512×384 — `RenderReq`
  sizes became `Option<f64>` so given-ness survives to commands),
  `supersample`, `out`, `view`, `stats`. Base's `view` propagates into
  every merged frame, so `view: true` + frames sweeps the user's tab.
- Compositing is CPU-side in `odm_render::sheet`: pixel-near-square
  grid (aspect-aware, row-major, last row may be short), 2px gutters,
  caption strip under each tile stamped with the frame's overrides
  (`t=0.75`, `look=top`; `{}` tile blank) via the `font8x8` crate
  (public domain, dep-free) at 2× — survives image-reader downscaling.
  Long captions truncate with `..`.
- Validation: per-tile 16..=8192 as before, plus the *sheet* must fit
  8192 per side (that's the only frame-count cap). Errors name the
  frame: `frames[2] (t=1.5): …`.
- Response: sheet path, `tile`, `grid` [cols, rows], per-frame
  `overrides` echo, the shared resolved `camera` (from the first frame
  that didn't override camera fields), per-frame `camera` where a frame
  did, warnings/logs deduped across frames, `stats` per frame when
  asked. GPU mesh cache is pruned once against the union of frames.
- Docs-only per policy (spec-table entry + docs/cli.md section); the
  prompt is unchanged.

## Async diagnostics (landed 2026-08-17, plans/js-diagnostics.md)

- **Per-slot encoding is value + freshness, not a state machine**:
  `build` = `ok`/`error`/`pending` (the last-published value; `error`
  rides next to it) plus `stale: true` when a newer answer is queued or
  building. This *revised* `status`'s old `ok|error|building|pending`,
  where "building" masked the value. One helper
  (`commands.rs::build_fields`) feeds both `status.views` and poll
  `builds`.
- **Every poll response carries `builds` + `health`** (failures-only
  file list from the background sweep; `status` gets the same `health`).
  Logs never ride poll — querying the view returns error + logs, and
  memoization makes that byte-identical to the original run. Reporting
  per-file ok/skipped rows would re-run the 191 KB `tree` mistake;
  failures only, absence claims nothing.
- **Errors are per-view, not per-doohickey**: an error can genuinely
  depend on inputs, so no surface may say "this file is broken/fine" —
  slots report the exact views on screen, `health` a default-inputs
  canary per file, and the two may disagree (that's information).
- Engine host warnings are poll messages with `"from": "engine"`;
  absence = user (the shape the prompt teaches).
- `events: true` on poll (what `--follow` sets): answer on diagnostic
  value changes, compared per connection against what it last reported.
- `--follow` reconnects (2026-08-17, from field report): engine death
  prints `{"engine":"down"}`, then the CLI retries the socket (300ms)
  forever and prints `{"engine":"back"}` — the standing listener must
  survive engine restarts or the viewer says nobody is listening.
  Launched with no engine it parks in the same state (agent may start
  before the engine); bad project path still errors at launch.
  Shutdown's `stopped` error is swallowed (the down notice is its
  line); other refusals still print + exit 1, as does a dead stdout.
  Prompt leads with `--follow` and names the harness mechanism —
  agents were reaching for `--timeout` relaunch loops and shell `&`.
- **The watcher must notify per line, not per exit** (2026-08-18, field
  report): the prompt used to name Claude Code `run_in_background`/
  BashOutput, but that harness only notifies on task *completion* —
  and `--follow` never exits, so lines sat unread while the viewer
  showed an active poll. Prompt now names Claude Code's `Monitor` tool
  (`persistent: true`), calls out exit-notify mechanisms as broken for
  `--follow`, and gives the exit-notify fallback: background a one-shot
  `odm poll` (exits at first batch → the completion notification wakes
  the agent), relaunched per batch. No grep filter recommended:
  `--follow` already emits only actionable lines.

## `feedback` is in the prompt (2026-09-17)

The one deliberate exception to "new features default to docs-only": an
agent cannot consult the docs about a command it doesn't know exists,
and the whole point of `odm feedback` is that agents use it. So the
prompt's command list carries one line plus one sentence on when; the
depth (what belongs in a body, no attachments, no reply) is
docs/cli.md's "Reporting problems". Everything else about it follows
the standing rules: engine-routed, one JSON grammar, four required
fields, response of id + path and nothing more.

Deliberately nothing agent-visible about what happens next — no "sent"
state, no queue length, no reply channel. A report is a write into the
project, and the human is the only reader that matters.

## Standing cuts (don't reintroduce)

- No standalone `sync` command: every command syncs first; `status` is the
  answer if the rescan is all you want.
- **No `build` command** (2026-08): split into the `inspect` root entry's
  view-level fields (`"fields": ["description", "inputs", "presets"]`),
  `"stats": true` on any view command, and the one error path (build
  errors return through whichever command triggered the build, with the
  declared interface attached when meta evaluated). Input lints ride the
  `warnings` channel of every view-targeting success.
- **No `selection` command** (2026-08): `status` reports the active
  slot's selection; every poll carries it (`view.selection`).
- **No `prompt` command** (2026-08): it's a docs topic (`odm docs
  prompt`) — the prompt's standing home is the AGENTS.md marker block,
  so the command's only remaining use was inspection, which is docs.
- One "target what the user sees" knob: `"view": true` = active viewer
  tab, `"view": "<slot>"` = a named slot. There is no `--viewer-state`.
  Adoption covers path+inputs only, never the camera — per
  design-decisions.md "User state: sent, not sampled" (the camera the
  user saw travels with their message in the poll snapshot instead).
- `generation` appears only in `status` (internal consistency counter).
- Nothing agent-visible prints content hashes.
- Removed-command redirects live engine-side (`requests.rs::removed`),
  so raw-socket users get them too; `prompt` alone redirects client-side
  (its replacement needs no engine).
