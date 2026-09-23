//! The viewer opens a real window and paints: `odm run` under
//! `ODM_VIEWER_SMOKE=1` paints a few frames, reports adapter and window
//! system, and exits 0. Windows and macOS always run it (their CI runners have
//! a desktop session). On Linux the display in the environment is usually the
//! developer's own, so it runs only with `ODM_TEST_VIEWER=1`, meant for a
//! disposable session (the gui-testing skill's guibox).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_odm");

fn wanted() -> bool {
    !cfg!(any(target_os = "linux", target_os = "freebsd"))
        || std::env::var("ODM_TEST_VIEWER").as_deref() == Ok("1")
}

#[test]
fn viewer_paints_and_exits() {
    if !wanted() {
        eprintln!("skipping the viewer smoke test (Linux: set ODM_TEST_VIEWER=1 in a disposable session)");
        return;
    }
    let dir = tempfile::Builder::new().prefix("odm-smoke-").tempdir().unwrap();
    std::fs::write(dir.path().join("odm.toml"), "name = \"smoke\"\nengine = 0\n").unwrap();
    std::fs::write(dir.path().join("root.js"), "//! ODM API unstable\nexport default () => odm.box(10);\n")
        .unwrap();
    let mut child = Command::new(BIN)
        .args(["run", &dir.path().display().to_string()])
        .env("ODM_VIEWER_SMOKE", "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let scale: f64 =
        std::env::var("ODM_TEST_TIMEOUT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let deadline = Instant::now() + Duration::from_secs(30).mul_f64(scale);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().ok();
            panic!("viewer smoke run did not exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    eprintln!("{out}");
    assert!(status.success(), "viewer exited with {status}");
    assert!(out.contains("viewer smoke: adapter "), "no smoke report in stdout: {out:?}");
}
