# Renaming a root's inputs strands the viewer tab, with no CLI way out

From agent feedback (real session, 2026-08), plus user-reported follow-up.

The agent rewrote `root.js` with a new input set (`bays_x`/`bays_y` →
`a_*`/`b_*`). Every CLI query built fine (defaults), but the user's tab had
the old inputs pinned, so their slot went red with
`unknown input "bays_x" in args` — and the CLI has no way to clear or
re-point a view slot's inputs, so the only fix was asking the user to reset
the tab by hand. Meanwhile `odm status` and `odm render` disagree about
whether the project is broken, which is confusing to read.

Wanted: either the viewer drops pinned inputs the target no longer declares
(they're gone; keeping them can only error), or a CLI verb to set/clear a
slot's inputs.

Error-message asks from the same incident:

- **One unknown input reported per rebuild.** The user cleared stale fields
  one at a time: `unknown input "bays_x"` → clear → `unknown input
  "bays_y"` → clear → ok. Report *all* unknown inputs at once, and say in
  the message that these are values pinned on the view rather than anything
  wrong with the file.
- **The declared-args list omits cascade inputs.** The error read
  `declared args: a_length, ..., post, tarp` — no `size`, even though
  `root.js` declares `size` with `cascade: true` and it is settable on the
  view. A user reading that list would conclude `size` isn't settable.
