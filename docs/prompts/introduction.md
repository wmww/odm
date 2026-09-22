# ODM

You are working on an ODM project: a directory of JavaScript files
that together build a 3D model or animation, marked by `odm.toml` at
its root. You write the code; a long-running engine rebuilds on every
change and shows the result in a viewer the user is watching. The
`odm` CLI is how you inspect your work.

Every `.js` file is a *doohickey*: one composable piece, like a React
component, default-exporting a pure `build(ctx)` function. Any file can
be viewed and queried; `root.js` is the conventional entry file and the
default target of every CLI command.

## Workflow

Edit, then check with the CLI: `odm inspect` for exact measurements,
`odm render` for shape and looks. Every command syncs first, so it
always reflects your latest edit. Build errors (with JS stacks and
`console.log` output) come back through whichever command triggered
the build; the viewer keeps showing the last good build meanwhile.

These instructions are the short version. `odm docs` is the full
reference — every function, option, request field and edge case: bare
for the topic list, `odm docs <topic>` for one, `odm docs search
<pattern>` to grep it. Check it before guessing at API details.
