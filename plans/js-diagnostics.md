# JS diagnostics surfacing

From `issues/js-diagnostics-surfacing.md`; claims re-validated against the
code 2026-08-17, design rethink same day. Background that already landed:
failed builds return logs as data (`FailedBuild`), viewer has a Console
panel with latest-attempt logs, `Published` carries status ingredients +
`error` + `logs` (`state.rs:26-44`), and `odm poll --follow` exists
(CLI-side re-poll loop, `odm-cli/src/lib.rs:222-249`).

## Framing: diagnostics are part of build() output

The invariant every item below serves: **error and logs are part of a
build's output — a pure function of (generation, view)**. How many times
`build()` actually ran, and whether a result came from cache, a fresh run,
or a partial revalidation, is an implementation detail. Every surface
(viewer console, poll, view commands) must present the same bytes for the
same (generation, view); querying a cached build and triggering a rebuild
must be indistinguishable in what they show.

Where that stands today:
- Success logs are memoized (`MemoEntry.logs`) and replayed on hits — the
  invariant holds by construction. Item 1 is a bug against exactly this.
- **Failures are not memoized**: `run_one` only `memo_insert`s on Ok
  (`scheduler.rs:561-564`), so a failing build re-runs every pass, and its
  error + logs match across runs only because isolates are deterministic
  (Date/Math.random frozen). That satisfies the invariant in practice;
  item 6 makes it hold by construction (and stops re-paying for broken
  builds).
- **A failed invoke records no dep at all** (`ops.rs:292-298`: the dep is
  pushed only on Ok), and `ctx.invoke` throws a plain catchable JS error.
  So a build that catches the failure and returns fallback output is
  memoized with *no record of the broken child* — fixing the child leaves
  the parent's stale entry validating forever. Confirmed by test
  2026-08-17: after fixing the child, the incremental root hash differs
  from a fresh engine's. A live violation of the consistency invariant
  ("every published result is byte-equivalent to a from-scratch build");
  item 5 is the fix, and item 6 depends on its representation.
- Deliberate carve-outs, not violations: `"stats": true` is the *one*
  surface that intentionally exposes run counts and cache behavior —
  that's its purpose (see issues/memo-cache-policy.md's thrash
  observability); engine-host warnings (item 2) are session events, not
  build output, hence a separate channel.

Second axis: **errors are per-view, not per-doohickey**. (path + inputs +
cascades) determines the result; most doohickeys fail identically at any
inputs, but nothing guarantees it — an error can genuinely depend on
inputs. So no surface may claim "this doohickey is broken/fine". Item 4
therefore reports two per-view things: the active slots (the exact views
on screen) and a default-inputs canary per file (the views nobody is
looking at). The two can disagree — fails at defaults but fine at the
slot's inputs, or vice versa — and that's information, not inconsistency.

## 1. Duplicate log replay on late validation failure

Confirmed in `odm-build/src/scheduler.rs`. `get_or_build`'s memo-hit path
(:434-447) calls `validate` (:585-618), which recurses through
`Dep::Invoke` via `get_or_build` (:610) — every child hit replays its logs
into `pass.logs` (:442-445), every child miss appends via `run_one`
(:546-548). If a *later* dep then invalidates the entry, nothing rolls
back; the parent reruns and re-invokes the same children (now hits), so
the whole already-validated subtree's logs appear twice. Since all
cascade reads are declared (`ctx.input` rejects undeclared names) and
declared cascade deps are pre-recorded ahead of `out.deps`, cascade deps
sort ahead of invoke deps in `entry.deps` — late invalidation is
exclusively the invoke case. This is a direct violation of the framing
invariant: what the user sees depends on whether a revalidation happened
to run.

Same non-rollback also skews stats: `memo_hits` (:438-439) counts child
hits from a discarded validation *and* again on the rerun; child rebuilds
inside `validate` run with no frame pushed for the parent, so their time
is charged to the grandparent's accumulator and effectively double-counted
(:503-504, :529-542).

**Fix** (all inside the memo-hit block of `get_or_build`):
- Record `pass.logs` length before `validate`; on `false`, truncate back
  to the mark. Take the mark per loop iteration (an `Acquire::Retry`
  re-validates). This also covers a child that *fails* during validation
  (failures aren't memoized, so the parent's rerun re-runs it and
  re-appends its logs exactly once). Safe: one pass's build tree is
  strictly single-threaded (nested invokes are inline, LIFO), so the mark
  is stable — the mutex only guards cross-thread access to distinct
  passes; nested memo-hit frames each hold their own mark, and an inner
  truncate always precedes the outer frame's continuation.
- Stats sit outside the purity invariant (their job is to show what work
  the implementation really did), so their accuracy here is best-effort,
  not correctness: still snapshot/restore `memo_hits` so one logical
  validate-then-rerun doesn't report a hit twice, but do **not** restore
  run counts or time for children actually rebuilt during a failed
  validation — the work was real. Timer attribution for those children
  stays approximate (fixing it needs a frame for the not-yet-entered
  parent); leave unless it falls out naturally.
- Test: two-dep root where dep A logs and dep B's source changes between
  passes; assert A's line appears exactly **once** (count, not
  `contains` — the existing `logs_are_collected_per_pass` can't catch
  duplicates).

Out of scope: replay *ordering* (on a hit, subtree lines all precede the
parent's, whereas from-scratch interleaves around invokes). A documented
deviation from the invariant — making it literal needs interleaving
positions in `MemoEntry.logs`; not worth it now.

## 2. Engine-host warnings as data

Confirmed all stderr-only: odm.toml newer-engine warning + `sync_marker`
failure + `odm_prompt::sync` warnings (`session.rs:128-135`), watcher
creation/watch failure — which silently ends live rebuild for the session
(`watcher.rs:38-44`), server-thread death (`session.rs:152`). Invisible to
viewer and agent; headless drops even the open-time questions. Item 4's
headless parity makes the watcher warnings agent-relevant too.

These are session events, not build output — explicitly outside the
framing invariant, which is why they get the transcript channel rather
than riding any build result.

**Fix: put host warnings in the transcript.** Add a third `Who::Engine`
(or a parallel kind) to `TranscriptEntry` and route these warnings through
`send_message`-style queuing:
- Viewer chat panel renders engine lines in a distinct style — the user
  sees "file watcher unavailable: …" where they already look.
- Poll delivers them with the existing Pending/InFlight/Done handshake for
  free — no new queue, no lost-on-dead-CLI path. Engine entries in a poll
  response carry `"from": "engine"`; user lines stay bare (absence =
  user — the shape the prompt already teaches; docs get the full story).
- Keep the stderr prints too (daemon logs).
- Mechanics: the open-time warnings fire before `EngineState` exists
  (`sync_on_open` runs first in both `session::open` and `run_headless`).
  Return them alongside the questions and queue them right after
  construction, like `set_agent_questions` — the transcript persists, so
  the agent's first poll collects them; no ordering problem.
- Marginal, take or leave: per-accept errors (`server.rs:72`).
  Viewer-local render eprintlns (`viewer/mod.rs`) stay out of scope — the
  viewer is its own surface.

## 3. Log levels: normalize, don't weaponize

Confirmed: `LogLine.level` is a freeform `String` set only by the shim
(`framework/odm/index.js:683-693` — `log`/`debug`/`warn`/`error`, `info`
→ `log`) plus the engine-made `"warn"` truncation notice; viewer styles
by string match with silent fallback (`viewer/mod.rs:1032-1037`); nothing
filters; `console.error` doesn't fail builds (docs promise this —
`docs/api/doohickeys.md:86-89`).

**Decision:** this is mostly working as intended. Do only:
- Make the level an enum (`Log`/`Debug`/`Warn`/`Error`) in
  `odm-store::LogLine`, converted at the `op_log` boundary (unknown →
  `Log`); viewer match becomes exhaustive. Levels are part of the
  memoized log output, so the enum lives store-side; on the CLI wire it
  still serializes as the same lowercase strings (`logs_json`), so no
  surface change. Contract instead of convention, no behavior change.
- Explicitly **keep**: no level filtering (1000-line cap is the only
  limit), `console.error` never fails or flags a build.

## 4. Async diagnostics over poll (the real gap)

Confirmed: `cmd_poll` returns `{"messages": [...]}` only; per-message
`view` is the viewer's send-time stamp with no build fields; a timed-out
poll says nothing at all. Build transitions cannot even wake a parked
poller — `publish_success/_failure` call `wake()` (viewer repaint), not
`wake_pollers()` (`state.rs:580,603`). `--follow` never reaches the
engine, so it streams chat batches only. `odm status` does give per-slot
status + error (not logs), but only when asked — the agent still only
learns of a red tab if the user types "it broke".

Everything needed is already in `Published` (per-slot `revision`,
`building`, `error`, `logs`, `view`), readable under the `published`
mutex alone — the poll-never-takes-cmd_lock rule holds throughout.

### First: headless parity (the design's precondition)

**Nothing publishes in headless.** `run_headless` starts the socket
server *only* (`lib.rs:24-34`); the build loop and watcher are spawned
solely by viewer sessions (`session.rs::spawn_threads`). One-off CLI
builds never publish, so in the agent's primary mode (`odm run
--headless`) the `published` map stays empty forever — today's
`odm status` already reports the default slot as eternally "pending"
there, and everything below (snapshot, transitions, sweep) would be
dead on arrival.

**Fix: headless runs the same threads as a viewer session** — build loop
+ watcher (reuse/share `spawn_threads`), `rebuild_active()` on open. This
removes an asymmetry rather than adding a feature: an engine keeps its
slots' values current; a viewer is just eyes on them (`wake` stays unset
when headless — it already no-ops). Side benefits: `status` becomes
truthful headless, background builds warm the memo cache so agent
queries after an edit are near-instant, and item 2's watcher warnings
have a headless audience. Cost: background rebuilds on every save —
bounded by the 150 ms debounce, latest-wins cancellation, and
memoization, and the project already chose consistency over avoiding
redundant work.

### The value, and its encoding

Everything below is a *read of the published value*; builds are only ever
the trigger that changes the value, never the thing being reported. A
slot's diagnostic value is `error: Option<String>` (plus `pending` when
no build has ever finished); whether a newer generation's answer is on
the way is a **freshness flag layered on top, not part of the value**.

Encode that split literally: per slot `{slot, path, build: ok|error|
pending, error?, stale?: true}` — `stale: true` whenever the slot is
queued or building (a newer answer is coming). This deliberately revises
`cmd_status`'s current collapsed `ok|error|building|pending`, where
"building" *masks* the value (an ok-slot mid-rebuild and a
first-build-in-progress are indistinguishable). Share one derivation
helper between `status` and poll; docs regen picks up the change.
Honesty fix required for the flag: `rebuild_active`/`note_generation`
enqueue without marking the published entry (only `set_view` and the
publish-time recompute do), so a watcher-triggered rebuild is invisible
until it lands — set the flag on every enqueue.

### Design

- **Every poll response gains a top-level `builds` snapshot**: the per-
  active-slot encoding above, read from `published` at answer time.
  A one-shot poll thus always answers with current build state alongside
  whatever messages it collected; the user's "it broke" arrives with the
  red slot attached. Slots are few (viewer tabs, or the one headless
  default), so the full list rides every response.
- **Follow mode emits on value changes, not on builds.** The poll request
  gains a boolean (say `"events": true`), sent by `odm poll --follow` —
  revising the "`--follow` never reaches the engine" rule in
  agent-surface: the *loop* stays CLI-side, but the engine must know to
  wake. With it set, a blocked poll also returns (possibly with
  `messages: []`) when the diagnostic value differs from what this
  connection last reported. **One comparison rule covers everything**:
  the value is a map (slot or file → error-or-none), absent keys read as
  none; emit iff any key's value differs from the connection's
  last-reported map. The old special cases fall out instead of being
  rules: the agent's own ok→ok mid-refactor saves don't emit (value
  unchanged), `stale` flips don't (not part of the value), redundant
  wakes coalesce (compare at collect time, latest-wins), opening an ok
  tab doesn't emit (none→none), switching a tab to a broken view does
  (none→error — intended: the stream reports what the user is looking
  at), closing a broken tab emits the heal (error→none), and a deleted
  file heals the same way.
- Mechanics: `publish_success`/`publish_failure` (and sweep-result
  stores) also call `wake_pollers()`. Wakes are triggers only — an
  events-poll keeps the per-connection last-reported map (like
  `Conn::hold`) and re-compares against `published` on every wake, so a
  missed or spurious wake — or a follower dying mid-batch and
  reconnecting with a fresh baseline — can neither lose nor duplicate an
  event. State-compare, not an event queue: delivery guarantees come
  free.
- **Logs never ride poll responses.** The canonical way to get a
  failure's logs is to query the view (any view command returns error +
  logs synchronously), and by the framing invariant that returns the
  same bytes whether it's a cached replay or a re-run — so nothing is
  lost, the response shape stays uniform (messages + builds + health,
  always), and a 1000-line console dump never lands in a follow batch.
  (This cuts the earlier draft's "logs ride transition batches" — a
  special case for bytes the agent can pull when it wants them.)
- Engine-host warnings (item 2) ride the same stream via the transcript —
  no separate channel.
- Agent-initiated command errors stay synchronous return values, as
  decided; nothing routes through poll.
- One-shot polls (with or without `--timeout`) don't wake on value
  changes — they just carry the snapshot whenever they return. The
  prompt already routes harnesses: park `--follow` if it can watch
  lines, else loop `--timeout`; both get diagnostics at least
  per-batch/per-round. Prompt unchanged (poll/say stays the story).

### Project health sweep (default-inputs canary)

Slots only cover what's on screen. Today a doohickey nobody has open has
*no* failure signal at all — meta extraction is lazy, per-build, cached
by code hash (`scheduler.rs:259-285`), and sync just content-hashes
files, so even a syntax error in an unviewed file is invisible.

Shape it as a value, not a process: per generation, per file, define a
pure **health value**, evaluated in the background. Two tiers:

- **Meta check, every file.** Extracting `meta` evaluates the module, so
  it catches syntax errors, load-time throws, and bad meta for the whole
  project. Cheap (hash-cached, already exists) — just needs to be *run*
  per generation for all files instead of only on demand.
- **Default-view build, standalone-buildable files only.** Build each
  file as its default view (declared defaults, nothing set —
  `View::of`). Cascade inputs always resolve (declarations must carry
  defaults), so only plain inputs can gate: skip files whose meta
  declares a required no-default plain input — those fail standalone by
  design (`effective_args`; solids can't even have defaults,
  `meta.rs:247`) — for them the meta tier is the check. Memoization
  makes unchanged files near-free; a file open in a tab with default
  inputs is the same view, already built. This is a canary at *one
  specific view*, not a verdict on the file — per the framing's second
  axis, `ok` here never precludes a slot failing at other inputs.

Scheduling: sweep items ride the build loop's queue *behind* slot builds
(slots pop first) and a new generation supersedes pending ones
(latest-wins, same as slots) — the sweep never delays a user scrub or an
agent command. Results land in a per-file state map (value + whether the
current generation has re-evaluated it); sweep passes never publish
slots.

Two costs to carry consciously:
- Until item 6, broken files are the expensive case — failures aren't
  memoized, so every generation re-runs every broken file's default
  build.
- **The sweep is gated on a decision for
  issues/uncancellable-module-eval-loops.md.** A top-level `while(true)`
  is uncancellable during module eval; today it hangs only when someone
  *looks at* the file — the sweep upgrades that to "hangs the build loop
  whenever such a file exists", killing all slot rebuilds until the
  engine restarts (the hung pass also pins the build gate shared).
  Agents do occasionally write runaway top-level code. Before shipping
  the sweep, either land the issue's watchdog (generous timeout on
  module eval, accepting the possibly-fixed-upstream rusty_v8 #830 risk)
  or verify a newer rusty_v8 allows pre-eval handle registration.
  Shipping the sweep without one of these is a regression in blast
  radius, not a judgment call to make silently.

Also note: the sweep materializes every standalone file's default-view
output in the memo cache (meshes pinned, kernel cache live) — same
footprint as "user opened every file once", automatic per project. Fine
at current scale; feeds the issues/memo-cache-policy.md ledger.

Reporting: a `health` section next to `builds` — **failures only**:
`{path, error, stale?: true}` per file whose last-evaluated check
failed; `stale: true` when the current generation hasn't re-evaluated it
(the value shown is the last evaluated one — required by the invariant:
a poll right after an edit must not present old results as current;
staleness is visible instead of silent). Absence of a file claims
nothing beyond "no known failure" — reporting per-file `ok`/`skipped`
rows on every poll re-runs the 191 KB `tree` mistake (agent-surface: the
scarce resource is output shape); the ok/skipped detail is derivable on
demand (query the file) and earns a field the day someone needs it. (No
generation numbers in the response — standing cut, `generation` appears
only in `status`.) Health values feed the follow stream through the same
one comparison rule as slots. `status` gains the same failures-only
`health` list next to `views`. Viewer surfacing (e.g. marking files in
tab pickers) can come later.

**Surface updates:** `requests.rs` spec table (`events` field),
`docs/cli.md` poll + status sections (builds/health shapes, `from`),
`notes/agent-surface.md` (the `--follow` exception wording, the status
`build`-field re-encoding, response shapes), `notes/architecture.md`
(headless thread parity). Prompt only if the core-loop wording needs it
(likely not — poll/say stays the story).

**Tests:** the value-compare emit rules as `state.rs` unit tests (emit on
ok→error/error→different-error/error→none, no emit on ok→ok republish or
stale flips, reconnect re-reports); a follow poll woken by
`publish_failure` over a real socket (pattern: `server::tests`);
headless-parity coverage (a build loop running without a viewer
publishes the default slot); sweep transitions for an unviewed file
breaking and healing.

## 5. Record failed invokes as deps (correctness bug, confirmed)

The framing section's third bullet, standalone because it's a live
invariant violation independent of everything above. Repro (verified
2026-08-17): root catches `ctx.invoke('parts/maybe.js')` failing and
returns a fallback; child fixed; incremental rebuild still returns the
fallback root hash while a fresh engine returns the real one — the
memo entry recorded no dep on the child, so nothing invalidates it.

**Fix:** give `Dep::Invoke` an outcome — success (output hash, as today)
or failure (identity hash of the child's `(kind, message)`) — and record
it in `op_invoke`'s error arm before throwing. `validate`'s invoke arm
re-runs the child as today and compares outcomes: recorded-success
matches `Ok` with the same hash; recorded-failure matches `Err` with the
same failure identity; `Err(Cancelled)` matches nothing (cancellation is
not a value — conservatively invalidate, the entry gets revalidated next
pass). One representation, no catch-specific special case — and item 6
needs exactly this to validate memoized failures whose throw came from a
failing invoke (the most common propagation path).

Test: the repro above as a build.rs test (incremental hash ==
fresh-engine hash after fixing a caught-failing child), plus the
mirror: child *starts* working → caught-fallback parent rebuilds.

## 6. Memoize failures (follow-up; blocks nothing above)

Makes the framing invariant hold by construction instead of by
determinism, and stops re-running broken builds on every generation
(which the health sweep amplifies): `MemoEntry.output` becomes
success-or-error, and a hit on a failure entry replays error + logs
exactly like success logs replay today.

- Needs deps on the error path: odm-js must return the deps recorded up
  to the throw so the entry can be validated like any other. With item 5
  landed, a parent failing *because a child failed* has that child as an
  ordinary failed-invoke dep — validation is uniform, no special case.
- Memoize only pure failure kinds: `Js`, `BadOutput`. Never `Cancelled`
  (not an output of the function) or `Internal` (environmental). Exclude
  `Cycle` — its message embeds the in-progress chain, which is calling
  context, not a function of (code, args). `Input` failures on cascades
  fail in the caller's frame before the memo key exists; nothing to
  memoize, and they're cheap to re-derive.
- Store ripples: failure entries pin no output, so `Store::gc`'s
  memo-output roots skip them; one-entry-per-key overwrite and
  no-eviction (issues/memo-cache-policy.md) apply unchanged.

## Order

1, 3, and 5 are independent and small — do them first (5 is a live
correctness bug). 2 → 4 ship together (transcript kind feeds the
stream), with headless parity as 4's first step; 4's health sweep waits
on the uncancellable-eval decision (watchdog or rusty_v8 check) and can
trail the rest of 4. 6 is a follow-up on 5's representation, worth doing
once the sweep's re-run cost on broken files shows up in practice.
