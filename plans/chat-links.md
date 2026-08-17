# Links in the chat action log

The action lines the viewer writes for the agent's work (`Who::Action`,
2026-08-17) name things the viewer can already show: a doohickey file, a
scene node. Make those names links — click a file to open it in a tab,
click a node to select it — so the log is a way *into* the model, not
just a receipt.

## What already exists

- `ViewerApp::add_tab(path)` (viewer/mod.rs) — the + picker's handler.
- `Viewer::set_selection(engine, tab, sel)` (odm-viewer-core viewer.rs)
  — sets the selection, `tree.reveal`s its ancestors, tells the engine.
- `scene::locate(store, root, addr)` (odm-engine scene.rs) — name *or*
  index path → the id the selection uses; the error lists candidates.
  Private module, but the viewer is in the same crate.

So both click handlers are one call each. The work is all in getting a
clickable *name* out of a log line and knowing which view to aim it at.

## The two links

| span | text | click |
| --- | --- | --- |
| file | `root.js` in `inspect root.js`, `edit part.js` | open (or focus) a tab on that path |
| node | the node an action addressed | select it in that path's tab |

Deleted files (`deleted x.js`) are not links. Nothing else is.

## Span model

`TranscriptEntry.text: String` becomes, for actions only, a list:

```rust
enum Span { Text(String), File(String), Node { path: String, addr: String } }
```

- Lives in state.rs next to `TranscriptEntry`. Viewer-side only: action
  entries are `Delivery::Done` from birth and never polled, so nothing
  on the wire changes.
- Keep a `fn text(&self) -> String` that concatenates, so existing
  assertions (`commands.rs` tests compare against `"inspect root.js"`)
  and any non-viewer reader are untouched. Simplest shape: entries keep
  `text` and gain `spans: Vec<Span>`, built together; the string stays
  the source of truth for equality, the spans for painting.
- A node span carries its own `path` because the dock is shared by every
  tab — the click has to know which view the address was resolved
  against.

## Producing spans

`file_edits` (state.rs) is trivial: `new`/`edit` → verb text + file span,
`deleted` → plain text, the over-cap "N files changed" line plain.

`action_line` (commands.rs) becomes `action_spans(verb, &out)` and reads
what the response already resolved:

- `view` → the file span (as today).
- `inspect`: `node.id` — `cmd_inspect` already ran `scene::locate` and
  the response's node object carries the resolved index path.
- `raycast`: each hit's `id`. One span per hit, so a multi-ray call is
  several links; cap the line (the log line is a glance, not a report) —
  first hit plus `+N` past two.
- `render` (`focus`) and `clearance` (`pairs`) are not echoed resolved,
  so take the address as the agent wrote it from `dispatch`, which
  already destructures `&req` for the verb.

Both kinds go in the same `addr` field: an index path *is* a valid
address, so the click path is one `locate` call either way and nothing
has to know which sort it holds.

Failure lines (`verb failed: …`) keep the file span if the response
resolved a view, and get no node span.

## Resolving a node click

An address is only meaningful against a build, and the log outlives
builds. Resolution happens **on click**, never per frame — the transcript
is uncapped and egui's scroll area does not cull children, so a
per-frame walk of every line's tree would be real work for a decoration.

1. Find the tab whose path matches the span's `path`; if none, open one
   (same as the file link). Make it active.
2. `scene::locate` against that tab's published root.
3. Hit → `Viewer::set_selection` with `[(id, name)]`. Miss → no-op.

Selection only: no camera move. F frames the selection and the user may
well be looking at something they want kept; a link that teleports the
camera is harder to undo than one that doesn't. Easy to add later if it
feels short.

A miss is silent for now. The alternative — surfacing `locate`'s
candidate list — wants a message line the viewer doesn't have; if silence
turns out to be confusing, the status band is the place, as an
`Option<String>` field cleared by the next click. Not in this plan.

## Look: links, not buttons

Bevelled buttons in a transcript would be wrong for both the era and the
density. Inline hypertext, as the era's help viewers did it:

- The line keeps its speaker colour (`ACTION_TEXT` green); the link is
  the same colour **underlined**. Colour stays what it means — who spoke
  — and the underline says what's clickable. (egui's `hyperlink_color`
  blue is set in the theme and unused; rejected here, since a blue word
  inside a green line reads as a different speaker.)
- Hover: `CursorIcon::PointingHand`, no recolour. The theme's
  no-hover-feedback rule is about widgets lighting up; a cursor change is
  the pointer's business, so the rule stands unamended.
- Underline drawn by hand (`Painter::hline` on the galley rect, snapped
  to the pixel grid) — the bitmap font sits on a pixel grid and egui's
  underline is a feathered stroke.

A `theme::link(ui, text, color) -> Response` wrapper, so viewer code
never hand-rolls it.

## Viewer wiring

`chat_ui` draws inside `state.with_transcript(|…|)`, which borrows
`self` — so clicks cannot act in place. Collect into a local
`Option<Link>` and act after the closure, the way `browse.rs` hands back
a `Hit` rather than navigating mid-layout.

Each entry becomes a `horizontal_wrapped` with `item_spacing.x = 0`
instead of one label. Non-action entries keep the single label — no
reason to pay layout for them.

## Steps

1. `Span` + `spans` on `TranscriptEntry`; `file_edits` emits them.
   Existing tests unchanged.
2. `action_spans` in commands.rs, replacing `action_line`; unit-test the
   span split per verb (inspect with/without a node, a failure, raycast
   with two hits).
3. `theme::link`.
4. `chat_ui`: wrapped span layout, click collection, `Link` handling —
   `open_or_focus_tab(path)` shared by both link kinds.
5. Headless-`egui::Context` test in the inputs.rs/tree.rs style: click a
   painted file link, assert a tab opened; click a node link, assert the
   selection.

Roughly 250 lines across state.rs, commands.rs, viewer/mod.rs and the
theme.

## Out of scope

- Linking a `render` line back to its activity card (the card queue
  keeps the last one; replaying an evicted one would mean holding
  pixels for the whole log). Obvious next link if the two present ones
  earn their keep.
- Links in agent (`odm say`) or user messages — that's parsing prose,
  and the agent can already be told to reference things.
- Pre-resolving links to grey out dead ones (see above).
