//! The managed agent: the process ODM spawns and talks to over ACP
//! (`odm-agent`), and the transcript the viewer's Agent panel draws.
//!
//! ACP carries only the conversation. The agent still drives ODM through the
//! CLI from its own shell tool, cwd = the project — no MCP server, no client
//! fs/terminal capabilities.
//!
//! - The user always picks the agent (`odm-config`); unconfigured, nothing
//!   spawns. It is spawned lazily, by the first message, and never headless.
//! - A turn ends with the `session/prompt` response, full stop: working /
//!   not working is the ACP turn state, so it cannot go stale.
//! - Build diagnostics are *pushed* as engine-authored prompts (see
//!   [`AgentHost::run_pusher`]) — each one a paid turn nobody typed, hence
//!   the bounds there.

pub mod table;

pub use odm_agent::{ConfigOption, PermissionOption, PlanEntry, ToolCall, ToolContent};

use crate::state::{DiagnosticMap, EngineState};
use odm_agent::{Agent, Event, ExitReason, Launch, SessionOptions, Update};
use odm_config::{AgentState, Config, Files};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How an engine-authored prompt starts, so it replays as an `engine:` line
/// rather than as something the user said.
const ENGINE_TAG: &str = "[odm engine]";

/// Engine prompts in a row, with no user message between, before the engine
/// stops forwarding: bounds a fix-fail loop nobody is watching.
const ENGINE_PROMPT_CAP: u32 = 3;

/// How long the diagnostic value must sit unchanged (with no build in
/// flight) before it is pushed — dragging a slider through a failing range
/// is one prompt at most, and none if it ends ok.
const QUIET: Duration = Duration::from_secs(2);

/// The session header: the first item of each session's transcript.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    /// None = no agent selected.
    pub agent: Option<String>,
    pub version: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub account: Option<String>,
    pub cwd: String,
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolItem {
    pub call: ToolCall,
    /// Part of a loaded session's history.
    replayed: bool,
    /// CLI action lines logged before it started / whether any came while
    /// it ran: an `odm …` call that reached the engine has an action line
    /// standing in for it.
    actions_before: u64,
    reached_engine: bool,
}

impl ToolItem {
    pub fn running(&self) -> bool {
        matches!(self.call.status.as_deref(), None | Some("pending" | "in_progress"))
    }

    pub fn failed(&self) -> bool {
        self.call.status.as_deref() == Some("failed")
    }

    /// An `odm render` by the managed agent shows up twice: as this, and as
    /// the engine's own action line. The action line wins (it is semantic,
    /// and feeds the activity view) — unless the command failed without
    /// ever reaching the engine, or comes from history, where no action
    /// lines survive.
    pub fn visible(&self) -> bool {
        let odm = self.call.kind.as_deref() == Some("execute")
            && self.call.title.as_deref().is_some_and(|t| t == "odm" || t.starts_with("odm "));
        !odm || self.replayed || (self.failed() && !self.reached_engine)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Header(Header),
    User { id: Option<String>, text: String },
    Agent { id: Option<String>, text: String },
    Thought { id: Option<String>, text: String },
    Tool(ToolItem),
    Plan(Vec<PlanEntry>),
    /// A question from the agent. `answer` = the chosen option's name, or
    /// "cancelled"; answered once, then a log line.
    Permission { id: u64, tool: ToolCall, options: Vec<PermissionOption>, answer: Option<String> },
    /// A CLI command the agent ran, a file it changed.
    Action(String),
    /// A host warning, or a note that the engine prompted the agent itself.
    Engine { id: Option<String>, text: String },
    /// Session events worth a line: resumed, stopped, …
    Notice(String),
    /// The agent crashed, could not start, is logged out.
    Error(String),
}

/// The lamp on the Agent tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    /// Not configured, or not running.
    Off,
    /// A live session, nothing in hand.
    Idle,
    Working,
    /// A turn blocked on a permission question — which otherwise looks
    /// exactly like a busy one.
    Waiting,
}

struct Live {
    agent: Agent,
    id: String,
    serial: u64,
    /// The session exists: prompts go straight through.
    ready: bool,
    login: Option<String>,
}

struct Inner {
    config: Config,
    memo: AgentState,
    items: Vec<Item>,
    /// A loading session's history, spliced in at `replay_at` once it is
    /// whole — before whatever the user typed to wake the agent up.
    replayed: Vec<Item>,
    replay_at: usize,
    live: Option<Live>,
    serial: u64,
    turn: bool,
    options: Vec<ConfigOption>,
    usage: Option<(u64, u64)>,
    actions: u64,
    /// The diagnostic value the agent was last told.
    baseline: DiagnosticMap,
    engine_prompts: u32,
    muted: bool,
    /// Host warnings not yet forwarded.
    warnings: Vec<String>,
    /// Bumped by every poke, so the pusher cannot sleep through one that
    /// landed while it was looking at the engine.
    pokes: u64,
}

pub struct AgentHost {
    project: PathBuf,
    files: Files,
    inner: Mutex<Inner>,
    /// Pokes the pusher: the diagnostic value, the turn state or the
    /// warnings changed.
    cv: Condvar,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// [`QUIET`], shortened by tests.
    quiet: Duration,
}

impl AgentHost {
    pub(crate) fn new(project: &Path) -> Arc<AgentHost> {
        Self::with_files(project, Files::for_project(project))
    }

    /// A host that reads no system config: for tests, which must never see
    /// (let alone spawn) the agent the user has configured.
    #[cfg(test)]
    pub(crate) fn detached(project: &Path) -> Arc<AgentHost> {
        let files = Files { system: None, project: odm_config::project_path(project) };
        Self::with_files(project, files)
    }

    pub(crate) fn with_files(project: &Path, files: Files) -> Arc<AgentHost> {
        Self::build(project, files, QUIET)
    }

    fn build(project: &Path, files: Files, quiet: Duration) -> Arc<AgentHost> {
        let config = odm_config::load(&files);
        let mut inner = Inner {
            memo: AgentState::load(project),
            items: Vec::new(),
            replayed: Vec::new(),
            replay_at: 0,
            live: None,
            serial: 0,
            turn: false,
            options: Vec::new(),
            usage: None,
            actions: 0,
            baseline: DiagnosticMap::new(),
            engine_prompts: 0,
            muted: false,
            warnings: Vec::new(),
            pokes: 0,
            config,
        };
        let header = inner.cached_header(project);
        inner.items.push(Item::Header(header));
        for warning in inner.config.warnings.clone() {
            inner.items.push(Item::Engine { id: None, text: warning });
        }
        Arc::new(AgentHost {
            project: project.to_owned(),
            files,
            inner: Mutex::new(inner),
            cv: Condvar::new(),
            wake: Mutex::new(None),
            quiet,
        })
    }

    pub(crate) fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock().unwrap() = Some(wake);
    }

    fn wake(&self) {
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    // --- what the panel reads ---

    /// Read the transcript. `f` runs under the host's lock: keep it to
    /// reading.
    pub fn with_transcript<R>(&self, f: impl FnOnce(&[Item]) -> R) -> R {
        f(&self.inner.lock().unwrap().items)
    }

    /// The action lines, for tests that assert on what was logged.
    #[cfg(test)]
    pub(crate) fn actions(&self) -> Vec<String> {
        self.with_transcript(|items| {
            items
                .iter()
                .filter_map(|i| match i {
                    Item::Action(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
    }

    pub fn lamp(&self) -> Lamp {
        let inner = self.inner.lock().unwrap();
        match (&inner.live, inner.turn) {
            (None, _) => Lamp::Off,
            (Some(_), false) => Lamp::Idle,
            (Some(_), true) if inner.pending_permission() => Lamp::Waiting,
            (Some(_), true) => Lamp::Working,
        }
    }

    /// The working status line, derived — never set by the agent: the plan
    /// entry in progress, else the running tool call's title, else
    /// "Working". None when no turn is running.
    pub fn working(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        if !inner.turn {
            return None;
        }
        if inner.pending_permission() {
            return Some("Waiting for you".to_owned());
        }
        // Only this turn's items: everything after the last user message.
        let turn = inner.items.iter().rev().take_while(|i| !matches!(i, Item::User { .. }));
        let mut tool = None;
        for item in turn {
            match item {
                Item::Plan(entries) => {
                    if let Some(entry) = entries.iter().find(|e| e.status == "in_progress") {
                        return Some(entry.content.clone());
                    }
                }
                Item::Tool(t) if t.running() && tool.is_none() => tool = t.call.title.clone(),
                _ => {}
            }
        }
        Some(tool.unwrap_or_else(|| "Working".to_owned()))
    }

    /// The input box's placeholder: "Message <model>", "Message <agent>"
    /// when the model has no name worth showing, blank when unconfigured.
    pub fn placeholder(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let Some(Header { agent: Some(agent), .. }) = inner.header() else {
            return String::new();
        };
        let model = match inner.options.iter().find(|o| o.category.as_deref() == Some("model")) {
            Some(option) => model_name(option),
            // Not running: what it said last time.
            None => {
                let id = inner.config.agent.selected.as_deref().unwrap_or_default();
                inner.memo.agents.get(id).and_then(|m| m.model.clone())
            }
        };
        format!("Message {}", model.as_deref().unwrap_or(agent))
    }

    pub fn options(&self) -> Vec<ConfigOption> {
        self.inner.lock().unwrap().options.clone()
    }

    /// Context used / size, when the agent reports it.
    pub fn usage(&self) -> Option<(u64, u64)> {
        self.inner.lock().unwrap().usage
    }

    pub fn config(&self) -> Config {
        self.inner.lock().unwrap().config.clone()
    }

    pub fn selected(&self) -> Option<String> {
        self.inner.lock().unwrap().config.agent.selected.clone()
    }

    pub fn running(&self) -> bool {
        self.inner.lock().unwrap().live.is_some()
    }

    /// How to log in, when the live agent has said it is logged out.
    pub fn login_hint(&self) -> Option<String> {
        self.inner.lock().unwrap().live.as_ref()?.login.clone()
    }

    // --- what the panel does ---

    /// A message from the user: a prompt, or a steering message when a turn
    /// is running. Spawns the agent if it is not up. `snapshot` — what the
    /// user is looking at as they hit Enter — rides as a second content
    /// block ("user state: sent, not sampled").
    pub fn send(self: &Arc<Self>, text: String, snapshot: Option<Value>) {
        let mut inner = self.inner.lock().unwrap();
        let Some(id) = inner.config.agent.selected.clone() else {
            inner.items.push(Item::Notice("No agent selected — pick one in Agent Settings.".into()));
            drop(inner);
            return self.wake();
        };
        inner.items.push(Item::User { id: None, text: text.clone() });
        // The user is back: whatever loop the engine was holding off is
        // theirs to restart.
        inner.engine_prompts = 0;
        inner.muted = false;
        if inner.live.is_none()
            && let Err(e) = self.spawn(&mut inner, &id)
        {
            inner.items.push(Item::Error(e));
            drop(inner);
            return self.wake();
        }
        let mut blocks = vec![json!({"type": "text", "text": text})];
        if let Some(snapshot) = snapshot {
            blocks.push(resource("odm://user-state", &snapshot));
        }
        // Lit from Enter, not from the agent's first sign of life.
        inner.turn = true;
        if let Some(live) = &inner.live {
            live.agent.prompt(blocks);
        }
        drop(inner);
        self.wake();
    }

    /// Stop the running turn (`session/cancel`; the agent is killed if the
    /// turn has not ended 5 s later).
    pub fn stop(&self) {
        if let Some(live) = &self.inner.lock().unwrap().live {
            live.agent.cancel();
        }
    }

    pub fn answer(&self, permission: u64, option: &PermissionOption) {
        let mut inner = self.inner.lock().unwrap();
        for item in inner.items.iter_mut() {
            if let Item::Permission { id, answer: answer @ None, .. } = item
                && *id == permission
            {
                *answer = Some(option.name.clone());
            }
        }
        if let Some(live) = &inner.live {
            live.agent.answer_permission(permission, Some(&option.id));
        }
        drop(inner);
        self.wake();
    }

    /// Change one of the session's selectors. The mode is also a persisted
    /// setting, re-applied to every later session.
    pub fn set_option(&self, id: &str, value: &str) {
        let mut inner = self.inner.lock().unwrap();
        let Some(live) = &inner.live else { return };
        live.agent.set_config_option(id, value);
        if inner.options.iter().any(|o| o.id == id && o.is_mode()) {
            let agent = live.id.clone();
            let saved = odm_config::set_mode(&self.files, &agent, Some(value));
            inner.config.agent.mode.insert(agent, value.to_owned());
            inner.note_config_error(saved);
        }
    }

    /// Forget the session and start over: the next message opens a new one.
    pub fn new_session(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.retire();
        if let Some(id) = inner.config.agent.selected.clone() {
            inner.memo.agents.entry(id).or_default().session_id = None;
            inner.memo.save(&self.project);
        }
        inner.fresh_header(&self.project);
        drop(inner);
        self.wake();
    }

    /// Pick an agent (project `use` + system `default`). A different one
    /// retires whatever is running.
    pub fn select(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        let saved = odm_config::set_agent(&self.files, Some(id));
        inner.note_config_error(saved);
        if inner.config.agent.selected.as_deref() == Some(id) {
            return;
        }
        inner.config.agent.selected = Some(id.to_owned());
        inner.retire();
        inner.fresh_header(&self.project);
        drop(inner);
        self.wake();
    }

    pub fn set_quiet_odm(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        let saved = odm_config::set_quiet_odm(&self.files, on);
        inner.config.agent.quiet_odm = on;
        inner.note_config_error(saved);
    }

    pub fn set_custom(&self, id: &str, agent: odm_config::CustomAgent) -> Result<(), String> {
        odm_config::set_custom(&self.files, id, &agent)?;
        self.inner.lock().unwrap().config.agent.custom.insert(id.to_owned(), agent);
        Ok(())
    }

    /// Kill the agent, if one is running. `wait`: block until it is reaped
    /// — for the process's own exit, when nothing would be left to do it.
    pub fn shutdown(&self, wait: bool) {
        let live = self.inner.lock().unwrap().live.take();
        if let Some(live) = live {
            match wait {
                true => live.agent.shutdown_wait(),
                false => live.agent.shutdown(),
            }
        }
    }

    // --- what the engine tells it ---

    /// A CLI command the agent ran, a file it changed.
    pub(crate) fn action(&self, text: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.actions += 1;
        inner.items.push(Item::Action(text));
        drop(inner);
        self.wake();
    }

    /// A host warning: a transcript line now, and forwarded to the agent
    /// when a session is live and idle.
    pub(crate) fn warning(&self, text: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.push(Item::Engine { id: None, text: text.clone() });
        inner.warnings.push(text);
        inner.pokes += 1;
        drop(inner);
        self.cv.notify_all();
        self.wake();
    }

    /// The diagnostic value may have changed; the pusher should look.
    pub(crate) fn poke(&self) {
        self.inner.lock().unwrap().pokes += 1;
        self.cv.notify_all();
    }

    fn spawn(self: &Arc<Self>, inner: &mut Inner, id: &str) -> Result<(), String> {
        let resolved = table::resolve(id, &inner.config.agent).map_err(|e| e.to_string())?;
        let mut launch = Launch::new(resolved.command, self.project.clone());
        launch.env = resolved.env;
        // The agent's `odm` is *this* odm, even when the user has none (or
        // another) on PATH.
        launch.path_prepend =
            std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned));
        let options = SessionOptions {
            resume: inner.memo.agents.get(id).and_then(|m| m.session_id.clone()),
            meta: resolved.meta,
            mode: inner.config.agent.mode.get(id).cloned(),
        };
        let (agent, events) = Agent::spawn(launch, options, Arc::new(|| {}))
            .map_err(|e| format!("could not start {}: {e}", table::title(id)))?;
        inner.serial += 1;
        let serial = inner.serial;
        inner.live = Some(Live { agent, id: id.to_owned(), serial, ready: false, login: None });
        // History goes before what was just typed.
        inner.replay_at = inner.items.len().saturating_sub(1);
        inner.replayed.clear();
        inner.baseline.clear();
        let host = self.clone();
        std::thread::Builder::new()
            .name("odm-agent-events".into())
            .spawn(move || {
                for event in events {
                    host.apply(serial, event);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn apply(&self, serial: u64, event: Event) {
        let mut inner = self.inner.lock().unwrap();
        // A retired agent's last words are not this one's.
        if inner.live.as_ref().is_none_or(|live| live.serial != serial) {
            return;
        }
        let agent_id = inner.live.as_ref().map(|l| l.id.clone()).unwrap_or_default();
        let mut poke = false;
        match event {
            Event::Initialized(info) => {
                let memo = inner.memo.agents.entry(agent_id).or_default();
                memo.title = info.title.clone().or(memo.title.take());
                memo.version = info.version.clone();
                inner.memo.save(&self.project);
                if let Some(header) = inner.header_mut() {
                    header.agent = info.title.or(header.agent.take());
                    header.version = info.version;
                }
            }
            Event::SessionStarted { id, resumed } => {
                inner.splice_replay();
                if let Some(live) = &mut inner.live {
                    live.ready = true;
                }
                // A new session under a header that already had one is a
                // new header; otherwise this is the one it was waiting for.
                if !resumed && inner.header().is_some_and(|h| h.session.is_some()) {
                    let header = inner.header().cloned().unwrap_or_default();
                    let at = inner.replay_at.min(inner.items.len());
                    inner.items.insert(at, Item::Header(header));
                }
                if let Some(header) = inner.header_mut() {
                    header.session = Some(id.clone());
                }
                inner.memo.agents.entry(agent_id).or_default().session_id = Some(id);
                inner.memo.save(&self.project);
                poke = true;
            }
            Event::SessionLost => {
                inner.replayed.clear();
                if let Some(header) = inner.header_mut() {
                    header.session = None;
                }
                let at = inner.replay_at.min(inner.items.len());
                inner.items.insert(
                    at,
                    Item::Notice("The last session could not be resumed; this is a new one.".into()),
                );
            }
            Event::SessionFailed { message, auth_required } => {
                inner.turn = false;
                let login = table::built_in(&agent_id).map(|a| a.login.to_owned());
                let text = match (auth_required, &login) {
                    (true, Some(login)) => format!(
                        "Not logged in. Run `{login}` in a terminal, then send your message again."
                    ),
                    (true, None) => {
                        "Not logged in. Log the agent in from a terminal, then send your message again."
                            .to_owned()
                    }
                    (false, _) => format!("The agent could not start a session: {message}"),
                };
                if let (true, Some(live)) = (auth_required, &mut inner.live) {
                    live.login = login;
                }
                inner.items.push(Item::Error(text));
            }
            Event::Update { update, replay } => {
                let actions = inner.actions;
                let list = if replay { &mut inner.replayed } else { &mut inner.items };
                fold(list, update, replay, actions);
            }
            Event::ConfigOptions(options) => {
                let name = |category: &str| {
                    options
                        .iter()
                        .find(|o| o.category.as_deref() == Some(category))
                        .and_then(|o| o.current_name())
                        .map(str::to_owned)
                };
                let (model, mode) = (name("model"), name("mode"));
                inner.memo.agents.entry(agent_id).or_default().model = options
                    .iter()
                    .find(|o| o.category.as_deref() == Some("model"))
                    .and_then(model_name);
                inner.memo.save(&self.project);
                if let Some(header) = inner.header_mut() {
                    header.model = model;
                    header.mode = mode;
                }
                inner.options = options;
            }
            Event::Auth { label, logged_in, .. } => {
                if let Some(live) = &mut inner.live
                    && logged_in
                {
                    live.login = None;
                }
                if let Some(header) = inner.header_mut() {
                    header.account = Some(label);
                }
            }
            Event::Usage { used, size } => inner.usage = Some((used, size)),
            Event::Permission { id, tool, options } => {
                inner.items.push(Item::Permission { id, tool, options, answer: None });
            }
            Event::TurnStarted => inner.turn = true,
            Event::TurnEnded { stop_reason } => {
                inner.turn = false;
                inner.close_turn();
                match stop_reason.as_str() {
                    "end_turn" => {}
                    "cancelled" => inner.items.push(Item::Notice("Stopped.".into())),
                    other => match other.strip_prefix("error: ") {
                        Some(error) => inner.items.push(Item::Error(error.to_owned())),
                        None => inner.items.push(Item::Notice(format!("Turn ended: {other}."))),
                    },
                }
                poke = true;
            }
            Event::Exited { reason, stderr } => {
                inner.splice_replay();
                inner.live = None;
                inner.turn = false;
                inner.options.clear();
                inner.close_turn();
                let tail = if stderr.is_empty() { String::new() } else { format!("\n{stderr}") };
                match reason {
                    ExitReason::Shutdown => {}
                    ExitReason::Hung => inner.items.push(Item::Error(format!(
                        "The agent did not stop when asked and was killed. The next message restarts it.{tail}"
                    ))),
                    ExitReason::Crashed(status) => inner.items.push(Item::Error(format!(
                        "The agent exited ({status}). The next message restarts it.{tail}"
                    ))),
                }
            }
        }
        inner.pokes += poke as u64;
        drop(inner);
        if poke {
            self.cv.notify_all();
        }
        self.wake();
    }

    /// Forward build diagnostics and host warnings to the agent, as prompts
    /// the engine authors. Runs on its own thread for a viewer session;
    /// returns when the session stops.
    ///
    /// State-compare, not an event queue: the diagnostic value (failing
    /// slots/files → error) is compared against what the agent was last
    /// told. Each push starts a paid turn nobody typed, so:
    /// - only into a session that is already live (this never spawns);
    /// - never while a turn runs — the agent's own half-done edits make
    ///   transient failures, and its CLI responses already carry the build
    ///   state; what still stands at turn end goes as one follow-up;
    /// - only once the value has been quiet for [`QUIET`] with no build in
    ///   flight;
    /// - a value that went back to empty sends nothing;
    /// - at most [`ENGINE_PROMPT_CAP`] in a row without a user message
    ///   between, then one `engine:` line and silence.
    pub(crate) fn run_pusher(&self, state: &EngineState) {
        let mut last = state.diagnostic_map();
        let mut changed = Instant::now();
        loop {
            if state.stopping() {
                return;
            }
            // Engine locks are never taken under ours, so everything that
            // reads the engine comes first — and `pokes` says whether any
            // of it went out of date while we looked.
            let seen = self.inner.lock().unwrap().pokes;
            let map = state.diagnostic_map();
            let building = state.building();
            let failing = describe(state, &map);
            if map != last {
                last = map.clone();
                changed = Instant::now();
            }
            let mut inner = self.inner.lock().unwrap();
            let quiet_for = changed.elapsed();
            let wanted = inner.push_wanted(&map);
            if wanted && !building && quiet_for >= self.quiet {
                inner.push(&map, &failing);
                drop(inner);
                self.wake();
                continue;
            }
            if inner.pokes != seen {
                continue;
            }
            match wanted {
                // Something to send once it settles: look again shortly.
                true => {
                    let wait = self.quiet.saturating_sub(quiet_for).max(Duration::from_millis(20));
                    drop(self.cv.wait_timeout(inner, wait).unwrap());
                }
                false => drop(self.cv.wait_while(inner, |inner| inner.pokes == seen).unwrap()),
            }
        }
    }
}

/// A model's display name — unless it is the agent's `default`, which reads
/// "Default (recommended)" and names nothing.
fn model_name(option: &ConfigOption) -> Option<String> {
    (option.current != "default").then(|| option.current_name().map(str::to_owned)).flatten()
}

/// The failing things as the agent should read them: (label, error).
fn describe(state: &EngineState, map: &DiagnosticMap) -> Vec<(String, String)> {
    map.iter()
        .map(|(key, error)| {
            let label = match key.split_once(':') {
                Some(("slot", slot)) => match state.view_of(slot) {
                    Some(view) => format!("view {slot} ({})", view.path),
                    None => format!("view {slot}"),
                },
                Some((_, path)) => path.to_owned(),
                None => key.clone(),
            };
            (label, error.clone())
        })
        .collect()
}

fn resource(uri: &str, body: &Value) -> Value {
    json!({"type": "resource", "resource": {
        "uri": uri, "mimeType": "application/json", "text": body.to_string(),
    }})
}

impl Inner {
    fn header(&self) -> Option<&Header> {
        self.items.iter().rev().find_map(|i| match i {
            Item::Header(h) => Some(h),
            _ => None,
        })
    }

    fn header_mut(&mut self) -> Option<&mut Header> {
        self.items.iter_mut().rev().find_map(|i| match i {
            Item::Header(h) => Some(h),
            _ => None,
        })
    }

    /// A header from what is remembered about the selected agent, for
    /// before it has been spawned.
    fn cached_header(&self, project: &Path) -> Header {
        let selected = self.config.agent.selected.as_deref();
        let memo = selected.and_then(|id| self.memo.agents.get(id)).cloned().unwrap_or_default();
        Header {
            agent: selected.map(|id| memo.title.unwrap_or_else(|| table::title(id))),
            version: memo.version,
            model: memo.model,
            cwd: project.display().to_string(),
            ..Default::default()
        }
    }

    /// Start a new header — or, when the current one never got a session,
    /// take its place: a header is a session's first line, not a log of
    /// what was picked.
    fn fresh_header(&mut self, project: &Path) {
        let header = self.cached_header(project);
        match self.header_mut() {
            Some(old) if old.session.is_none() => *old = header,
            _ => self.items.push(Item::Header(header)),
        }
    }

    fn pending_permission(&self) -> bool {
        self.items.iter().any(|i| matches!(i, Item::Permission { answer: None, .. }))
    }

    /// The turn is over, however it ended: nothing in it is still pending.
    fn close_turn(&mut self) {
        for item in &mut self.items {
            match item {
                Item::Permission { answer: answer @ None, .. } => *answer = Some("cancelled".into()),
                Item::Tool(tool) if tool.running() => tool.call.status = Some("failed".into()),
                _ => {}
            }
        }
    }

    fn splice_replay(&mut self) {
        let at = self.replay_at.min(self.items.len());
        let replayed = std::mem::take(&mut self.replayed);
        self.replay_at = at + replayed.len();
        self.items.splice(at..at, replayed);
    }

    /// Drop the running agent; its session stays resumable.
    fn retire(&mut self) {
        if let Some(live) = self.live.take() {
            live.agent.shutdown();
        }
        self.turn = false;
        self.options.clear();
        self.usage = None;
        self.replayed.clear();
        self.close_turn();
    }

    fn note_config_error(&mut self, saved: Result<(), String>) {
        if let Err(e) = saved {
            self.items.push(Item::Engine { id: None, text: format!("setting not saved: {e}") });
        }
    }

    fn push_wanted(&mut self, map: &DiagnosticMap) -> bool {
        if !self.live.as_ref().is_some_and(|l| l.ready) || self.turn {
            return false;
        }
        // Healed: nothing to say, and the next failure is news again.
        if map.is_empty() {
            self.baseline.clear();
        }
        !self.muted && (*map != self.baseline && !map.is_empty() || !self.warnings.is_empty())
    }

    fn push(&mut self, map: &DiagnosticMap, failing: &[(String, String)]) {
        if self.engine_prompts >= ENGINE_PROMPT_CAP {
            self.muted = true;
            self.baseline = map.clone();
            self.warnings.clear();
            self.items.push(Item::Engine {
                id: None,
                text: "not forwarding further build errors until you next message the agent".into(),
            });
            return;
        }
        let warnings = std::mem::take(&mut self.warnings);
        let news = *map != self.baseline && !map.is_empty();
        let mut text = format!("{ENGINE_TAG} Nobody typed this: the engine is reporting on the project.");
        if news {
            text.push_str("\nBuild errors changed. Failing now:");
            for (label, error) in failing {
                text.push_str(&format!("\n- {label}: {}", error.lines().next().unwrap_or_default()));
            }
            text.push_str("\nFull errors are attached (odm://diagnostics).");
            let labels: Vec<&str> = failing.iter().map(|(l, _)| l.as_str()).collect();
            self.items.push(Item::Engine {
                id: None,
                text: format!("told the agent about build errors: {}", labels.join(", ")),
            });
        }
        for warning in &warnings {
            text.push_str(&format!("\nEngine warning: {warning}"));
        }
        // A turn nobody typed must say where it came from.
        if !news {
            self.items.push(Item::Engine { id: None, text: "told the agent about the warnings above".into() });
        }
        let failing: serde_json::Map<String, Value> =
            failing.iter().map(|(l, e)| (l.clone(), json!(e))).collect();
        let blocks = vec![
            json!({"type": "text", "text": text}),
            resource("odm://diagnostics", &json!({"failing": failing, "warnings": warnings})),
        ];
        self.baseline = map.clone();
        self.engine_prompts += 1;
        self.turn = true;
        if let Some(live) = &self.live {
            live.agent.prompt(blocks);
        }
    }
}

/// Context a prompt carried for the agent's eyes only: the bare uri chunk,
/// and the `<context ref=…>` wrapper adapters replay embedded resources as.
fn is_context(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("odm://") || text.starts_with("<context ref=\"odm://")
}

/// Fold one session update into a transcript. Chunks are appended to their
/// message: the one with the same id within this turn, else (no ids) the
/// last item when it is the same kind.
fn fold(items: &mut Vec<Item>, update: Update, replay: bool, actions: u64) {
    /// How far back an id is looked for: a message's chunks are never far
    /// apart, and a long transcript must not cost a scan per chunk.
    const REACH: usize = 64;

    /// The text of the message a chunk belongs to, if it is already there:
    /// an item of the right kind (`id_of` answers for those) with the same id.
    fn open<'a>(
        items: &'a mut [Item],
        id: &Option<String>,
        id_of: fn(&Item) -> Option<&Option<String>>,
    ) -> Option<&'a mut String> {
        let last = items.len().checked_sub(1)?;
        let from = if id.is_some() { last.saturating_sub(REACH) } else { last };
        let index = (from..=last).rev().find(|&i| id_of(&items[i]) == Some(id))?;
        match &mut items[index] {
            Item::User { text, .. }
            | Item::Engine { text, .. }
            | Item::Agent { text, .. }
            | Item::Thought { text, .. } => Some(text),
            _ => None,
        }
    }
    fn user(item: &Item) -> Option<&Option<String>> {
        match item {
            Item::User { id, .. } | Item::Engine { id, .. } => Some(id),
            _ => None,
        }
    }
    fn agent(item: &Item) -> Option<&Option<String>> {
        match item {
            Item::Agent { id, .. } => Some(id),
            _ => None,
        }
    }
    fn thought(item: &Item) -> Option<&Option<String>> {
        match item {
            Item::Thought { id, .. } => Some(id),
            _ => None,
        }
    }

    match update {
        // Live, the user's message is already in the transcript: we put it
        // there. Only history needs rebuilding from the agent's record.
        Update::UserChunk(_) if !replay => {}
        Update::UserChunk(chunk) => {
            if is_context(&chunk.text) {
                return;
            }
            match open(items, &chunk.message_id, user) {
                Some(text) => text.push_str(&chunk.text),
                // An engine-authored prompt replays as the engine, not the user.
                None => match chunk.text.trim_start().strip_prefix(ENGINE_TAG) {
                    Some(rest) => items.push(Item::Engine {
                        id: chunk.message_id,
                        text: rest.trim_start().to_owned(),
                    }),
                    None => items.push(Item::User { id: chunk.message_id, text: chunk.text }),
                },
            }
        }
        Update::AgentChunk(chunk) => {
            match open(items, &chunk.message_id, agent) {
                Some(text) => text.push_str(&chunk.text),
                None => items.push(Item::Agent { id: chunk.message_id, text: chunk.text }),
            }
        }
        Update::ThoughtChunk(chunk) => {
            match open(items, &chunk.message_id, thought) {
                Some(text) => text.push_str(&chunk.text),
                None => items.push(Item::Thought { id: chunk.message_id, text: chunk.text }),
            }
        }
        Update::ToolCall(call) => {
            let known = items.iter_mut().rev().find_map(|i| match i {
                Item::Tool(tool) if tool.call.id == call.id => Some(tool),
                _ => None,
            });
            match known {
                Some(tool) => {
                    let ToolCall { title, kind, status, content, .. } = call;
                    tool.call.title = title.or(tool.call.title.take());
                    tool.call.kind = kind.or(tool.call.kind.take());
                    tool.call.status = status.or(tool.call.status.take());
                    tool.call.content = content.or(tool.call.content.take());
                    if !tool.running() {
                        tool.reached_engine = actions > tool.actions_before;
                    }
                }
                None => items.push(Item::Tool(ToolItem {
                    call,
                    replayed: replay,
                    actions_before: actions,
                    reached_engine: false,
                })),
            }
        }
        // One plan per turn, updated in place.
        Update::Plan(entries) => {
            let turn = items.iter_mut().rev().take_while(|i| !matches!(i, Item::User { .. }));
            match turn.into_iter().find_map(|i| match i {
                Item::Plan(plan) => Some(plan),
                _ => None,
            }) {
                Some(plan) => *plan = entries,
                None => items.push(Item::Plan(entries)),
            }
        }
    }
}

#[cfg(test)]
mod tests;
