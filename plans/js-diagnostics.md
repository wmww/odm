# JS diagnostics surfacing

From `issues/js-diagnostics-surfacing.md`; all four claims re-validated
against the code 2026-08-17 (all still current). Background that already
landed: failed builds return logs as data (`FailedBuild`), viewer has a
Console panel with latest-attempt logs, `Published` carries status
ingredients + `error` + `logs` (`state.rs:26-44`), and `odm poll --follow`
exists (CLI-side re-poll loop, `odm-cli/src/lib.rs:222-249`).

## 1. Duplicate log replay on late validation failure

Confirmed in `odm-build/src/scheduler.rs`. `get_or_build`'s memo-hit path
(:434-447) calls `validate` (:585-618), which recurses through
`Dep::Invoke` via `get_or_build` (:610) — every child hit replays its logs
into `pass.logs` (:442-445), every child miss appends via `run_one`
(:546-548). If a *later* dep then invalidates the entry, nothing rolls
back; the parent reruns and re-invokes the same children (now hits), so
the whole already-validated subtree's logs appear twice. Since cascade
deps sort ahead of invoke deps in `entry.deps`, late invalidation is
exclusively the invoke case, exactly as the issue said.

Same non-rollback also skews stats: `memo_hits` (:438-439) counts child
hits from a discarded validation *and* again on the rerun; child rebuilds
inside `validate` run with no frame pushed for the parent, so their time
is charged to the grandparent's accumulator and effectively double-counted
(:503-504, :529-542).

**Fix** (all inside the memo-hit block of `get_or_build`):
- Record `pass.logs` length before `validate`; on `false`, truncate back
  to the mark. Safe: one pass's build tree is strictly single-threaded
  (nested invokes are inline, LIFO), so the mark is stable — the mutex
  only guards cross-thread access to distinct passes.
- Snapshot/restore the pass stat counters the same way (`memo_hits` at
  minimum).
- Timer attribution for children rebuilt during a failed validation stays
  approximate — fixing it needs a frame for the not-yet-entered parent;
  leave unless it falls out naturally, but don't restore-snapshot *time*
  (the work was real).
- Test: two-dep root where dep A logs and dep B's source changes between
  passes; assert A's line appears exactly **once** (count, not
  `contains` — the existing `logs_are_collected_per_pass` can't catch
  duplicates).

Out of scope: replay *ordering* (on a hit, subtree lines all precede the
parent's, whereas from-scratch interleaves around invokes). Making "pass
logs ≈ from-scratch" literal needs interleaving positions in
`MemoEntry.logs`; not worth it now.

## 2. Engine-host warnings as data

Confirmed all stderr-only: odm.toml newer-engine warning + `sync_marker`
failure + `odm_prompt::sync` warnings (`session.rs:128-135`), watcher
creation/watch failure — which silently ends live rebuild for the session
(`watcher.rs:38-44`), server-thread death (`session.rs:152`). Invisible to
viewer and agent; headless drops even the open-time questions.

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
  `Log`); viewer match becomes exhaustive. Contract instead of
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

Errors are per-*view* (path + inputs + cascades), not per-doohickey —
there is no "this doohickey fails" state. What poll reports is therefore
two things: the **slots** (what the user is looking at, exact inputs and
all) and a **project health sweep** (below) covering everything they
aren't.

**Design:**
- **Every poll response gains a top-level `builds` snapshot**: per active
  slot `{slot, path, build: ok|error|building|pending, error?}` — the
  same derivation `cmd_status` uses (share the helper). Latest-wins by
  construction; cheap (no logs). One-shot poll thus always answers with
  current build state alongside whatever messages it collected; the
  user's "it broke" arrives with the red slot attached. This is "errors
  as the user sees them" — each tab's current view.
- **Follow mode wakes on material transitions.** The poll request gains a
  boolean (say `"events": true`), sent by `odm poll --follow` — revising
  the "`--follow` never reaches the engine" rule: the *loop* stays
  CLI-side, but the engine must know to wake. With it set, a blocked poll
  also returns (possibly with `messages: []`) when any slot's
  `(failed?, error)` tuple materially changes — ok→failed, failed→ok,
  failed→different-failure. Plain successful rebuilds (agent's own
  mid-refactor saves) don't emit; `building` flips don't either. That is
  the issue's coalescing answer: opt-in by being a follower, latest-wins
  because the response reads `published` at collect time.
- Mechanics: `publish_success`/`publish_failure` also call
  `wake_pollers()`; an events-poll tracks per-slot last-reported
  `(revision, failed, error)` on the connection (like `Conn::hold`) and
  re-checks on each wake.
- **Logs ride only transition-triggered responses**, for the slot(s) that
  transitioned. One-shot pollers fetch logs on demand (a view command or
  rebuild returns error + logs synchronously). This trims the original
  direction ("status + failure message + logs" on the snapshot) to keep
  every poll answer small; the follower still gets each failure's logs
  exactly once.
- Engine-host warnings (item 2) ride the same stream via the transcript —
  no separate channel.
- Agent-initiated command errors stay synchronous return values, as
  decided; nothing routes through poll.

### Project health sweep (default-inputs canary)

Slots only cover what's on screen. Today a doohickey nobody has open has
*no* failure signal at all — meta extraction is lazy, per-build, cached
by code hash (`scheduler.rs:259-285`), and sync just content-hashes
files, so even a syntax error in an unviewed file is invisible. Most
doohickeys take no inputs or fail the same way regardless of them, so
building each at its defaults is a good canary. Two tiers:

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
  inputs is the same view, already built.

Scheduling: sweep items queue *behind* slot builds on the build thread
and a new generation supersedes pending ones (latest-wins, same as
slots) — the sweep never delays a user scrub or an agent command.

Reporting: a `health` section next to `builds` — per file
`{path, check: ok|error|skipped, error?}` (`skipped` = not
standalone-buildable, meta ok). Same material-change rule feeds the
follow stream: an unviewed file breaking (or healing) is a transition
line. No logs here either; the agent reproduces with a view command.
Viewer surfacing (e.g. marking files in tab pickers) can come later.

**Surface updates:** `requests.rs` spec table (`events` field),
`docs/cli.md` poll section, `notes/agent-surface.md` (the `--follow`
exception wording + response shape), prompt only if the core-loop wording
needs it (likely not — poll/say stays the story).

## Order

2 → 4 ship together (transcript kind feeds the stream); 1 and 3 are
independent and small. Suggested: 1, 3, then 2+4.
