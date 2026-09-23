//! The ACP client against the scripted fake agent: a real child process, real
//! pipes, no network and no bill.

use odm_agent::{Agent, Event, ExitReason, Launch, SessionOptions, Update};
use serde_json::json;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_odm-fake-agent");

/// The opening every scenario shares.
const HANDSHAKE: &str = r#"
{"reply": "initialize", "result": {"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1"}, "agentCapabilities": {"loadSession": true}, "_meta": {"steering": {"supported": true}}}}
"#;
const NEW: &str = r#"
{"reply": "session/new", "result": {"sessionId": "s1", "configOptions": [{"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{"value": "default", "name": "Manual"}, {"value": "plan", "name": "Plan"}]}]}}
"#;

/// `d` stretched by `ODM_TEST_TIMEOUT_SCALE` (CI sets 4: runner cores are
/// slow and shared).
fn scaled(d: Duration) -> Duration {
    let scale = std::env::var("ODM_TEST_TIMEOUT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    d * scale
}

struct Run {
    agent: Agent,
    events: Receiver<Event>,
    _dir: tempfile::TempDir,
}

fn run_with(script: &str, options: SessionOptions, tune: impl FnOnce(&mut Launch)) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scenario.ndjson");
    std::fs::write(&path, script).unwrap();
    let mut launch =
        Launch::new(vec![FAKE.to_owned(), path.display().to_string()], dir.path().to_owned());
    launch.cancel_grace = scaled(Duration::from_millis(300));
    launch.exit_grace = scaled(Duration::from_millis(300));
    tune(&mut launch);
    let (agent, events) = Agent::spawn(launch, options, Arc::new(|| {})).unwrap();
    Run { agent, events, _dir: dir }
}

fn run(script: &str) -> Run {
    run_with(script, SessionOptions::default(), |_| {})
}

impl Run {
    /// Events up to and including the first that `last` accepts. A scenario
    /// that fails its own expectations exits 3, which lands here as a panic.
    fn until(&self, last: impl Fn(&Event) -> bool) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = self
                .events
                .recv_timeout(scaled(Duration::from_secs(10)))
                .unwrap_or_else(|_| panic!("timed out; saw {seen:#?}"));
            let done = last(&event);
            if let Event::Exited { reason: ExitReason::Crashed(status), stderr } = &event
                && !done
            {
                panic!("agent died ({status}): {stderr}\nsaw {seen:#?}");
            }
            seen.push(event);
            if done {
                return seen;
            }
        }
    }

    fn until_exit(&self) -> (ExitReason, String) {
        match self.until(|e| matches!(e, Event::Exited { .. })).pop() {
            Some(Event::Exited { reason, stderr }) => (reason, stderr),
            _ => unreachable!(),
        }
    }
}

fn text(s: &str) -> Vec<serde_json::Value> {
    vec![json!({"type": "text", "text": s})]
}

fn turn_ended(e: &Event) -> bool {
    matches!(e, Event::TurnEnded { .. })
}

#[test]
fn a_prompt_streams_and_ends_with_its_response() {
    let script = format!(
        r#"{HANDSHAKE}{NEW}
{{"expect": "session/prompt"}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {{"type": "text", "text": "Hel"}}}}}}}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "something_from_the_future", "x": 1}}}}}}
{{"notify": "_vendor/whatever", "params": {{}}}}
{{"raw": "not json at all"}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "odm status", "kind": "execute", "status": "pending"}}}}}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {{"type": "text", "text": "lo"}}}}}}}}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
"#
    );
    let run = run(&script);
    // Sent before the session exists: it waits for it.
    run.agent.prompt(text("hi"));
    let events = run.until(turn_ended);
    let said: String = events
        .iter()
        .filter_map(|e| match e {
            Event::Update { update: Update::AgentChunk(c), replay: false } => Some(c.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(said, "Hello");
    assert!(events.iter().any(|e| matches!(e, Event::Initialized(info) if info.steering && info.title.as_deref() == Some("fake"))));
    assert!(events.contains(&Event::SessionStarted { id: "s1".into(), resumed: false }));
    assert!(events.iter().any(
        |e| matches!(e, Event::Update { update: Update::ToolCall(t), .. } if t.title.as_deref() == Some("odm status"))
    ));
    assert_eq!(events.last(), Some(&Event::TurnEnded { stop_reason: "end_turn".into() }));
    // Both adapters exit on stdin close; so does the fake.
    run.agent.shutdown();
    assert_eq!(run.until_exit().0, ExitReason::Shutdown);
}

#[test]
fn a_permission_request_round_trips() {
    let script = format!(
        r#"{HANDSHAKE}{NEW}
{{"expect": "session/prompt"}}
{{"request": "session/request_permission", "params": {{"sessionId": "s1", "toolCall": {{"toolCallId": "c1", "title": "touch x"}}, "options": [{{"optionId": "yes", "name": "Allow", "kind": "allow_once"}}, {{"optionId": "no", "name": "Reject", "kind": "reject_once"}}]}}}}
{{"expect_reply": {{"outcome": {{"outcome": "selected", "optionId": "yes"}}}}}}
{{"request": "fs/read_text_file", "params": {{"path": "/etc/passwd"}}}}
{{"expect_reply": null}}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
"#
    );
    let run = run(&script);
    run.agent.prompt(text("go"));
    let events = run.until(|e| matches!(e, Event::Permission { .. }));
    let Some(Event::Permission { id, tool, options }) = events.last() else { unreachable!() };
    assert_eq!(tool.title.as_deref(), Some("touch x"));
    assert_eq!(options.len(), 2);
    run.agent.answer_permission(*id, Some("yes"));
    run.agent.answer_permission(*id, Some("no")); // a second answer is dropped
    run.until(turn_ended);
}

#[test]
fn cancel_closes_open_permissions_and_the_turn_ends_cancelled() {
    let script = format!(
        r#"{HANDSHAKE}{NEW}
{{"expect": "session/prompt"}}
{{"request": "session/request_permission", "params": {{"toolCall": {{"toolCallId": "c1"}}, "options": []}}}}
{{"expect": "session/cancel"}}
{{"expect_reply": {{"outcome": {{"outcome": "cancelled"}}}}}}
{{"reply": "session/prompt", "result": {{"stopReason": "cancelled"}}}}
"#
    );
    let run = run(&script);
    run.agent.prompt(text("go"));
    run.until(|e| matches!(e, Event::Permission { .. }));
    run.agent.cancel();
    let events = run.until(turn_ended);
    assert_eq!(events.last(), Some(&Event::TurnEnded { stop_reason: "cancelled".into() }));
}

/// No timer ever ends a turn — but a turn that ignores its cancel is killed.
#[test]
fn a_prompt_that_ignores_cancel_is_killed() {
    let script = format!("{HANDSHAKE}{NEW}\n{{\"expect\": \"session/prompt\"}}\n{{\"hang\": true}}\n");
    let run = run(&script);
    run.agent.prompt(text("go"));
    run.until(|e| matches!(e, Event::TurnStarted));
    run.agent.cancel();
    assert_eq!(run.until_exit().0, ExitReason::Hung);
}

#[test]
fn steering_outcomes() {
    // injected: nothing more to do. promptRequired and failed: ours to
    // deliver — as one ordinary prompt once the turn is over.
    let script = format!(
        r#"{HANDSHAKE}{NEW}
{{"expect": "session/prompt"}}
{{"reply": "_session/steering", "result": {{"outcome": "injected"}}}}
{{"reply": "_session/steering", "result": {{"outcome": "failed"}}}}
{{"reply": "_session/steering", "error": {{"code": -32603, "message": "boom"}}}}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
{{"expect": "session/prompt"}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "agent_message_chunk", "content": {{"type": "text", "text": "second turn"}}}}}}}}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
"#
    );
    let run = run(&script);
    run.agent.prompt(text("go"));
    run.until(|e| matches!(e, Event::TurnStarted));
    for word in ["one", "two", "three"] {
        run.agent.prompt(text(word));
    }
    run.until(turn_ended);
    let events = run.until(turn_ended);
    assert!(events.contains(&Event::TurnStarted), "the undelivered ones became a prompt: {events:#?}");
}

#[test]
fn a_crash_mid_turn_reports_the_stderr_tail() {
    let script = format!(
        "{HANDSHAKE}{NEW}\n{{\"expect\": \"session/prompt\"}}\n{{\"stderr\": \"out of cheese\"}}\n{{\"exit\": 7}}\n"
    );
    let run = run(&script);
    run.agent.prompt(text("go"));
    let (reason, stderr) = run.until_exit();
    assert!(matches!(reason, ExitReason::Crashed(status) if status.contains('7')), "{stderr}");
    assert!(stderr.contains("out of cheese"));
}

/// An undrained stderr pipe would block the child at ~64 KB.
#[test]
fn a_stderr_flood_does_not_block_the_agent() {
    let script = format!(
        r#"{HANDSHAKE}{NEW}
{{"expect": "session/prompt"}}
{{"stderr": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "times": 5000}}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
"#
    );
    let run = run(&script);
    run.agent.prompt(text("go"));
    run.until(turn_ended);
}

#[test]
fn load_replays_then_falls_back_to_a_new_session() {
    let resume = SessionOptions { resume: Some("old".into()), mode: Some("plan".into()), ..Default::default() };
    // Loads: history arrives flagged as replay, and the saved mode is applied.
    let script = format!(
        r#"{HANDSHAKE}
{{"expect": "session/load"}}
{{"notify": "session/update", "params": {{"update": {{"sessionUpdate": "user_message_chunk", "messageId": "u1", "content": {{"type": "text", "text": "earlier"}}}}}}}}
{{"reply": "session/load", "result": {{"modes": {{"currentModeId": "default", "availableModes": [{{"id": "default", "name": "Manual"}}, {{"id": "plan", "name": "Plan"}}]}}}}}}
{{"reply": "session/set_mode", "result": {{}}}}
"#
    );
    let run = run_with(&script, resume.clone(), |_| {});
    let events = run.until(|e| matches!(e, Event::SessionStarted { .. }));
    assert!(events.iter().any(|e| matches!(e, Event::Update { replay: true, .. })));
    assert_eq!(events.last(), Some(&Event::SessionStarted { id: "old".into(), resumed: true }));
    run.agent.shutdown();
    run.until_exit(); // …having answered session/set_mode, or the fake exits 3

    // Gone: a fresh session, and the panel is told.
    let script = format!(
        "{HANDSHAKE}\n{{\"reply\": \"session/load\", \"error\": {{\"code\": -32002, \"message\": \"not found\"}}}}{NEW}"
    );
    let run = run_with(&script, resume, |_| {});
    let events = run.until(|e| matches!(e, Event::SessionStarted { .. }));
    assert!(events.contains(&Event::SessionLost));
    assert_eq!(events.last(), Some(&Event::SessionStarted { id: "s1".into(), resumed: false }));
}

#[test]
fn logged_out_then_retried_by_the_next_prompt() {
    let script = format!(
        r#"{HANDSHAKE}
{{"reply": "session/new", "error": {{"code": -32000, "message": "Authentication required"}}}}
{NEW}
{{"reply": "session/prompt", "result": {{"stopReason": "end_turn"}}}}
"#
    );
    let run = run(&script);
    let events = run.until(|e| matches!(e, Event::SessionFailed { .. }));
    assert!(matches!(events.last(), Some(Event::SessionFailed { auth_required: true, .. })));
    run.agent.prompt(text("I logged in"));
    run.until(turn_ended);
}

#[test]
fn the_agents_odm_is_ours() {
    // `path_prepend` wins over the inherited PATH: a shell script agent
    // reports which `odm` it would run.
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("odm"), "").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin.join("odm"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut launch = Launch::new(
        vec!["sh".into(), "-c".into(), "command -v odm >&2; pwd >&2; exit 1".into()],
        dir.path().to_owned(),
    );
    launch.path_prepend = Some(bin.clone());
    let (_agent, events) = Agent::spawn(launch, SessionOptions::default(), Arc::new(|| {})).unwrap();
    let stderr = loop {
        if let Event::Exited { stderr, .. } = events.recv_timeout(scaled(Duration::from_secs(10))).unwrap() {
            break stderr;
        }
    };
    let cwd = dir.path().canonicalize().unwrap();
    assert_eq!(stderr, format!("{}\n{}", bin.join("odm").display(), cwd.display()));
}

#[test]
fn a_missing_program_fails_the_spawn() {
    let launch = Launch::new(vec!["/nonexistent/odm-agent".into()], std::env::temp_dir());
    assert!(Agent::spawn(launch, SessionOptions::default(), Arc::new(|| {})).is_err());
}
