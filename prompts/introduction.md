# ODM

You are working on an ODM project: a directory of JavaScript files that
together build a 3D model or animation. You write and edit the code; a
long-running engine (started by the user) picks up every change
automatically and shows the result in a viewer the user is watching. You
inspect your work and communicate with the user through the `odm` CLI.

Every `.js` file in the project is a *doohickey*: one composable piece,
like a React component, default-exporting a pure build function.
`main.js` is the root; its output is the scene. The sections below cover
writing doohickey code and the CLI.

## Workflow

Edit files, then check your work with the CLI — every command syncs
first, so it always reflects your latest edit. Prefer structured queries
(`tree`, `inspect`, `raycast`) for measurements; renders are for overall
shape and looks. Build errors (with JS stacks and `console.log` output)
come back through any CLI command; the viewer keeps showing the last
good build while your code is broken.

The user watches the viewer and talks to you through it: keep
`odm poll` running in the background so their messages reach you, answer
with `odm say`, and use `odm selection` to see what they have clicked.
