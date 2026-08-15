# One answer to "what's settable" (build report rework)

## Why
The build response nests `inputs.args` (target's plain inputs) beside
`inputs.inputs` (cascade fall-through). In the live test the agent printed
`inputs.inputs`, saw only `t`, and wrongly concluded a just-declared plain
input wasn't view-settable — `odm interface` had to un-confuse them. The
deeper problem: the *write* path is already mechanism-blind (`--set` routes
plain→args / everything-else→cascade automatically, `commands.rs:380-387`;
poll's `view.set` snapshot merges the two maps), so only the read path forces
the plain/cascade distinction on agents.

## Design

### Flat inputs list
Replace the split with one `inputs` list: every view-settable name, one entry —
`{name, value, source, kind: "plain"|"cascade", type, minimum, maximum,
default, choices, declared_in}`. `kind` is ignorable; `declared_in` tells the
interesting story (declared on the target vs bubbled up from a part).
`InputReport::to_json` in `odm-build/src/report.rs`.

### Absorb `interface` into `build`
Two commands answer "what can I set" with different scopes; one should own it.
`build` adds the target's description and presets. On a *failed* build, still
report the target's declared schema (interface's one niche — the fall-through
report needs a successful pass, the declared schema doesn't). Then remove
`interface`.

### Collision lint
Plain `size` on the target + a descendant's cascade `size` falling through:
today `--set size=` routes to the plain input and the cascade one silently
keeps its default. Flag it via the existing lint infra (`report.rs`
warnings/errors).

### Cascade becomes authoring-only
Docs restructure: *using* a view never mentions cascade — "a view has inputs;
here they are". The cascade mechanism moves to the authoring section ("make an
input declared deep inside settable at the top without threading it through
every invoke"), with `t` as the worked example. Prompt updated to match.

### Memo stats
Surface the scheduler's `Stats { builds, memo_hits }` (exists, test-only —
`odm-build/src/scheduler.rs:184`) plus per-distinct-build timing in the build
response. Turns "structure your model for the cache" from advice into
something verifiable. (Live test: per-swing `seatColor` correctly misses the
memo — different inputs, different key — but the agent couldn't confirm or
cost it.) Cross-ref `issues/memo-cache-policy.md`.

### Response slimming
Drop `root` (node hash hex) from the build response — content addressing is
internal. (`generation` cut is in `cli-diet.md`.)
