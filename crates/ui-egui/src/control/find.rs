//! `ui.find` and `control` targets: a control of the window found by its accessibility name.
//!
//! egui builds the accessibility tree of a pass only after `enable_accesskit`, and hands it out only
//! with the pass's output, so [`KeepTree`] keeps the root viewport's last update for the requests
//! that read it. `{name, role?, within?, index?}` is the one way to name a control, for `ui.find`
//! and for the `control` of the pointer methods.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::accesskit::{Node, NodeId, Toggled, TreeUpdate};
use serde_json::{Value, json};

/// The error before the first pass was drawn with the tree on.
pub(crate) const NO_TREE: &str = "no accessibility tree yet: send the request after a frame was drawn";
/// The error for a control that has no bounds or an empty rectangle.
pub(crate) const NO_PLACE: &str = "the control has no place on the screen";
/// Largest `index` a target may give.
const MAX_INDEX: usize = 1024;
/// The fields a target may hold.
const FIELDS: [&str; 4] = ["name", "role", "within", "index"];

fn tree_slot() -> egui::Id {
    egui::Id::new("vectorcraft control accessibility tree")
}

fn viewport_slot() -> egui::Id {
    egui::Id::new("vectorcraft control accessibility viewport")
}

/// Keeps the root viewport's accessibility update of the last pass in the context.
struct KeepTree;

impl egui::Plugin for KeepTree {
    fn debug_name(&self) -> &'static str {
        "vectorcraft control accessibility tree"
    }

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        ctx.data_mut(|d| d.insert_temp(viewport_slot(), input.viewport_id));
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let root = ctx.data(|d| d.get_temp::<egui::ViewportId>(viewport_slot())) == Some(egui::ViewportId::ROOT);
        if let (true, Some(update)) = (root, &output.platform_output.accesskit_update) {
            ctx.data_mut(|d| d.insert_temp(tree_slot(), Arc::new(update.clone())));
        }
    }
}

/// Turns the tree on and starts keeping it, once per context. Its cost (the nodes of every pass and
/// one clone) is paid only by an app with a control channel.
pub(crate) fn keep(ctx: &egui::Context) {
    if ctx.with_plugin(|_: &mut KeepTree| ()).is_none() {
        ctx.add_plugin(KeepTree);
        ctx.enable_accesskit();
    }
}

/// Which control a request means.
struct Target {
    name: String,
    role: Option<String>,
    within: Option<String>,
    index: Option<usize>,
}

/// An optional string field: none when absent or null.
fn text(map: &serde_json::Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// A name as it is compared: without surrounding white space and one trailing `:` (row labels carry it).
fn plain(s: &str) -> &str {
    let s = s.trim();
    s.strip_suffix(':').unwrap_or(s)
}

impl Target {
    /// Reads `{name, role?, within?, index?}`; any other field is an error naming it.
    fn from_params(value: &Value) -> Result<Self, String> {
        let map = value.as_object().ok_or("missing `name`")?;
        if let Some(field) = map.keys().find(|k| !FIELDS.contains(&k.as_str())) {
            return Err(format!("unknown field `{field}`"));
        }
        let name = text(map, "name")?.ok_or("missing `name`")?;
        if name.is_empty() {
            return Err("`name` must not be empty".into());
        }
        let index = match map.get("index").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i <= MAX_INDEX)
                    .ok_or_else(|| format!("`index` must be a non-negative integer, at most {MAX_INDEX}"))?,
            ),
        };
        Ok(Self { name, role: text(map, "role")?, within: text(map, "within")?, index })
    }

    /// The error for no match, naming each field the request gave.
    fn no_control(&self) -> String {
        // PLAN: the separator between the appended fields is not fixed by the sheet; ", " assumed.
        let given = [
            self.role.as_ref().map(|r| format!("role `{r}`")),
            self.within.as_ref().map(|w| format!("within `{w}`")),
            self.index.map(|i| format!("index {i}")),
        ];
        let mut message = format!("no control matches: name `{}`", self.name);
        for part in given.into_iter().flatten() {
            message.push_str(", ");
            message.push_str(&part);
        }
        message
    }
}

/// A kept update, read by node.
struct Tree<'a> {
    update: &'a TreeUpdate,
    nodes: HashMap<NodeId, &'a Node>,
    root: NodeId,
}

/// A node's own label when it has one.
fn label(node: &Node) -> Option<&str> {
    node.label().filter(|l| !l.is_empty())
}

/// The place of a node on the screen, in points: none without bounds or when empty.
// PLAN: the egui fork's `bounds` convention (points or pixels) was not readable here; read as points
// as the framework's `bounds_of` does. Divide by `pixels_per_point` if the fork writes pixels.
fn rect(node: &Node) -> Option<egui::Rect> {
    let b = node.bounds()?;
    let r = egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32));
    (r.is_finite() && r.is_positive()).then_some(r)
}

impl<'a> Tree<'a> {
    fn new(update: &'a TreeUpdate) -> Option<Self> {
        let root = update.tree.as_ref()?.root;
        Some(Self { update, nodes: update.nodes.iter().map(|(id, node)| (*id, node)).collect(), root })
    }

    /// A node's name: its own label, else the first label among the nodes that label it.
    fn name(&self, node: &'a Node) -> Option<&'a str> {
        label(node).or_else(|| node.labelled_by().iter().find_map(|id| self.nodes.get(id).copied().and_then(label)))
    }

    /// The nodes that match, depth first in child order. Hidden nodes and what is below them are skipped.
    fn matching(&self, target: &Target) -> Vec<(NodeId, &'a Node)> {
        let want = plain(&target.name);
        let within = target.within.as_deref().map(plain);
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![(self.root, within.is_none())];
        while let Some((id, under)) = stack.pop() {
            // A malformed update may reach a node twice.
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(&id).copied() else { continue };
            if node.is_hidden() {
                continue;
            }
            let name = self.name(node).map(plain);
            let role_fits = || target.role.as_deref().is_none_or(|r| format!("{:?}", node.role()).eq_ignore_ascii_case(r));
            if under && name == Some(want) && role_fits() {
                found.push((id, node));
            }
            let under = under || within.is_some_and(|w| name == Some(w));
            stack.extend(node.children().iter().rev().map(|child| (*child, under)));
        }
        found
    }

    /// The one node a target means.
    fn find(&self, target: &Target) -> Result<(NodeId, &'a Node), String> {
        let found = self.matching(target);
        match (target.index, found.as_slice()) {
            (_, []) => Err(target.no_control()),
            (Some(i), all) => all.get(i).copied().ok_or_else(|| format!("index {i} is past the {} matching controls", all.len())),
            (None, [one]) => Ok(*one),
            (None, all) => Err(format!("more than one control matches ({}): add `role`, `within` or `index`", all.len())),
        }
    }

    fn describe(&self, id: NodeId, node: &'a Node) -> Value {
        let mut out = serde_json::Map::new();
        out.insert("name".into(), json!(self.name(node)));
        out.insert("role".into(), json!(format!("{:?}", node.role())));
        out.insert("rect".into(), json!(rect(node).map(|r| [r.left(), r.top(), r.width(), r.height()])));
        out.insert("enabled".into(), json!(!node.is_disabled()));
        out.insert("focused".into(), json!(self.update.focus == id));
        if let Some(value) = node.value() {
            out.insert("value".into(), json!(value));
        }
        if let Some(value) = node.numeric_value() {
            let numeric = json!({"value": value, "min": node.min_numeric_value(), "max": node.max_numeric_value(), "step": node.numeric_value_step()});
            out.insert("numeric".into(), numeric);
        }
        if let Some(expanded) = node.is_expanded() {
            out.insert("expanded".into(), json!(expanded));
        }
        if let Some(selected) = node.is_selected() {
            out.insert("selected".into(), json!(selected));
        }
        if let Some(toggled) = node.toggled() {
            let shown = match toggled {
                Toggled::False => json!(false),
                Toggled::True => json!(true),
                _ => json!("mixed"),
            };
            out.insert("toggled".into(), shown);
        }
        Value::Object(out)
    }
}

/// Finds the control `spec` names in the kept tree and reads it with `read`.
fn resolve<R>(ctx: &egui::Context, spec: &Value, read: impl FnOnce(&Tree, NodeId, &Node) -> Result<R, String>) -> Result<R, String> {
    let target = Target::from_params(spec)?;
    let update = ctx.data(|d| d.get_temp::<Arc<TreeUpdate>>(tree_slot())).ok_or(NO_TREE)?;
    let tree = Tree::new(&update).ok_or(NO_TREE)?;
    let (id, node) = tree.find(&target)?;
    read(&tree, id, node)
}

/// `ui.find`: the control `params` names, described.
pub(super) fn answer(ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    resolve(ctx, params, |tree, id, node| Ok(tree.describe(id, node)))
}

/// The centre of the control `spec` names, in screen points.
pub(super) fn centre(ctx: &egui::Context, spec: &Value) -> Result<egui::Pos2, String> {
    resolve(ctx, spec, |_, _, node| rect(node).map(|r| r.center()).ok_or_else(|| NO_PLACE.to_string()))
}
