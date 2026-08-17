use odm_ir::{Canonical, Hash, Node};
use odm_js::{BuildInput, BuildOutput, FailedBuild, Invoker, JsEnv, run_build};
use odm_kernel::Kernel;
use odm_store::{Dep, Object, Store};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

fn env() -> &'static JsEnv {
    static ENV: OnceLock<JsEnv> = OnceLock::new();
    ENV.get_or_init(|| JsEnv::new().expect("snapshot build"))
}

struct World {
    store: Arc<Store>,
    kernel: Arc<Kernel>,
}

fn world() -> World {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    World { store, kernel }
}

fn build(w: &World, code: &str) -> Result<BuildOutput, FailedBuild> {
    build_full(w, code, &json!({}), &json!({}), &HashMap::new(), None)
}

fn build_full(
    w: &World,
    code: &str,
    args: &Value,
    decls: &Value,
    cascade: &HashMap<String, Value>,
    invoker: Option<Box<dyn Invoker>>,
) -> Result<BuildOutput, FailedBuild> {
    run_build(
        env(),
        BuildInput {
            path: "main.js",
            code,
            api: odm_js::ApiVersion::Unstable,
            args,
            decls,
            cascade,
            kernel: w.kernel.clone(),
            store: w.store.clone(),
            cancel: None,
            invoker,
            on_handle: None,
        },
    )
}

fn node_at(w: &World, h: Hash) -> Node {
    match &*w.store.get(h).unwrap_or_else(|| panic!("{h} not in store")) {
        Object::Node(n) => n.clone(),
        other => panic!("expected node, got {other:?}"),
    }
}

fn output_node(w: &World, out: &BuildOutput) -> Node {
    node_at(w, out.output)
}

/// Child `i` of `node`, read back from the store.
fn child(w: &World, node: &Node, i: usize) -> Node {
    node_at(w, node.children[i])
}

/// Mesh hashes in a stored subtree, in walk order.
fn mesh_refs(w: &World, root: Hash) -> Vec<Hash> {
    let node = node_at(w, root);
    let mut out: Vec<Hash> = node.mesh.into_iter().collect();
    for c in &node.children {
        out.extend(mesh_refs(w, *c));
    }
    out
}

#[test]
fn basic_csg_build() {
    let w = world();
    let out = build(
        &w,
        r#"
        export default function build(ctx) {
            const plate = odm.box([20, 10, 4]);
            const hole = odm.cylinder(2, 10);
            return plate.subtract(hole).color('#4682b4').name('plate');
        }
        "#,
    )
    .unwrap();
    let node = output_node(&w, &out);
    assert_eq!(node.name.as_deref(), Some("plate"));
    let mesh_hash = node.mesh.expect("solid output has geometry");
    assert!(matches!(&*w.store.get(mesh_hash).unwrap(), Object::Mesh(m) if m.triangle_count() > 0));
    // Hex lands in the IR untouched: sRGB in, sRGB stored.
    let color = node.color.unwrap();
    assert_eq!(
        [color.r, color.g, color.b, color.a],
        [0x46 as f32 / 255.0, 0x82 as f32 / 255.0, 0xb4 as f32 / 255.0, 1.0]
    );
}

#[test]
fn determinism_same_code_same_hash() {
    let w1 = world();
    let w2 = world();
    let code = r#"
        export default function build(ctx) {
            const s = new THREE.Shape().moveTo(0, 0).lineTo(4, 0).lineTo(4, 2)
                .absarc(2, 2, 1, 0, Math.PI, false).lineTo(0, 2);
            const body = odm.extrude(s, 1.5);
            const noise = Math.random(); // seeded PRNG: same in every isolate
            return body.translate(noise, 0, 0);
        }
    "#;
    let a = build(&w1, code).unwrap();
    let b = build(&w2, code).unwrap();
    assert_eq!(a.output, b.output, "two isolates, same code -> same IR hash");
}

#[test]
fn cascade_reads_recorded_as_deps() {
    let w = world();
    let mut ctx = HashMap::new();
    ctx.insert("t".to_string(), json!(1.5));
    ctx.insert("width".to_string(), json!(30.0));
    let decls = json!({
        "width": { "cascade": true, "type": "number" },
        "t": { "cascade": true, "type": "number" },
        "depth": { "cascade": false, "type": "number" },
    });
    let out = build_full(
        &w,
        r#"
        export default function build(ctx) {
            return odm.box([ctx.input('width'), ctx.input('depth'), 5]).translate(ctx.input('t'), 0, 0);
        }
        "#,
        &json!({ "depth": 7 }),
        &decls,
        &ctx,
        None,
    )
    .unwrap();
    let keys: Vec<&str> = out
        .deps
        .iter()
        .map(|d| match d {
            Dep::Cascade { key, .. } => key.as_str(),
            other => panic!("unexpected dep {other:?}"),
        })
        .collect();
    // Only cascade reads touch the environment; `depth` came from args.
    assert_eq!(keys, vec!["width", "t"]);

    // A build that reads nothing has no deps.
    let out2 = build(&w, "export default () => odm.sphere(1)").unwrap();
    assert!(out2.deps.is_empty());
}

#[test]
fn undeclared_get_is_an_error() {
    let w = world();
    let err = build_full(
        &w,
        "export default (ctx) => odm.box(ctx.input('nope'))",
        &json!({}),
        &json!({ "size": { "cascade": false, "type": "number" } }),
        &HashMap::new(),
        None,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not declared in meta.inputs"), "{msg}");
    assert!(msg.contains("size"), "should list declared inputs: {msg}");
}

#[test]
fn extension_types_hydrate_to_three_instances() {
    let w = world();
    let out = build_full(
        &w,
        r#"
        export default function build(ctx) {
            const off = ctx.input('off');
            if (!(off instanceof THREE.Vector3)) throw new Error('off not a Vector3');
            const m = ctx.input('m');
            if (!(m instanceof THREE.Matrix4)) throw new Error('m not a Matrix4');
            const q = ctx.input('q');
            if (!(q instanceof THREE.Quaternion)) throw new Error('q not a Quaternion');
            const c = ctx.input('c');
            return odm.box(1).translate(off.x, off.y, off.z).color(c);
        }
        "#,
        &json!({
            "off": [1, 2, 3],
            "m": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1],
            "q": [0, 0, 0, 1],
            "c": "#4682b4",
        }),
        &json!({
            "off": { "cascade": false, "type": "vector3" },
            "m": { "cascade": false, "type": "matrix4" },
            "q": { "cascade": false, "type": "quaternion" },
            "c": { "cascade": false, "type": "color" },
        }),
        &HashMap::new(),
        None,
    )
    .unwrap();
    let node = output_node(&w, &out);
    assert!(!node.transform.is_identity());
    assert!(node.color.is_some());
}

#[test]
fn syntax_error_is_agent_readable() {
    let w = world();
    let err = build(&w, "export default function build( {").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("main.js"), "should name the file: {msg}");
}

#[test]
fn runtime_error_has_stack_and_logs() {
    let w = world();
    let err = build(
        &w,
        r#"
        export default function build(ctx) {
            console.log('about to fail', { step: 3 });
            function inner() { throw new Error('boom'); }
            inner();
        }
        "#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("boom"), "{msg}");
    assert!(msg.contains("inner"), "stack should name the frame: {msg}");
    // Console output is data next to the error, never text inside it.
    assert!(!msg.contains("about to fail"), "logs must not leak into the message: {msg}");
    let logged: Vec<&str> = err.logs.iter().map(|l| l.message.as_str()).collect();
    assert_eq!(logged, vec![r#"about to fail {"step":3}"#], "logs ride along as data");
}

#[test]
fn open_surface_rejected_with_explanation() {
    let w = world();
    let err = build(
        &w,
        r#"
        export default function build(ctx) {
            const flat = new THREE.ShapeGeometry(new THREE.Shape()
                .moveTo(0, 0).lineTo(1, 0).lineTo(0, 1));
            return odm.fromThreeGeometry(flat);
        }
        "#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("open surface"), "{msg}");
}

#[test]
fn bad_return_value_rejected() {
    let w = world();
    let err = build(&w, "export default () => 42").unwrap_err();
    assert!(err.to_string().contains("scene value"), "{err}");

    let err = build(&w, "export const nope = 1;").unwrap_err();
    assert!(err.to_string().contains("default"), "{err}");
}

#[test]
fn three_generator_round_trip() {
    let w = world();
    let out = build(
        &w,
        r#"
        export default function build(ctx) {
            return odm.fromThreeGeometry(new THREE.TorusGeometry(3, 1, 16, 32))
                .rotateX(odm.deg(90));
        }
        "#,
    )
    .unwrap();
    let node = output_node(&w, &out);
    assert!(node.mesh.is_some());
    assert!(!node.transform.is_identity());
}

#[test]
fn groups_and_arrays_nest() {
    let w = world();
    let out = build(
        &w,
        r#"
        export default function build(ctx) {
            const wheel = odm.cylinder(2, 1);
            return odm.group(
                wheel.translate(-3, 0, 0).name('left'),
                [wheel.translate(3, 0, 0).name('right'), null],
            ).color('#f00');
        }
        "#,
    )
    .unwrap();
    let node = output_node(&w, &out);
    assert_eq!(node.children.len(), 2);
    assert_eq!(child(&w, &node, 0).name.as_deref(), Some("left"));
    // Both wheels reference the SAME geometry blob (content addressing).
    let refs = mesh_refs(&w, out.output);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0], refs[1]);
}

/// `{ref}` nodes (what an untransformed Instance emits) name a stored
/// subtree, and nothing else.
#[test]
fn subtree_refs_are_validated() {
    let w = world();
    let leaf = w.store.put(Object::Node(Node { name: Some("leaf".into()), ..Default::default() }));
    let out = odm_js::node_from_json(&w.store, &json!({ "ref": leaf.to_hex() })).unwrap();
    assert_eq!(out, leaf, "a bare ref reuses the referenced subtree's hash");

    let err = |v: Value| odm_js::node_from_json(&w.store, &v).unwrap_err();
    assert!(err(json!({ "ref": leaf.to_hex(), "name": "x" })).contains("no other keys"));
    assert!(err(json!({ "ref": "zz" })).contains("invalid"));
    assert!(err(json!({ "ref": Hash::of_bytes(b"nope").to_hex() })).contains("unknown"));
    let mesh = w.kernel.cube(1.0, 1.0, 1.0, true).unwrap();
    assert!(err(json!({ "ref": mesh.to_hex() })).contains("geometry"));
}

// --- invoke ---

/// Invoker that runs a real nested build with its own isolate (the LIFO
/// nesting pattern the scheduler will use).
struct NestedInvoker {
    world: World,
    /// path -> (code, decls). At this layer the scheduler's meta machinery
    /// doesn't exist, so tests hand the declaration table over directly.
    codes: HashMap<String, (String, Value)>,
    calls: Vec<(String, Value)>,
}

impl Invoker for NestedInvoker {
    fn invoke(
        &mut self,
        path: &str,
        args: &Value,
        cascade: &serde_json::Map<String, Value>,
    ) -> Result<Hash, odm_js::InvokeError> {
        self.calls.push((path.to_string(), args.clone()));
        let (code, decls) = self
            .codes
            .get(path)
            .ok_or_else(|| odm_js::InvokeError::from(format!("no doohickey at {path}")))?
            .clone();
        // The invoke's cascade values become the nested build's environment
        // (the scheduler additionally overlays declaration defaults).
        let child_env: HashMap<String, Value> =
            cascade.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let out = run_build(
            env(),
            BuildInput {
                path,
                code: &code,
                api: odm_js::ApiVersion::Unstable,
                args,
                decls: &decls,
                cascade: &child_env,
                kernel: self.world.kernel.clone(),
                store: self.world.store.clone(),
                cancel: None,
                invoker: None,
                on_handle: None,
            },
        )
        .map_err(|e| odm_js::InvokeError::from(e.to_string()))?;
        Ok(out.output)
    }
}

#[test]
fn invoke_runs_nested_isolate_and_records_dep() {
    let w = world();
    let mut codes = HashMap::new();
    codes.insert(
        "parts/wheel.js".to_string(),
        (
            r#"
            export default function build(ctx) {
                return odm.cylinder(ctx.input('radius'), 1).name('wheel');
            }
            "#
            .to_string(),
            json!({ "radius": { "cascade": false, "type": "number" } }),
        ),
    );
    let invoker = NestedInvoker {
        world: World { store: w.store.clone(), kernel: w.kernel.clone() },
        codes,
        calls: vec![],
    };
    let out = build_full(
        &w,
        r#"
        export default function build(ctx) {
            const wheel = ctx.invoke('parts/wheel.js', { radius: 2 });
            return odm.group(
                wheel.translate(-4, 0, 0),
                wheel.translate(4, 0, 0).color('#000'),
            );
        }
        "#,
        &json!({}),
        &json!({}),
        &HashMap::new(),
        Some(Box::new(invoker)),
    )
    .unwrap();

    let invoke_deps: Vec<_> = out
        .deps
        .iter()
        .filter(|d| matches!(d, Dep::Invoke { path, .. } if path == "parts/wheel.js"))
        .collect();
    assert_eq!(invoke_deps.len(), 1, "one invoke dep recorded: {:?}", out.deps);

    let node = output_node(&w, &out);
    assert_eq!(node.children.len(), 2);
    // The referenced subtree carries the wheel's name and geometry...
    let first = child(&w, &node, 0);
    assert_eq!(first.children.len(), 1);
    let wheel = child(&w, &first, 0);
    assert_eq!(wheel.name.as_deref(), Some("wheel"));
    assert!(wheel.mesh.is_some());
    // ...and both placements point at that one stored subtree.
    let second = child(&w, &node, 1);
    assert_eq!(first.children[0], second.children[0], "one wheel subtree, two refs");
}

/// Repeated invokes with the same args produce one stored subtree: the store
/// grows only by the per-placement wrapper node.
#[test]
fn repeated_invokes_share_one_stored_subtree() {
    let objects = |placements: usize| {
        let w = world();
        let mut codes = HashMap::new();
        codes.insert(
            "parts/wheel.js".to_string(),
            (
                "export default (ctx) => odm.cylinder(ctx.input('radius'), 1).name('wheel')"
                    .to_string(),
                json!({ "radius": { "cascade": false, "type": "number" } }),
            ),
        );
        let invoker = NestedInvoker {
            world: World { store: w.store.clone(), kernel: w.kernel.clone() },
            codes,
            calls: vec![],
        };
        let out = build_full(
            &w,
            &format!(
                r#"
                export default function build(ctx) {{
                    const parts = [];
                    for (let i = 0; i < {placements}; i++) {{
                        parts.push(ctx.invoke('parts/wheel.js', {{ radius: 2 }}).translate((i + 1) * 3, 0, 0));
                    }}
                    return odm.group(...parts);
                }}
                "#
            ),
            &json!({}),
            &json!({}),
            &HashMap::new(),
            Some(Box::new(invoker)),
        )
        .unwrap();

        let root = output_node(&w, &out);
        assert_eq!(root.children.len(), placements);
        let subtrees: std::collections::HashSet<Hash> =
            root.children.iter().map(|&c| node_at(&w, c).children[0]).collect();
        assert_eq!(subtrees.len(), 1, "every placement refers to the same subtree");
        w.store.object_count()
    };
    assert_eq!(objects(8) - objects(2), 6, "each extra placement costs one wrapper node");
}

#[test]
fn solids_serialize_through_invoke_args() {
    let w = world();
    let mut codes = HashMap::new();
    codes.insert(
        "cut.js".to_string(),
        (
            r#"
            export default function build(ctx) {
                // The Solid arrives revived: subtract it from a plate.
                return odm.box([10, 10, 2]).subtract(ctx.input('tool'));
            }
            "#
            .to_string(),
            json!({ "tool": { "cascade": false, "type": "solid" } }),
        ),
    );
    let invoker = NestedInvoker {
        world: World { store: w.store.clone(), kernel: w.kernel.clone() },
        codes,
        calls: vec![],
    };
    let out = build_full(
        &w,
        r#"
        export default function build(ctx) {
            const tool = odm.cylinder(1, 5).translate(2, 2, 0);
            return ctx.invoke('cut.js', { tool });
        }
        "#,
        &json!({}),
        &json!({}),
        &HashMap::new(),
        Some(Box::new(invoker)),
    )
    .unwrap();
    // A plain Instance embeds its tree directly: the root IS the cut result.
    let node = output_node(&w, &out);
    assert!(node.mesh.is_some());
}

/// `ctx.invoke(path, args, cascade)`: cascade values reach the child's
/// environment and are recorded on the invoke dep.
#[test]
fn cascade_values_flow_to_the_nested_build() {
    let w = world();
    let mut codes = HashMap::new();
    codes.insert(
        "spinner.js".to_string(),
        (
            "export default (ctx) => odm.box(1).rotateZ(ctx.input('t'))".to_string(),
            json!({ "t": { "cascade": true, "type": "number" } }),
        ),
    );
    let invoker = NestedInvoker {
        world: World { store: w.store.clone(), kernel: w.kernel.clone() },
        codes,
        calls: vec![],
    };
    let out = build_full(
        &w,
        "export default (ctx) => ctx.invoke('spinner.js', {}, { t: 0.5 })",
        &json!({}),
        &json!({}),
        &HashMap::new(),
        Some(Box::new(invoker)),
    )
    .unwrap();
    let dep = out.deps.iter().find_map(|d| match d {
        Dep::Invoke { path, cascade, .. } if path == "spinner.js" => Some(cascade.clone()),
        _ => None,
    });
    assert_eq!(dep.unwrap().get("t"), Some(&json!(0.5)));
    // A bare Instance return embeds the child's tree directly: the root IS
    // the spun box, transform applied inside the nested build.
    let node = output_node(&w, &out);
    assert!(!node.transform.is_identity());
    assert!(node.mesh.is_some());
}

#[test]
fn logs_captured_on_success() {
    let w = world();
    let out = build(
        &w,
        r#"
        export default function build(ctx) {
            console.log('hello', 42);
            console.warn('careful');
            return odm.sphere(1);
        }
        "#,
    )
    .unwrap();
    let joined: Vec<String> =
        out.logs.iter().map(|l| format!("{}:{}", l.level, l.message)).collect();
    assert_eq!(joined, vec!["log:hello 42", "warn:careful"]);
}

#[test]
fn queries_work_inside_build() {
    let w = world();
    let out = build(
        &w,
        r#"
        export default function build(ctx) {
            const s = odm.box([2, 2, 2]);
            const v = s.volume();
            if (Math.abs(v - 8) > 1e-9) throw new Error('volume ' + v);
            const b = s.bounds();
            if (Math.abs(b.min.x + 1) > 1e-9) throw new Error('bounds ' + JSON.stringify(b));
            const hit = s.raycast([0, 0, 5], [0, 0, -1]);
            if (Math.abs(hit.distance - 4) > 1e-9) throw new Error('ray ' + JSON.stringify(hit));
            const miss = s.translate(10, 0, 0).raycast([0, 0, 5], [0, 0, -1]);
            if (miss !== null) throw new Error('expected miss');
            const c = s.clearance(s.translate(5, 0, 0));
            if (Math.abs(c.distance - 3) > 1e-9 || Math.abs(c.closest[1].x - 4) > 1e-9) {
                throw new Error('clearance ' + JSON.stringify(c));
            }
            const o = s.clearance(s.translate(1, 0, 0));
            if (!(o.distance < 0) || Math.abs(o.separate.length() + o.distance) > 1e-9) {
                throw new Error('expected overlap ' + JSON.stringify(o));
            }
            return s;
        }
        "#,
    );
    out.unwrap();
}

#[test]
fn args_hash_uses_canonical_json() {
    // Sanity: hashing of args is order-insensitive (framework passes plain JSON).
    let a = odm_ir::hash_json(&json!({"x": 1, "y": 2}));
    let b = odm_ir::hash_json(&json!({"y": 2, "x": 1}));
    assert_eq!(a, b);
    let _ = Hash::of_bytes(b"x"); // silence unused import if asserts compiled out
    let _ = json!({}).hash();
}

/// run_build with an on_handle hook handing the handle to a killer thread
/// that terminates after `delay`. V8 occasionally swallows a lone terminate
/// (a pending flag consumed in a JS-idle gap, or cleared by deno_core's
/// exception conversion — measured ~0.2% on the TLA path), so after 2s the
/// killer re-terminates until the build dies; the scheduler's cancel
/// watchdog does the same. Returns the build result and whether a retry was
/// needed.
fn build_with_killer(
    w: &World,
    code: &'static str,
    delay: std::time::Duration,
    cancel_token_too: bool,
) -> (Result<BuildOutput, FailedBuild>, bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancel = odm_kernel::CancelToken::new();
    let cancel2 = cancel.clone();
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let (tx, rx) = std::sync::mpsc::channel::<odm_js::InterruptHandle>();
    let killer = std::thread::spawn(move || -> bool {
        let handle = rx.recv().unwrap();
        std::thread::sleep(delay);
        if cancel_token_too {
            cancel2.cancel();
        }
        handle.interrupt();
        for _ in 0..200 {
            if done2.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut retried = false;
        while !done2.load(Ordering::SeqCst) {
            retried = true;
            handle.interrupt();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        retried
    });
    let res = run_build(
        env(),
        BuildInput {
            path: "main.js",
            code,
            api: odm_js::ApiVersion::Unstable,
            args: &json!({}),
            decls: &json!({}),
            cascade: &HashMap::new(),
            kernel: w.kernel.clone(),
            store: w.store.clone(),
            cancel: Some(cancel),
            invoker: None,
            on_handle: Some(Box::new(move |h| {
                let _ = tx.send(h);
            })),
        },
    );
    done.store(true, Ordering::SeqCst);
    let retried = killer.join().unwrap();
    (res, retried)
}

#[test]
fn top_level_infinite_loop_is_terminable() {
    let w = world();
    let (res, _) = build_with_killer(
        &w,
        "export const meta = { inputs: {} };\nexport function build() {}\nwhile (true) {}",
        std::time::Duration::from_millis(100),
        true,
    );
    match res.unwrap_err().error {
        odm_js::BuildError::Cancelled => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }
}

const TLA_SPIN: &str = "export const meta = { inputs: {} };\nexport function build() {}\n\
                        await Promise.resolve();\nfor (;;) {}";

#[test]
fn terminate_during_tla_eval_does_not_crash() {
    // rusty_v8 #830 / v8 12379: terminating during async-module (top-level
    // await) machinery used to crash V8. Sweep termination timing across the
    // eval window; passing = every iteration errors out, no process abort.
    for i in 0..40u64 {
        let w = world();
        let (res, _) =
            build_with_killer(&w, TLA_SPIN, std::time::Duration::from_micros(i * 50), false);
        assert!(res.is_err(), "iteration {i} unexpectedly succeeded");
    }
}

/// Bigger sweep of the same race (run explicitly with --ignored); reports
/// how often the first terminate was swallowed.
#[test]
#[ignore]
fn tla_terminate_stress() {
    let mut swallowed = 0u32;
    for i in 0..1000u64 {
        let w = world();
        let (res, retried) =
            build_with_killer(&w, TLA_SPIN, std::time::Duration::from_micros((i % 40) * 50), false);
        assert!(res.is_err(), "iteration {i} unexpectedly succeeded");
        if retried {
            swallowed += 1;
            println!("iteration {i}: first terminate swallowed, retry un-wedged it");
        }
    }
    println!("swallowed terminations: {swallowed}/1000");
}

#[test]
fn extract_export_times_out_on_top_level_loop() {
    let w = world();
    let err = odm_js::extract_export(
        env(),
        "main.js",
        "export const meta = { inputs: {} };\nwhile (true) {}",
        odm_js::ApiVersion::Unstable,
        "meta",
        w.kernel.clone(),
        w.store.clone(),
        std::time::Duration::from_millis(300),
    )
    .unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
}

#[test]
fn stalled_top_level_await_errors_not_hangs() {
    // A never-resolving top-level await must surface as an error (either
    // deno_core's stalled-TLA detection or the extract watchdog), not hang.
    let w = world();
    let err = odm_js::extract_export(
        env(),
        "main.js",
        "export const meta = { inputs: {} };\nawait new Promise(() => {});",
        odm_js::ApiVersion::Unstable,
        "meta",
        w.kernel.clone(),
        w.store.clone(),
        std::time::Duration::from_millis(500),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("timed out") || msg.to_lowercase().contains("await"),
        "unexpected error: {msg}"
    );
}
