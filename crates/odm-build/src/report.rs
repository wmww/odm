//! Post-build fall-through report: which cascade inputs resolved at the
//! view level (explicitly set, or by a declaration's auto-provided default),
//! and the winning declarations. This is the input panel's data source, the
//! CLI's `--set` typo check, and the conflict lint.
//!
//! Computed by walking the pass's memo entries (they are all fresh or
//! revalidated after a successful build), so memo hits cost nothing extra
//! during the build itself.

use crate::meta::Meta;
use crate::scheduler::{BuildEngine, Pass};
use odm_ir::hash_json;
use odm_store::{Dep, MemoKey};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// One view-settable cascade input: `name` fell through to the view level
/// somewhere in the built tree.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportEntry {
    pub name: String,
    /// What it resolved to at the view level this pass.
    pub value: Value,
    /// Explicitly set by the view (vs. the winning declared default).
    pub set: bool,
    /// From the winning (shallowest) declaration.
    pub ty: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub description: Option<String>,
    pub default: Value,
    /// `enum` choices from the winning declaration, for choice controls.
    pub choices: Option<Vec<Value>>,
    /// Declaring files, shallowest first.
    pub declared_in: Vec<String>,
}

impl ReportEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "value": self.value,
            "set": self.set,
            "type": self.ty,
            "minimum": self.minimum,
            "maximum": self.maximum,
            "description": self.description,
            "default": self.default,
            "choices": self.choices,
            "declared_in": self.declared_in,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputReport {
    /// The view target's own plain (non-cascade) inputs — the "root args"
    /// half of the input panel. Sorted by name.
    pub args: Vec<ReportEntry>,
    /// Cascade inputs that fell through to the view level. Sorted by name.
    pub entries: Vec<ReportEntry>,
    /// The view target's presets: name → input values.
    pub presets: Vec<(String, Map<String, Value>)>,
    /// Conflict lint: same name falling through in unrelated subtrees with
    /// conflicting defaults (warning) or conflicting types (error).
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl InputReport {
    pub fn to_json(&self) -> Value {
        json!({
            "args": self.args.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
            "inputs": self.entries.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
            "presets": self.presets.iter().map(|(n, v)| (n.clone(), Value::Object(v.clone()))).collect::<serde_json::Map<_, _>>(),
            "warnings": self.warnings,
            "errors": self.errors,
        })
    }
}

/// One declaration site for a name that fell through to the view.
struct Site {
    depth: usize,
    path: String,
    ty: Option<String>,
    default: Value,
    minimum: Option<f64>,
    maximum: Option<f64>,
    description: Option<String>,
    choices: Option<Vec<Value>>,
}

impl BuildEngine {
    /// The fall-through report for a pass whose `build_view` succeeded.
    /// Walks memo entries; a hole in the walk (evicted entry, missing
    /// source) just prunes that subtree.
    pub fn input_report(self: &Arc<Self>, pass: &Pass) -> InputReport {
        let view = pass.view();
        let mut sites: BTreeMap<String, Vec<Site>> = BTreeMap::new();
        let mut visited = HashSet::new();
        let mut provide_warnings = BTreeSet::new();
        self.walk(
            pass,
            &view.path,
            &Value::Object(view.args.clone()),
            &HashSet::new(),
            0,
            &mut sites,
            &mut visited,
            &mut provide_warnings,
        );

        let mut report = InputReport::default();
        report.warnings.extend(provide_warnings);

        // The target's own plain inputs and presets, for the panel.
        if let Some(source) = pass.snapshot().sources.get(&view.path)
            && let Ok(meta) = self.meta(&view.path, source).as_ref()
        {
            for (name, input) in &meta.inputs {
                if input.cascade {
                    continue;
                }
                let set = view.args.contains_key(name);
                let default = input.default.clone().unwrap_or(Value::Null);
                report.args.push(ReportEntry {
                    name: name.clone(),
                    value: view.args.get(name).cloned().unwrap_or_else(|| default.clone()),
                    set,
                    ty: input.type_name().map(|t| t.to_string()),
                    minimum: input.minimum(),
                    maximum: input.maximum(),
                    description: input
                        .authored
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|d| d.to_string()),
                    default,
                    choices: choices_of(&input.authored),
                    declared_in: vec![view.path.clone()],
                });
            }
            report.presets =
                meta.presets.iter().map(|(n, v)| (n.clone(), v.clone())).collect();
        }

        for (name, mut found) in sites {
            found.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
            let win_depth = found[0].depth;
            // Equal-depth winners: ranges union; the lint below flags
            // default/type disagreements.
            let mut minimum = found[0].minimum;
            let mut maximum = found[0].maximum;
            for s in found.iter().take_while(|s| s.depth == win_depth).skip(1) {
                minimum = match (minimum, s.minimum) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    _ => None,
                };
                maximum = match (maximum, s.maximum) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            for s in &found[1..] {
                if s.ty != found[0].ty {
                    report.errors.push(format!(
                        "input {name:?} is declared with type {} in {} but type {} in {}",
                        fmt_ty(&found[0].ty),
                        found[0].path,
                        fmt_ty(&s.ty),
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
            let set = view.provides.contains_key(&name);
            let value =
                view.provides.get(&name).cloned().unwrap_or_else(|| found[0].default.clone());
            let mut declared_in: Vec<String> = Vec::new();
            for s in &found {
                if !declared_in.contains(&s.path) {
                    declared_in.push(s.path.clone());
                }
            }
            report.entries.push(ReportEntry {
                name,
                value,
                set,
                ty: found[0].ty.clone(),
                minimum,
                maximum,
                description: found[0].description.clone(),
                default: found[0].default.clone(),
                choices: found[0].choices.clone(),
                declared_in,
            });
        }
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
        provide_warnings: &mut BTreeSet<String>,
    ) {
        let Some(source) = pass.snapshot().sources.get(path) else { return };
        let meta = self.meta(path, source);
        let Ok(meta) = meta.as_ref() else { return };
        let Ok(effective) = crate::scheduler::effective_args(path, meta, args) else { return };
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
                    ty: input.type_name().map(|t| t.to_string()),
                    default: input.default.clone().unwrap_or(Value::Null),
                    minimum: input.minimum(),
                    maximum: input.maximum(),
                    description: input
                        .authored
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|d| d.to_string()),
                    choices: choices_of(&input.authored),
                });
            }
        }

        let key = MemoKey { code: source.hash, args: args_hash };
        let Some(entry) = self.store.memo_get(&key) else { return };
        for dep in &entry.deps {
            if let Dep::Invoke { path: child, args, provides, .. } = dep {
                for name in provides.keys() {
                    if !self.subtree_declares(pass, child, args, name, &mut HashSet::new()) {
                        provide_warnings.insert(unconsumed_provide_warning(
                            self, pass, path, child, name,
                        ));
                    }
                }
                let mut child_covered = covered.clone();
                child_covered.extend(provides.keys().cloned());
                self.walk(
                    pass,
                    child,
                    args,
                    &child_covered,
                    depth + 1,
                    sites,
                    visited,
                    provide_warnings,
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
        let Ok(effective) = crate::scheduler::effective_args(path, meta, args) else {
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
            Dep::Invoke { path: child, args, provides, .. } if !provides.contains_key(name) => {
                self.subtree_declares(pass, child, args, name, seen)
            }
            _ => false,
        })
    }
}

/// The message for a provide no descendant can read: a misplaced plain
/// input gets a pointed channel hint, anything else is likely a typo.
fn unconsumed_provide_warning(
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
             pass it in the invoke's args (second argument), not provides"
        )
    } else {
        format!(
            "{parent} provides {name:?} to {child}, but nothing in that subtree declares a \
             cascade input {name:?} — the value is never read"
        )
    }
}

/// A view `--set`/provide that nothing in the built tree can read is almost
/// certainly a typo; names must be view-settable (in the report) or the
/// target's own declared inputs (which includes plain args, checked at the
/// boundary).
pub fn check_set_names(
    provides: &Map<String, Value>,
    target_meta: &Meta,
    report: &InputReport,
) -> Result<(), String> {
    for name in provides.keys() {
        let declared_on_target = target_meta.inputs.contains_key(name);
        let in_report = report.entries.iter().any(|e| &e.name == name);
        if !declared_on_target && !in_report {
            let mut known: Vec<&str> = target_meta
                .inputs
                .keys()
                .map(|s| s.as_str())
                .chain(report.entries.iter().map(|e| e.name.as_str()))
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

fn choices_of(authored: &Map<String, Value>) -> Option<Vec<Value>> {
    authored.get("enum").and_then(|e| e.as_array()).map(|a| a.to_vec())
}

fn fmt_ty(t: &Option<String>) -> String {
    match t {
        Some(t) => format!("{t:?}"),
        None => "(untyped)".into(),
    }
}
