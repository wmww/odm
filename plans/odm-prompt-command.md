# `odm prompt` comes back; agent files carry one pointer line

Today the whole standard prompt sits between markers in a project's
`AGENTS.md`/`CLAUDE.md`, and the engine splices in the current text each
time it opens a project. Instead:

- `odm prompt` prints the prompt again. It is generated when you run it,
  so it can change with the situation.
- The agent files only carry one plain line, with no markers, saying to
  run `odm prompt`.
- The agent ODM runs itself gets the prompt from us directly, plus a note
  saying it doesn't need to run `odm prompt`.

Why: once the prompt is generated rather than a frozen copy in a file, it
can depend on whether ODM launched the agent, which agent it is, the
project (units, …) and the system (OS, sandbox). A file that is shared by
every agent and checked into git can't do that. The file also stops being
266 lines of churn in the user's repo, with a diff every time the prompt
changes.

## The pointer line

- One exact line, kept in `odm-prompt` as `LINE`, plus `OLD_LINES: &[&str]`
  holding every earlier version (empty to start). Draft wording:
  `This is an ODM project. Before working on it, run \`odm prompt\` for how
  to use ODM.` It must be imperative: an agent that skips the command
  knows nothing about ODM.
- Matching is on the trimmed line, as with the markers now. In a file:
  - `LINE` is present: nothing to do (no write, no mtime change).
  - A line from `OLD_LINES` is present: replace that line with `LINE`,
    keeping the file's own line ending.
  - Neither, but some line mentions `odm prompt`: the user wrote their own
    version. Leave the file alone and don't ask. Otherwise a user who edits
    the line gets asked again on every open.
  - None of the above: the file is unmarked. Offer the line (viewer
    question) or warn (`odm init`), as now.
- Having the line, in any version, is the file's opt-in to updates, the
  same role the markers play now. Any file without it still needs the
  user's yes. The engine's write exceptions stay the same in kind; the
  CLAUDE.md wording needs updating.
- `append` adds the line the same way the block is added now: blank-line
  gap, file's own line endings.
- `create` (new project): `AGENTS.md` holds just the line, and `CLAUDE.md`
  is a symlink to it, as now.

## Migrating marked files

The markers become legacy: keep `BEGIN`/`END` only to recognise them. On
open, a well-formed pair is replaced, markers included, by `LINE`. That
file already agreed to have the block managed, so this is no new
permission. Everything outside the pair is kept byte-for-byte. A lone
marker still gets a warning and is left alone. Drop this migration once
nothing old enough to have written a block is around (note it in
`plans/release.md` if that is still before 1.0).

## `odm prompt`

- Client-side and doesn't need an engine (odm-cli stays V8-free), the way
  `docs` is now. Remove the `bail!` redirect in `odm-cli/src/lib.rs` and
  the `"prompt"` entry in `requests.rs::removed`.
- The `prompt` docs topic goes. The prompt now changes with the situation,
  so it doesn't count as docs, and two ways in is one too many. `odm docs
  prompt` gets a redirect to `odm prompt` for a while.
- Remove the "No `prompt` command" standing cut from
  `notes/agent-surface.md` and write down why it came back.

## A prompt that depends on the situation

- `odm_prompt::text(&Context)` replaces `text()`. `Context` starts small:
  `managed: bool` (ODM started the agent). Variants are still built by
  including or excluding sections of `PROMPTS`, with no templating (the
  existing comment on `PROMPTS` already expects this). The `Context` field
  is the only seam we build now. Possible later fields: agent id (for
  example, Codex's sandbox and the mailbox), project units, OS, headless
  vs viewer.
- `odm prompt` builds its `Context` from what it can see without an
  engine: cwd and env. Reading `odm.toml` would mean giving odm-cli a TOML
  dependency, so wait until a variant needs it.

## Giving the managed agent the prompt

- In `agent/mod.rs`, the first prompt of each session, whichever path
  sends it (`send`, or an engine push after `restart`), gets the prompt
  first, built with `managed: true`. The `managed` text tells the agent
  it already has the instructions and doesn't need to run `odm prompt`,
  even though the agent file says to. Sessions are never resumed (`resume:
  None`), so "first prompt of a session" is a flag on `Live` and nothing
  more.
- How it is delivered, best first:
  1. Where the adapter supports it, the system prompt through session
     `_meta` (`table.rs::session_meta`). Claude: check whether
     claude-agent-acp accepts `_meta.systemPrompt` (or an append form) at
     `session/new`. A system prompt survives compaction; a first message
     may not.
  2. Otherwise, a `resource` block (`odm://prompt`, markdown) on the first
     prompt. Embedded resources already work on Claude and Codex
     (`odm://user-state`). Don't show it in the panel transcript.
- Spike (1) before building it. If neither adapter supports it, ship (2)
  on its own.

## Touch list

- `crates/odm-prompt`: `LINE`/`OLD_LINES`, line-level splice that also
  migrates a marker block, `text(&Context)`, rewritten tests (current /
  old line / own variant / unmarked / block migration / CRLF / lone
  marker).
- `crates/odm-engine`: `session.rs` docs, `viewer/agent.rs` question text,
  first-prompt priming plus a fake-adapter test in `agent/tests.rs`.
- `crates/odm-cli`: the `prompt` command, docs topic removal and redirect.
- `crates/odm` `init`: warning text (the flag `--append-prompt` can keep
  its name).
- `crates/odm-build/tests/suite/new_project.rs`: assert `LINE`, not the
  markers.
- Docs: `docs/README.md`, `docs/cli.md` (`odm docs prompt` → `odm prompt`),
  `prompts/cli.md` if it mentions it.
- Notes: architecture.md "Agent files", agent-surface.md. Also the
  Engine paragraph in this repo's `CLAUDE.md`/`AGENTS.md` ("standard prompt
  is kept current inside its marker pair").

## Open questions

- Line wording. It will be in user repos indefinitely, and every change
  to it adds an entry to `OLD_LINES`, so settle it once.
- Should `odm init`'s JSON output also say to run `odm prompt`? An agent
  that runs `init` itself has no other way to find out in that turn.
