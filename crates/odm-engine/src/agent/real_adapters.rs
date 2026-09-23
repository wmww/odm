//! The real npm adapters, logged out: install, spawn, handshake, the
//! auth-required outcome, and a group kill that leaves nothing behind. No
//! credentials and no API calls. Needs network + npm (~600 MiB), so opt-in:
//! `cargo test --workspace real_adapters -- --ignored` (test.yml's
//! `real_adapters` input).

use super::table::{self, BUILT_IN, Source};
use odm_agent::{Agent, Event, ExitReason, Launch, SessionOptions};
use serde_json::json;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

fn scaled(d: Duration) -> Duration {
    let scale = std::env::var("ODM_TEST_TIMEOUT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    d * scale
}

/// Events until `last` accepts one; panics on exit or timeout.
fn until(events: &Receiver<Event>, what: &str, last: impl Fn(&Event) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + scaled(Duration::from_secs(60));
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(left).unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {seen:#?}"));
        let done = last(&event);
        if let Event::Exited { .. } = &event
            && !done
        {
            panic!("agent exited while waiting for {what}: {event:#?}\nsaw {seen:#?}");
        }
        seen.push(event);
        if done {
            return seen;
        }
    }
}

/// Live (non-zombie) members of process group `pgid`, as `pid comm`.
#[cfg(target_os = "linux")]
fn group_alive(pgid: u32) -> Vec<String> {
    let mut alive = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // pid (comm) state ppid pgrp ...; comm may hold spaces and parens.
        let Some((head, rest)) = stat.rsplit_once(')') else { continue };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() > 2 && fields[0] != "Z" && fields[2] == pgid.to_string() {
            alive.push(head.replace(" (", " "));
        }
    }
    alive
}

#[test]
#[ignore = "installs the real adapters from npm; run with --ignored"]
fn real_adapters_start_logged_out_and_die_cleanly() {
    if let Some(problem) = table::node_problem() {
        panic!("{problem}");
    }
    for agent in BUILT_IN {
        let Source::Npm { package, bin, .. } = &agent.source else { continue };
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("agents").join(agent.id);
        table::install_into(agent.id, &install).unwrap();
        assert!(table::installed_version(&install, package).is_some(), "{package} not installed");

        // An empty home: no credentials to find, so nothing can be billed.
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let mut launch = Launch::new(table::npm_command(&install, bin), project);
        let home_str = home.display().to_string();
        for var in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            launch.env.insert(var.into(), home_str.clone());
        }
        for var in ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "OPENAI_API_KEY", "CODEX_API_KEY"] {
            launch.env.insert(var.into(), String::new());
        }
        launch.exit_grace = scaled(Duration::from_secs(2));
        let (live, events) = Agent::spawn(launch, SessionOptions::default(), std::sync::Arc::new(|| {})).unwrap();
        let pgid = live.pid();

        let mut seen = until(&events, "initialize", |e| matches!(e, Event::Initialized(_)));
        // Logged out shows at session/new, or — for an adapter that makes
        // sessions regardless — when the first prompt runs.
        let session = until(&events, "a session or the auth failure", |e| {
            matches!(e, Event::SessionFailed { .. } | Event::SessionStarted { .. })
        });
        let failed_early = matches!(session.last(), Some(Event::SessionFailed { .. }));
        seen.extend(session);
        if !failed_early {
            live.prompt(vec![json!({"type": "text", "text": "hi"})]);
            seen.extend(until(&events, "the first turn", |e| matches!(e, Event::TurnEnded { .. })));
        }
        let auth = match seen.last() {
            Some(Event::SessionFailed { auth_required, .. }) => *auth_required,
            Some(Event::TurnEnded { stop_reason }) => stop_reason.to_lowercase().contains("auth"),
            _ => false,
        };
        assert!(auth, "{}: not the logged-out outcome: {seen:#?}", agent.id);

        live.shutdown_wait();
        let exit = until(&events, "exit", |e| matches!(e, Event::Exited { .. }));
        let Some(Event::Exited { reason, .. }) = exit.last() else { unreachable!() };
        assert_eq!(*reason, ExitReason::Shutdown, "{}", agent.id);
        #[cfg(target_os = "linux")]
        {
            let deadline = Instant::now() + scaled(Duration::from_secs(2));
            while !group_alive(pgid).is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            let left = group_alive(pgid);
            assert!(left.is_empty(), "{}: outlived shutdown: {left:?}", agent.id);
        }
    }
}
