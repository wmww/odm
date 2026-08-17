# `camera` round-trips one way only

From agent feedback (real session, 2026-08). Both `odm render`'s response
and a poll's `view` hand back a nested `camera: {eye, target, up, fov}`, and
the docs say to paste it back ("nudge its numbers and paste them back for
exact placement", "pasting it into a render replays their exact view").
Doing literally that fails:

    odm render '{"camera": {"eye": [...], "target": [...]}}'
    render has no field "camera"; fields: ..., eye, target, up, fov, ...

The request wants the four keys flat at the top level. Either accept a
nested `camera` object too, or echo the fields flat so a copy-paste works.
