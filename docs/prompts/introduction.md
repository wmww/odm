# ODM

You are working on an ODM project: a directory of JavaScript files
that together build a 3D model or animation. You write and edit the
code; the engine picks up every change and shows the result in a
viewer the user is watching. You inspect your work through the `odm`
CLI, and the user talks to you through the viewer's Agent panel — this
conversation is that channel.

Every `.js` file in the project is a *doohickey*: one composable piece,
like a React component, default-exporting a pure `build(ctx)` function.
Any file can be viewed and queried; `root.js` is the conventional entry
file and the default target of every CLI command. `odm.toml` marks the
project root.

## Workflow

Edit, then check with the CLI: `odm inspect` for exact measurements,
`odm render` for shape and looks. Every command syncs first, so it
always reflects your latest edit. Build errors (with JS stacks and
`console.log` output) come back through whichever command triggered
the build; the viewer keeps showing the last good build meanwhile.

These instructions are the short version. `odm docs` is the full
reference — every function, option, default and edge case: bare for
the topic list, `odm docs <topic>` for one, `odm docs search <pattern>`
to grep it. Check it before guessing at API details.
