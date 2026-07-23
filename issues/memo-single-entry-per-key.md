# Memo cache stores one entry per (code, args)

`odm-store`'s memo map holds a single `MemoEntry` per `MemoKey`. A doohickey
that reads `ctx.t` gets its entry overwritten on every distinct t, so
scrubbing the timeline back and forth re-runs every t-reading doohickey each
time (content addressing still dedups the *outputs*, and non-t-readers stay
cached — so this is wasted CPU, never wasted memory or incorrectness).

Fix idea (from Salsa-ish designs): store a small bounded list of entries per
key and validate each candidate's recorded deps on lookup
(`odm-build/src/scheduler.rs` `get_or_build`/`validate`). Would make timeline
playback after one full scrub nearly free. Needs an eviction policy (LRU per
key, cap ~8?) and a benchmark on `examples/piston`.
