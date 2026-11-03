//! A sandboxed WebAssembly tool: pointer events in, actions and guides out.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_plugins::{Kind, Plugin, registry, tool};

use crate::guides::GuideStyle;
use crate::{Action, Cursor, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

const MAX_ACTIONS: usize = 64;
const MAX_GUIDES: usize = 64;
const MAX_STATE_BYTES: usize = 1 << 20;

/// A tool whose pointer state and edits are computed by an installed sandboxed plug-in.
pub struct PluginTool {
    id: &'static str,
    plugin: Arc<Plugin>,
    state: Value,
    circles: Vec<Value>,
    guides: Vec<Overlay>,
    pressed: bool,
    last_error: Option<String>,
}

impl PluginTool {
    /// The installed tool plug-in `id` (`plugin.<manifest-id>`), if it is installed and is a tool.
    pub fn new(id: &str) -> Option<Self> {
        let plugin_id = id.strip_prefix("plugin.")?;
        let plugin = registry::get(plugin_id)?;
        (plugin.manifest().kind == Kind::Tool).then(|| Self {
            id: registry::intern(id),
            plugin,
            state: Value::Null,
            circles: Vec::new(),
            guides: Vec::new(),
            pressed: false,
            last_error: None,
        })
    }

    #[cfg(test)]
    fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn input(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Result<Vec<u8>, String> {
        let event = serde_json::to_value(ev).map_err(|e| e.to_string())?;
        let selected = Value::Array(cx.selection.objects.iter().map(|id| Value::from(id.0)).collect());
        if ev.kind == PointerKind::Down || self.circles.is_empty() {
            self.circles = tool::circles(cx.doc, 4096);
        }
        serde_json::to_vec(&json!({
            "event": event,
            "state": self.state,
            "selection": selected,
            "circles": self.circles,
            "zoom": cx.zoom,
            "tolerance": cx.snap_tol(),
        }))
        .map_err(|e| e.to_string())
    }

    fn run(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Result<Vec<Action>, String> {
        let input = self.input(cx, ev)?;
        let output = self.plugin.run_tool(&input).map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_slice(&output).map_err(|e| format!("plug-in output: {e}"))?;
        let obj = value.as_object().ok_or("plug-in tool output must be a JSON object")?;
        if let Some(error) = obj.get("error") {
            return Err(error.as_str().ok_or("plug-in `error` must be a string")?.to_string());
        }
        let state = obj.get("state").cloned().unwrap_or(Value::Null);
        if serde_json::to_vec(&state).map_err(|e| e.to_string())?.len() > MAX_STATE_BYTES {
            return Err(format!("plug-in state is larger than {MAX_STATE_BYTES} bytes"));
        }
        let actions = obj.get("actions").map_or(&[][..], |v| v.as_array().map_or(&[][..], Vec::as_slice));
        if actions.len() > MAX_ACTIONS {
            return Err(format!("plug-in returned more than {MAX_ACTIONS} actions"));
        }
        let mut out = Vec::with_capacity(actions.len());
        for action in actions {
            out.push(parse_action(action)?);
        }
        let raw_guides = obj.get("guides").map_or(&[][..], |v| v.as_array().map_or(&[][..], Vec::as_slice));
        if raw_guides.len() > MAX_GUIDES {
            return Err(format!("plug-in returned more than {MAX_GUIDES} guides"));
        }
        let style = GuideStyle::of(cx);
        let mut guides = Vec::with_capacity(raw_guides.len());
        for guide in raw_guides {
            if let Some(guide) = parse_guide(guide, style)? {
                guides.push(guide);
            }
        }
        self.state = state;
        self.guides = guides;
        self.last_error = None;
        if ev.kind == PointerKind::Up {
            self.circles.clear();
        }
        Ok(out)
    }

    fn cancel_gesture(&mut self) -> Vec<Action> {
        if !std::mem::take(&mut self.pressed) {
            return vec![];
        }
        self.state = Value::Null;
        self.circles.clear();
        self.guides.clear();
        self.last_error = None;
        vec![Action::Cancel]
    }
}

impl Tool for PluginTool {
    fn id(&self) -> &'static str {
        self.id
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind == PointerKind::Up {
            self.pressed = false;
        } else if ev.kind == PointerKind::Down {
            self.pressed = true;
        }
        match self.run(cx, ev) {
            Ok(actions) => actions,
            Err(e) => {
                self.last_error = Some(e);
                vec![]
            }
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        self.guides.clone()
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: crate::Mods) -> Vec<Action> {
        if key == ToolKey::Escape { self.cancel_gesture() } else { vec![] }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.cancel_gesture()
    }

    fn cursor(&self, _cx: &ToolContext, _p: vectorcraft_geom::Point, _m: crate::Mods) -> Cursor {
        Cursor::Move
    }

    fn busy(&self) -> bool {
        self.pressed
    }
}

fn parse_action(v: &Value) -> Result<Action, String> {
    let obj = v.as_object().ok_or("plug-in action must be an object")?;
    let kind = obj.get("type").and_then(Value::as_str).ok_or("plug-in action has no string `type`")?;
    let string = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("action `{kind}` has no string `{key}`"));
    Ok(match kind {
        "begin" => Action::Begin(string("label")?),
        "preview" => {
            let command = string("command")?;
            let params = obj.get("params").cloned().unwrap_or(Value::Null);
            if command == "object.transform" {
                let matrix = params.get("matrix").and_then(Value::as_array).ok_or("object.transform preview has no matrix")?;
                if matrix.len() != 6 || matrix.iter().any(|x| x.as_f64().is_none_or(|x| !x.is_finite())) {
                    return Err("object.transform matrix must hold six finite numbers".into());
                }
            }
            Action::Preview(command, params)
        }
        "exec" => Action::Exec(string("command")?, obj.get("params").cloned().unwrap_or(Value::Null)),
        "commit" => Action::Commit,
        "cancel" => Action::Cancel,
        "dialog" => Action::Dialog(string("dialog")?, obj.get("params").cloned().unwrap_or(Value::Null)),
        "switchTool" => Action::SwitchTool(string("tool")?),
        _ => return Err(format!("unknown action type `{kind}`")),
    })
}

fn point(v: Option<&Value>) -> Option<vectorcraft_geom::Point> {
    let [x, y] = v?.as_array()?.as_slice() else { return None };
    let (x, y) = (x.as_f64()?, y.as_f64()?);
    (x.is_finite() && y.is_finite()).then(|| vectorcraft_geom::Point::new(x, y))
}

fn parse_guide(v: &Value, style: GuideStyle) -> Result<Option<Overlay>, String> {
    let obj = v.as_object().ok_or("plug-in guide must be an object")?;
    match obj.get("type").and_then(Value::as_str) {
        Some("line") if style.lines => {
            let a = point(obj.get("a")).ok_or("line guide needs finite `a`")?;
            let b = point(obj.get("b")).ok_or("line guide needs finite `b`")?;
            Ok(Some(Overlay::Line { a, b, color: style.color, dashed: false }))
        }
        Some("label") if style.labels => {
            let p = point(obj.get("p")).ok_or("label guide needs finite `p`")?;
            let text = obj.get("text").and_then(Value::as_str).ok_or("label guide needs string `text`")?;
            if text.chars().count() > 64 || text.chars().any(char::is_control) {
                return Err("guide label must be at most 64 printable characters".into());
            }
            Ok(Some(Overlay::Label { p, text: text.to_string(), color: style.color }))
        }
        Some("line" | "label") => Ok(None),
        Some(kind) => Err(format!("unknown guide type `{kind}`")),
        None => Err("plug-in guide has no string `type`".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{Action, PointerEvent, PointerKind, Tool, ToolKey};
    use vectorcraft_doc::Selection;
    use vectorcraft_plugins::{Limits, Plugin, registry, wat as tpl};

    fn install(id: &str, output: &str) -> String {
        let manifest = format!(r#"{{"id":"{id}","name":"Tangent Test","version":"1","kind":"tool","toolGroup":"ellipse"}}"#);
        let src = tpl::returning(&manifest, output);
        let bytes = wat::parse_str(src).unwrap();
        registry::install(Plugin::load(&bytes, Limits::default()).unwrap());
        format!("plugin.{id}")
    }

    fn doc_with_circle(center: vectorcraft_geom::Point, radius: f64) -> (vectorcraft_doc::Document, vectorcraft_doc::NodeId) {
        use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
        use vectorcraft_geom::{Affine, Rect, shapes};
        let mut d = vectorcraft_doc::Document::new(400.0, 300.0);
        let id = d.alloc_id();
        let live =
            LiveShape::Ellipse { w: radius * 2.0, h: radius * 2.0, pie: (0.0, 360.0), xf: Affine::translate((center.x - radius, center.y - radius)) };
        let mut n = Node::path(id, shapes::ellipse(Rect::from_center_size(center, (radius * 2.0, radius * 2.0))), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(live));
        }
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        (d, id)
    }

    #[test]
    fn circle_context_is_gathered_once_per_gesture() {
        let id = install("test.tool.cache", "{}");
        let (mut d, circle) = doc_with_circle(vectorcraft_geom::Point::new(100.0, 80.0), 20.0);
        let (s, p) = (Selection::default(), paint());
        let mut t = PluginTool::new(&id).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&t.input(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 100.0, 80.0)).unwrap()).unwrap();
        assert_eq!(first["circles"][0]["center"], serde_json::json!([100.0, 80.0]));
        if let vectorcraft_doc::NodeKind::Path { live: Some(live), .. } = &mut d.node_mut(circle).unwrap().kind
            && let vectorcraft_doc::LiveShape::Ellipse { xf, .. } = live.as_mut()
        {
            *xf = vectorcraft_geom::Affine::translate((100.0, 0.0)) * *xf;
        }
        let drag: serde_json::Value =
            serde_json::from_slice(&t.input(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Drag, 110.0, 80.0)).unwrap()).unwrap();
        assert_eq!(drag["circles"][0]["center"], serde_json::json!([100.0, 80.0]), "references stay gathered from the press");
        registry::remove("test.tool.cache");
    }

    #[test]
    fn a_tool_plugin_can_select_on_press() {
        let id = install("test.tool.select", r#"{"state":{"pressed":true},"actions":[{"type":"exec","command":"select.set","params":{"ids":[3]}}]}"#);
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = PluginTool::new(&id).unwrap();
        let got = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 20.0));
        assert_eq!(got, vec![Action::Exec("select.set".into(), serde_json::json!({"ids": [3]}))]);
        registry::remove("test.tool.select");
    }

    #[test]
    fn a_tool_plugin_can_preview_a_transform() {
        let id = install(
            "test.tool.preview",
            r#"{"state":{"dragged":true},"actions":[{"type":"begin","label":"Move"},{"type":"preview","command":"object.transform","params":{"matrix":[1,0,0,1,12,8]}}],"guides":[{"type":"line","a":[0,0],"b":[10,10]},{"type":"label","p":[10,10],"text":"tangent"}]}"#,
        );
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = PluginTool::new(&id).unwrap();
        let got = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 30.0, 40.0));
        assert!(
            matches!(&got[..], [Action::Begin(name), Action::Preview(cmd, v)] if name == "Move" && cmd == "object.transform" && v["matrix"] == serde_json::json!([1,0,0,1,12,8])),
            "{got:?}"
        );
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Line { .. }, Overlay::Label { text, .. }] if text == "tangent"));
        registry::remove("test.tool.preview");
    }

    #[test]
    fn malformed_actions_are_refused() {
        for (id, output) in [
            ("test.tool.unknown", r#"{"actions":[{"type":"runAnything","command":"document.delete"}]}"#),
            ("test.tool.nan", r#"{"actions":[{"type":"preview","command":"object.transform","params":{"matrix":[1,0,0,1,0,null]}}]}"#),
        ] {
            let id = install(id, output);
            let (d, _) = doc_with_rect();
            let (s, p) = (Selection::default(), paint());
            let cx = cx(&d, &s, &p);
            let mut t = PluginTool::new(&id).unwrap();
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 10.0, 10.0)).is_empty());
            assert!(t.last_error().is_some(), "{id}");
            registry::remove(id.trim_start_matches("plugin."));
        }
    }

    #[test]
    fn escape_and_switching_away_cancel_an_active_tool_gesture() {
        let id = install(
            "test.tool.cancel",
            r#"{"state":{"pressed":true},"actions":[{"type":"begin","label":"Move"},{"type":"preview","command":"object.transform","params":{"matrix":[1,0,0,1,1,0]}}]}"#,
        );
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = PluginTool::new(&id).unwrap();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 20.0, 10.0));
        assert!(t.busy());
        assert_eq!(t.key(&cx, ToolKey::Escape, Default::default()), vec![Action::Cancel]);
        assert!(!t.busy());
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 20.0, 10.0));
        assert_eq!(t.deactivate(&cx), vec![Action::Cancel]);
        assert!(!t.busy());
        registry::remove("test.tool.cancel");
    }
}
