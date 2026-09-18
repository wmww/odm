//! An ACP (Agent Client Protocol) client: spawns an agent process and holds a
//! conversation with it over ndjson JSON-RPC on its stdio.
//!
//! No async runtime and no UI: commands go in from any thread
//! ([`Agent::prompt`], [`Agent::cancel`], …), [`Event`]s come out of a
//! channel, and a wake callback fires after each one — the same shape as the
//! engine's own wake hook. Threads: one reads the agent's stdout and runs the
//! whole protocol state machine, one drains its stderr (an undrained pipe
//! blocks the child).
//!
//! The client advertises no fs/terminal capabilities and hands the session no
//! MCP servers: the agent works through its own tools, in `cwd`.

mod process;
mod wire;

pub use wire::{
    AgentInfo, Choice, Chunk, ConfigOption, PermissionOption, PlanEntry, ToolCall, ToolContent,
    Update,
};

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::ChildStdin;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use wire::{OptionKind, Parsed};

/// Lines of the agent's stderr kept for a crash report.
const STDERR_TAIL: usize = 12;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// What to run, and how.
#[derive(Clone, Debug)]
pub struct Launch {
    /// Program + arguments.
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// Put first on the child's PATH.
    pub path_prepend: Option<PathBuf>,
    /// How long a cancelled turn gets to end before the agent is killed.
    pub cancel_grace: Duration,
    /// How long the agent gets to exit once its stdin is closed.
    pub exit_grace: Duration,
}

impl Launch {
    pub fn new(command: Vec<String>, cwd: PathBuf) -> Launch {
        Launch {
            command,
            env: BTreeMap::new(),
            cwd,
            path_prepend: None,
            cancel_grace: Duration::from_secs(5),
            exit_grace: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionOptions {
    /// Resume this session (`session/load`) when the agent can; a session
    /// that will not load falls back to a fresh one.
    pub resume: Option<String>,
    /// `_meta` for `session/new` / `session/load` — agent-specific extras.
    pub meta: Option<Value>,
    /// Mode id to apply to the session once it exists.
    pub mode: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExitReason {
    /// We asked it to ([`Agent::shutdown`]).
    Shutdown,
    /// A cancelled turn never ended; killed.
    Hung,
    /// It went on its own; the text is its exit status.
    Crashed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Initialized(AgentInfo),
    SessionStarted {
        id: String,
        resumed: bool,
    },
    /// The session to resume would not load; a fresh one is being made.
    SessionLost,
    /// No session could be made. The next [`Agent::prompt`] tries again.
    SessionFailed {
        message: String,
        auth_required: bool,
    },
    /// `replay`: part of a loaded session's history, not something new.
    Update {
        update: Update,
        replay: bool,
    },
    /// The session's selectors, whole, whenever any of them changes.
    ConfigOptions(Vec<ConfigOption>),
    /// `_auth/status_update`: which account the agent runs as.
    Auth {
        label: String,
        email: Option<String>,
        logged_in: bool,
    },
    Usage {
        used: u64,
        size: u64,
    },
    /// The agent is blocked on the user. Answer with
    /// [`Agent::answer_permission`].
    Permission {
        id: u64,
        tool: ToolCall,
        options: Vec<PermissionOption>,
    },
    TurnStarted,
    /// The `session/prompt` response — the only thing that ends a turn.
    /// `end_turn`, `cancelled`, `refusal`, …, or `error: <message>`.
    TurnEnded {
        stop_reason: String,
    },
    /// The process is gone (and reaped, with its process group). Last event.
    Exited {
        reason: ExitReason,
        stderr: String,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Starting,
    Loading,
    /// `session/new` failed (logged out, usually); a prompt retries it.
    NoSession,
    Ready,
}

enum Pending {
    Initialize,
    NewSession,
    LoadSession,
    Prompt,
    Steer(Vec<Value>),
    SetConfig,
}

struct State {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    phase: Phase,
    options: SessionOptions,
    cwd: PathBuf,
    info: AgentInfo,
    session: Option<String>,
    config: Vec<ConfigOption>,
    turn_running: bool,
    /// Counts prompts sent, so a cancel's watchdog knows its turn from the next.
    turn_serial: u64,
    /// Content blocks waiting for a session, or for the running turn to end.
    queued: Vec<Value>,
    /// Our permission id → the agent's JSON-RPC request id.
    permissions: HashMap<u64, Value>,
    next_permission: u64,
    closing: bool,
    hung: bool,
}

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    state: Mutex<State>,
    events: Mutex<Sender<Event>>,
    wake: Wake,
    pgid: i32,
    stderr: Mutex<VecDeque<String>>,
    exited: (Mutex<bool>, Condvar),
    launch: Launch,
}

/// A running agent. Dropping it shuts the agent down.
pub struct Agent {
    shared: Arc<Shared>,
}

impl Agent {
    /// Start the agent and begin the handshake (`initialize`, then a
    /// session). Everything after the spawn itself is reported as events.
    pub fn spawn(
        launch: Launch,
        options: SessionOptions,
        wake: Wake,
    ) -> std::io::Result<(Agent, Receiver<Event>)> {
        let (events, rx) = channel();
        let (started, start) = channel();
        // The child is spawned *on* the thread that reads it: Linux delivers
        // PDEATHSIG when the spawning thread dies, and this one lives exactly
        // as long as the child's stdout.
        std::thread::Builder::new().name("odm-agent".into()).spawn(move || {
            let mut child = match process::spawn(&launch) {
                Ok(child) => child,
                Err(e) => return drop(started.send(Err(e))),
            };
            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let shared = Arc::new(Shared {
                stdin: Mutex::new(child.stdin.take()),
                state: Mutex::new(State {
                    next_id: 1,
                    pending: HashMap::new(),
                    phase: Phase::Starting,
                    options,
                    cwd: launch.cwd.clone(),
                    info: AgentInfo::default(),
                    session: None,
                    config: Vec::new(),
                    turn_running: false,
                    turn_serial: 0,
                    queued: Vec::new(),
                    permissions: HashMap::new(),
                    next_permission: 1,
                    closing: false,
                    hung: false,
                }),
                events: Mutex::new(events),
                wake,
                pgid: child.id() as i32,
                stderr: Mutex::new(VecDeque::new()),
                exited: (Mutex::new(false), Condvar::new()),
                launch,
            });
            let _ = started.send(Ok(shared.clone()));
            let drain = shared.clone();
            let drained = std::thread::Builder::new().name("odm-agent-stderr".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = drain.stderr.lock().unwrap();
                    if tail.len() == STDERR_TAIL {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
            shared.initialize();
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // Anything that is not a JSON object is noise on stdout.
                if let Ok(msg) = serde_json::from_str::<Value>(&line)
                    && msg.is_object()
                {
                    shared.handle(&msg);
                }
            }
            shared.reap(child, drained.ok());
        })?;
        let shared = start.recv().map_err(|_| std::io::Error::other("agent thread died"))??;
        Ok((Agent { shared }, rx))
    }

    /// Send a user message: a prompt, or — while a turn is running — a
    /// steering message when the agent takes those, else queued until the
    /// turn ends. Before the session exists it waits for it.
    pub fn prompt(&self, blocks: Vec<Value>) {
        let shared = &self.shared;
        let mut state = shared.state.lock().unwrap();
        let out = match state.phase {
            Phase::Starting | Phase::Loading => {
                state.queued.extend(blocks);
                Vec::new()
            }
            Phase::NoSession => {
                state.queued.extend(blocks);
                vec![state.new_session()]
            }
            Phase::Ready if !state.turn_running => {
                state.queued.extend(blocks);
                shared.flush(&mut state)
            }
            Phase::Ready if state.info.steering => {
                let session = state.session.clone();
                vec![state.request(
                    "_session/steering",
                    json!({
                        "sessionId": session,
                        "prompt": blocks,
                        // Idle by the time it lands: hand it back rather than
                        // start a detached turn whose end we would never see.
                        "_meta": {"steering": {"idleBehavior": "promptRequired"}},
                    }),
                    Pending::Steer(blocks),
                )]
            }
            Phase::Ready => {
                state.queued.extend(blocks);
                Vec::new()
            }
        };
        drop(state);
        shared.send_all(out);
    }

    /// Stop the running turn. The turn still ends with its `TurnEnded`; if
    /// that has not come `cancel_grace` later, the agent is killed.
    pub fn cancel(&self) {
        let shared = &self.shared;
        let mut state = shared.state.lock().unwrap();
        if !state.turn_running {
            return;
        }
        let mut out = vec![json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {"sessionId": state.session},
        })];
        // ACP: a cancelled turn's open permission requests must be answered.
        for (_, rpc) in state.permissions.drain() {
            out.push(json!({"jsonrpc": "2.0", "id": rpc, "result": {"outcome": {"outcome": "cancelled"}}}));
        }
        let serial = state.turn_serial;
        drop(state);
        shared.send_all(out);
        let watch = shared.clone();
        let _ = std::thread::Builder::new().name("odm-agent-cancel".into()).spawn(move || {
            if watch.wait_exit(watch.launch.cancel_grace) {
                return;
            }
            let mut state = watch.state.lock().unwrap();
            if state.turn_running && state.turn_serial == serial {
                state.hung = true;
                drop(state);
                process::kill_group(watch.pgid);
            }
        });
    }

    /// Answer an [`Event::Permission`]: the chosen option's id, or None for
    /// "cancelled". A second answer, or one for a request already closed by
    /// [`Agent::cancel`], does nothing.
    pub fn answer_permission(&self, id: u64, option: Option<&str>) {
        let Some(rpc) = self.shared.state.lock().unwrap().permissions.remove(&id) else { return };
        let outcome = match option {
            Some(option) => json!({"outcome": "selected", "optionId": option}),
            None => json!({"outcome": "cancelled"}),
        };
        self.shared.send(json!({"jsonrpc": "2.0", "id": rpc, "result": {"outcome": outcome}}));
    }

    /// Change one of the session's selectors. The answer comes back as
    /// [`Event::ConfigOptions`].
    pub fn set_config_option(&self, id: &str, value: &str) {
        let mut state = self.shared.state.lock().unwrap();
        let out = state.set_config(id, value);
        drop(state);
        self.shared.send_all(out.into_iter().collect());
    }

    /// Ask the agent to go: close its stdin, and kill its process group if
    /// it has not exited `exit_grace` later. Returns at once.
    pub fn shutdown(&self) {
        if let Some(wait) = self.shared.begin_shutdown() {
            let _ = std::thread::Builder::new().name("odm-agent-reap".into()).spawn(wait);
        }
    }

    /// [`Agent::shutdown`], returning once the agent is gone — for the
    /// process's own exit, where nothing would be left to reap it.
    pub fn shutdown_wait(&self) {
        if let Some(wait) = self.shared.begin_shutdown() {
            wait();
        }
        self.shared.wait_exit(Duration::from_secs(1));
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl State {
    fn request(&mut self, method: &str, params: Value, pending: Pending) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn session_params(&self) -> Value {
        let mut params = json!({"cwd": self.cwd, "mcpServers": []});
        if let Some(meta) = &self.options.meta {
            params["_meta"] = meta.clone();
        }
        params
    }

    fn new_session(&mut self) -> Value {
        self.phase = Phase::Starting;
        let params = self.session_params();
        self.request("session/new", params, Pending::NewSession)
    }

    fn set_config(&mut self, id: &str, value: &str) -> Option<Value> {
        let session = self.session.clone()?;
        let option = self.config.iter().find(|o| o.id == id)?;
        let (method, params) = match option.kind {
            OptionKind::LegacyMode => {
                ("session/set_mode", json!({"sessionId": session, "modeId": value}))
            }
            OptionKind::Boolean => (
                "session/set_config_option",
                json!({"sessionId": session, "configId": id, "type": "boolean", "value": value == "true"}),
            ),
            OptionKind::Select => (
                "session/set_config_option",
                json!({"sessionId": session, "configId": id, "value": value}),
            ),
        };
        Some(self.request(method, params, Pending::SetConfig))
    }
}

impl Shared {
    fn emit(&self, event: Event) {
        let _ = self.events.lock().unwrap().send(event);
        (self.wake)();
    }

    fn send(&self, msg: Value) {
        self.send_all(vec![msg]);
    }

    /// Never called with the state lock held: a full pipe blocks, and the
    /// reader needs that lock to make the agent drain it.
    fn send_all(&self, msgs: Vec<Value>) {
        let mut stdin = self.stdin.lock().unwrap();
        let Some(pipe) = stdin.as_mut() else { return };
        for msg in msgs {
            // A dead pipe shows up as EOF on the reader; nothing to do here.
            let _ = writeln!(pipe, "{msg}");
        }
        let _ = pipe.flush();
    }

    fn initialize(&self) {
        let msg = self.state.lock().unwrap().request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": {"name": "odm", "title": "ODM", "version": env!("CARGO_PKG_VERSION")},
            }),
            Pending::Initialize,
        );
        self.send(msg);
    }

    /// Send whatever is queued as one prompt, if a prompt can go now.
    fn flush(&self, state: &mut State) -> Vec<Value> {
        if state.phase != Phase::Ready || state.turn_running || state.queued.is_empty() {
            return Vec::new();
        }
        let blocks = std::mem::take(&mut state.queued);
        state.turn_running = true;
        state.turn_serial += 1;
        let session = state.session.clone();
        let msg = state.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": blocks}),
            Pending::Prompt,
        );
        self.emit(Event::TurnStarted);
        vec![msg]
    }

    fn handle(&self, msg: &Value) {
        let method = msg.get("method").and_then(Value::as_str);
        let out = match (method, msg.get("id")) {
            (Some(method), Some(id)) => self.on_request(method, id, msg.get("params")),
            (Some(method), None) => {
                self.on_notification(method, msg.get("params").unwrap_or(&Value::Null));
                Vec::new()
            }
            (None, Some(id)) => self.on_response(id, msg),
            (None, None) => Vec::new(),
        };
        self.send_all(out);
    }

    fn on_request(&self, method: &str, id: &Value, params: Option<&Value>) -> Vec<Value> {
        let params = params.unwrap_or(&Value::Null);
        if method != "session/request_permission" {
            // fs/*, terminal/*: capabilities we never advertised.
            return vec![json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("{method}: not supported by this client")},
            })];
        }
        let mut state = self.state.lock().unwrap();
        let ours = state.next_permission;
        state.next_permission += 1;
        state.permissions.insert(ours, id.clone());
        drop(state);
        let tool = params.get("toolCall").and_then(wire::tool_call).unwrap_or_default();
        self.emit(Event::Permission { id: ours, tool, options: wire::permission_options(params) });
        Vec::new()
    }

    fn on_notification(&self, method: &str, params: &Value) {
        match method {
            "session/update" => {
                let Some(parsed) = params.get("update").and_then(wire::update) else { return };
                let mut state = self.state.lock().unwrap();
                let event = match parsed {
                    Parsed::Update(update) => {
                        Event::Update { update, replay: state.phase == Phase::Loading }
                    }
                    Parsed::ConfigOptions(options) => {
                        state.config = options.clone();
                        Event::ConfigOptions(options)
                    }
                    Parsed::CurrentMode(mode) => {
                        match state.config.iter_mut().find(|o| o.is_mode()) {
                            Some(option) => option.current = mode,
                            None => return,
                        }
                        Event::ConfigOptions(state.config.clone())
                    }
                    Parsed::Usage { used, size } => Event::Usage { used, size },
                };
                drop(state);
                self.emit(event);
            }
            "_auth/status_update" => {
                let status = params.get("authStatus").unwrap_or(&Value::Null);
                let Some(label) = status.get("label").and_then(Value::as_str) else { return };
                self.emit(Event::Auth {
                    label: label.to_owned(),
                    email: status.get("email").and_then(Value::as_str).map(str::to_owned),
                    logged_in: status.get("kind").and_then(Value::as_str) != Some("none"),
                });
            }
            // Adapters ship extensions freely; an unknown one is not ours.
            _ => {}
        }
    }

    fn on_response(&self, id: &Value, msg: &Value) -> Vec<Value> {
        let mut state = self.state.lock().unwrap();
        let Some(pending) = id.as_u64().and_then(|id| state.pending.remove(&id)) else {
            return Vec::new();
        };
        let result = match msg.get("error") {
            None => Ok(msg.get("result").unwrap_or(&Value::Null)),
            Some(error) => Err(error),
        };
        let message = |error: &Value| {
            error.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_owned()
        };
        match (pending, result) {
            (Pending::Initialize, Ok(result)) => {
                state.info = wire::agent_info(result);
                self.emit(Event::Initialized(state.info.clone()));
                match (state.options.resume.clone(), state.info.load_session) {
                    (Some(session), true) => {
                        state.phase = Phase::Loading;
                        let mut params = state.session_params();
                        params["sessionId"] = json!(session);
                        vec![state.request("session/load", params, Pending::LoadSession)]
                    }
                    _ => vec![state.new_session()],
                }
            }
            (Pending::LoadSession, Ok(result)) => {
                let id = state.options.resume.clone().unwrap_or_default();
                self.session_ready(&mut state, id, true, result)
            }
            (Pending::LoadSession, Err(_)) => {
                self.emit(Event::SessionLost);
                vec![state.new_session()]
            }
            (Pending::NewSession, Ok(result)) => {
                match result.get("sessionId").and_then(Value::as_str) {
                    Some(id) => self.session_ready(&mut state, id.to_owned(), false, result),
                    None => self.session_failed(&mut state, "no session id".to_owned(), false),
                }
            }
            (Pending::Initialize | Pending::NewSession, Err(error)) => {
                // -32000: ACP's auth_required.
                let auth = error.get("code").and_then(Value::as_i64) == Some(-32000);
                self.session_failed(&mut state, message(error), auth)
            }
            (Pending::Prompt, result) => {
                state.turn_running = false;
                let stop_reason = match result {
                    Ok(result) => result
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn")
                        .to_owned(),
                    Err(error) => format!("error: {}", message(error)),
                };
                self.emit(Event::TurnEnded { stop_reason });
                self.flush(&mut state)
            }
            (Pending::Steer(blocks), result) => {
                let outcome = result.ok().and_then(|r| r.get("outcome")).and_then(Value::as_str);
                match outcome {
                    // In the turn — or, from an agent that ignores our
                    // idleBehavior, in a turn of its own. Delivered either
                    // way; sending it again would say it twice.
                    Some("injected" | "startedNewTurn") => Vec::new(),
                    // promptRequired, failed, an error, anything else: ours
                    // to deliver, as soon as a prompt can go.
                    _ => {
                        state.queued.extend(blocks);
                        self.flush(&mut state)
                    }
                }
            }
            (Pending::SetConfig, Ok(result)) => {
                if result.get("configOptions").is_some() {
                    state.config = wire::config_options(result);
                }
                self.emit(Event::ConfigOptions(state.config.clone()));
                Vec::new()
            }
            (Pending::SetConfig, Err(_)) => {
                // Refused: say what still stands, so a selector snaps back.
                self.emit(Event::ConfigOptions(state.config.clone()));
                Vec::new()
            }
        }
    }

    fn session_ready(&self, state: &mut State, id: String, resumed: bool, result: &Value) -> Vec<Value> {
        state.session = Some(id.clone());
        state.phase = Phase::Ready;
        state.config = wire::config_options(result);
        self.emit(Event::ConfigOptions(state.config.clone()));
        self.emit(Event::SessionStarted { id, resumed });
        let mut out = Vec::new();
        // The persisted mode, if this agent still has it and is not on it.
        if let Some(mode) = state.options.mode.clone()
            && let Some(option) = state.config.iter().find(|o| o.is_mode())
            && option.current != mode
            && option.choices.iter().any(|c| c.value == mode)
        {
            let id = option.id.clone();
            out.extend(state.set_config(&id, &mode));
        }
        out.extend(self.flush(state));
        out
    }

    fn session_failed(&self, state: &mut State, message: String, auth_required: bool) -> Vec<Value> {
        state.phase = Phase::NoSession;
        self.emit(Event::SessionFailed { message, auth_required });
        Vec::new()
    }

    /// Close stdin; the returned wait kills the group if that was not enough.
    fn begin_shutdown(self: &Arc<Self>) -> Option<impl FnOnce() + Send + 'static> {
        let mut state = self.state.lock().unwrap();
        if std::mem::replace(&mut state.closing, true) {
            return None;
        }
        drop(state);
        drop(self.stdin.lock().unwrap().take());
        let shared = self.clone();
        Some(move || {
            if !shared.wait_exit(shared.launch.exit_grace) {
                process::kill_group(shared.pgid);
            }
        })
    }

    /// Whether the agent exited within `timeout`.
    fn wait_exit(&self, timeout: Duration) -> bool {
        let (lock, cv) = &self.exited;
        let exited = lock.lock().unwrap();
        *cv.wait_timeout_while(exited, timeout, |exited| !*exited).unwrap().0
    }

    /// stdout is at EOF: collect the child, sweep its process group, report.
    fn reap(&self, mut child: std::process::Child, drained: Option<std::thread::JoinHandle<()>>) {
        // Closing stdout is not exiting. Give it the usual grace, then insist.
        let deadline = std::time::Instant::now() + self.launch.exit_grace;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break None,
            }
        };
        // Agents spawn shells that spawn children; none of them outlive it.
        process::kill_group(self.pgid);
        let status = status.or_else(|| child.wait().ok());
        // Let the stderr drain catch up, so a crash report has its last
        // words — briefly: a grandchild that escaped the group may hold the
        // pipe open for good.
        for _ in 0..20 {
            if drained.as_ref().is_none_or(|d| d.is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(self.stdin.lock().unwrap().take());
        let state = self.state.lock().unwrap();
        let reason = match (state.closing, state.hung) {
            (_, true) => ExitReason::Hung,
            (true, _) => ExitReason::Shutdown,
            _ => ExitReason::Crashed(status.map(|s| s.to_string()).unwrap_or_default()),
        };
        drop(state);
        *self.exited.0.lock().unwrap() = true;
        self.exited.1.notify_all();
        let stderr = self.stderr.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
        self.emit(Event::Exited { reason, stderr });
    }
}
