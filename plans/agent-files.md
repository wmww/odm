# Auto-maintained AGENTS.md / CLAUDE.md

Agents currently have to run `odm prompt` to get the ODM instructions. Instead,
the engine should put the prompt into the project's agent files and keep it
current, so any agent that reads AGENTS.md/CLAUDE.md just has it.

Decided with the user 2026-08-14 (three follow-ups answered):
- **Mixed setups ask per file** — every existing agent file without markers gets
  its own question, even when another file already carries the prompt.
- **No persistence of "No"** — a declined question reappears on the next open.
  Nothing is recorded anywhere.
- **Headless updates silently, never asks** — questions are viewer-only.

## The marked block

The prompt lives between exact marker lines (three dashes, as specified):

```
<!--- BEGIN STANDARD ODM PROMPT --->
...output of `odm prompt`...
<!--- END STANDARD ODM PROMPT --->
```

Everything outside the markers is the user's and is never touched. Updates
locate the BEGIN and END lines (trimmed-line equality) and replace only what is
between them, and only when the replacement actually differs (byte-compare
first; no rewrite, no mtime churn when current).

Markers are the opt-in: a file containing a well-formed pair auto-updates on
every open, no questions asked. BEGIN without END (or END before BEGIN) is
malformed: leave the file alone, `eprintln!` a warning, ask nothing — never
risk clobbering user text over a guess.

## Behavior

**On project create** (`create_project`, used by File ▸ New Project):
- `AGENTS.md` — just the marked block, nothing else.
- `CLAUDE.md` — relative symlink to `AGENTS.md` (`std::os::unix::fs::symlink`;
  on non-unix fall back to a second regular file with the block — it will
  auto-update independently).
- Same never-overwrite discipline as the rest of `create_project` (`write_new`;
  the symlink equivalent errors on an existing path).

**On project open** (viewer Open/startup and `odm run --headless`):
1. Consider `AGENTS.md` and `CLAUDE.md` at the project root. Resolve symlinks
   and dedupe by canonical path — the default symlink pair is one logical file,
   updated once, asked about once (named as AGENTS.md).
2. Each logical file with a well-formed marker pair: splice-update. This
   happens in both viewer and headless opens.
3. Each existing file *without* markers: a viewer question box per file —
   "Add the standard ODM instructions to CLAUDE.md? They will be kept up to
   date automatically." Yes → append `\n\n` + the marked block at EOF.
4. No agent file exists at all: one question box — "Create AGENTS.md with the
   standard ODM instructions? (CLAUDE.md is created as a symlink to it.)"
   Yes → same authoring as project create.
5. Headless does step 2 only; 3–4 need a UI and are skipped.

**Quiet handling** (assume the user set it up on purpose; never restructure):
- Only one of the two files exists → work with it; never offer to create the
  missing one (only the nothing-at-all case offers creation).
- CLAUDE.md is a regular file, or symlinks somewhere unexpected → fine; each
  logical file is classified independently.
- Symlink target outside the project: markers there still mean opt-in →
  update through it. Questions (append/create) stay restricted to
  project-root files, so we never *add* content outside the project.
- Dangling symlink → counts as "exists" (so no create offer) but is skipped
  quietly — no update, no question.
- File unreadable / is a directory → warn on stderr, skip.

## Implementation

### New crate: `crates/odm-prompt`

std-only, no odm deps — both odm-cli (which must stay V8-free) and
odm-build/odm-engine use it.

- Move `odm-cli/src/prompt.rs` here: the `include_str!` macro over
  `docs/prompts/`, `PROMPTS`, `text()`. Adjust the relative path in the macro.
- `BEGIN`/`END` marker constants; `block()` → markers wrapping `text()`.
- Pure splice: `fn splice(contents: &str) -> Splice` where
  `Splice = { Updated(String), Current, NoMarkers, Malformed }`. All the
  line-finding/replacement logic lives here, unit-testable without fs.
- Fs layer:
  - `fn sync(project: &Path) -> SyncReport` — classify + update marked files,
    per the rules above. `SyncReport { updated: Vec<String>, unmarked:
    Vec<String>, none_exist: bool, warnings: Vec<String> }` (names are what
    the dialogs display; symlink pair deduped).
  - `fn append(project: &Path, file: &str) -> Result<(), String>` — the Yes
    answer to an "add to existing file" question. Re-checks for markers
    before appending (the file may have changed since scan).
  - `fn create(project: &Path) -> Result<(), String>` — AGENTS.md + CLAUDE.md
    symlink; used by both the Yes-to-create answer and `create_project`.

### odm-cli

`prompt.rs` deletes down to a re-export / call of `odm_prompt::text()`.
Add `odm-prompt` to its deps. `odm prompt` stays — still useful for pasting
into other contexts.

### odm-build

- Dep on `odm-prompt`. `create_project` additionally calls
  `odm_prompt::create` after writing `odm.toml`/`root.js`.
- Update `crates/odm-build/tests/new_project.rs`: expect AGENTS.md with
  markers and CLAUDE.md as a symlink to it.

### odm-engine

- Extract an on-open helper (in `session.rs`) doing `sync_marker` **and**
  `odm_prompt::sync`, returning the `SyncReport`. Call it from
  `Sessions::open` *and* `run_headless` — note this also fixes the existing
  gap where headless never ran `sync_marker`.
- Plumb questions to the viewer: `Sessions::open` stashes the report's
  question list on `EngineState` (e.g. `pending_agent_questions:
  Mutex<Vec<Question>>`); the viewer drains it after a successful open —
  including the startup-opened project — and shows dialogs one at a time.
  `Question = AddTo(String) | CreateFiles`.
- New `viewer/agent.rs`: a small yes/no dialog via `theme::dialog` (like
  open.rs/new.rs but no browser — text + "Add"/"Not now" buttons, Enter =
  yes, Esc = dismissed = no). New `Dialog::AgentFiles` variant in
  `viewer/mod.rs`; on Yes call `odm_prompt::append`/`create`, report errors
  in the dialog like the other dialogs do; then pop the next question if any.
- Questions are transient: dismissing or answering No just drops the entry
  (re-asked next open, per the decision above).

### Docs & notes

- `CLAUDE.md` (repo) + `notes/architecture.md:497`: the "engine never writes
  project files, ONE exception" invariant becomes: exceptions are `odm.toml`
  engine version and agent files — and agent-file writes are marker-scoped
  (inside an existing marker pair) or user-consented (question box).
- `docs/prompts/introduction.md`: no change needed (it doesn't tell agents to
  run `odm prompt`).

## Tests

- `odm-prompt` unit: splice updates a stale block; is a no-op (`Current`) on a
  fresh one; preserves text before/after markers byte-exactly; `NoMarkers`;
  `Malformed` for lone/reordered markers; markers on lines with surrounding
  whitespace still match.
- `odm-prompt` fs (tempdir): sync dedupes the symlink pair; dangling symlink →
  skipped but `none_exist == false`; no files → `none_exist == true`; append
  puts the block at EOF after a blank line; create authors file + symlink.
- `odm-build`: new_project test additions above.
- Engine open paths: headless sync covered by an odm-engine test if there's a
  cheap seam; the viewer dialog is checked manually with the gui-testing
  skill (screenshot the question box; Yes and No paths).

## Order of work

1. `odm-prompt` crate + unit tests (pure splice first, then fs).
2. odm-cli re-export (behavior unchanged — `odm prompt` output identical).
3. `create_project` + test.
4. Engine on-open sync (headless + viewer), no UI yet.
5. Viewer dialogs + gui-testing pass.
6. Docs/notes/invariant updates.
