# Memo cache policy: one entry per key, no eviction

`odm-store`'s memo map holds a single `MemoEntry` per `MemoKey`. A doohickey
that reads the cascade input `t` gets its entry overwritten on every distinct t, so
scrubbing the timeline back and forth re-runs every t-reading doohickey each
time (content addressing still dedups the *outputs*, and non-t-readers stay
cached — so this is wasted CPU, never wasted memory or incorrectness).

The mirror-image problem: nothing ever evicts the memo cache
(`Store::memo_clear` is only called from a test). Every distinct
`(code, args)` pair keeps an entry for the life of the engine, and each entry
pins its output node — and transitively its meshes — against GC, each pinned
mesh keeping a live `Manifold` in the kernel cache. A parametric part invoked
with many distinct sizes accumulates without bound.

The thrash is now *observable*: `"stats": true` on any view command reports per-pass stats
(per-doohickey runs + self-time, memo hits), so "arm.js runs: 2" on every
identical rebuild is this issue showing itself.

Fix idea (from Salsa-ish designs), solving both together: store a small
bounded list of entries per key and validate each candidate's recorded deps
on lookup (`odm-build/src/scheduler.rs` `get_or_build`/`validate`), plus an
overall cap / LRU across keys. Would make timeline playback after one full
scrub nearly free while bounding memory. Needs an eviction policy (LRU per
key, cap ~8?) and a benchmark on `examples/piston`.
