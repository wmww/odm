//! The host against the fake agent (`odm-fake-agent --chat`): a real child
//! process, so these cover the lifecycle the panel depends on — and the
//! pure transcript folding, which needs no process at all.

use super::*;
use crate::state::tests::env;
use odm_agent::Chunk;
use odm_build::View;

/// `odm-fake-agent`, built next to this test binary by `cargo test
/// --workspace` (it is another package's bin, so cargo hands us no path).
fn fake_agent() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let path = exe.parent().and_then(Path::parent).unwrap().join(format!("odm-fake-agent{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "{} is missing: run `cargo test --workspace`", path.display());
    path
}

/// A project dir whose config picks the fake agent. The dir doubles as the
/// "system" config home, so nothing of the user's is read or written.
fn project() -> (tempfile::TempDir, Files) {
    let dir = tempfile::tempdir().unwrap();
    let system = dir.path().join("system/config.toml");
    std::fs::create_dir_all(system.parent().unwrap()).unwrap();
    std::fs::write(
        &system,
        format!(
            "[agent]\ndefault = \"custom\"\npermissions = \"yolo\"\n[agent.custom]\n\
             command = [{:?}, \"--chat\"]\nenv = {{ ODM_FAKE_AGENT_FAST = \"1\" }}\n",
            fake_agent().display().to_string()
        ),
    )
    .unwrap();
    let files = Files { system: Some(system), project: odm_config::project_path(dir.path()) };
    (dir, files)
}

/// `d` stretched by `ODM_TEST_TIMEOUT_SCALE` (CI sets 4: runner cores are
/// slow and shared).
fn scaled(d: Duration) -> Duration {
    let scale = std::env::var("ODM_TEST_TIMEOUT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    d * scale
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + scaled(Duration::from_secs(10));
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn texts(host: &AgentHost) -> Vec<String> {
    host.with_transcript(|items| {
        items
            .iter()
            .filter_map(|i| match i {
                Item::User { text, .. } => Some(format!("> {text}")),
                Item::Agent { text, .. } => Some(text.clone()),
                Item::Engine { text, .. } => Some(format!("engine: {text}")),
                Item::Error(text) => Some(format!("error: {text}")),
                _ => None,
            })
            .collect()
    })
}

#[test]
fn one_conversation_at_a_time() {
    let (dir, files) = project();
    let host = AgentHost::with_files(dir.path(), files.clone());
    assert_eq!(host.lamp(), Lamp::Off, "nothing spawns until the user speaks");
    assert_eq!(host.placeholder(), "Message Custom");

    host.send("make the post taller".into(), Some(json!({"path": "root.js"})));
    assert_eq!(host.lamp(), Lamp::Working, "lit from Enter");
    wait_until("the turn to end", || host.lamp() == Lamp::Idle);
    assert_eq!(
        texts(&host),
        ["> make the post taller", "Done. The post is now 40 mm tall,\nand everything still builds."]
    );
    assert_eq!(host.working(), None);
    assert_eq!(host.placeholder(), "Message Fake Big 1");
    host.with_transcript(|items| {
        let Some(Item::Header(header)) = items.first() else { panic!("{items:?}") };
        assert_eq!(header.agent.as_deref(), Some("Fake Agent"));
        assert_eq!(header.session.as_deref(), Some("fake-1"));
        assert_eq!(header.account.as_deref(), Some("Fake Max"));
        // YOLO found the mode the agent tags `full_access`.
        assert_eq!(header.mode.as_deref(), Some("Bypass"));
    });

    // A different model is a different conversation: new session, clean
    // transcript, and the model set before anything is said.
    host.set_option("model", "small");
    wait_until("the new session", || host.placeholder() == "Message Fake Small 1");
    assert!(texts(&host).is_empty());

    // `/clear` is never sent: it starts over.
    host.send("hello again".into(), None);
    wait_until("the turn to end", || host.lamp() == Lamp::Idle && texts(&host).len() == 2);
    host.send("/clear".into(), None);
    assert!(texts(&host).is_empty());
    host.shutdown(true);

    // Another run of the viewer remembers what the agent is called, and
    // nothing of what was said: the first message opens a new session.
    let host = AgentHost::with_files(dir.path(), files);
    assert_eq!(host.placeholder(), "Message Fake Small 1");
    host.send("and wider".into(), None);
    wait_until("the turn to end", || host.lamp() == Lamp::Idle);
    assert_eq!(texts(&host)[0], "> and wider");
    host.shutdown(true);
}

#[test]
fn a_crash_is_a_red_line_and_the_next_message_respawns() {
    let (dir, files) = project();
    let host = AgentHost::with_files(dir.path(), files);
    host.send("please crash".into(), None);
    wait_until("the agent to die", || host.lamp() == Lamp::Off);
    let said = texts(&host);
    assert!(said[1].starts_with("error: The agent exited"), "{said:?}");
    assert!(said[1].contains("something went badly wrong"), "its stderr tail: {said:?}");
    // What it knew died with it: the respawn starts a clean transcript.
    host.send("again".into(), None);
    wait_until("the turn to end", || host.lamp() == Lamp::Idle);
    assert_eq!(texts(&host)[0], "> again");
    host.shutdown(true);
}

#[test]
fn a_permission_question_waits_for_the_user() {
    let (dir, files) = project();
    let host = AgentHost::with_files(dir.path(), files);
    host.send("ask permission first".into(), None);
    wait_until("the question", || host.lamp() == Lamp::Waiting);
    assert_eq!(host.working().as_deref(), Some("Waiting for you"));
    let (id, option) = host.with_transcript(|items| {
        items
            .iter()
            .find_map(|i| match i {
                Item::Permission { id, options, answer: None, .. } => Some((*id, options[0].clone())),
                _ => None,
            })
            .expect("a pending question")
    });
    host.answer(id, &option);
    wait_until("the turn to end", || host.lamp() == Lamp::Idle);
    host.with_transcript(|items| {
        assert!(items.iter().any(|i| matches!(i, Item::Permission { answer: Some(a), .. } if a == "Allow")));
    });
    host.shutdown(true);
}

#[test]
fn stop_cancels_the_turn() {
    let (dir, files) = project();
    let host = AgentHost::with_files(dir.path(), files);
    host.send("be slow about it".into(), None);
    wait_until("the tool call", || host.working().as_deref() == Some("Write root.js"));
    host.stop();
    wait_until("the turn to end", || host.lamp() == Lamp::Idle);
    host.with_transcript(|items| {
        assert!(items.contains(&Item::Notice("Stopped.".into())), "{items:?}");
        assert!(items.iter().all(|i| !matches!(i, Item::Tool(t) if t.running())));
    });
    host.shutdown(true);
}

/// Every push is a paid turn nobody typed: only into a live idle session,
/// only on a changed non-empty value, and three in a row at most.
#[test]
fn diagnostics_are_pushed_and_bounded() {
    let (dir, files) = project();
    let host = AgentHost::build(dir.path(), files, Duration::from_millis(30));
    let state = EngineState::with_agent(dir.path().to_owned(), env(), host.clone()).unwrap();
    let pusher = {
        let state = state.clone();
        std::thread::spawn(move || state.agent().run_pusher(&state))
    };
    let view = View::of("root.js");
    let pushes = || texts(&host).iter().filter(|t| t.starts_with("engine: told the agent")).count();
    let fail = |error: &str| {
        state.publish_failure(crate::state::DEFAULT_SLOT, None, &view, error.into(), vec![], None)
    };

    // No session: a failure is nobody's news, and it never spawns one.
    fail("boom 0");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!((pushes(), host.lamp()), (0, Lamp::Off));

    // A live session is told what stands once its turn is over — once.
    host.send("hi".into(), None);
    wait_until("the follow-up prompt", || pushes() == 1);
    wait_until("its turn to end", || host.lamp() == Lamp::Idle);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(pushes(), 1, "an unchanged value is not news");

    for (n, error) in ["boom 1", "boom 2"].into_iter().enumerate() {
        fail(error);
        wait_until("the next push", || pushes() == n + 2);
        wait_until("its turn to end", || host.lamp() == Lamp::Idle);
    }
    // Three in a row: the fourth is a line in the panel, and silence.
    fail("boom 3");
    wait_until("the engine to give up", || {
        texts(&host).iter().any(|t| t.starts_with("engine: not forwarding"))
    });
    fail("boom 4");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(pushes(), 3);

    // The user speaking resets the count.
    host.send("try again".into(), None);
    wait_until("forwarding to resume", || pushes() == 4);

    state.stop();
    pusher.join().unwrap();
}

fn chunk(id: Option<&str>, text: &str) -> Chunk {
    Chunk { message_id: id.map(str::to_owned), text: text.to_owned() }
}

#[test]
fn chunks_without_ids_join_only_while_adjacent() {
    let mut items = Vec::new();
    fold(&mut items, Update::AgentChunk(chunk(None, "one ")), 0);
    fold(&mut items, Update::AgentChunk(chunk(None, "two")), 0);
    fold(&mut items, Update::ThoughtChunk(chunk(None, "hm")), 0);
    fold(&mut items, Update::AgentChunk(chunk(None, "three")), 0);
    // The user's own message is already there: echoes are dropped.
    fold(&mut items, Update::UserChunk(chunk(None, "echo")), 0);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], Item::Agent { id: None, text: "one two".into() });
}

/// An `odm …` call shows only when nothing else will: the engine's own
/// action line stands in for one that reached it.
#[test]
fn an_odm_tool_call_yields_to_its_action_line() {
    let call = |status: &str| ToolCall {
        id: "c".into(),
        title: Some("odm render".into()),
        kind: Some("execute".into()),
        status: Some(status.into()),
        content: None,
    };
    let visible = |actions_after: u64, status: &str| {
        let mut items = Vec::new();
        fold(&mut items, Update::ToolCall(call("in_progress")), 5);
        fold(&mut items, Update::ToolCall(call(status)), actions_after);
        matches!(&items[0], Item::Tool(t) if t.visible())
    };
    assert!(!visible(6, "completed"));
    assert!(!visible(6, "failed"), "it reached the engine, which logged the failure");
    assert!(visible(5, "failed"), "a sandbox-denied connect has no action line");
    let mut items = Vec::new();
    let write = ToolCall { title: Some("Write root.js".into()), kind: Some("edit".into()), ..call("completed") };
    fold(&mut items, Update::ToolCall(write), 0);
    assert!(matches!(&items[0], Item::Tool(t) if t.visible()));
}
