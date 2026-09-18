# The viewer draws `→` (and friends) as a .notdef box

Seen 2026-09-17 while testing the feedback page: a report body reading
`engine → viewer → form` paints as `engine □ viewer □ form`. The same
goes for any agent text in the chat transcript — agents write arrows
constantly.

`odm-sans-14` is converted from Adobe `helvR10`, which is ISO8859-1
and simply has no arrows, so `bdf2ttf.py`'s KEEP range for
`0x2190..0x21FF` keeps nothing. The fonts README says anything outside
the kept ranges "falls back to egui's built-in fonts" — that is what is
not happening here, either because egui's own fallback faces don't
cover U+2192 either, or because the fallback list isn't reached for
this range.

Worth checking which, since the fix differs: a bundled strike that has
arrows (misc-fixed covers more than helvR10), painting the handful that
matter from `theme::pixels`, or fixing the font-family fallback order.

Not urgent — the text is stored correctly and only the on-screen glyph
is wrong.
