# Spike 0a findings: build scheduler (2026-07-22)

Toy spike code deleted after review (registry design now lives in crates/odm-build/src/registry.rs). 8 tests, all green and stable over
repeated runs. Validates the decided design: nested invokes build inline on the
requesting worker (disposable per-thread isolate), memo keyed by (node, code, args),
Salsa-style dep validation.

## Dedup policy decision: WAIT with wait-graph cycle detection

Recommended and implemented: a nested (or top-level) request for an in-flight key
**waits**, guarded by a wait-for-graph walk done atomically under the registry lock.
Key insight making this safe:

- Wait edges: blocked thread → key it waits on → owning thread → key *that* thread
  waits on → ... Every hop is a genuine dependency edge (a worker's build stack is a
  chain of real invokes; a wait is a real invoke).
- Therefore **any would-be wait cycle is a genuine dependency cycle in the user's
  doohickey graph** — report it as a Cycle error instead of blocking. No false
  positives, no deadlock-by-scheduling: deadlock-freedom is invariant because every
  wait edge is checked for cycle-formation before blocking, under one lock.
- The walk is ~O(pool size), trivial cost.

Measured alternative (`always-duplicate` for nested in-flight hits): on randomized
layered DAGs (100 nodes, fan-in 1–3, 8 workers, all nodes requested top-level),
**14–21% of builds were duplicates (18% mean)**. Harmless for correctness but real
waste — in the real system each duplicate also pays isolate creation + a geometry
build. Wait policy: 0 duplicates, same wall time. Duplicate mode's only advantage
(no wait bookkeeping) isn't worth it.

## Data structures that worked (recommend for real scheduler)

- `Registry` (one global `Mutex` + `Condvar`):
  - `entries: HashMap<Key, State>`; `State = InFlight { owner: ThreadId } |
    Done(Result<Arc<MemoEntry>, BuildError>)`
  - `waiting_on: HashMap<ThreadId, Key>` — the wait-for graph edges.
- `Key = (node path, code hash, args hash)`. Because code hash is in the key,
  cached **Cycle errors can be cached permanently**: fixing the cycle necessarily
  changes code → new key. Elegant, no per-generation error invalidation needed.
- `MemoEntry { output hash, deps: Vec<(node, args, output hash)> }` — recorded
  invokes. Validation on hit: recursively `ensure` each recorded dep under the
  current generation and compare output hashes; all match → reuse (this *is* early
  cutoff). Mismatch → claim entry (guard against concurrent validators by
  pointer-compare-then-swap) and rebuild.
- Per-thread build stack (`thread_local Vec<Key>`): pushed around each inline build.
  Catches same-thread cycles before touching the registry and provides the cycle
  path for error messages.
- Cancellation: `Arc<AtomicBool>` per generation. Checked at ensure entry, inside
  fake work (real system: TerminateExecution + kernel ExecutionContext), and after
  each condvar wake. `cancel()` sets flag + `notify_all`. Waiters use
  `wait_timeout(5ms)` as a belt against missed wakeups.

## Semantics validated by tests

- Pool of 1, 200-deep nested chain: fine (no deadlock by construction; recursion
  depth = invoke nesting depth — note: give real workers a generous stack size and
  a depth cap for error reporting).
- Scheduler-level dedup: 8 concurrent top-level requests for one node → 1 build.
- Same-thread cycle A→B→A: Cycle error with path, cached, no hang.
- Cross-worker cycle (staged so A,B claim on different workers, then invoke each
  other): wait-graph detection fires on the second waiter; both sides get Cycle
  errors within ms. No deadlock.
- Cancellation: mid-build cancel observed <150ms (1ms check granularity);
  cancelled builds publish nothing (transactional — claim removed, waiters wake and
  re-decide); sub-results completed before cancel stay published and are reused by
  the next generation (memo is cross-generation, keyed by code hash).
- Early cutoff: dep's code changed but output identical → dep rebuilds once,
  dependent's memo validates and is NOT rebuilt. Consequential change → both rebuild.
- Duplicate-policy variant also implemented behind a flag purely for measurement;
  cycles there are caught by the thread-local stack alone (a cycle always manifests
  on one stack when you never wait).

## Surprises / notes for the real implementation

- `gen` is a reserved keyword in Rust edition 2024 — don't name variables that.
- Error caching: build errors (cycle, and by extension JS exceptions) are
  deterministic outcomes of (code, args) → cache them like successes. Only
  Cancelled is transient (entry removed, never cached).
- Waiter wake-up on cancel needs `notify_all` in `cancel()` *and* a timeout on the
  wait — a waiter's generation can be cancelled while the build it waits on (from
  another generation) keeps running.
- Validation happens outside any claim (entry stays Done while validators recurse).
  Two threads may validate the same entry concurrently — harmless; the
  claim-for-rebuild race is settled by pointer-compare under the lock.
- Cross-generation reuse falls out naturally: nothing in the registry is
  generation-scoped except the cancel flag; the generation only supplies
  node → (code hash, spec) resolution.
