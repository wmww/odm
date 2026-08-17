# Memoize failures

Deferred follow-up from plans/js-diagnostics.md (item 6; everything else
landed 2026-08-17). Do it when the health sweep's re-run cost on broken
files shows up in practice — until then, broken files re-run their
default build every generation (failures aren't memoized), which the
sweep amplifies.

Goal: the "error + logs are part of a build's output" invariant holds by
construction instead of by isolate determinism, and broken builds stop
re-running. `MemoEntry.output` becomes success-or-error; a hit on a
failure entry replays error + logs exactly like success logs replay.

- Needs deps on the error path: odm-js must return the deps recorded up
  to the throw so the entry validates like any other. With failed-invoke
  deps recorded (`Dep::Invoke` outcome, landed), a parent failing
  *because a child failed* has that child as an ordinary dep —
  validation is uniform, no special case.
- Memoize only pure failure kinds: `Js`, `BadOutput`. Never `Cancelled`
  (not a value of the function) or `Internal` (environmental). Exclude
  `Cycle` — its message embeds the in-progress chain (calling context,
  not a function of code+args). `Input` failures on cascades fail in the
  caller's frame before the memo key exists; nothing to memoize, cheap
  to re-derive.
- Store ripples: failure entries pin no output, so `Store::gc`'s
  memo-output roots skip them; the bounded MRU/LRU cache policy applies
  unchanged (smaller pressure — no meshes pinned).
