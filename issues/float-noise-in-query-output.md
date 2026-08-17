# Query numbers can print 17 digits of float noise

Original problem (2026-08): mesh positions were f32, so `odm inspect`
bounds read `-25.600000023841858` where the model says `-25.6`. Fixed
2026-08-17 by the mesh f64 switch — bounds of authored geometry now print
clean.

What remains is ordinary f64 arithmetic noise: volume/area of CSG results
(and bounds of rotated/CSG'd geometry) are computed values and can still
print `1000.0000000000005`-style tails. Rarer and always honest, but the
output exists to be read by agents, and 17-digit tails invite treating
noise as signal.

## Where things stand (surveyed 2026-08-17)

- All one-shot CLI responses funnel through one printer:
  `crates/odm-cli/src/lib.rs:141` → `pretty()` (`lib.rs:270`), which
  renders scalars via serde_json `Value::to_string()` (`lib.rs:284-287`,
  `:344`) — full shortest-roundtrip precision, no rounding.
  `odm poll --follow` bypasses `pretty` (`lib.rs:230`).
- The engine already cleans some fields ad hoc: camera echo
  (`crates/odm-engine/src/commands.rs:842`, f32-shortest trick), colors
  (`crates/odm-engine/src/scene.rs:457`), stats ms (`commands.rs:887`).
  Measurements (bounds/volume/area/matrices in `scene.rs:343-378`,
  raycast `scene.rs:687`, clearance `commands.rs:631`) get nothing —
  that asymmetry is the current inconsistency.

## Proposed fix (small, decision-ready)

Round for display at the CLI choke point: in `pretty`'s scalar path,
re-render f64s at ~13 significant digits, then take the shortest repr of
that rounded value (`format!("{:.12e}", v).parse::<f64>()` →
`to_string()`). `1000.0000000000005` → `1000.0`; `-25.6` unchanged.
Protocol keeps full precision; only human/agent-facing text changes.
Also apply to the poll path or leave poll raw (it's machine-shaped).
Engine-side per-field cleaning is the alternative but touches ~a dozen
sites and would double-round the already-cleaned camera/color fields.
