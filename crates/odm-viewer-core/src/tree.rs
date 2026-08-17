//! The scene tree: expansion state, row drawing, and click-to-select.

use crate::icons::Icon;
use crate::theme;
use eframe::egui;
use odm_ir::{Hash, Node};
use odm_store::{Object, Store};
use std::collections::{HashMap, HashSet};

/// The scene tree as the UI needs it: IR children are content hashes, so the
/// viewer materializes one snapshot per published build instead of walking
/// the store every frame.
pub(crate) struct TreeNode {
    pub(crate) name: Option<String>,
    pub(crate) has_mesh: bool,
    pub(crate) children: Vec<TreeNode>,
}

impl TreeNode {
    /// Materialize a subtree; None if a child hash is missing from the store
    /// (the caller retries, as with a failed flatten).
    pub(crate) fn from_node(store: &Store, node: &Node) -> Option<TreeNode> {
        let children = node
            .children
            .iter()
            .map(|&h| Self::from_hash(store, h))
            .collect::<Option<Vec<_>>>()?;
        Some(TreeNode { name: node.name.clone(), has_mesh: node.mesh.is_some(), children })
    }

    fn from_hash(store: &Store, h: Hash) -> Option<TreeNode> {
        match store.get(h).as_deref() {
            Some(Object::Node(n)) => Self::from_node(store, n),
            _ => None,
        }
    }
}

/// Which scene-tree nodes are expanded. Kept here rather than in egui's
/// collapsing-header memory so auto-expand can tell its own doing from the
/// user's, and undo only its own.
#[derive(Default)]
pub(crate) struct TreeState {
    /// Open/closed where it differs from the default (open above `AUTO_DEPTH`).
    open: HashMap<String, bool>,
    /// Nodes opened to reveal a selection, each with the `open` entry it
    /// displaced — put back when no selection needs the node any more.
    auto: HashMap<String, Option<bool>>,
}

/// Depth below which nodes start expanded.
const AUTO_DEPTH: usize = 2;

impl TreeState {
    fn is_open(&self, id: &str, depth: usize) -> bool {
        self.open.get(id).copied().unwrap_or(depth < AUTO_DEPTH)
    }

    /// A user toggle takes the node out of auto-expand's hands for good.
    fn set_manual(&mut self, id: &str, open: bool) {
        self.open.insert(id.to_string(), open);
        self.auto.remove(id);
    }

    /// Expand every ancestor of a selected node, and collapse the ones expanded
    /// for a selection that has since gone away.
    pub(crate) fn reveal(&mut self, selected: &[(String, Option<String>)]) {
        let mut needed: HashSet<String> = HashSet::new();
        for (id, _) in selected {
            if id.is_empty() {
                continue;
            }
            needed.insert(String::new());
            needed.extend(id.match_indices('/').map(|(i, _)| id[..i].to_string()));
        }
        let stale: Vec<String> =
            self.auto.keys().filter(|id| !needed.contains(*id)).cloned().collect();
        for id in stale {
            match self.auto.remove(&id).expect("stale key came from auto") {
                Some(prev) => self.open.insert(id, prev),
                None => self.open.remove(&id),
            };
        }
        for id in needed {
            if !self.is_open(&id, node_depth(&id)) {
                self.auto.insert(id.clone(), self.open.get(&id).copied());
                self.open.insert(id, true);
            }
        }
    }
}

/// Depth of a node id: "" is 0, "3" is 1, "3/1" is 2, ...
fn node_depth(id: &str) -> usize {
    if id.is_empty() { 0 } else { id.matches('/').count() + 1 }
}

/// The tree's shared state for one pass of `tree_node_ui`.
pub(crate) struct TreeUi<'a> {
    pub(crate) tree: &'a mut TreeState,
    pub(crate) selected: &'a [(String, Option<String>)],
    /// The row clicked this frame: (node id, name, shift held).
    pub(crate) clicked: Option<(String, Option<String>, bool)>,
}

/// Draw `node` and, if it is open, its subtree. `trunk` carries, per ancestor
/// depth, whether that ancestor's sibling line runs past these rows.
pub(crate) fn tree_node_ui(
    ui: &mut egui::Ui,
    node: &TreeNode,
    id: &str,
    depth: usize,
    trunk: &mut Vec<bool>,
    last: bool,
    tv: &mut TreeUi<'_>,
) {
    let label = match &node.name {
        Some(n) => n.clone(),
        None => {
            if id.is_empty() {
                "(root)".to_string()
            } else {
                format!("#{}", id.rsplit('/').next().unwrap())
            }
        }
    };
    let row_id = ui.make_persistent_id(format!("tree-{id}"));
    let has_children = !node.children.is_empty();
    let mut open = has_children && tv.tree.is_open(id, depth);

    let res = theme::tree_row(
        ui,
        row_id,
        theme::TreeRow {
            depth,
            trunk,
            last,
            expander: has_children.then_some(open),
            icon: if node.has_mesh { Icon::Mesh } else { Icon::Empty },
            selected: tv.selected.iter().any(|(sel, _)| sel == id),
        },
        &label,
    );
    if res.row.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        tv.clicked = Some((id.to_string(), node.name.clone(), shift));
    }
    // The box toggles, the name selects — and double-clicking the name does
    // both, as the era's tree controls did.
    if has_children && (res.expander.is_some_and(|e| e.clicked()) || res.row.double_clicked()) {
        open = !open;
        tv.tree.set_manual(id, open);
    }

    if open {
        trunk.push(!last);
        let n = node.children.len();
        for (i, child) in node.children.iter().enumerate() {
            let child_id = odm_render::node_id(id, i);
            tree_node_ui(ui, child, &child_id, depth + 1, trunk, i + 1 == n, tv);
        }
        trunk.pop();
    }
}

/// Apply a click on `hit` (`None` = empty space) to the selection: shift adds
/// the node, or removes it if it was already selected; a plain click replaces
/// whatever was selected.
pub(crate) fn click_selection(
    selected: Vec<(String, Option<String>)>,
    hit: Option<(String, Option<String>)>,
    additive: bool,
) -> Vec<(String, Option<String>)> {
    let mut sel = if additive { selected } else { Vec::new() };
    if let Some((id, name)) = hit {
        match sel.iter().position(|(s, _)| *s == id) {
            Some(i) => drop(sel.remove(i)),
            None => sel.push((id, name)),
        }
    }
    sel
}

/// Selecting a group highlights its whole subtree.
pub fn selection_covers(selected: &str, id: &str) -> bool {
    selected.is_empty() || id == selected || id.starts_with(&format!("{selected}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(ids: &[&str]) -> Vec<(String, Option<String>)> {
        ids.iter().map(|id| (id.to_string(), None)).collect()
    }

    fn open(tree: &TreeState, id: &str) -> bool {
        tree.is_open(id, node_depth(id))
    }

    fn node(name: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode { name: Some(name.to_string()), has_mesh: false, children }
    }

    /// A headless tree, one frame at a time, so clicks can be aimed at real
    /// row rects and land through egui's own interaction path.
    struct Harness {
        ctx: egui::Context,
        tree: TreeState,
        root: TreeNode,
        selected: Vec<(String, Option<String>)>,
        /// Row and expander rects from the last frame, by node id.
        rects: HashMap<String, (egui::Rect, Option<egui::Rect>)>,
        clicked: Option<(String, Option<String>, bool)>,
    }

    impl Harness {
        fn new(root: TreeNode) -> Harness {
            let mut h = Harness {
                ctx: egui::Context::default(),
                tree: TreeState::default(),
                root,
                selected: Vec::new(),
                rects: HashMap::new(),
                clicked: None,
            };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            // Modifiers ride on the frame, not only on the events, exactly as
            // the windowing backend delivers them.
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 800.0),
                )),
                modifiers,
                events,
                ..Default::default()
            };
            let (tree, root, selected) = (&mut self.tree, &self.root, &self.selected);
            let (mut clicked, mut rects) = (None, HashMap::new());
            let _ = self.ctx.run_ui(input, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut tv = TreeUi { tree, selected, clicked: None };
                tree_node_ui(ui, root, "", 0, &mut Vec::new(), true, &mut tv);
                clicked = tv.clicked;
                for id in ["", "0", "0/0", "0/0/0", "0/0/1", "1"] {
                    let row_id = ui.make_persistent_id(format!("tree-{id}"));
                    let rect = |w: egui::Id| ui.ctx().read_response(w).map(|r| r.rect);
                    if let Some(r) = rect(row_id.with("row")) {
                        rects.insert(id.to_string(), (r, rect(row_id.with("expander"))));
                    }
                }
            });
            (self.clicked, self.rects) = (clicked, rects);
        }

        /// Press and release over `pos`, which takes two frames to become a
        /// click; the reported click lands on the release frame.
        fn click_at(&mut self, pos: egui::Pos2, shift: bool) {
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
            self.frame(vec![button(false)]);
        }

        fn row(&self, id: &str) -> egui::Pos2 {
            self.rects.get(id).unwrap_or_else(|| panic!("no row {id:?}")).0.center()
        }

        fn expander(&self, id: &str) -> egui::Pos2 {
            self.rects.get(id).and_then(|r| r.1).unwrap_or_else(|| panic!("no +/- box {id:?}")).center()
        }

        /// Click a row the way the viewer does: report it, then apply it.
        fn select_row(&mut self, id: &str, shift: bool) {
            self.click_at(self.row(id), shift);
            let (id, name, additive) = self.clicked.clone().expect("row click");
            self.selected =
                click_selection(std::mem::take(&mut self.selected), Some((id, name)), additive);
            self.tree.reveal(&self.selected);
            self.frame(Vec::new());
        }
    }

    fn ids(sel: &[(String, Option<String>)]) -> Vec<&str> {
        sel.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// cart > wheel-fl > wheel > (tire, hub): deep enough that the leaves
    /// start hidden, since only the top two levels are open by default.
    fn cart() -> TreeNode {
        let wheel = node("wheel", vec![node("tire", vec![]), node("hub", vec![])]);
        node("cart", vec![node("wheel-fl", vec![wheel]), node("chassis", vec![])])
    }

    /// Clicks land on rows, and shift reaches the handler: plain clicks
    /// replace the selection, shift-clicks add to it and toggle back off.
    #[test]
    fn tree_rows_multi_select() {
        let mut h = Harness::new(cart());

        h.select_row("0", false);
        assert_eq!(ids(&h.selected), ["0"]);
        h.select_row("1", true);
        assert_eq!(ids(&h.selected), ["0", "1"]);
        h.select_row("0", true);
        assert_eq!(ids(&h.selected), ["1"], "shift-clicking a selected row deselects it");
        h.select_row("0", false);
        assert_eq!(ids(&h.selected), ["0"], "a plain click replaces the selection");
    }

    /// Selecting a hidden node brings it into view by expanding its parents,
    /// and the rows that appear are clickable.
    #[test]
    fn tree_reveals_selected_rows() {
        let mut h = Harness::new(cart());
        assert!(!h.rects.contains_key("0/0/1"), "great-grandchildren start folded");

        h.selected = sel(&["0/0/1"]);
        h.tree.reveal(&h.selected);
        h.frame(Vec::new());
        assert!(h.rects.contains_key("0/0/1"), "revealed by the selection");

        h.select_row("0/0/0", true);
        assert_eq!(ids(&h.selected), ["0/0/1", "0/0/0"]);
    }

    /// The +/- box toggles without selecting, and its state sticks.
    #[test]
    fn expander_box_toggles() {
        let mut h = Harness::new(cart());
        h.click_at(h.expander("0/0"), false);
        assert!(h.clicked.is_none(), "the box is not a selection");
        assert!(open(&h.tree, "0/0"));
        h.frame(Vec::new());
        assert!(h.rects.contains_key("0/0/0"));
        h.click_at(h.expander("0/0"), false);
        assert!(!open(&h.tree, "0/0"));
    }

    /// Revealing a deep node opens its ancestors; dropping the selection puts
    /// them back exactly as they were.
    #[test]
    fn auto_expand_is_undone() {
        let mut tree = TreeState::default();
        assert!(!open(&tree, "0/1/2"));
        tree.reveal(&sel(&["0/1/2/3"]));
        assert!(open(&tree, "0/1/2") && open(&tree, "0/1") && open(&tree, "0"));
        tree.reveal(&[]);
        assert!(!open(&tree, "0/1/2"));
        assert!(tree.open.is_empty(), "no leftover overrides: {:?}", tree.open);
    }

    /// A manual expand outlives the selection that happened to reveal it —
    /// whether it came before the auto-expand or after.
    #[test]
    fn manual_expand_survives() {
        let mut tree = TreeState::default();
        tree.set_manual("0/1/2", true);
        tree.reveal(&sel(&["0/1/2/3/4"]));
        tree.reveal(&[]);
        assert!(open(&tree, "0/1/2"), "expanded before the selection");
        assert!(!open(&tree, "0/1/2/3"), "only auto-expanded");

        tree.reveal(&sel(&["0/1/2/3/4"]));
        tree.set_manual("0/1/2/3", false);
        tree.set_manual("0/1/2/3", true);
        tree.reveal(&[]);
        assert!(open(&tree, "0/1/2/3"), "re-expanded by hand since the auto-expand");
    }

    /// A node collapsed by hand goes back to collapsed, not to its default.
    #[test]
    fn manual_collapse_is_restored() {
        let mut tree = TreeState::default();
        tree.set_manual("0", false);
        tree.reveal(&sel(&["0/1"]));
        assert!(open(&tree, "0"));
        tree.reveal(&[]);
        assert!(!open(&tree, "0"));
    }

    /// Shift accumulates and toggles; a plain click starts over.
    #[test]
    fn shift_click_accumulates() {
        let hit = |id: &str| Some((id.to_string(), None));
        let s = click_selection(Vec::new(), hit("0"), false);
        let s = click_selection(s, hit("1/2"), true);
        assert_eq!(s, sel(&["0", "1/2"]));
        // Shift-clicking a selected node takes it back out.
        let s = click_selection(s, hit("0"), true);
        assert_eq!(s, sel(&["1/2"]));
        // Shift on empty space leaves the selection alone; a plain click clears.
        let s = click_selection(s, None, true);
        assert_eq!(s, sel(&["1/2"]));
        assert_eq!(click_selection(s, None, false), sel(&[]));
    }

    /// Ancestors stay open while any selected node still needs them.
    #[test]
    fn reveal_keeps_shared_ancestors() {
        let mut tree = TreeState::default();
        tree.reveal(&sel(&["0/1/2/9", "0/5/6"]));
        assert!(open(&tree, "0/1/2") && open(&tree, "0/5"));
        tree.reveal(&sel(&["0/1/2/9"]));
        assert!(open(&tree, "0/1/2"), "still selected");
        assert!(!open(&tree, "0/5"), "no longer selected");
    }
}
