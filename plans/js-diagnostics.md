# JS diagnostics surfacing

From `issues/js-diagnostics-surfacing.md`; claims re-validated against the
code 2026-08-17. Background that already landed: failed builds return logs
as data (`FailedBuild`), viewer has a Console panel with latest-attempt
logs, `Published` carries status ingredients + `error` + `logs`
(`state.rs:26-44`), and `odm poll --follow` exists (CLI-side re-poll loop,
`odm-cli/src/lib.rs:222-249`).

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
  item 5 makes it hold by construction (and stops re-paying for broken
  builds).
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
the whole already-validated subtree's logs appear twice. Since cascade
deps sort ahead of invoke deps in `entry.deps`, late invalidation is
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
  to the mark. This also covers a child that *fails* during validation
  (failures aren't memoized, so the parent's rerun re-runs it and
  re-appends its logs exactly once). Safe: one pass's build tree is
  strictly single-threaded (nested invokes are inline, LIFO), so the mark
  is stable — the mutex only guards cross-thread access to distinct
  passes.
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
viewer and agent; headless drops even the open-time questions.

These are session events, not build output — explicitly outside the
framing invariant, which is why they get the transcript channel rather
than riding any build result.

**Fix: put host warnings in the transcript.** Add a third `Who::Engine`
(or a parallel kind) to `TranscriptEntry` and route these warnings through
`send_message`-style queuing:
- Viewer chat panel renders engine lines in a distinct style — the user
  sees "file watcher unavailable: …" where they already look.
- Poll delivers them with the existing Pending/InFlight/Done handshake for
  free — no new queue, no lost-on-dead-CLI path. Poll response entries
  gain `"from": "user" | "engine"` (user stays the default the prompt
  teaches; docs get the full story).
- Keep the stderr prints too (daemon logs).
- Sites: the session-open warnings fire before any client connects; the
  transcript persists, so the agent's first poll collects them — no
  ordering problem.

## 3. Log levels: normalize, don't weaponize

Confirmed: `LogLine.level` is a freeform `String` set only by the shim
(`framework/odm/index.js:682-691` — `log`/`debug`/`warn`/`error`, `info`
→ `log`) plus the engine-made `"warn"` truncation notice; viewer styles
by string match with silent fallback (`viewer/mod.rs:1032-1037`); nothing
filters; `console.error` doesn't fail builds (docs promise this —
`docs/api/doohickeys.md:86-89`).

**Decision:** this is mostly working as intended. Do only:
- Make the level an enum (`Log`/`Debug`/`Warn`/`Error`) in
  `odm-store::LogLine`, converted at the `op_log` boundary (unknown →
  `Log`); viewer match becomes exhaustive. Levels are part of the
  memoized log output, so the enum lives store-side. Contract instead of
  convention, no behavior change.
- Explicitly **keep**: no level filtering (1000-line cap is the only
  limit), `console.error` never fails or flags a build.

## 4. Async diagnostics over poll (the real gap)

Confirmed: `cmd_poll` returns `{"messages": [...]}` only
(`commands.rs:647-686`); per-message `view` is the viewer's send-time
stamp with no build fields; a timed-out poll says nothing at all. Build
transitions cannot even wake a parked poller — `publish_success/_failure`
call `wake()` (viewer repaint), not `wake_pollers()`
(`state.rs:580,603`). `--follow` never reaches the engine, so it streams
chat batches only. `odm status` does give per-slot status + error (not
logs), but only when asked — the agent still only learns of a red tab if
the user types "it broke".

Everything needed is already in `Published` (per-slot `revision`,
`building`, `error`, `logs`, `view`), readable under the `published`
mutex alone — the poll-never-takes-cmd_lock rule holds throughout.

**Design.** Everything below is a *read of the published value*; builds
are only ever the trigger that changes the value, never the thing being
reported:
- **Every poll response gains a top-level `builds` snapshot**: per active
  slot `{slot, path, build: ok|error|building|pending, error?}` — the
  same derivation `cmd_status` uses (share the helper), read from
  `published` at answer time. `ok|error` is the slot's current diagnostic
  value; `building|pending` is a freshness marker layered on top (a newer
  generation's answer is on the way), not part of the value. One-shot
  poll thus always answers with current build state alongside whatever
  messages it collected; the user's "it broke" arrives with the red slot
  attached.
- **Follow mode emits on value changes, not on builds.** The poll request
  gains a boolean (say `"events": true`), sent by `odm poll --follow` —
  revising the "`--follow` never reaches the engine" rule in
  agent-surface: the *loop* stays CLI-side, but the engine must know to
  wake. With it set, a blocked poll also returns (possibly with
  `messages: []`) when any slot's `(failed?, error)` value differs from
  what this connection last reported. The old special cases fall out of
  value-compare instead of being rules: the agent's own ok→ok
  mid-refactor saves don't emit (value unchanged), `building` flips don't
  (not part of the value), redundant wakes coalesce (compare at collect
  time, latest-wins). A slot's value can also change because the *user
  navigated* — switching a tab to a broken view emits ok→failed; that's
  intended: the stream reports what the user is looking at.
- Mechanics: `publish_success`/`publish_failure` also call
  `wake_pollers()`. Wakes are triggers only — an events-poll keeps
  per-slot last-reported `(failed?, error)` on the connection (like
  `Conn::hold`) and re-compares against `published` on every wake, so a
  missed or spurious wake can neither lose nor duplicate an event.
- **Logs ride only transition-triggered responses**, for the slot(s) that
  transitioned — a delivery convenience, not the access path. The
  canonical way to get a failure's logs is to query the view (any view
  command returns error + logs synchronously), and by the framing
  invariant that returns the same bytes whether it's a cached replay or a
  re-run — so a one-shot poller, or a follower that missed an event,
  loses nothing.
- Engine-host warnings (item 2) ride the same stream via the transcript —
  no separate channel.
- Agent-initiated command errors stay synchronous return values, as
  decided; nothing routes through poll.

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
  `scheduler.rs:30`). Skip files whose meta declares a required
  no-default plain input: those fail standalone by design
  (`scheduler.rs:668-675`; solids can't even have defaults,
  `meta.rs:247`) — for them the meta tier is the check. Memoization
  makes unchanged files near-free; a file open in a tab with default
  inputs is the same view, already built. This is a canary at *one
  specific view*, not a verdict on the file — per the framing's second
  axis, `ok` here never precludes a slot failing at other inputs.

Scheduling: sweep items queue *behind* slot builds on the build thread
and a new generation supersedes pending ones (latest-wins, same as
slots) — the sweep never delays a user scrub or an agent command. Cost
note: until item 5, broken files are the expensive case — failures
aren't memoized, so every generation re-runs every broken file's default
build. The sweep also widens exposure to
issues/uncancellable-module-eval-loops.md (a hanging eval in an
*unviewed* file can now occupy the build thread).

Reporting: a `health` section next to `builds` — per file
`{path, check: ok|error|skipped, stale?: true, error?}`. `skipped` = not
standalone-buildable, meta ok. `stale: true` marks entries the current
generation's sweep hasn't re-evaluated yet — the value shown is the last
evaluated one. Required by the invariant: a poll right after an edit must
not present old results as current just because the sweep hasn't run yet;
staleness is visible instead of silent. (No generation numbers in the
response — standing cut, `generation` appears only in `status`.)
Transitions feed the follow stream on *evaluated*-value changes, same
compare-at-collect rule as slots: an unviewed file breaking (or healing)
is a transition line. No logs here either; the agent reproduces with a
view command. Viewer surfacing (e.g. marking files in tab pickers) can
come later.

**Surface updates:** `requests.rs` spec table (`events` field),
`docs/cli.md` poll section, `notes/agent-surface.md` (the `--follow`
exception wording + response shape), prompt only if the core-loop wording
needs it (likely not — poll/say stays the story).

## 5. Memoize failures (follow-up; blocks nothing above)

Makes the framing invariant hold by construction instead of by
determinism, and stops re-running broken builds on every generation
(which the health sweep amplifies): `MemoEntry.output` becomes
success-or-error, and a hit on a failure entry replays error + logs
exactly like success logs replay today.

- Needs deps on the error path: today only `Ok` carries `out.deps`
  (`run_one`, `scheduler.rs:544-582`); odm-js must return the deps
  recorded up to the throw so the entry can be validated like any other.
- Memoize only pure failure kinds: `Js`, `BadOutput`. Never `Cancelled`
  (not an output of the function) or `Internal` (environmental). Exclude
  `Cycle` — its message embeds the in-progress chain, which is calling
  context, not a function of (code, args). `Input` failures on cascades
  fail in the caller's frame before the memo key exists
  (`scheduler.rs:400-413`); nothing to memoize, and they're cheap to
  re-derive.
- Interacts with issues/memo-cache-policy.md: one-entry-per-key overwrite
  and no-eviction apply to failure entries the same way (they pin no
  meshes, so the memory pressure is smaller).

## Order

2 → 4 ship together (transcript kind feeds the stream); 1 and 3 are
independent and small; 5 is a follow-up, worth doing after 4 if the
sweep's re-run cost on broken files shows up in practice. Suggested: 1,
3, then 2+4, then 5 as warranted.
