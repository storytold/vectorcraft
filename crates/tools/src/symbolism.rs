//! Symbolism tools (Symbol Sprayer, Shifter, Scruncher, Sizer, Spinner, Stainer, Screener,
//! Styler) and the brush-aware Paintbrush.
//!
//! Symbolism gestures preview one command (`symbol.spray` / `symbol.adjust`) with every point of
//! the drag so far, re-applied on the interaction snapshot, and commit on release — one undo
//! step, replayable from the journal. The Paintbrush wraps the freehand gesture tool and turns its
//! `path.freehand` previews into `brush.freehand` when a brush is current (tool option `brush`,
//! else the document's current brush).

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Shape};

use crate::draw2::{FEEDBACK, GestureTool, points_json};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Create a symbolism tool or the brush-aware Paintbrush (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "paintbrush" => Box::new(BrushPaintTool::default()),
        "symbolSprayer" | "symbolShifter" | "symbolScruncher" | "symbolSizer" | "symbolSpinner" | "symbolStainer" | "symbolScreener"
        | "symbolStyler" => Box::new(SymbolismTool::new(id)),
        _ => return None,
    })
}

pub struct SymbolismTool {
    id: &'static str,
    points: Vec<Point>,
    active: bool,
    hover: Option<Point>,
    alt: bool,
    /// Brush diameter in points.
    pub diameter: f64,
    /// 1..10.
    pub intensity: f64,
    /// Symbol set density 1..10 (Sprayer).
    pub density: f64,
    /// Symbol to spray (None = the document's current symbol).
    pub symbol: Option<String>,
    /// Graphic style for the Styler (None = the document's first non-default style).
    pub style: Option<String>,
}

impl SymbolismTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "symbolShifter" => "symbolShifter",
            "symbolScruncher" => "symbolScruncher",
            "symbolSizer" => "symbolSizer",
            "symbolSpinner" => "symbolSpinner",
            "symbolStainer" => "symbolStainer",
            "symbolScreener" => "symbolScreener",
            "symbolStyler" => "symbolStyler",
            _ => "symbolSprayer",
        };
        Self { id, points: vec![], active: false, hover: None, alt: false, diameter: 80.0, intensity: 5.0, density: 5.0, symbol: None, style: None }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "symbolShifter" => "Symbol Shifter",
            "symbolScruncher" => "Symbol Scruncher",
            "symbolSizer" => "Symbol Sizer",
            "symbolSpinner" => "Symbol Spinner",
            "symbolStainer" => "Symbol Stainer",
            "symbolScreener" => "Symbol Screener",
            "symbolStyler" => "Symbol Styler",
            _ => "Symbol Sprayer",
        }
    }

    /// The command + params for the gesture so far.
    pub fn command(&self, cx: &ToolContext) -> (String, Value) {
        let radius = self.diameter / 2.0;
        let pts = points_json(&self.points);
        if self.id == "symbolSprayer" {
            let mut v = json!({"points": pts, "radius": radius, "density": self.density, "alt": self.alt});
            if let Some(s) = &self.symbol {
                v["name"] = json!(s);
            }
            return ("symbol.spray".into(), v);
        }
        let tool = match self.id {
            "symbolShifter" => "shift",
            "symbolScruncher" => "scrunch",
            "symbolSizer" => "size",
            "symbolSpinner" => "spin",
            "symbolStainer" => "stain",
            "symbolScreener" => "screen",
            _ => "style",
        };
        let mut v = json!({"tool": tool, "points": pts, "radius": radius, "intensity": self.intensity, "alt": self.alt});
        if tool == "stain"
            && let Some(c) = cx.paint.fill.color()
        {
            v["color"] = serde_json::to_value(c).unwrap_or(Value::Null);
            v["keepModel"] = json!(true);
        }
        if let Some(s) = &self.style {
            v["style"] = json!(s);
        }
        ("symbol.adjust".into(), v)
    }
}

impl Tool for SymbolismTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        self.hover = Some(p);
        match ev.kind {
            PointerKind::Down => {
                self.points = vec![p];
                self.active = true;
                self.alt = ev.mods.alt;
                let (c, v) = self.command(cx);
                vec![Action::Begin(self.label().into()), Action::Preview(c, v)]
            }
            PointerKind::Drag if self.active => {
                // Sprayer: thin the samples to a fraction of the brush; others: every few pixels.
                let step = if self.id == "symbolSprayer" { (self.diameter * 0.15).max(cx.tol(3.0)) } else { cx.tol(3.0) };
                if self.points.last().is_some_and(|l| l.distance(p) < step) {
                    return vec![];
                }
                self.points.push(p);
                let (c, v) = self.command(cx);
                vec![Action::Preview(c, v)]
            }
            PointerKind::Up if self.active => {
                self.active = false;
                self.points.clear();
                vec![Action::Commit]
            }
            _ => vec![],
        }
    }
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::BracketLeft | ToolKey::BracketRight)
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, m: Mods) -> Vec<Action> {
        match key {
            ToolKey::BracketLeft if m.shift => self.intensity = (self.intensity - 1.0).max(1.0),
            ToolKey::BracketRight if m.shift => self.intensity = (self.intensity + 1.0).min(10.0),
            ToolKey::BracketLeft => self.diameter = (self.diameter - 10.0).max(2.0),
            ToolKey::BracketRight => self.diameter = (self.diameter + 10.0).min(2000.0),
            ToolKey::Escape if self.active => {
                self.active = false;
                self.points.clear();
                return vec![Action::Cancel];
            }
            _ => {}
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if self.active {
            self.active = false;
            self.points.clear();
            return vec![Action::Commit];
        }
        vec![]
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        let Some(p) = self.hover else { return vec![] };
        let circle = kurbo::Circle::new(p, self.diameter / 2.0).to_path(0.1);
        vec![Overlay::Path { path: circle, color: FEEDBACK, width: 1.0, dashed: false }]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        match self.id {
            "symbolShifter" => Cursor::SymbolShifter,
            "symbolScruncher" => Cursor::SymbolScruncher,
            "symbolSizer" => Cursor::SymbolSizer,
            "symbolSpinner" => Cursor::SymbolSpinner,
            "symbolStainer" => Cursor::SymbolStainer,
            "symbolScreener" => Cursor::SymbolScreener,
            "symbolStyler" => Cursor::SymbolStyler,
            _ => Cursor::SymbolSprayer,
        }
    }
    fn options(&self) -> Value {
        json!({"diameter": self.diameter, "intensity": self.intensity, "density": self.density, "symbol": self.symbol, "style": self.style})
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "diameter" => self.diameter = v.as_f64().unwrap_or(self.diameter).clamp(2.0, 2000.0),
            "intensity" => self.intensity = v.as_f64().unwrap_or(self.intensity).clamp(1.0, 10.0),
            "density" => self.density = v.as_f64().unwrap_or(self.density).clamp(1.0, 10.0),
            "symbol" => self.symbol = v.as_str().map(str::to_string),
            "style" => self.style = v.as_str().map(str::to_string),
            _ => {}
        }
    }
}

/// Paintbrush: the freehand gesture, painting with the current brush.
pub struct BrushPaintTool {
    inner: GestureTool,
    /// Brush override (None = the document's current brush, if any).
    pub brush: Option<String>,
}

impl Default for BrushPaintTool {
    fn default() -> Self {
        Self { inner: GestureTool::new("paintbrush"), brush: None }
    }
}

impl BrushPaintTool {
    fn brush_name(&self, cx: &ToolContext) -> Option<String> {
        self.brush.clone().or_else(|| cx.doc.unknown.get("currentBrush").and_then(Value::as_str).map(str::to_string))
    }
    fn map(&self, cx: &ToolContext, acts: Vec<Action>) -> Vec<Action> {
        let Some(b) = self.brush_name(cx) else { return acts };
        acts.into_iter()
            .map(|a| match a {
                Action::Preview(c, mut v) if c == "path.freehand" => {
                    v["brush"] = json!(b);
                    Action::Preview("brush.freehand".into(), v)
                }
                o => o,
            })
            .collect()
    }
}

impl Tool for BrushPaintTool {
    fn id(&self) -> &'static str {
        "paintbrush"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let acts = self.inner.pointer(cx, ev);
        self.map(cx, acts)
    }
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        self.inner.claims_key(cx, key)
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, m: Mods) -> Vec<Action> {
        let acts = self.inner.key(cx, key, m);
        self.map(cx, acts)
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.inner.overlays(cx)
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        self.inner.cursor(cx, p, m)
    }
    fn options(&self) -> Value {
        let mut v = self.inner.options();
        v["brush"] = json!(self.brush);
        v
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        if key == "brush" {
            self.brush = v.as_str().map(str::to_string);
        } else {
            self.inner.set_option(key, v);
        }
    }
    fn busy(&self) -> bool {
        self.inner.busy()
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        let acts = self.inner.deactivate(cx);
        self.map(cx, acts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn sprayer_previews_all_points_and_commits() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolSprayer");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        assert_eq!(a[0], Action::Begin("Symbol Sprayer".into()));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 0.0));
        assert!(matches!(&a[0], Action::Preview(c, v) if c == "symbol.spray" && v["points"].as_array().unwrap().len() == 2));
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 51.0, 0.0)).is_empty(), "thinned");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 51.0, 0.0)), vec![Action::Commit]);
        assert!(!t.busy());
    }

    #[test]
    fn adjust_tools_map_to_symbol_adjust() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        for (id, tool) in [("symbolShifter", "shift"), ("symbolSizer", "size"), ("symbolStainer", "stain"), ("symbolStyler", "style")] {
            let mut t = create(id).unwrap();
            assert_eq!(t.id(), id);
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 5.0, 5.0).with_mods(Mods { alt: true, ..Default::default() }));
            assert!(matches!(&a[1], Action::Preview(c, v) if c == "symbol.adjust" && v["tool"] == tool && v["alt"] == true), "{a:?}");
        }
        // The Symbol Stainer sends the fill color in its own model.
        let fill = serde_json::to_value(p.fill.color().unwrap()).unwrap();
        let a = create("symbolStainer").unwrap().pointer(&cx, &PointerEvent::new(PointerKind::Down, 5.0, 5.0));
        assert!(matches!(&a[1], Action::Preview(_, v) if v["color"] == fill && v["keepModel"] == true), "{a:?}");
    }

    #[test]
    fn brackets_change_diameter_and_intensity() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolSizer");
        t.key(&cx, ToolKey::BracketRight, Mods::default());
        t.key(&cx, ToolKey::BracketLeft, Mods { shift: true, ..Default::default() });
        assert_eq!(t.options()["diameter"], json!(90.0));
        assert_eq!(t.options()["intensity"], json!(4.0));
    }

    #[test]
    fn paintbrush_rewrites_freehand_to_brush_freehand() {
        let (mut d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        {
            let cx = cx(&d, &s, &p);
            let mut t = BrushPaintTool::default();
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 30.0, 0.0));
            assert!(matches!(&a[1], Action::Preview(c, _) if c == "path.freehand"), "no brush: plain freehand");
        }
        d.unknown.insert("currentBrush".into(), json!("Charcoal"));
        let cx = cx(&d, &s, &p);
        let mut t = BrushPaintTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 30.0, 0.0));
        assert!(matches!(&a[1], Action::Preview(c, v) if c == "brush.freehand" && v["brush"] == "Charcoal"));
        t.set_option("brush", &json!("Dots"));
        assert_eq!(t.options()["brush"], "Dots");
    }
}
