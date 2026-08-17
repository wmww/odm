# Short prompt doesn't say where a doohickey's description comes from

From agent feedback (real session, 2026-08): the agent put `description` in
`meta` and got it rejected. `AGENTS.md` says `odm inspect '{"fields":
["inputs", "presets"]}'` "reports a file's description, settable inputs, and
presets", and the `meta` example right above shows `inputs` and `presets` —
so `description` reads like a sibling meta key. It isn't; it's the leading
`//!` comment block, which is only documented in `odm docs doohickeys`.

One clause in the short prompt ("the leading `//!` block is the file's
description") would have saved a failed build.

The error message itself was praised — it lists the allowed keys. But
per-input `description` *is* allowed inside an input schema, which makes the
top-level rejection more surprising, not less; worth keeping in mind if the
meta key is ever reconsidered.
