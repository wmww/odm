# A purely numeric node name is unreachable by name

Node addressing (`scene::locate`) tries `is_index_path` first, so
`odm inspect '{"node": "12"}'` always means "child 12 of the root", never
the node someone called `.name(12)` (or `.name('12')` — `name()`
stringifies, so both land in the same place).

Not hypothetical: numbering parts is a natural habit, and the failure is
silent-ish — you get "no node at index path" or, worse, a *different*
node that happens to sit at that index.

Pinned around rather than fixed in
`tests/conformance/unstable/names.js`, which checks `name(123)` through
`raycast` instead.

Options: prefer a name match and fall back to the index path (a name is
explicit, an index is not); or require index paths to contain a `/` or a
leading marker. Either way the duplicate-name tiebreaker in
docs/cli.md "Addressing" needs updating.
