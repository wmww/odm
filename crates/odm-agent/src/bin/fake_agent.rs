//! A scripted ACP agent, for tests and for driving the viewer's agent panel
//! without a real agent (or a bill).
//!
//! `odm-fake-agent <scenario.ndjson>` plays a script, one JSON step a line
//! (`#` lines are comments):
//!
//! - `{"reply": "<method>", "result": {…}}` / `"error": {…}` — answer the
//!   client's request of that method, waiting for it if it has not come yet.
//! - `{"expect": "<method>"}` — wait for a message of that method.
//! - `{"notify": "<method>", "params": {…}}`
//! - `{"request": "<method>", "params": {…}}` then
//!   `{"expect_reply": {…}}` — a request of our own, and the result the
//!   client must answer it with (omit the value to accept any).
//! - `{"raw": "text"}`, `{"stderr": "text", "times": N}`, `{"sleep": ms}`,
//!   `{"exit": code}`, `{"hang": true}` (ignore everything, stdin EOF too).
//!
//! At the end of the script it waits for stdin to close and exits 0, as the
//! real adapters do. A failed expectation exits 3 with the reason on stderr.
//!
//! `odm-fake-agent --chat` is a canned conversationalist instead: every
//! prompt gets a thought, a tool call and a reply. Words in the prompt pick
//! extras: `plan`, `permission`, `slow` (a turn long enough to Stop),
//! `crash`. `ODM_FAKE_AGENT_FAST=1` cuts its pauses to a tenth, for tests.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

struct Wire {
    incoming: Receiver<Option<Value>>,
    /// method → id of the client's last request of it.
    requests: HashMap<String, Value>,
    next_id: u64,
    eof: bool,
}

impl Wire {
    fn new() -> Wire {
        let (tx, incoming) = channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(Some(msg));
                }
            }
            let _ = tx.send(None);
        });
        Wire { incoming, requests: HashMap::new(), next_id: 1, eof: false }
    }

    fn send(&self, msg: Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{msg}");
        let _ = out.flush();
    }

    fn note(&mut self, msg: &Value) {
        if let (Some(method), Some(id)) = (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            self.requests.insert(method.to_owned(), id.clone());
        }
    }

    /// The next message, or None at EOF / after `timeout`.
    fn next(&mut self, timeout: Option<Duration>) -> Option<Value> {
        if self.eof {
            return None;
        }
        let msg = match timeout {
            None => self.incoming.recv().ok().flatten(),
            Some(t) => match self.incoming.recv_timeout(t) {
                Ok(msg) => msg,
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => None,
            },
        };
        match &msg {
            Some(msg) => self.note(msg),
            None => self.eof = true,
        }
        msg
    }

    fn wait_for(&mut self, test: impl Fn(&Wire, &Value) -> bool, what: &str) -> Value {
        loop {
            match self.next(None) {
                Some(msg) if test(self, &msg) => return msg,
                Some(_) => {}
                None => fail(&format!("stdin closed while waiting for {what}")),
            }
        }
    }

    fn reply(&mut self, method: &str, body: (&str, Value)) {
        if !self.requests.contains_key(method) {
            self.wait_for(|w, _| w.requests.contains_key(method), method);
        }
        let id = self.requests.remove(method).expect("just seen");
        self.send(json!({"jsonrpc": "2.0", "id": id, body.0: body.1}));
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn update(&self, update: Value) {
        self.notify("session/update", json!({"sessionId": "fake-1", "update": update}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = json!(format!("a{}", self.next_id));
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }
}

fn fail(why: &str) -> ! {
    eprintln!("fake agent: {why}");
    std::process::exit(3);
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| fail("usage: <scenario.ndjson> | --chat"));
    let mut wire = Wire::new();
    if arg == "--chat" {
        return chat(&mut wire);
    }
    let script = std::fs::read_to_string(&arg).unwrap_or_else(|e| fail(&format!("{arg}: {e}")));
    let mut asked: Option<Value> = None;
    for line in script.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let step: Value =
            serde_json::from_str(line).unwrap_or_else(|e| fail(&format!("bad step `{line}`: {e}")));
        let text = |key: &str| step.get(key).and_then(Value::as_str);
        let params = || step.get("params").cloned().unwrap_or(Value::Null);
        if let Some(method) = text("reply") {
            let body = match step.get("error") {
                Some(error) => ("error", error.clone()),
                None => ("result", step.get("result").cloned().unwrap_or(json!({}))),
            };
            wire.reply(method, body);
        } else if let Some(method) = text("expect") {
            wire.wait_for(|_, m| m.get("method").and_then(Value::as_str) == Some(method), method);
        } else if let Some(method) = text("notify") {
            wire.notify(method, params());
        } else if let Some(method) = text("request") {
            asked = Some(wire.request(method, params()));
        } else if let Some(want) = step.get("expect_reply") {
            let id = asked.take().unwrap_or_else(|| fail("expect_reply without a request"));
            let got = wire.wait_for(|_, m| m.get("id") == Some(&id) && m.get("method").is_none(), "a reply");
            if !want.is_null() && got.get("result") != Some(want) {
                fail(&format!("reply was {got}, wanted result {want}"));
            }
        } else if let Some(raw) = text("raw") {
            println!("{raw}");
        } else if let Some(line) = text("stderr") {
            for _ in 0..step.get("times").and_then(Value::as_u64).unwrap_or(1) {
                eprintln!("{line}");
            }
        } else if let Some(ms) = step.get("sleep").and_then(Value::as_u64) {
            std::thread::sleep(Duration::from_millis(ms));
        } else if let Some(code) = step.get("exit").and_then(Value::as_i64) {
            std::process::exit(code as i32);
        } else if step.get("hang").is_some() {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        } else {
            fail(&format!("unknown step `{line}`"));
        }
    }
    while wire.next(None).is_some() {}
}

// --- chat mode ---

fn options(mode: &str, model: &str) -> Value {
    json!([
        {"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": mode,
         "options": [{"value": "default", "name": "Manual"}, {"value": "acceptEdits", "name": "Accept Edits"},
                     {"value": "plan", "name": "Plan"}]},
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
         "options": [{"value": "default", "name": "Default (recommended)"}, {"value": "big", "name": "Fake Big 1"},
                     {"value": "small", "name": "Fake Small 1"}]},
    ])
}

fn chunk(kind: &str, id: &str, text: &str) -> Value {
    json!({"sessionUpdate": kind, "messageId": id, "content": {"type": "text", "text": text}})
}

fn chat(wire: &mut Wire) {
    let (mut mode, mut model) = ("default".to_owned(), "big".to_owned());
    let mut turn = 0u32;
    while let Some(msg) = wire.next(None) {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match method.as_str() {
            "initialize" => {
                wire.reply(&method, ("result", json!({
                    "protocolVersion": 1,
                    "agentInfo": {"name": "fake", "title": "Fake Agent", "version": "0.1.0"},
                    "agentCapabilities": {"loadSession": true},
                    "authMethods": [],
                    "_meta": {"steering": {"supported": true}},
                })));
                wire.notify("_auth/status_update", json!({"authStatus":
                    {"kind": "subscription", "label": "Fake Max", "email": "user@example.com"}}));
            }
            "session/new" => wire.reply(&method, ("result", json!({
                "sessionId": "fake-1", "configOptions": options(&mode, &model)}))),
            "session/load" => {
                wire.update(chunk("user_message_chunk", "h1", "make the post taller"));
                wire.update(chunk("user_message_chunk", "h1", "odm://user-state"));
                wire.update(chunk("agent_message_chunk", "h2", "Done: the post is 40 mm now."));
                wire.reply(&method, ("result", json!({"configOptions": options(&mode, &model)})));
            }
            "session/set_config_option" => {
                let value = params["value"].as_str().unwrap_or_default().to_owned();
                match params["configId"].as_str() {
                    Some("mode") => mode = value,
                    _ => model = value,
                }
                wire.reply(&method, ("result", json!({"configOptions": options(&mode, &model)})));
            }
            "session/prompt" => {
                turn += 1;
                let stop = run_turn(wire, turn, &params);
                wire.reply("session/prompt", ("result", json!({"stopReason": stop})));
            }
            _ if msg.get("id").is_some() => {
                wire.reply(&method, ("error", json!({"code": -32601, "message": "method not found"})));
            }
            _ => {}
        }
    }
}

/// Wait out `ms`, watching for a cancel or a steering message. True = cancelled.
fn idle(wire: &mut Wire, turn: u32, ms: u64) -> bool {
    let ms = if std::env::var_os("ODM_FAKE_AGENT_FAST").is_some() { ms / 10 } else { ms };
    let deadline = std::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        let Some(msg) = wire.next(Some(left)) else {
            if wire.eof {
                std::process::exit(0);
            }
            return false;
        };
        match msg.get("method").and_then(Value::as_str) {
            Some("session/cancel") => return true,
            Some("_session/steering") => {
                wire.reply("_session/steering", ("result", json!({"outcome": "injected"})));
                let text = msg.pointer("/params/prompt/0/text").and_then(Value::as_str).unwrap_or("");
                wire.update(chunk("agent_message_chunk", &format!("s{turn}"), &format!("Steered: {text}\n")));
            }
            _ => {}
        }
    }
}

fn run_turn(wire: &mut Wire, turn: u32, params: &Value) -> &'static str {
    let text = params.pointer("/prompt/0/text").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let has = |word: &str| text.contains(word);
    let id = |part: &str| format!("{part}{turn}");
    wire.update(chunk("agent_thought_chunk", &id("t"), "The user wants something. Looking at the project first."));
    if idle(wire, turn, 300) {
        return "cancelled";
    }
    if has("crash") {
        eprintln!("fake agent: something went badly wrong");
        std::process::exit(1);
    }
    if has("plan") {
        wire.update(json!({"sessionUpdate": "plan", "entries": [
            {"content": "Read the project", "priority": "medium", "status": "completed"},
            {"content": "Stretch the post", "priority": "medium", "status": "in_progress"},
            {"content": "Render a check", "priority": "medium", "status": "pending"}]}));
    }
    let call = id("c");
    wire.update(json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": "Write root.js",
        "kind": "edit", "status": "pending"}));
    if has("permission") {
        let asked = wire.request("session/request_permission", json!({
            "sessionId": "fake-1",
            "toolCall": {"toolCallId": call, "title": "Write root.js", "kind": "edit"},
            "options": [
                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "always", "name": "Always allow", "kind": "allow_always"},
                {"optionId": "reject", "name": "Reject", "kind": "reject_once"}],
        }));
        let reply = wire.wait_for(|_, m| m.get("id") == Some(&asked) && m.get("method").is_none(), "a reply");
        if reply.pointer("/result/outcome/optionId").and_then(Value::as_str) != Some("allow")
            && reply.pointer("/result/outcome/optionId").and_then(Value::as_str) != Some("always")
        {
            wire.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed"}));
            wire.update(chunk("agent_message_chunk", &id("m"), "Okay, leaving root.js alone."));
            return if reply.pointer("/result/outcome/outcome").and_then(Value::as_str) == Some("cancelled") {
                "cancelled"
            } else {
                "end_turn"
            };
        }
    }
    if idle(wire, turn, if has("slow") { 30_000 } else { 300 }) {
        wire.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed"}));
        return "cancelled";
    }
    wire.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
        "content": [{"type": "diff", "path": "root.js", "oldText": "height: 30", "newText": "height: 40"}]}));
    for part in ["Done. ", "The post is now 40 mm tall,\n", "and everything still builds."] {
        wire.update(chunk("agent_message_chunk", &id("m"), part));
        if idle(wire, turn, 150) {
            return "cancelled";
        }
    }
    wire.update(json!({"sessionUpdate": "usage_update", "used": 12_000 + turn * 900, "size": 200_000}));
    "end_turn"
}
