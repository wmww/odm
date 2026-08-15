# CLI surface diet (cross-cutting cuts + prompt-vs-docs policy)

## Why
Every concept in the agent-facing surface costs tokens and attention. Audit
conclusion (2026-08): the command count is fine; the fat is in concepts and
output fields. Prompt is ~3K tokens once per session; one un-dieted `tree`
was 191 KB — so output shape ≫ concept count ≫ command count. The big items
have their own plans (`scene-query.md`, `build-report.md`); this one holds
the small cuts and the policy.

## Cuts
- **`sync`**: every command syncs first — a standalone sync command invites
  thinking about a concept the design deliberately hides. Fold into `status`.
- **`--view <slot>` + `--viewer-state`** → one flag: bare form = the user's
  active tab, argument form = a named slot. One concept: "target what the
  user sees."
- **`generation`**: internal consistency counter; drop from all responses
  except `status`.
- **Hashes**: nothing agent-visible prints content hashes (see also
  `build-report.md` `root`, `scene-query.md` mesh hashes).
- **Prompt demotions**: `raycast` (its "find a node id" role dies with name
  addressing; precise probing is advanced) and the explicit camera flags
  (see `render-workflow.md`) move to docs-only.

## Policy: prompt vs docs
The prompt teaches only the core loop — edit → `inspect` → `render` →
`poll`/`say`, plus `--set`/`--preset` and "parts have names". Everything else
lives in `odm docs`, paid for only when consulted. New features default to
docs-only; prompt space is the scarce resource. After basic work the concept
load should be: files are doohickeys; queries target a view; `--set` sets
inputs; parts have names; poll/say to talk. Cascade mechanics, memoization,
generations, color spaces, camera math, viewer slots, index paths — optional
depth.

Record the policy in notes once adopted, so future features follow it.

## Keep (audited, earns its place)
`poll`/`say`/`selection` (each did its job in the live test), the sync-first
invariant, errors-through-every-command, `docs` (its quality is what lets
everything else leave the prompt), `prompt`, `status`. Cuts are about
defaults and prompt space — full capability stays reachable via CLI + docs.
