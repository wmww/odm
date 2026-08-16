//! The socket request grammar: one JSON object per command, `cmd` plus the
//! command's fields. The field spec table here is both the validation front
//! line (unknown commands and fields are answered from it, with their
//! siblings listed) and the source the CLI reference's per-command section
//! prints from — one source, so docs can't drift (see `reference()` and the
//! drift test at the bottom).

use crate::commands::CmdError;
use serde::Deserialize;
use serde_json::{Map, Value};

/// One parsed socket request. [`parse`] checks the command and field names
/// against the spec table first (errors list siblings), then hands each
/// body to serde per command — so shape errors carry a field path, and
/// `deny_unknown_fields` is only a backstop.
pub(crate) enum Request {
    Status,
    Inspect(InspectReq),
    Render(RenderReq),
    Raycast(RaycastReq),
    Poll(PollReq),
    Say(SayReq),
    /// "I have the messages the last poll on this connection gave me." Sent by
    /// the CLI after it prints them, and hidden from the reference because it
    /// is part of poll's delivery handshake, not something an agent types.
    Ack,
}

/// `view`, the one "target what the user sees" knob: `true` adopts the
/// user's active viewer tab, a string names a specific slot (see
/// status.views).
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub(crate) enum ViewSel {
    Active(bool),
    Slot(String),
}

// The (path, inputs, preset, view) quartet is spelled out per request
// instead of a #[serde(flatten)] core: flatten silently disables
// deny_unknown_fields, and a typo'd field must stay an error.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectReq {
    pub path: Option<String>,
    #[serde(default)]
    pub inputs: Map<String, Value>,
    pub preset: Option<String>,
    pub view: Option<ViewSel>,
    /// Name or index path; absent (or "") is the root.
    pub node: Option<String>,
    pub depth: Option<f64>,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub full: bool,
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub stats: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenderReq {
    pub path: Option<String>,
    #[serde(default)]
    pub inputs: Map<String, Value>,
    pub preset: Option<String>,
    pub view: Option<ViewSel>,
    // Sizes stay f64: JSON numbers may carry a `.0`.
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
    pub out: Option<String>,
    #[serde(default)]
    pub wireframe: bool,
    #[serde(default)]
    pub no_grid: bool,
    #[serde(default)]
    pub ortho: bool,
    pub eye: Option<[f64; 3]>,
    pub target: Option<[f64; 3]>,
    pub up: Option<[f64; 3]>,
    pub direction: Option<[f64; 3]>,
    pub fov: Option<f64>,
    pub ortho_height: Option<f64>,
    #[serde(default)]
    pub stats: bool,
}

fn default_width() -> f64 {
    1024.0
}
fn default_height() -> f64 {
    768.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RaycastReq {
    pub path: Option<String>,
    #[serde(default)]
    pub inputs: Map<String, Value>,
    pub preset: Option<String>,
    pub view: Option<ViewSel>,
    pub rays: Vec<Ray>,
    #[serde(default)]
    pub stats: bool,
}

/// One ray of a `raycast` request. The unit query is JS's
/// `s.raycast(origin, dir, maxDist?)`; the CLI maps over `rays`.
#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ray {
    pub origin: [f64; 3],
    pub dir: [f64; 3],
    /// Same default as the JS twin.
    #[serde(default = "default_max_dist")]
    pub max_dist: f64,
}

fn default_max_dist() -> f64 {
    1e9
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PollReq {
    pub timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SayReq {
    pub text: String,
}

// --- the spec table ------------------------------------------------------

// `ty`/`doc`/`summary`/`js_twin` are read by `reference()`, which outside
// of the drift test only exists for regenerating docs/cli.md.
#[allow(dead_code)]
struct FieldSpec {
    name: &'static str,
    ty: &'static str,
    doc: &'static str,
}

const fn f(name: &'static str, ty: &'static str, doc: &'static str) -> FieldSpec {
    FieldSpec { name, ty, doc }
}

#[allow(dead_code)]
struct CommandSpec {
    name: &'static str,
    summary: &'static str,
    /// Takes the shared view-targeting fields ([`VIEW_FIELDS`]).
    view: bool,
    fields: &'static [FieldSpec],
    /// The JS method this command is the CLI twin of (geometry queries).
    js_twin: Option<&'static str>,
    /// Protocol plumbing: valid, but kept out of the reference.
    hidden: bool,
}

/// The fields every view-targeting command shares: which view to build,
/// with which input values.
const VIEW_FIELDS: &[FieldSpec] = &[
    f("path", "string", "the doohickey to build (default `root.js`)"),
    f(
        "inputs",
        "object",
        "input values by name, e.g. `{\"t\": 1.5}`; plain inputs of the target become view \
         args, everything else a view-level cascade value — a name nothing reads is an error \
         listing the settable inputs",
    ),
    f(
        "preset",
        "string",
        "apply a named bundle from the target's `meta.presets` first; explicit `inputs` \
         override it",
    ),
    f(
        "view",
        "true | string",
        "target what the user sees: `true` = the active viewer tab (its path and inputs) as \
         the base, a string = a specific slot (`status` lists them)",
    ),
    f(
        "stats",
        "bool",
        "add build stats to the response: which doohickeys re-ran (with self-time) vs. were \
         served from the memo cache",
    ),
];

const SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "status",
        summary: "project and session state: files, view slots with their inputs and build \
                  state, the user's active tab and selection. Reports last-published \
                  outcomes — never waits on a build, so it answers when everything else \
                  fails with a build error",
        view: false,
        fields: &[],
        js_twin: None,
        hidden: false,
    },
    CommandSpec {
        name: "inspect",
        summary: "the scene tree, measured exactly: names, bounds, counts — and, on the \
                  root entry, the view's interface",
        view: true,
        fields: &[
            f(
                "node",
                "string",
                "one node, by the name you gave it (`\"seat\"`) or by index path \
                 (`\"1/0/2\"`); default the root",
            ),
            f("depth", "number", "expand this many levels below the addressed node"),
            f("recursive", "bool", "expand fully"),
            f("full", "bool", "every measurement field, repeats expanded"),
            f(
                "fields",
                "array of strings",
                "exactly these fields; per-node: `name`, `color`, `bounds`, `tris`, `verts`, \
                 `volume`, `area`, `position`, `rotation`, `scale`, `matrix`, \
                 `world_matrix`; view-level, on the root entry only: `description`, \
                 `inputs` (every settable input: value, type/range, default, declaration \
                 site, plain vs cascade), `presets`",
            ),
        ],
        js_twin: None,
        hidden: false,
    },
    CommandSpec {
        name: "render",
        summary: "render a PNG; prints its path",
        view: true,
        fields: &[
            f("width", "number", "pixels, 16..=8192 (default 1024)"),
            f("height", "number", "pixels, 16..=8192 (default 768)"),
            f(
                "out",
                "string",
                "output file (default under `.odm/renders/`); the CLI resolves it against \
                 its own cwd, and the engine refuses to overwrite project source files",
            ),
            f(
                "wireframe",
                "bool",
                "edges only, in each object's own color — surfaces are not drawn",
            ),
            f("no_grid", "bool", "hide the ground grid"),
            f("ortho", "bool", "orthographic projection"),
            f(
                "direction",
                "[x,y,z]",
                "auto-framed camera looking along this vector (`[0,0,-1]` = top view); \
                 default isometric",
            ),
            f("eye", "[x,y,z]", "explicit camera position (pairs with `target`)"),
            f("target", "[x,y,z]", "explicit look-at point (default `[0,0,0]`)"),
            f("up", "[x,y,z]", "camera up (default `[0,0,1]`)"),
            f("fov", "number", "perspective field of view in degrees (default 45)"),
            f(
                "ortho_height",
                "number",
                "with `ortho` and an explicit camera: view height in world units",
            ),
        ],
        js_twin: None,
        hidden: false,
    },
    CommandSpec {
        name: "raycast",
        summary: "nearest surface hit along each ray",
        view: true,
        fields: &[f(
            "rays",
            "array",
            "rays to fire, each `{\"origin\": [x,y,z], \"dir\": [x,y,z]}` (optional \
             `\"max_dist\"`); all against the request's one view, answered in order — \
             `hits` holds `{id, name, distance, point, normal}` or `null` per ray",
        )],
        js_twin: Some(
            "`s.raycast(origin, dir, maxDist?)` — same query, same result shape; the CLI \
             adds `id`/`name` per hit and maps over `rays`",
        ),
        hidden: false,
    },
    CommandSpec {
        name: "poll",
        summary: "wait for messages the user typed in the viewer",
        view: false,
        fields: &[f(
            "timeout",
            "number",
            "seconds to wait before answering with no messages (default: wait until a \
             message arrives or the engine stops)",
        )],
        js_twin: None,
        hidden: false,
    },
    CommandSpec {
        name: "say",
        summary: "send a message to the user",
        view: false,
        fields: &[f("text", "string", "the message")],
        js_twin: None,
        hidden: false,
    },
    CommandSpec {
        name: "ack",
        summary: "poll's delivery handshake",
        view: false,
        fields: &[],
        js_twin: None,
        hidden: true,
    },
];

fn command_list() -> String {
    let names: Vec<&str> = SPECS.iter().filter(|s| !s.hidden).map(|s| s.name).collect();
    names.join(", ")
}

/// Commands that no longer exist, redirected to what replaced them —
/// agents remember them, so the error teaches instead of shrugging.
fn removed(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "build" | "interface" => {
            "`build` was split: `inspect {\"fields\": [\"inputs\", \"presets\"]}` reports the \
             interface, `\"stats\": true` on any view command reports build stats, and build \
             errors come back through whichever command triggered the build"
        }
        "selection" => {
            "`selection` is part of `status` now (the active view's `selection`); every \
             poll carries it too (`view.selection`)"
        }
        "tree" => "`tree` is now `inspect`",
        "sync" => {
            "every command syncs first, so there is no `sync`; `status` if the rescan is \
             all you want"
        }
        "prompt" => "`prompt` is a docs topic now: `odm docs prompt`",
        _ => return None,
    })
}

/// Parse one request against the spec table, then serde. All errors are
/// `bad-request` with enough in the message to fix the call.
pub(crate) fn parse(req: Value) -> Result<Request, CmdError> {
    let bad = |m: String| CmdError::bad_request(m);
    let Value::Object(mut obj) = req else {
        return Err(bad("request must be a JSON object".into()));
    };
    let cmd = match obj.remove("cmd") {
        None => return Err(bad(format!("missing field `cmd`; commands: {}", command_list()))),
        Some(Value::String(s)) => s,
        Some(_) => return Err(bad("`cmd` must be a string".into())),
    };
    let cmd = cmd.as_str();
    let Some(spec) = SPECS.iter().find(|s| s.name == cmd) else {
        return Err(bad(match removed(cmd) {
            Some(hint) => hint.to_string(),
            None => format!("unknown command {cmd:?}; commands: {}", command_list()),
        }));
    };
    let known = |name: &str| {
        spec.fields.iter().any(|f| f.name == name)
            || (spec.view && VIEW_FIELDS.iter().any(|f| f.name == name))
    };
    for key in obj.keys() {
        if !known(key) {
            let mut fields: Vec<&str> = spec.fields.iter().map(|f| f.name).collect();
            if spec.view {
                fields.extend(VIEW_FIELDS.iter().map(|f| f.name));
            }
            return Err(bad(match fields.is_empty() {
                true => format!("{cmd} takes no fields, got {key:?}"),
                false => format!("{cmd} has no field {key:?}; fields: {}", fields.join(", ")),
            }));
        }
    }
    // Field names are vetted, so serde only ever fails on value shape;
    // path_to_error says where, the message says what was expected.
    fn de<T: serde::de::DeserializeOwned>(
        cmd: &str,
        body: Map<String, Value>,
    ) -> Result<T, CmdError> {
        serde_path_to_error::deserialize(Value::Object(body)).map_err(|e| {
            let path = e.path().to_string();
            CmdError::bad_request(match path.as_str() {
                "." => format!("{cmd}: {}", e.inner()),
                _ => format!("{cmd}.{path}: {}", e.inner()),
            })
        })
    }
    Ok(match cmd {
        "status" => Request::Status,
        "inspect" => Request::Inspect(de(cmd, obj)?),
        "render" => Request::Render(de(cmd, obj)?),
        "raycast" => Request::Raycast(de(cmd, obj)?),
        "poll" => Request::Poll(de(cmd, obj)?),
        "say" => Request::Say(de(cmd, obj)?),
        "ack" => Request::Ack,
        _ => unreachable!("matched a spec above"),
    })
}

// --- the generated reference ---------------------------------------------

/// The markdown between `docs/cli.md`'s GENERATED markers: per-command
/// request fields, printed from the same table [`parse`] validates with.
/// Called by the drift test (which doubles as the regenerator).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn reference() -> String {
    let mut out = String::new();
    let push_fields = |out: &mut String, fields: &[FieldSpec]| {
        for f in fields {
            out.push_str(&format!("- `{}` ({}) — {}\n", f.name, f.ty, f.doc));
        }
    };
    let view_cmds: Vec<&str> =
        SPECS.iter().filter(|s| s.view && !s.hidden).map(|s| s.name).collect();
    out.push_str(&format!(
        "Request fields every view-targeting command ({}) shares:\n\n",
        view_cmds.join(", ")
    ));
    push_fields(&mut out, VIEW_FIELDS);
    for spec in SPECS.iter().filter(|s| !s.hidden) {
        out.push_str(&format!("\n### {}\n\n{}.\n", spec.name, spec.summary));
        if !spec.fields.is_empty() {
            out.push('\n');
            push_fields(&mut out, spec.fields);
        } else if !spec.view {
            out.push_str("\nNo request fields.\n");
        }
        if let Some(twin) = spec.js_twin {
            out.push_str(&format!("\nJS twin: {twin}.\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_str(s: &str) -> Result<Request, String> {
        parse(serde_json::from_str(s).unwrap()).map_err(|e| e.message().to_string())
    }

    #[test]
    fn defaults_and_typos() {
        assert!(matches!(parse_str(r#"{"cmd":"status"}"#), Ok(Request::Status)));
        match parse_str(r#"{"cmd":"inspect","path":"wheel.js","inputs":{"t":1.5},"preset":"big"}"#)
        {
            Ok(Request::Inspect(r)) => {
                assert_eq!(r.path.as_deref(), Some("wheel.js"));
                assert_eq!(r.inputs["t"], 1.5);
                assert_eq!(r.preset.as_deref(), Some("big"));
            }
            other => panic!("{:?}", other.err()),
        }
        match parse_str(r#"{"cmd":"render","width":800}"#) {
            Ok(Request::Render(r)) => assert_eq!((r.width, r.height), (800.0, 768.0)),
            other => panic!("{:?}", other.err()),
        }
        // Numbers may arrive as JSON floats.
        assert!(matches!(
            parse_str(r#"{"cmd":"inspect","depth":2.0}"#),
            Ok(Request::Inspect(r)) if r.depth == Some(2.0)
        ));
        match parse_str(r#"{"cmd":"inspect","node":"seat","full":true}"#) {
            Ok(Request::Inspect(r)) => {
                assert_eq!(r.node.as_deref(), Some("seat"));
                assert!(r.full && !r.recursive && r.fields.is_none());
            }
            other => panic!("{:?}", other.err()),
        }

        // Typos list the siblings.
        let e = parse_str(r#"{"cmd":"inspect","recurse":true}"#).err().unwrap();
        assert!(e.contains("recurse") && e.contains("recursive") && e.contains("preset"), "{e}");
        let e = parse_str(r#"{"cmd":"status","typo":1}"#).err().unwrap();
        assert!(e.contains("takes no fields"), "{e}");
        let e = parse_str(r#"{"cmd":"render","typo":1}"#).err().unwrap();
        assert!(e.contains("typo") && e.contains("wireframe"), "{e}");

        let e = parse_str(r#"{"cmd":"nope"}"#).err().unwrap();
        assert!(e.contains("status") && e.contains("raycast"), "{e}");
        let e = parse_str(r#"{}"#).err().unwrap();
        assert!(e.contains("commands: "), "{e}");

        // `set` was renamed while every call site changed.
        let e = parse_str(r#"{"cmd":"render","set":{"t":1}}"#).err().unwrap();
        assert!(e.contains("inputs"), "{e}");
    }

    #[test]
    fn removed_commands_are_redirected() {
        let e = parse_str(r#"{"cmd":"build"}"#).err().unwrap();
        assert!(e.contains("inspect") && e.contains("stats"), "{e}");
        let e = parse_str(r#"{"cmd":"selection"}"#).err().unwrap();
        assert!(e.contains("status"), "{e}");
        let e = parse_str(r#"{"cmd":"sync"}"#).err().unwrap();
        assert!(e.contains("status"), "{e}");
        let e = parse_str(r#"{"cmd":"prompt"}"#).err().unwrap();
        assert!(e.contains("docs prompt"), "{e}");
        let e = parse_str(r#"{"cmd":"tree"}"#).err().unwrap();
        assert!(e.contains("inspect"), "{e}");
    }

    #[test]
    fn wrong_shapes_name_the_field() {
        let e = parse_str(r#"{"cmd":"raycast","rays":[{"origin":[0,0],"dir":[0,0,-1]}]}"#)
            .err()
            .unwrap();
        assert!(e.contains("rays[0].origin"), "{e}");
        let e = parse_str(r#"{"cmd":"inspect","fields":"name,bounds"}"#).err().unwrap();
        assert!(e.contains("fields") && e.contains("sequence"), "{e}");
        // Unknown ray fields are caught by serde (the spec table sees only
        // the top level), still located by the path.
        let e = parse_str(r#"{"cmd":"raycast","rays":[{"origin":[0,0,9],"dri":[0,0,-1]}]}"#)
            .err()
            .unwrap();
        assert!(e.contains("dri"), "{e}");
    }

    #[test]
    fn rays_parse_with_optional_max_dist() {
        match parse_str(
            r#"{"cmd":"raycast","rays":[{"origin":[0,0,9],"dir":[0,0,-1]},
                {"origin":[1,2,3],"dir":[0,1,0],"max_dist":50}]}"#,
        ) {
            Ok(Request::Raycast(r)) => {
                assert_eq!(r.rays.len(), 2);
                assert_eq!(r.rays[0].max_dist, 1e9);
                assert_eq!(r.rays[1].max_dist, 50.0);
            }
            other => panic!("{:?}", other.err()),
        }
        let e = parse_str(r#"{"cmd":"raycast"}"#).err().unwrap();
        assert!(e.contains("rays"), "{e}");
    }

    #[test]
    fn view_takes_true_or_a_slot() {
        assert!(matches!(
            parse_str(r#"{"cmd":"inspect","view":true}"#),
            Ok(Request::Inspect(r)) if matches!(r.view, Some(ViewSel::Active(true)))
        ));
        assert!(matches!(
            parse_str(r#"{"cmd":"inspect","view":"tab-1"}"#),
            Ok(Request::Inspect(r)) if matches!(&r.view, Some(ViewSel::Slot(s)) if s == "tab-1")
        ));
    }

    #[test]
    fn chat_commands() {
        assert!(matches!(parse_str(r#"{"cmd":"poll"}"#), Ok(Request::Poll(p)) if p.timeout.is_none()));
        assert!(
            matches!(parse_str(r#"{"cmd":"poll","timeout":1.5}"#), Ok(Request::Poll(p)) if p.timeout == Some(1.5))
        );
        assert!(
            matches!(parse_str(r#"{"cmd":"say","text":"hi"}"#), Ok(Request::Say(s)) if s.text == "hi")
        );
        assert!(matches!(parse_str(r#"{"cmd":"ack"}"#), Ok(Request::Ack)));

        let e = parse_str(r#"{"cmd":"poll","timout":1}"#).err().unwrap();
        assert!(e.contains("timout") && e.contains("timeout"), "{e}");
        let e = parse_str(r#"{"cmd":"say"}"#).err().unwrap();
        assert!(e.contains("text"), "{e}");
    }

    /// The spec table and the serde structs describe the same grammar: a
    /// request using every documented field of every command must parse.
    /// (An extra serde field the table doesn't know would be unreachable —
    /// caught the moment anyone adds a call site, since [`parse`] fronts it.)
    #[test]
    fn every_documented_field_parses() {
        let sample = |f: &FieldSpec| -> Value {
            match f.name {
                "inputs" => json!({"t": 1.5}),
                "view" => json!(true),
                "rays" => json!([{"origin": [0.0, 0.0, 9.0], "dir": [0.0, 0.0, -1.0]}]),
                "fields" => json!(["name", "bounds"]),
                _ if f.ty == "number" => json!(32.0),
                _ if f.ty == "bool" => json!(true),
                _ if f.ty == "[x,y,z]" => json!([1.0, 2.0, 3.0]),
                _ => json!("x"),
            }
        };
        for spec in SPECS {
            let mut req = Map::new();
            req.insert("cmd".into(), json!(spec.name));
            let all =
                spec.fields.iter().chain(spec.view.then_some(VIEW_FIELDS).into_iter().flatten());
            for field in all {
                req.insert(field.name.into(), sample(field));
            }
            if let Err(e) = parse(Value::Object(req)) {
                panic!("{}: {}", spec.name, e.message());
            }
        }
    }

    /// `docs/cli.md`'s generated section is byte-identical to what the spec
    /// table renders. Regen: UPDATE_CLI_DOCS=1 cargo test -p odm-engine cli_reference.
    #[test]
    fn cli_reference_is_current() {
        const BEGIN: &str = "<!--- BEGIN GENERATED COMMAND REFERENCE --->";
        const END: &str = "<!--- END GENERATED COMMAND REFERENCE --->";
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli.md");
        let doc = std::fs::read_to_string(path).expect("read docs/cli.md");
        let begin = doc.find(BEGIN).expect("docs/cli.md has no BEGIN marker") + BEGIN.len();
        let end = doc.find(END).expect("docs/cli.md has no END marker");
        let want = format!("\n{}\n", reference());
        if doc[begin..end] == want {
            return;
        }
        if std::env::var_os("UPDATE_CLI_DOCS").is_some() {
            let updated = format!("{}{}{}", &doc[..begin], want, &doc[end..]);
            std::fs::write(path, updated).expect("write docs/cli.md");
            return;
        }
        panic!(
            "docs/cli.md's command reference is stale; regen with \
             UPDATE_CLI_DOCS=1 cargo test -p odm-engine cli_reference"
        );
    }
}
