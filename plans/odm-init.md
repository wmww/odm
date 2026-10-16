# `odm init`: create a project from the CLI

Today only File ▸ New Project creates a project. An agent run outside ODM
(autonomous, or the user's own terminal harness) has no way to start one
short of hand-writing `odm.toml` and pasting `odm docs prompt` into an
agent file. This is the first piece of making external agents a
supported use case again (design-decisions.md "The agent is managed"
says they aren't; this plan doesn't flip that, but nothing here assumes
the managed panel).

## Command

```
odm init [<dir>] [--name <name>] [--units mm|m|in|ft]
```

- `<dir>`: defaults to cwd, like `git init`. Created (with parents) if
  missing. If it already exists it is adopted, even when it isn't
  empty: an existing `root.js` stays as the root, and existing agent
  files are left alone (see below).
- `--name`: the `name` in `odm.toml`. Defaults to the dir's leaf name
  (after canonicalizing, so `odm init .` works). Fails if there is no
  leaf (`/`).
- `--units`: defaults to `mm`, the same default as the dialog.
- Flags, not a JSON body. Like `run` and `export --web`, this is a
  binary-level, engineless command, and the one-JSON-grammar rule
  (agent-surface.md) covers engine requests. Add `init` to that note's
  exception list.
- It does not start an engine. The response names the next step.

Refusals (exit 1, one line each):
- `<dir>` is already a project (`odm.toml` present): "already an ODM
  project — `odm run <dir>`".
- `<dir>` or any ancestor is a project: "a project cannot live inside
  another project (<ancestor>)". The dialog only checks the immediate
  parent; init walks all the way up, since every `.js` under a project
  belongs to it.
- `<dir>` exists and is not a directory.

## Output

One JSON object, matching what the client commands print, since agents
are the main callers:

```json
{"project": "/abs/dir", "name": "widget", "units": "mm",
 "created": ["odm.toml", "root.js", "AGENTS.md", "CLAUDE.md"],
 "warnings": ["AGENTS.md exists without the ODM prompt block: …"],
 "next": "odm run /abs/dir --headless"}
```

`created` lists only what was written. `warnings` is omitted when empty.
`next` is a plain string, so a human sees the step too. Open question:
also suggest plain `odm run` for the viewer? Lean no, since headless is
the documented agent default and a human reading this already knows
about the viewer.

## Existing agent files

`odm_prompt::create` writes nothing if `AGENTS.md` or `CLAUDE.md`
already exists, leaving the decision to the viewer's open-time question.
Headless never asks, so an init that only runs headless would silently
end up with no prompt anywhere. That matters here: an autonomous agent
initializing inside its own repo is likely to have a `CLAUDE.md` already.

Plan: the default stays write-nothing, plus a warning per unmarked file
that says how to opt in. A new flag `--agent-files` makes init append
the marked block to existing unmarked agent files, through the same
splice the viewer's "append" answer uses (`odm_prompt`'s append path).
Typing the flag is the user's yes. The engine-never-writes rule is about
the engine on open, and this is an explicit authoring command, the same
standing as New Project. Naming: `--agent-files` vs `--append-prompt`.
Pick one while implementing; lean `--append-prompt`, which says what it
does.

The symlink/quiet cases (a lone `CLAUDE.md`, a symlink out of the
project, a dangling link) get the same treatment `sync` gives them on
open: warn and leave them. No new rules.

## Implementation

- **Shared validation.** Move the checks that apply regardless of
  surface out of `viewer/new.rs::resolve` into `odm_build` (for example
  `create_project` itself, or a `check_new_project(dir)` it calls). That
  covers already-a-project and nested in any ancestor. The dialog keeps
  its UI-only rules (a name is one path component, with no leading
  `.`). Both the dialog and init then go through one
  `odm_build::create_project`, so they can't drift. The dialog gains the
  full-ancestor nested check for free.
- **Report what was written.** `create_project` returns which files it
  wrote and which it kept, instead of `()`. `odm_prompt::create`
  likewise reports the unmarked files it skipped, using `sync`'s scan,
  which already classifies them.
- **Wiring.** `init` lives in `crates/odm/src/main.rs` beside
  `run_engine` and `export_site`. `odm` already links odm-engine, so it
  can use `odm_build` directly (add the dep). odm-cli stays V8-free and
  untouched apart from the hints below. Parse `--units` with a `FromStr`
  on `Units` (add it; the wire/toml spellings are `as_str`).
- **Discoverability.** Agents find commands through errors and
  `--help`, not the prompt (they have no project prompt yet):
  - `usage()` gets a line: `odm init [<dir>] [--name …] [--units …]
    create a project`.
  - `odm_cli::project_dir`'s "not an ODM project: no odm.toml here" and
    the walk-up failure both append "— create one with `odm init`".
  - `odm run --headless` in a non-project gets the same hint, since it
    shares `project_dir`.
  - `docs/cli.md`: a short "Starting a project" section near the top,
    next to "Start an engine with…".
  - Not in the agent prompt. The prompt only exists once a project
    does, so it can't help an agent that needs init.

## Tests

- odm-build unit tests: ancestor-nested refusal, already-a-project
  refusal, adopting a dir with an existing `root.js`, the
  created/kept report.
- odm-prompt: `--append-prompt` against an unmarked `AGENTS.md`, the
  symlink pair (appended once), and a regular `CLAUDE.md`.
- e2e (`crates/odm/tests/e2e.rs`): `odm init <tmp>/new` then `odm run
  --headless` then `odm status` answers, with the starter's `block` node
  in `inspect`. Also `odm init` inside a project → exit 1.
- The existing `viewer/new.rs` tests keep passing after the move, and
  the ones covering moved checks move with them.

## Docs and notes when done

- CLAUDE.md's Architecture/Engine paragraph: "(File ▸ New Project
  authors a new project's first files.)" becomes "File ▸ New Project /
  `odm init`".
- architecture.md "Agent files" and the project-format notes: init is
  the second creator; `--append-prompt` is the CLI's yes.
- agent-surface.md: `init` joins `docs` as a grammar exception.
- Delete this plan.

## Not in scope

The rest of the external-agent gaps: the prompt's panel-only
"Talking with the user" section, pulling diagnostics and user-state
without the panel, and any chat channel. Those belong in their own plan
once the policy is flipped.
