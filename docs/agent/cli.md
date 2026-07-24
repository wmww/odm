# CLI usage

The `odm` CLI talks to the running engine (the user starts it:
`odm-engine <project-dir>`, `--headless` for no viewer). Run `odm` from
anywhere inside the project; it finds the engine socket by walking up, or
pass `--project <dir>` (must come first).

Every command **syncs first** — it rescans your files, rebuilds what changed,
then answers. Editing a file and immediately running a command always
reflects the edit. All commands print one JSON object; exit code 0 = ok.

## Commands

```
odm status                    # files, generation, animation duration
odm sync                      # force a rescan (every command syncs anyway)
odm build [--t 1.5]           # build only; errors + console logs
odm tree [--t] [--depth N]    # node ids, names, meshes, world bounds
odm inspect <node-id> [--t]   # volume, area, bounds, world matrix
odm raycast --origin 0,0,50 --dir 0,0,-1 [--t]
odm render [options]          # PNG → prints path
odm selection                 # current viewer selection (node id + name)
```

Render options: `--t sec`, `--width/--height px` (default 1024×768),
`--out file.png` (default under `.odm/renders/`), `--wireframe` (edges only,
in each object's own color — surfaces are not drawn), `--no-grid`, `--ortho`, `--direction x,y,z` (auto-framed view from
that direction; default isometric), or explicit `--eye x,y,z --target x,y,z
[--up x,y,z] [--fov deg | --ortho-height h]`.

Useful render recipes:

```
odm render                                   # framed isometric
odm render --direction 0,0,-1 --ortho        # top view (plan)
odm render --direction -1,0,0 --ortho        # side elevation
odm render --wireframe --width 1600          # inspect topology
odm render --t 2.5 --out /tmp/frame.png      # animation frame
```

## Workflow tips

- Prefer `tree`/`inspect`/`raycast` for measurements — they're exact.
  Renders are for overall shape/sanity; don't read dimensions off pixels.
- Node ids are child-index paths from the root (`""`, `0`, `0/2`); get them
  from `odm tree`.
- Build errors return as `{"ok": false, "error": {...}}` with the doohickey
  path, message, JS stack, and any `console.log` output. Fix and re-run —
  the engine picks up changes automatically.
- The viewer shows the last good build while your code is broken; the CLI
  always tells you the current truth.
- When the user clicks a part in the viewer, `odm selection` tells you which
  node they selected — useful for "make *this* one longer" instructions.
