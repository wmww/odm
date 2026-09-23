//! End to end through the shipped binary: spawn `odm run --headless`, then
//! drive it with the same `odm <cmd>` invocations an agent types. This is the
//! only place arg dispatch, engine startup, the transport lifecycle and the
//! inotify watcher run as shipped.
//!
//! Assertions are structural — exit codes, JSON fields, substrings that name
//! a fix command. Wording is guarded elsewhere (`cli_reference_is_current`
//! diffs docs/cli.md against the parser), so nothing here pins prose.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_odm");

const BOX10: &str = "//! ODM API unstable\nexport default () => odm.box(10);\n";
const BOX20: &str = "//! ODM API unstable\nexport default () => odm.box(20);\n";

/// `ODM_TEST_TIMEOUT_SCALE` (default 1) stretches every deadline, for slow
/// CI machines.
fn deadline(secs: u64) -> Instant {
    let scale: f64 = std::env::var("ODM_TEST_TIMEOUT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    Instant::now() + Duration::from_secs(secs).mul_f64(scale)
}

fn tempdir() -> TempDir {
    tempfile::Builder::new().prefix("odm-e2e-").tempdir().unwrap()
}

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = tempdir();
    write(dir.path(), "odm.toml", "name = \"e2e\"\nengine = 0\n");
    for (name, body) in files {
        write(dir.path(), name, body);
    }
    dir
}

/// A project seeded from one of the repo's examples.
fn example_project(name: &str) -> TempDir {
    let dir = tempdir();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    copy_dir(&src, dir.path());
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_dir(&entry.path(), &dst);
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &dst).unwrap();
        }
    }
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A headless engine, killed when the guard drops. One per test: tests are
/// threads in one process and each holds its own temp project.
struct Engine {
    child: Child,
    project: PathBuf,
}

impl Engine {
    fn start(project: &Path) -> Engine {
        let child = Command::new(BIN)
            .args(["run", &project.display().to_string(), "--headless"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn odm run");
        let mut engine = Engine { child, project: project.to_path_buf() };
        engine.await_engine();
        engine
    }

    /// Up once `odm status` answers: the real client path, nothing
    /// platform-specific.
    fn await_engine(&mut self) {
        let deadline = deadline(10);
        while Instant::now() < deadline {
            if odm(&self.project, &["status"]).code == 0 {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("engine exited early ({status}):\n{}", self.drain_stderr());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("engine never answered `status`:\n{}", self.drain_stderr());
    }

    fn drain_stderr(&mut self) -> String {
        use std::io::Read;
        let mut buf = String::new();
        if let Some(err) = self.child.stderr.as_mut() {
            let _ = err.read_to_string(&mut buf);
        }
        buf
    }
}

impl Drop for Engine {
    /// SIGKILL (TerminateProcess on Windows): the headless engine installs no
    /// signal handler, so this is also what a crash looks like — the stale
    /// files it leaves behind are what `stale_state_is_reclaimed` exercises.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}\n{}", self.stdout, self.stderr))
    }
    fn ok_json(&self) -> Value {
        assert_eq!(self.code, 0, "expected success:\n{}\n{}", self.stdout, self.stderr);
        self.json()
    }
}

/// Run the client against a project. `args` is everything after `odm`.
fn odm(project: &Path, args: &[&str]) -> Out {
    let mut cmd = Command::new(BIN);
    cmd.arg("--project").arg(project).args(args);
    let out = cmd.output().expect("run odm");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The root node's volume from `odm inspect`.
fn root_volume(project: &Path) -> f64 {
    let v = odm(project, &["inspect", r#"{"fields":["volume"]}"#]).ok_json();
    v["node"]["volume"].as_f64().expect("root volume")
}

// ---------------------------------------------------------------- 1. startup

#[test]
fn engine_starts_and_answers_status() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    let status = await_published(dir.path());
    assert_eq!(status["ok"], Value::Bool(true));
    let first = &status["views"][0];
    assert_eq!(first["build"], Value::String("ok".into()), "{status}");
    assert_eq!(first["path"], Value::String("root.js".into()));
}

/// `status` never builds — it reports the last *published* value, and the
/// background build loop publishes the default slot shortly after startup.
/// Poll until it has settled (or the deadline calls it a hang).
fn await_published(project: &Path) -> Value {
    let deadline = deadline(10);
    loop {
        let status = odm(project, &["status"]).ok_json();
        if status["views"][0]["build"] != "pending" {
            return status;
        }
        assert!(Instant::now() < deadline, "the default slot never left `pending`: {status}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ------------------------------------------------------- 2. sync on query

/// The headline invariant: every query rescans first, so a query can never
/// answer from anything older than the last save. No sleep, no sync step.
#[test]
fn an_edit_is_visible_to_the_very_next_query() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    assert_eq!(root_volume(dir.path()), 1000.0);
    write(dir.path(), "root.js", BOX20);
    assert_eq!(root_volume(dir.path()), 8000.0, "the next query must see the edit");
}

#[test]
fn editing_a_part_is_visible_to_the_very_next_query() {
    let dir = example_project("assembly");
    let _engine = Engine::start(dir.path());

    let before = root_volume(dir.path());
    let wheel = std::fs::read_to_string(dir.path().join("parts/wheel.js")).unwrap();
    let thicker = wheel.replace(".cylinder(r, 6)", ".cylinder(r, 9)");
    assert_ne!(thicker, wheel, "wheel.js's tire is no longer `cylinder(r, 6)`; pick another dimension");
    write(dir.path(), "parts/wheel.js", &thicker);
    assert_ne!(root_volume(dir.path()), before, "a part edit must reach the root view");
}

/// Sync must notice deletions, not just edits: a renamed part breaks the
/// invoke that names it, and renaming it back heals with no other action.
#[test]
fn renaming_a_part_breaks_and_unbreaks_the_next_query() {
    let dir = example_project("assembly");
    let _engine = Engine::start(dir.path());
    let before = root_volume(dir.path());

    let (from, to) = (dir.path().join("parts/wheel.js"), dir.path().join("parts/hub.js"));
    std::fs::rename(&from, &to).unwrap();
    let broken = odm(dir.path(), &["inspect", r#"{"fields":["volume"]}"#]);
    assert_ne!(broken.code, 0, "a missing part must fail the build:\n{}", broken.stdout);
    let msg = broken.json()["error"]["message"].as_str().unwrap_or_default().to_owned();
    assert!(msg.contains("parts/wheel.js"), "the error must name the missing path: {msg}");

    std::fs::rename(&to, &from).unwrap();
    assert_eq!(root_volume(dir.path()), before, "renaming back must heal it");
}

// ------------------------------------------------------------- 3. the watcher
//
// Every CLI command syncs, so nothing here can tell the inotify watcher from
// the query that asked: `the_watcher_rebuilds_without_any_query` lives in
// odm-engine's state.rs, where a published slot can be watched directly.

// ------------------------------------------------------- 4. failure/recovery

#[test]
fn a_thrown_build_fails_loudly_and_heals() {
    const BOOM: &str = "//! ODM API unstable\n\
        export const meta = { inputs: { size: { type: 'number', default: 3 } } };\n\
        export default () => { throw new Error('boom'); };\n";
    let dir = project(&[("root.js", BOOM)]);
    let _engine = Engine::start(dir.path());

    let out = odm(dir.path(), &["inspect"]);
    assert_ne!(out.code, 0, "a throwing build must exit nonzero");
    let v = out.json();
    assert_eq!(v["ok"], Value::Bool(false));
    assert!(
        v["error"]["message"].as_str().unwrap_or_default().contains("boom"),
        "the thrown message must survive: {v}"
    );
    // meta still evaluated, so the failure carries the declared interface —
    // what an agent needs to fix a wrong input (docs/cli.md "Errors").
    let inputs = v["inputs"].as_array().expect("a failed build still reports inputs");
    assert!(inputs.iter().any(|i| i["name"] == "size"), "{v}");

    let status = await_published(dir.path());
    assert_eq!(status["views"][0]["build"], Value::String("error".into()), "{status}");

    write(dir.path(), "root.js", BOX10);
    assert_eq!(root_volume(dir.path()), 1000.0, "healing the file heals the view");
}

// ---------------------------------------------------------------- 5. render

#[test]
fn render_writes_a_deterministic_png() {
    if std::env::var("ODM_TEST_NO_GPU").as_deref() == Ok("1") {
        eprintln!("ODM_TEST_NO_GPU=1: skipping the e2e render case");
        return;
    }
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());
    let out = dir.path().join("a.png");
    let req = json!({"out": out, "width": 64, "height": 64}).to_string();

    odm(dir.path(), &["render", &req]).ok_json();
    let first = std::fs::read(&out).expect("render wrote its file");
    let (w, h, pixels) = decode(&first);
    assert_eq!((w, h), (64, 64));
    let distinct: std::collections::HashSet<&[u8]> = pixels.chunks_exact(4).collect();
    assert!(distinct.len() > 1, "the render is one flat color — nothing was drawn");

    odm(dir.path(), &["render", &req]).ok_json();
    assert_eq!(first, std::fs::read(&out).unwrap(), "same request, same bytes");
}

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

// ------------------------------------------------------------------ 6. chat

/// The chat loop went with the managed agent panel. An agent that still
/// types the old commands must be told why, engine or no engine — not get a
/// JSON-grammar error about its flags.
#[test]
fn poll_and_say_say_where_they_went() {
    let dir = project(&[("root.js", BOX10)]);
    for args in [&["poll", "--follow"][..], &["say", "hello", "there"]] {
        let out = odm(dir.path(), args);
        assert_ne!(out.code, 0);
        assert!(out.stderr.contains("runs the agent itself"), "{}", out.stderr);
    }
}

// ----------------------------------------------------------------- 7. errors

#[test]
fn no_engine_names_the_command_that_starts_one() {
    let dir = project(&[("root.js", BOX10)]);
    let out = odm(dir.path(), &["status"]);
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert!(out.stderr.contains("odm run"), "the fix must be in the message: {}", out.stderr);
}

#[test]
fn a_second_engine_refuses_and_leaves_the_first_serving() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    let out = Command::new(BIN)
        .args(["run", &dir.path().display().to_string(), "--headless"])
        .output()
        .expect("run odm");
    assert!(!out.status.success(), "a second engine must refuse the project");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already running"), "{stderr}");

    odm(dir.path(), &["status"]).ok_json();
}

/// `odm export '{…}'` is the engine's STL export; `odm export --web` stays
/// the standalone site export. Without `--project`, as a user types it.
#[test]
fn export_writes_an_stl_in_the_projects_units() {
    let dir = project(&[("root.js", BOX10)]);
    write(dir.path(), "odm.toml", "name = \"e2e\"\nengine = 0\nunits = \"in\"\n");
    let _engine = Engine::start(dir.path());
    let run = |args: &[&str]| {
        let out = Command::new(BIN).args(args).current_dir(dir.path()).output().expect("run odm");
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    assert_eq!(odm(dir.path(), &["status"]).ok_json()["units"], "in");
    // `out` is relative to the CLI's cwd; the unit is the project's.
    let v = run(&["export", r#"{"out": "part.stl"}"#]).ok_json();
    assert_eq!((&v["units"], &v["union"], &v["bodies"], &v["tris"]), (&json!("in"), &json!(true), &json!(1), &json!(12)));
    assert_eq!(v["size_mm"], json!([254.0, 254.0, 254.0]));
    let file = std::fs::read(dir.path().join("part.stl")).unwrap();
    assert_eq!(file.len(), 84 + 12 * 50);
    assert!(file.starts_with(b"ODM e2e root.js\0"));

    // A request's `units` beats the project's.
    let v = run(&["export", r#"{"out": "part.stl", "units": "mm", "union": false}"#]).ok_json();
    assert_eq!((&v["units"], &v["union"]), (&json!("mm"), &json!(false)));
    assert_eq!(v["size_mm"], json!([10.0, 10.0, 10.0]));

    let err = run(&["export", r#"{"out": "part.obj"}"#]);
    assert_ne!(err.code, 0);
    assert!(err.stdout.contains(".stl"), "{}", err.stdout);
    let err = run(&["export", r#"{"out": "root.js"}"#]);
    assert_ne!(err.code, 0);
    assert_eq!(std::fs::read_to_string(dir.path().join("root.js")).unwrap(), BOX10);

    // Flags still mean the web export, which has its own usage error.
    let web = run(&["export", "--nonsense"]);
    assert!(web.stderr.contains("unknown option --nonsense for export"), "{}", web.stderr);
    let bare = run(&["export"]);
    assert!(bare.stderr.contains("--web") && bare.stderr.contains("part.stl"), "{}", bare.stderr);
}

#[test]
fn bad_commands_and_bad_json_fail_with_something_to_read() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    // Unknown commands are forwarded: the engine is the one error path.
    let unknown = odm(dir.path(), &["bogus"]);
    assert_ne!(unknown.code, 0);
    assert_eq!(unknown.json()["ok"], Value::Bool(false), "{}", unknown.stdout);

    // Malformed JSON never reaches the engine — the CLI parses it.
    let malformed = odm(dir.path(), &["inspect", "{not json"]);
    assert_ne!(malformed.code, 0);
    assert!(!malformed.stderr.is_empty(), "a parse failure must say something");

    // An unknown request field is the engine's to reject.
    let field = odm(dir.path(), &["inspect", r#"{"nosuchfield": 1}"#]);
    assert_ne!(field.code, 0);
    assert_eq!(field.json()["ok"], Value::Bool(false), "{}", field.stdout);
}

// ------------------------------------------------------ 8. transport state

/// Run the client with extra environment.
fn odm_env(project: &Path, args: &[&str], env: &[(&str, &str)]) -> Out {
    let out = Command::new(BIN).arg("--project").arg(project).args(args).envs(env.iter().copied()).output().unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// What a CLI inside a sandbox that denies `connect` (Codex's) falls back to.
#[test]
fn the_mailbox_answers_when_the_socket_is_denied() {
    let dir = project(&[("root.js", BOX10)]);
    let mailed = |args: &[&str]| odm_env(dir.path(), args, &[("ODM_TRANSPORT", "mailbox")]);
    let out = mailed(&["status"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("no engine at"), "{}", out.stderr);

    let engine = Engine::start(dir.path());
    let v = mailed(&["inspect"]).ok_json();
    assert_eq!(v["ok"], Value::Bool(true));
    // The client takes its files with it.
    let left: Vec<_> = std::fs::read_dir(dir.path().join(".odm/mailbox")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(left.is_empty(), "{left:?}");

    // A killed engine leaves its mailbox behind, unread.
    drop(engine);
    let out = mailed(&["status"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("no engine at"), "{}", out.stderr);
}

/// A socket that won't take the client's token (here: the wrong one in
/// `engine.json`) is just another socket failure — the mailbox answers.
#[test]
fn a_rejected_token_falls_back_to_the_mailbox() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());
    let info = dir.path().join(".odm/engine.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&info).unwrap()).unwrap();
    v["token"] = json!("0".repeat(64));
    std::fs::write(&info, v.to_string()).unwrap();

    assert_eq!(root_volume(dir.path()), 1000.0);

    // With the mailbox gone too, nothing answers: the socket really refused.
    std::fs::remove_dir_all(dir.path().join(".odm/mailbox")).unwrap();
    let out = odm(dir.path(), &["status"]);
    assert_eq!(out.code, 2, "{}", out.stdout);
    assert!(out.stderr.contains("socket") && out.stderr.contains("mailbox"), "{}", out.stderr);
}

/// The crashed-engine restart every user eventually needs: a kill leaves
/// the lock file, `engine.json` and the mailbox behind, and the next engine
/// must take them over.
#[test]
fn stale_state_is_reclaimed() {
    let dir = project(&[("root.js", BOX10)]);
    let odm_dir = dir.path().join(".odm");
    drop(Engine::start(dir.path()));
    for left in ["engine.lock", "engine.json", "mailbox"] {
        assert!(odm_dir.join(left).exists(), "a killed engine leaves {left}");
    }
    let before = std::fs::read_to_string(odm_dir.join("engine.json")).unwrap();

    let _engine = Engine::start(dir.path());
    assert_eq!(root_volume(dir.path()), 1000.0);
    assert_ne!(std::fs::read_to_string(odm_dir.join("engine.json")).unwrap(), before, "a fresh token");
}

// ------------------------------------------------------------------ 9. docs

/// The agent's documentation surface, engineless: every topic must print.
#[test]
fn every_docs_topic_prints() {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let mut topics = vec!["prompt".to_string()];
    for dir in [docs.join("api"), docs.clone()] {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "md") {
                let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                if stem != "README" {
                    topics.push(stem);
                }
            }
        }
    }
    assert!(topics.len() > 10, "found only {topics:?}");

    for topic in &topics {
        let out = Command::new(BIN).args(["docs", topic]).output().unwrap();
        assert!(out.status.success(), "odm docs {topic}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(!out.stdout.is_empty(), "odm docs {topic} printed nothing");
    }

    let index = Command::new(BIN).arg("docs").output().unwrap();
    assert!(index.status.success());
    let index = String::from_utf8_lossy(&index.stdout);
    for topic in &topics {
        assert!(index.contains(topic.as_str()), "the index omits {topic}");
    }

    let search = Command::new(BIN).args(["docs", "search", "box"]).output().unwrap();
    assert!(search.status.success());
    assert!(!search.stdout.is_empty(), "`docs search box` found nothing");
}
