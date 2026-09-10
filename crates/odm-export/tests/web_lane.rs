//! The web export, driven for real in a headless browser — both rendering
//! lanes. Opt-in and `#[ignore]`d: it needs the wasm toolchain (the template
//! build alone dwarfs the whole gate) and chromium.
//!
//! ```sh
//! cargo xtask test-web        # builds the template, then runs this
//! # or, with a template already built:
//! cargo test -p odm-export --test web_lane -- --ignored
//! ```
//!
//! Automates the recipe in notes/web-export.md. What it asserts is what is
//! actually the web lane's own: that both backends come up, say nothing at
//! error level, draw something, and — the one that matters — that the build
//! running through the JS-glue executor and the wasm kernel produces the
//! *same root hash* as odm-build does natively for the same view. Everything
//! past that point is shared renderer code.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};

/// The example project both lanes build.
const EXAMPLE: &str = "piston";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// One JsEnv for the whole binary: building a second snapshot while another
/// thread runs JS aborts the process (notes/architecture.md "Testing").
fn env() -> &'static std::sync::Arc<odm_js::JsEnv> {
    static ENV: OnceLock<std::sync::Arc<odm_js::JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| std::sync::Arc::new(odm_js::JsEnv::new().expect("js env")))
}

#[test]
#[ignore = "needs the wasm template and chromium: cargo xtask test-web"]
fn both_lanes_boot_build_and_draw() {
    let template = repo_root().join("target").join(odm_export::TEMPLATE_NAME);
    assert!(
        template.is_file(),
        "no web template at {} — build it with `cargo xtask build-web-template` \
         (or run the whole lane with `cargo xtask test-web`)",
        template.display()
    );
    require("chromium");

    let want = native_root_hash();
    let dir = tempfile::Builder::new().prefix("odm-web-lane-").tempdir_in("/tmp").unwrap();
    let site = dir.path().join("site");
    // Named outright: the lookup would otherwise prefer an installed
    // template, which is not the one xtask just built from these sources.
    export(&site, &template);

    // The GL lane is the same site with navigator.gpu hidden before any
    // script runs — the app then takes its documented WebGL2 path.
    let gl_site = dir.path().join("site-gl");
    copy_dir(&site, &gl_site);
    hide_webgpu(&gl_site.join("index.html"));

    for (lane, root, flags) in [
        ("WebGPU", &site, &["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-angle=vulkan"][..]),
        ("WebGL2", &gl_site, &["--enable-unsafe-swiftshader"][..]),
    ] {
        let server = Server::start(root);
        let shot = dir.path().join(format!("shot-{lane}.png"));
        let log = run_chromium(&server.url(), &shot, flags);

        let announced = log
            .lines()
            .find_map(|l| l.split_once("ODM viewer: "))
            .map(|(_, rest)| rest.trim().to_owned())
            .unwrap_or_else(|| panic!("{lane}: the app never said which lane it took:\n{log}"));
        assert!(
            announced.starts_with(lane),
            "{lane}: the app took the {announced} lane instead"
        );

        // Only the *page's* errors: chromium logs its own GPU driver
        // performance notes at ERROR severity too, and those are noise.
        let errors: Vec<&str> = log
            .lines()
            .filter(|l| l.contains(":ERROR:CONSOLE") || l.contains("Uncaught"))
            .collect();
        assert!(errors.is_empty(), "{lane}: console errors: {errors:#?}");

        let got = log
            .lines()
            .find_map(|l| l.split_once("ODM root: "))
            .map(|(_, rest)| rest.trim().chars().take(64).collect::<String>())
            .unwrap_or_else(|| panic!("{lane}: no build was published:\n{log}"));
        assert_eq!(got, want, "{lane}: the wasm build disagrees with the native one");

        // A GL failure is silently black, so "it drew something" is a real
        // assertion here, not a formality.
        let pixels = decode(&std::fs::read(&shot).unwrap_or_else(|e| {
            panic!("{lane}: chromium wrote no screenshot ({e}):\n{log}")
        }));
        let distinct: std::collections::HashSet<&[u8]> = pixels.chunks_exact(4).collect();
        assert!(distinct.len() > 8, "{lane}: the page is one flat colour");
    }
}

/// The root hash odm-build produces for the same view, natively.
fn native_root_hash() -> String {
    let project = repo_root().join("examples").join(EXAMPLE);
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let engine = odm_build::BuildEngine::new(store, kernel, env().clone(), project);
    let sync = engine.sync().expect("scan");
    let view = odm_build::View::of(odm_build::DEFAULT_ROOT);
    engine
        .build_view(&engine.start_pass(&sync, view))
        .expect("the example must build natively")
        .root
        .to_hex()
}

fn export(out: &Path, template: &Path) {
    let project = repo_root().join("examples").join(EXAMPLE);
    let opts = odm_export::ExportOptions {
        template: Some(template.to_path_buf()),
        ..odm_export::ExportOptions::default()
    };
    odm_export::export_web_with_env(&project, out, &opts, env()).expect("export the example");
}

/// A one-shot static file server on a free port, alive until dropped.
struct Server {
    port: u16,
    stop: mpsc::Sender<()>,
}

impl Server {
    fn start(root: &Path) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let root = root.to_path_buf();
        let (stop, stopped) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.try_recv().is_ok() {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let root = root.clone();
                std::thread::spawn(move || serve(stream, &root));
            }
        });
        Server { port, stop }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        // Unblock the accept loop so the thread can notice.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(mut stream: TcpStream, root: &Path) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // Drain the headers; we answer every GET the same way.
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header.trim().is_empty() => break,
            Ok(_) => {}
        }
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let path = path.split('?').next().unwrap_or("/").trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    // No traversal: the browser only ever asks for our own files.
    let file = root.join(path);
    let body = (!path.contains("..")).then(|| std::fs::read(&file).ok()).flatten();
    let response = match body {
        Some(body) => {
            let mime = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html",
                Some("js") => "text/javascript",
                Some("wasm") => "application/wasm",
                Some("json") => "application/json",
                _ => "application/octet-stream",
            };
            let mut head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .into_bytes();
            head.extend_from_slice(&body);
            head
        }
        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

/// Load the page headless, screenshot it, and return everything chromium
/// logged (console lines included, thanks to `--enable-logging=stderr`).
fn run_chromium(url: &str, shot: &Path, flags: &[&str]) -> String {
    let profile = tempfile::tempdir().unwrap();
    let out = Command::new("chromium")
        .args([
            "--headless=new",
            "--no-sandbox",
            "--window-size=1280,800",
            "--virtual-time-budget=30000",
            "--enable-logging=stderr",
            "--v=1",
        ])
        .args(flags)
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg(format!("--screenshot={}", shot.display()))
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run chromium");
    let mut log = String::from_utf8_lossy(&out.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    log
}

fn hide_webgpu(index: &Path) {
    let html = std::fs::read_to_string(index).unwrap();
    let hide = "<script>Object.defineProperty(Navigator.prototype,\"gpu\",{value:undefined});</script>\n  ";
    let patched = html.replacen("<script src=\"./bundle.js\">", &format!("{hide}<script src=\"./bundle.js\">"), 1);
    assert_ne!(patched, html, "index.html no longer loads ./bundle.js the way this patch expects");
    std::fs::write(index, patched).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        match entry.file_type().unwrap().is_dir() {
            true => copy_dir(&entry.path(), &dst),
            false => {
                std::fs::copy(entry.path(), &dst).unwrap();
            }
        }
    }
}

fn decode(png_bytes: &[u8]) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

/// Fail with the fix rather than silently skipping.
fn require(program: &str) {
    let found = Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    assert!(found, "{program} is not on PATH; the web lane needs it (see notes/web-export.md)");
}

// Link the workspace stack dynamically (see odm-dylib).
use odm_dylib as _;
