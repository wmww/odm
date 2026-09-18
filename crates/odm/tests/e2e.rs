//! End to end through the shipped binary: spawn `odm run --headless`, then
//! drive it with the same `odm <cmd>` invocations an agent types. This is the
//! only place arg dispatch, engine startup, the socket lifecycle and the
//! inotify watcher run as shipped.
//!
//! Assertions are structural — exit codes, JSON fields, substrings that name
//! a fix command. Wording is guarded elsewhere (`cli_reference_is_current`
//! diffs docs/cli.md against the parser), so nothing here pins prose.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_odm");

const BOX10: &str = "//! ODM API unstable\nexport default () => odm.box(10);\n";
const BOX20: &str = "//! ODM API unstable\nexport default () => odm.box(20);\n";

/// A project directory under `/tmp`: the socket lives at
/// `<project>/.odm/engine.sock` and unix socket paths cap out around 104
/// bytes, so a deep scratch path would not fit.
fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::Builder::new().prefix("odm-e2e-").tempdir_in("/tmp").unwrap();
    write(dir.path(), "odm.toml", "name = \"e2e\"\nengine = 0\n");
    for (name, body) in files {
        write(dir.path(), name, body);
    }
    dir
}

/// A project seeded from one of the repo's examples.
fn example_project(name: &str) -> TempDir {
    let dir = tempfile::Builder::new().prefix("odm-e2e-").tempdir_in("/tmp").unwrap();
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
        } // else: a stale engine socket under .odm/ — not copyable, not wanted
    }
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn sock(project: &Path) -> PathBuf {
    project.join(".odm/engine.sock")
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
        engine.await_socket();
        engine
    }

    fn await_socket(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if UnixStream::connect(sock(&self.project)).is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("engine exited early ({status}):\n{}", self.drain_stderr());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("engine never opened its socket:\n{}", self.drain_stderr());
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
    /// SIGKILL: the headless engine installs no signal handler, so this is
    /// also what a crash looks like — the stale socket it leaves behind is
    /// what `stale_socket_is_reclaimed` exercises.
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
    let deadline = Instant::now() + Duration::from_secs(10);
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

/// `odm poll --follow` never syncs, so a line can only arrive via
/// watcher → rebuild → event. This is the only test that exercises the
/// inotify watcher at all.
#[test]
fn the_watcher_rebuilds_without_any_query() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    let mut follower = Command::new(BIN)
        .arg("--project")
        .arg(dir.path())
        .args(["poll", "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn poll --follow");
    let (tx, lines) = std::sync::mpsc::channel();
    let stdout = follower.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    // Give the follower time to connect and park before touching anything.
    std::thread::sleep(Duration::from_millis(300));
    write(dir.path(), "root.js", "//! ODM API unstable\nthis is not javascript(((\n");
    await_build_state(&lines, "error");
    write(dir.path(), "root.js", BOX20);
    await_build_state(&lines, "ok");

    let _ = follower.kill();
    let _ = follower.wait();
}

/// Drain follow lines until one reports the default slot in `want`.
fn await_build_state(lines: &std::sync::mpsc::Receiver<String>, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = vec![];
    while let Some(rest) = deadline.checked_duration_since(Instant::now()) {
        let Ok(line) = lines.recv_timeout(rest) else { break };
        let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        if v["builds"].as_array().is_some_and(|b| b.iter().any(|s| s["build"] == want)) {
            return;
        }
        seen.push(line);
    }
    panic!("no follow line reported build {want:?}; saw: {seen:#?}");
}

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
    let req = format!(r#"{{"out": "{}", "width": 64, "height": 64}}"#, out.display());

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

/// User messages only enter through the viewer, so the queue/ack semantics
/// are `state.rs`/`server.rs`'s to test. This proves the commands reach a
/// real engine and that a bounded poll comes back promptly.
#[test]
fn say_and_poll_reach_the_engine() {
    let dir = project(&[("root.js", BOX10)]);
    let _engine = Engine::start(dir.path());

    odm(dir.path(), &["say", "hello"]).ok_json();
    let started = Instant::now();
    let v = odm(dir.path(), &["poll", "--timeout", "0.2"]).ok_json();
    assert!(started.elapsed() < Duration::from_secs(5), "a bounded poll must not hang");
    assert_eq!(v["ok"], Value::Bool(true));
    assert_eq!(v["messages"], serde_json::json!([]), "nobody typed anything");
    assert!(v["builds"].is_array(), "every poll answer carries build state: {v}");
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
    assert!(!out.status.success(), "a second engine must refuse the socket");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already running"), "{stderr}");

    odm(dir.path(), &["status"]).ok_json();
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

// ---------------------------------------------------------- 8. stale socket

/// The crashed-engine restart every user eventually needs: SIGKILL leaves the
/// socket file behind, and `bind()` must probe and remove it.
#[test]
fn stale_socket_is_reclaimed() {
    let dir = project(&[("root.js", BOX10)]);
    drop(Engine::start(dir.path()));
    assert!(sock(dir.path()).exists(), "a killed engine leaves its socket file");

    let _engine = Engine::start(dir.path());
    assert_eq!(root_volume(dir.path()), 1000.0);
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
