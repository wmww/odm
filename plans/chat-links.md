# Links in the chat action log

The action lines the viewer writes for the agent's work (`Who::Action`)
name things the viewer can already show: a part file, a scene node.
Make those names links — click a file to open it in a tab, click a node
to select it — so the log is a way *into* the model, not just a receipt.

Reviewed against the code 2026-09-18.

## Relation to the agent panel (built 2026-09-18)

Independent; either order works. Action lines survive the ACP rewrite
(that plan's lean: keep the ODM action line, hide the duplicate ACP
tool-call line), and ACP does not tell us what node an `odm inspect`
addressed — only `dispatch` knows. Spans hang off the *action item*: today
that is `TranscriptEntry`, after phase 3 it is the action variant of the
new item type. Nothing here touches Delivery/poll/say. If this lands
first, phase 3 carries `spans` across; ~250 lines, so no reason to wait.

## What already exists

- `ViewerApp::add_tab(path)` / `switch_tab(index)` (viewer/mod.rs).
  `add_tab` always opens a new tab; there is no open-or-focus yet.
- `Viewer::set_selection(engine, tab, sel)` (odm-viewer-core viewer.rs)
  — sets the selection, `tree.reveal`s its ancestors, tells the engine.
- `tab.scene.root: TreeNode` (viewer-core tree.rs) — the tab's scene tree,
  already materialized with names; ids are index paths (`/1/0`, root `""`).
- Every project `.js` is a part (`generation_sources` is exactly
  them), so every `new`/`edit` path is a valid tab target when logged.

## The two links

| span | text | click |
| --- | --- | --- |
| file | the path in any action line | focus a tab on that path, else open one |
| node | a node the action addressed or hit | select it in that path's tab |

Deleted files (`deleted x.js`) and failure lines are plain.

Node links, by verb:

- `inspect` with a `node`: the response's `node.id` + `node.name`
  (already resolved by `scene::locate`). Bare inspect: no node span.
- `raycast`: each non-null hit's `id` + `name`.
- `clearance`: the pair addresses as the agent wrote them (the response
  echoes leaf labels in `between`, not the addressed nodes). `dispatch`
  must pull them from `&req` *before* `dispatch_inner` consumes it.
- `render` `focus`: **cut.** Per-frame focus in a contact sheet makes it
  fiddly, and a render's subject is the picture, not a node.

## Line text

The visible line changes — spec'd here since tests compare it:

```
inspect root.js                 bare: unchanged
inspect root.js wheel           node name, else its id (/1/0)
raycast root.js wheel, axle +3  first two distinct hits, then a count
raycast root.js                 every ray missed
clearance root.js seat, chain   first pair; "+N" for further pairs
edit part.js
inspect failed: …               unchanged, all plain
```

Failure lines get no links: today's text names no file, and the error
(bad path, failed build) is usually *about* the thing a link would open.

Activity-card captions (`push_activity`) stay as they are.

## Span model

```rust
pub enum Span {
    Text(String),
    File(String),
    Node { path: String, id: Option<String>, name: Option<String>, label: String },
}
```

- In state.rs. The action item gains `spans: Vec<Span>`; `text` stays
  and is always the spans concatenated (build both from one
  `fn line(spans) -> (String, Vec<Span>)`), so equality tests and any
  other reader keep using `text`. Non-action entries: `spans` empty.
- `Node.path` because the dock is shared by every tab.
- `id`/`name`: inspect and raycast know both; a clearance address is one
  or the other (leading `/` ⇒ id). At least one is always set.

## Resolving a node click

On click, never per frame (the transcript is uncapped and unculled).

Addresses go stale: the agent's next edit reorders children, and index
paths then point at a *different* node — a silent wrong selection is
worse than a miss. Names survive rebuilds. So, in viewer-core, against
`tab.scene.root` (no store, no `scene::locate`; unit-testable next to the
tree tests):

```
fn TreeNode::resolve(id: Option<&str>, name: Option<&str>) -> Option<(String, Option<String>)>
1. id given and a node is there: accept if `name` is None or equals that
   node's name.
2. else name given: accept iff exactly one node has it.
3. else miss.
```

Unnamed nodes (common for raycast hits) have only rule 1 and can go
stale undetected. Accepted: the worst case selects a neighbouring
unnamed mesh, visibly.

Which tab: the active tab if its path matches, else the first tab with
that path, else a new tab. **Inputs are not matched** — the agent may
have inspected under other inputs/preset than the tab shows; rules 1–2
are the guard, and `"view": true` queries (the common case when the user
asked about what they're looking at) match exactly. Carrying the full
view and opening a tab per input set was rejected: tab litter.

A tab with a scene resolves on the click. A new or not-yet-built tab has
none on that frame (builds are async), so the click parks
`Tab.pending_select: Option<(id, name)>`, consumed where `tab.scene` is
first assigned (viewer.rs, after `flatten_node`). Cleared by any user
selection in that tab, so a late build never yanks the selection back.
Not persisted in viewer.json.

Hit → `Viewer::set_selection` with `[(id, name)]`. Miss → silent no-op.
No camera move (F frames the selection; a teleporting link is harder to
undo). Surfacing a miss wants a message line the viewer lacks; not here.

## Resolving a file click

Focus per "which tab" above, else `add_tab`. If the path is no longer in
the current snapshot (`sync.snapshot.sources`, as `open_picker` reads
it), no-op — don't open a tab onto a file that is gone.

## Look: links, not buttons

- The line keeps its speaker colour (`ACTION_TEXT`, grey); the link is
  the same colour **underlined**. Colour stays what it means — who spoke.
  (`hyperlink_color` blue is set in the theme and unused; a blue word in
  a grey line reads as a different speaker.)
- Hover: `CursorIcon::PointingHand`, no recolour — a cursor change is the
  pointer's business, so the no-hover-feedback rule stands.
- `theme::link(ui, text, color) -> Response`:
  `Label::new(..).sense(Sense::click()).selectable(false).layout_in_ui(ui)`
  (public in egui 0.35), paint the galley, then one hand-drawn 1px line
  **per galley row** (a long path can wrap), endpoints through
  `theme::snap`. egui's own `TextFormat::underline` is a feathered stroke
  off the pixel grid; try it first and keep the hand-drawn one only if it
  looks wrong against the bitmap font.

## Viewer wiring

`chat_ui` draws inside `state.with_transcript(|…|)` (holds the chat
lock) — collect a local `Option<Link>` and act after the closure, the
way `browse.rs` hands back a `Hit`.

Pull the transcript body out of `chat_ui` into a free
`transcript_ui(ui, entries, task) -> Option<Link>`: `ViewerApp` needs a
wgpu renderer and an `eframe::Frame`, so it cannot be built in a test,
and this function can.

Action entries with a link span draw as `horizontal_wrapped` with
`item_spacing.x = 0` (egui's own inline-link idiom); everything else
keeps the single label.

## Steps

1. `Span`, `spans` on the action entry, `log_action(spans)`;
   `file_edits` emits them. Existing tests unchanged.
2. `action_spans(verb, nodes_from_req, &out)` replacing `action_line`;
   unit-test per verb: bare/named inspect, a failure (plain), raycast with 0/1/4
   hits, clearance with two pairs. Update the line-text assertions.
3. `TreeNode::resolve` + `Tab.pending_select` in viewer-core, with tests:
   id hit, id stale → name rescue, ambiguous name → miss, unnamed stale
   (documents the accepted case), pending consumed on scene arrival,
   cleared by a user click.
4. `theme::link`.
5. `transcript_ui`, `Link` handling, `open_or_focus_tab(path) -> index`.
6. Headless-`egui::Context` test of `transcript_ui` (tree.rs harness
   style): click the painted file span → `Some(Link::File)`; node span →
   `Some(Link::Node)`. One gui-testing pass for the look and for a click
   on a not-yet-open file.

## Out of scope

- Linking a `render` line back to its activity card (the queue keeps 8;
  replaying evicted ones means holding pixels for the whole log).
- Links in agent or user prose, and in ACP tool-call lines/diffs once
  the agent panel exists.
- Pre-resolving links to grey out dead ones.
