//! Post-build input report: one flat list of everything settable on the
//! view — the target's own plain inputs plus every cascade input that fell
//! through to the view level (explicitly set, or resolved by a
//! declaration's auto-provided default). This is the input panel's data
//! source, the CLI's input-name typo check, and the conflict lint.
//!
//! Computed by walking the pass's memo entries (they are all fresh or
//! revalidated after a successful build), so memo hits cost nothing extra
//! during the build itself.

use crate::meta::{Input, Meta};
use crate::scheduler::{BuildEngine, Pass, View};
use odm_ir::hash_json;
use odm_store::{Dep, MemoKey};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// One view-settable input. `kind` says which channel a set value travels
/// on (the write path routes automatically, so callers can ignore it);
/// `declared_in` tells the interesting story — declared on the target, or
/// bubbled up from a part.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportEntry {
    pub name: String,
    /// What it resolved to at the view level this pass.
    pub value: Value,
    /// Where that value came from this pass.
    pub source: ValueSource,
    pub kind: InputKind,
    /// The winning (shallowest) declaration's authored schema, `cascade`
    /// stripped and every `default` normalized — arbitrary depth, which is
    /// how the panel and the agent learn an input's element shape. When
    /// equal-depth cascade declarations disagree on range, `minimum`/
    /// `maximum` here carry the union.
    pub schema: Map<String, Value>,
    /// The winning declared default (`Null` when none is declared).
    pub default: Value,
    /// Declaring files, shallowest first.
    pub declared_in: Vec<String>,
}

/// Which declaration kind (and therefore which view channel) an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// A plain input declared on the view target; set values become view args.
    Plain,
    /// A cascade input that fell through; set values become view cascade values.
    Cascade,
}

impl InputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InputKind::Plain => "plain",
            InputKind::Cascade => "cascade",
        }
    }
}

/// The origin of a reported value: set at the view, or the winning
/// declared default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueSource {
    View,
    Default,
}

impl ValueSource {
    fn of(set_at_view: bool) -> ValueSource {
        if set_at_view { ValueSource::View } else { ValueSource::Default }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ValueSource::View => "view",
            ValueSource::Default => "default",
        }
    }
}

impl ReportEntry {
    /// Flat conveniences, derived from the schema.
    pub fn ty(&self) -> Option<&str> {
        self.schema.get("type").and_then(|t| t.as_str())
    }

    pub fn minimum(&self) -> Option<f64> {
        self.schema.get("minimum").and_then(|v| v.as_f64())
    }

    pub fn maximum(&self) -> Option<f64> {
        self.schema.get("maximum").and_then(|v| v.as_f64())
    }

    pub fn description(&self) -> Option<&str> {
        self.schema.get("description").and_then(|d| d.as_str())
    }

    /// `enum` choices from the winning declaration, for choice controls.
    pub fn choices(&self) -> Option<&Vec<Value>> {
        self.schema.get("enum").and_then(|e| e.as_array())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "value": self.value,
            "source": self.source.as_str(),
            "kind": self.kind.as_str(),
            "type": self.ty(),
            "minimum": self.minimum(),
            "maximum": self.maximum(),
            "description": self.description(),
            "default": self.default,
            "choices": self.choices(),
            "schema": self.schema,
            "declared_in": self.declared_in,
        })
    }

    /// An entry straight from a declaration, with `set` the view-level value
    /// if one was set.
    fn declared(
        name: &str,
        input: &Input,
        kind: InputKind,
        set: Option<&Value>,
        declared_in: Vec<String>,
    ) -> ReportEntry {
        let default = input.default.clone().unwrap_or(Value::Null);
        ReportEntry {
            name: name.to_string(),
            value: set.cloned().unwrap_or_else(|| default.clone()),
            source: ValueSource::of(set.is_some()),
            kind,
            schema: entry_schema(&input.authored),
            default,
            declared_in,
        }
    }
}

/// An entry's `schema`: the authored declaration minus `cascade` (a
/// resolution channel, not a shape).
fn entry_schema(authored: &Map<String, Value>) -> Map<String, Value> {
    let mut schema = authored.clone();
    schema.remove("cascade");
    schema
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputReport {
    /// Everything settable on the view, one entry per name, sorted by name:
    /// the target's own plain inputs plus the cascade inputs that fell
    /// through to the view level.
    pub inputs: Vec<ReportEntry>,
    /// The view target's presets: name → input values.
    pub presets: Vec<(String, Map<String, Value>)>,
    /// Conflict lint: same name falling through in unrelated subtrees with
    /// conflicting defaults (warning) or conflicting types (error), plus
    /// shadowing (a plain target input hiding a fall-through cascade name).
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// The view target's declared inputs as report entries — what a *failed*
/// build can still answer about "what can I set". No fall-through info:
/// that needs a successful pass; the declared schema doesn't.
pub fn declared_entries(path: &str, meta: &Meta, view: &View) -> Vec<ReportEntry> {
    meta.inputs
        .iter()
        .map(|(name, input)| {
            let (kind, set) = match input.cascade {
                true => (InputKind::Cascade, view.cascade.get(name)),
                false => (InputKind::Plain, view.args.get(name)),
            };
            ReportEntry::declared(name, input, kind, set, vec![path.to_string()])
        })
        .collect()
}

/// One declaration site for a name that fell through to the view.
struct Site {
    depth: usize,
    path: String,
    schema: Map<String, Value>,
    default: Value,
}

impl Site {
    fn ty(&self) -> Option<&str> {
        self.schema.get("type").and_then(|t| t.as_str())
    }

    fn range_end(&self, key: &str) -> Option<f64> {
        self.schema.get(key).and_then(|v| v.as_f64())
    }
}

impl BuildEngine {
    /// The fall-through report for a pass whose `build_view` succeeded.
    /// Walks memo entries; a hole in the walk (evicted entry, missing
    /// source) just prunes that subtree.
    pub fn input_report(self: &Arc<Self>, pass: &Pass) -> InputReport {
        let view = pass.view();
        let mut sites: BTreeMap<String, Vec<Site>> = BTreeMap::new();
        let mut visited = HashSet::new();
        let mut cascade_warnings = BTreeSet::new();
        self.walk(
            pass,
            &view.path,
            &Value::Object(view.args.clone()),
            &HashSet::new(),
            0,
            &mut sites,
            &mut visited,
            &mut cascade_warnings,
        );

        let mut report = InputReport::default();
        report.warnings.extend(cascade_warnings);

        // The target's own plain inputs and presets, for the panel.
        let mut plain_names: HashSet<String> = HashSet::new();
        if let Some(source) = pass.snapshot().sources.get(&view.path)
            && let Ok(meta) = self.meta(&view.path, source).as_ref()
        {
            for (name, input) in &meta.inputs {
                if input.cascade {
                    continue;
                }
                plain_names.insert(name.clone());
                report.inputs.push(ReportEntry::declared(
                    name,
                    input,
                    InputKind::Plain,
                    view.args.get(name),
                    vec![view.path.clone()],
                ));
            }
            report.presets =
                meta.presets.iter().map(|(n, v)| (n.clone(), v.clone())).collect();
        }

        for (name, mut found) in sites {
            found.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
            // A plain target input shadows a same-named fall-through cascade
            // name: set values route to the plain input, so the cascade one
            // silently keeps its default. Lint it and keep the one entry a
            // set value actually reaches.
            if plain_names.contains(&name) {
                report.warnings.push(format!(
                    "input {name:?} is a plain input of {} and also falls through as a \
                     cascade input (declared in {}) — a view-set value reaches only the \
                     plain input; rename one of them",
                    view.path, found[0].path
                ));
                continue;
            }
            let win_depth = found[0].depth;
            // Equal-depth winners: ranges union; the lint below flags
            // default/type disagreements.
            let mut minimum = found[0].range_end("minimum");
            let mut maximum = found[0].range_end("maximum");
            for s in found.iter().take_while(|s| s.depth == win_depth).skip(1) {
                minimum = match (minimum, s.range_end("minimum")) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    _ => None,
                };
                maximum = match (maximum, s.range_end("maximum")) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            for s in &found[1..] {
                if s.ty() != found[0].ty() {
                    report.errors.push(format!(
                        "input {name:?} is declared with type {} in {} but type {} in {}",
                        fmt_ty(found[0].ty()),
                        found[0].path,
                        fmt_ty(s.ty()),
                        s.path
                    ));
                } else if s.default != found[0].default {
                    report.warnings.push(format!(
                        "input {name:?} falls through with default {} in {} but {} in {}; \
                         subtrees disagree until it is set at the view",
                        found[0].default, found[0].path, s.default, s.path
                    ));
                }
            }
            let source = ValueSource::of(view.cascade.contains_key(&name));
            let value =
                view.cascade.get(&name).cloned().unwrap_or_else(|| found[0].default.clone());
            let mut declared_in: Vec<String> = Vec::new();
            for s in &found {
                if !declared_in.contains(&s.path) {
                    declared_in.push(s.path.clone());
                }
            }
            // The winning schema, with the (possibly unioned) range spliced
            // back in so the flat fields and the schema agree.
            let mut schema = found[0].schema.clone();
            for (key, end) in [("minimum", minimum), ("maximum", maximum)] {
                match end.and_then(serde_json::Number::from_f64) {
                    Some(n) => schema.insert(key.into(), Value::Number(n)),
                    None => schema.remove(key),
                };
            }
            report.inputs.push(ReportEntry {
                name,
                value,
                source,
                kind: InputKind::Cascade,
                schema,
                default: found[0].default.clone(),
                declared_in,
            });
        }
        report.inputs.sort_by(|a, b| a.name.cmp(&b.name));
        report
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        self: &Arc<Self>,
        pass: &Pass,
        path: &str,
        args: &Value,
        covered: &HashSet<String>,
        depth: usize,
        sites: &mut BTreeMap<String, Vec<Site>>,
        visited: &mut HashSet<(odm_ir::Hash, odm_ir::Hash, u64)>,
        cascade_warnings: &mut BTreeSet<String>,
    ) {
        let Some(source) = pass.snapshot().sources.get(path) else { return };
        let meta = self.meta(path, source);
        let Ok(meta) = meta.as_ref() else { return };
        let Ok(effective) = crate::scheduler::effective_args(path, meta, args, false) else {
            return;
        };
        let args_hash = hash_json(&Value::Object(effective));

        // A node can be reached along many paths (shared parts); its
        // *uncovered* set differs per path, so key the visited set on a
        // coarse cover fingerprint to bound rework while staying correct
        // for the common diamond case.
        let mut cover: Vec<&String> = covered.iter().collect();
        cover.sort();
        let cover_fp = {
            use std::hash::{Hash as _, Hasher as _};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for c in &cover {
                c.hash(&mut h);
            }
            h.finish()
        };
        if !visited.insert((source.hash, args_hash, cover_fp)) {
            return;
        }

        for (name, input) in &meta.inputs {
            if input.cascade && !covered.contains(name) {
                sites.entry(name.clone()).or_default().push(Site {
                    depth,
                    path: path.to_string(),
                    schema: entry_schema(&input.authored),
                    default: input.default.clone().unwrap_or(Value::Null),
                });
            }
        }

        let key = MemoKey { code: source.hash, args: args_hash };
        let Some(entry) = self.store.memo_get(&key) else { return };
        for dep in &entry.deps {
            if let Dep::Invoke { path: child, args, cascade, .. } = dep {
                for name in cascade.keys() {
                    if !self.subtree_declares(pass, child, args, name, &mut HashSet::new()) {
                        cascade_warnings.insert(unread_cascade_warning(
                            self, pass, path, child, name,
                        ));
                    }
                }
                let mut child_covered = covered.clone();
                child_covered.extend(cascade.keys().cloned());
                self.walk(
                    pass,
                    child,
                    args,
                    &child_covered,
                    depth + 1,
                    sites,
                    visited,
                    cascade_warnings,
                );
            }
        }
    }

    /// Does the subtree rooted at `(path, args)` reach a declaration of
    /// cascade input `name` without an intervening invoke re-providing it?
    /// Unknowns (missing source, broken meta, evicted memo entry) count as
    /// "yes" so holes in the walk never produce spurious warnings.
    fn subtree_declares(
        self: &Arc<Self>,
        pass: &Pass,
        path: &str,
        args: &Value,
        name: &str,
        seen: &mut HashSet<(odm_ir::Hash, odm_ir::Hash)>,
    ) -> bool {
        let Some(source) = pass.snapshot().sources.get(path) else { return true };
        let meta = self.meta(path, source);
        let Ok(meta) = meta.as_ref() else { return true };
        if meta.inputs.get(name).is_some_and(|i| i.cascade) {
            return true;
        }
        let Ok(effective) = crate::scheduler::effective_args(path, meta, args, false) else {
            return true;
        };
        let args_hash = hash_json(&Value::Object(effective));
        if !seen.insert((source.hash, args_hash)) {
            return false;
        }
        let Some(entry) = self.store.memo_get(&MemoKey { code: source.hash, args: args_hash })
        else {
            return true;
        };
        entry.deps.iter().any(|dep| match dep {
            Dep::Invoke { path: child, args, cascade, .. } if !cascade.contains_key(name) => {
                self.subtree_declares(pass, child, args, name, seen)
            }
            _ => false,
        })
    }
}

/// The message for a cascaded value no descendant reads: a misplaced plain
/// input gets a pointed channel hint, anything else is likely a typo.
fn unread_cascade_warning(
    engine: &Arc<BuildEngine>,
    pass: &Pass,
    parent: &str,
    child: &str,
    name: &str,
) -> String {
    let plain_input = pass
        .snapshot()
        .sources
        .get(child)
        .and_then(|s| {
            engine.meta(child, s).as_ref().as_ref().ok().map(|m| {
                m.inputs.get(name).is_some_and(|i| !i.cascade)
            })
        })
        .unwrap_or(false);
    if plain_input {
        format!(
            "{parent} provides {name:?} to {child}, but {name:?} is a plain input there — \
             pass it in the invoke's args (second argument), not the cascade"
        )
    } else {
        format!(
            "{parent} provides {name:?} to {child}, but nothing in that subtree declares a \
             cascade input {name:?} — the value is never read"
        )
    }
}

/// A view-set input value that nothing in the built tree can read is almost
/// certainly a typo; names must be view-settable (in the report) or the
/// target's own declared inputs (which includes plain args, checked at the
/// boundary).
pub fn check_input_names(
    cascade: &Map<String, Value>,
    target_meta: &Meta,
    report: &InputReport,
) -> Result<(), String> {
    for name in cascade.keys() {
        let declared_on_target = target_meta.inputs.contains_key(name);
        let in_report = report.inputs.iter().any(|e| &e.name == name);
        if !declared_on_target && !in_report {
            let mut known: Vec<&str> = target_meta
                .inputs
                .keys()
                .map(|s| s.as_str())
                .chain(report.inputs.iter().map(|e| e.name.as_str()))
                .collect();
            known.sort();
            known.dedup();
            return Err(format!(
                "nothing in this view reads input {name:?}; settable inputs: {}",
                if known.is_empty() { "(none)".into() } else { known.join(", ") }
            ));
        }
    }
    Ok(())
}

fn fmt_ty(t: Option<&str>) -> String {
    match t {
        Some(t) => format!("{t:?}"),
        None => "(untyped)".into(),
    }
}
