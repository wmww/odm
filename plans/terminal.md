# Embedded terminal tabs

Terminal emulator tabs in the viewer: a real PTY running the user's shell,
cwd = the project dir. Not agent-specific — the user can run `claude`,
`git`, anything — and nothing runs by default; a terminal exists only when
the user opens one. This is deliberately the generic substrate: a proper
agent UI is a possible later layer, not this plan.

Decisions from the 2026-08-17 design discussion:

- Emulation core is `alacritty_terminal` (the GUI-less state machine —
  alternate screen, colors, cursor addressing, scrollback, modes).
  Whether to adopt the `egui_term` widget on top of it or hand-roll the
  egui rendering layer is an **open question, decided in phase 1** —
  egui_term must survive two tests: our `egui =0.35.0` pin, and the theme
  (bitmap fonts, whole-pixel rules, custom scrollbars).
- Decoupled: new crate `odm-term` holding PTY + emulation behind a small
  API, no dependency on odm-engine or the theme. Ideally no egui
  dependency either — it exposes a grid snapshot (cells with
  fg/bg/flags) + input/resize/exit, and the viewer draws it with theme
  fonts; if egui_term is adopted the widget wrapper may pull egui into
  odm-term instead. Either way style (font, palette, cell metrics) is
  passed in, never imported.
- UI: a terminal is a **tab kind** in the existing strip; multiple
  terminal tabs allowed.

## odm-term

- PTY via `portable-pty`: spawn `$SHELL` (fallback `/bin/sh`) with cwd =
  project dir, resize (cols/rows), kill + reap. The odm CLI works
  immediately inside because `find_project` walks up from cwd.
- A reader thread drains the PTY into alacritty_terminal's parser/`Term`
  under a mutex and fires a wake callback on damage — same shape as the
  engine's existing wake hook, plain threads, no async.
- API sketch: `Terminal::new(cwd, cols, rows, wake)`, `resize`,
  `write(bytes)` (user input), `lock()` → grid + cursor + mode flags for
  rendering, `exited()` → Option<status>. Drop kills and reaps.
- Input encoding lives here too: a pure function from (key, modifiers,
  terminal modes) → bytes (arrows/Home/End/PgUp/PgDn/F-keys, Ctrl+letter,
  Alt prefix, Enter/Tab/Backspace/Esc, bracketed paste), so it's unit-
  testable without a UI.

## Viewer integration

- **Tab becomes an enum.** Today's `Tab` (odm-viewer-core) is strongly a
  doohickey view and `self.tabs[self.active]` assumes it everywhere.
  The enum is desktop chrome — it wraps the core `Tab`, which stays as is.
  Rename it `ViewTab`, introduce `Tab { View(ViewTab), Term(TermTab) }`,
  and route per-tab UI by kind. `TermTab` = terminal handle + label +
  scroll state; no engine slot — terminals never touch `EngineState`, the
  socket, or builds, so headless is untouched and project consistency
  invariants don't apply.
- Panels: with a terminal tab active, tree/inputs/timeline are view
  concepts — hide them. Menu bar, tab strip and the dock stay. CentralPanel = the terminal widget.
- The add-tab modal gets a Terminal entry; labels "Terminal", "Terminal
  2", … Tab close kills the PTY.
- Rendering (if hand-rolled): 7×14 cell grid in `odm-mono-14`, positions
  through `theme::snap`, background rect runs per row then text runs.
  Cursor is a solid block — the no-animation rule means no blink, which
  is period-correct anyway. ANSI 16 palette tuned to the dark-95 look;
  256/truecolor passed through. Scrollback scrolls via `theme::scroll`'s
  bar like every other pane.
- Keyboard: the widget takes egui focus on click and feeds
  `Event::Text`/`Event::Key` through the encoder. The `F` shortcut
  already gates on `egui_wants_keyboard_input()`, so a focused terminal
  is safe. Verify the chat input only acts on Enter/refocuses when its
  own field has focus (viewer/mod.rs:938-945) — it must not steal keys
  from a terminal tab.
- Repaint: wake callback → `ctx.request_repaint()`. Required — the
  viewer repaints on demand and `SlowIdle` clamps idle polling, so
  streaming output is invisible without it.
- Resize: widget rect → cols/rows; call `resize` only on change.
- Selection + copy/paste: alacritty_terminal has selection; Ctrl+Shift+C
  copies, Ctrl+Shift+V pastes (bracketed). Mouse-reporting forwarding
  (for TUI apps that want clicks) with shift-bypass for selection —
  phase 4, not core.

## Lifecycle & policy

- One PTY per terminal tab; killed on tab close and viewer exit. File ▸
  Open closes terminal tabs with the old session — their cwd is the old
  project.
- Shell exit: the tab stays, showing the final screen plus an "[exited]"
  line; user closes it. No auto-respawn.
- Persistence: `viewer.json` records that terminal tabs existed (a
  `kind` field on `SavedTab`, default `"view"` for back-compat) and
  restores them as *fresh* shells; scrollback is not persisted.
- Security: what runs is `$SHELL`, started only by user gesture. If a
  configurable command ever appears it is user-level config, never
  `odm.toml` — the engine must not execute a command a project file
  chose.

## Testing

- odm-term unit tests: byte streams in → grid out (our wiring — resize,
  exit detection, damage/wake), and the full input-encoding table.
- Widget: headless-egui tests where feasible (click-to-focus, rect →
  cols/rows, scrollback offset), per the inputs/tree pattern.
- Interactive verification via the gui-testing skill: type into the
  terminal, run a TUI app (vim/htop) as a smoke test, screenshot. Note
  wdotool's held-key quirks (notes/architecture.md "Seeing the viewer").

## Phases

1. **Spike + decide**: egui_term vs custom rendering. Stand up a minimal
   shell-in-a-window with each candidate; judge on the egui 0.35 pin,
   themability, and code size. Fixes where the widget layer lives.
2. **odm-term**: PTY + emulation + input encoding + tests.
3. **Viewer**: Tab enum refactor, terminal widget (render + keyboard +
   repaint + resize), add-tab entry, lifecycle (close/swap/exit),
   panel routing.
4. **Polish**: scrollback UI, selection/copy/paste, persistence,
   exited state, optional mouse reporting.

## Open questions

- egui_term vs custom rendering (phase 1 decides; lean custom if
  egui_term fights the theme or the version pin).
- Persist terminal tabs at all, or always start clean? (Lean persist —
  cheap, keeps the user's layout.)
- Mouse reporting scope — skip entirely if the target TUIs don't need
  it.
