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
use crate::{Action, Cursor, MAX_STROKE_SAMPLES, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, previews_at};

/// Create a symbolism tool or the brush-aware Paintbrush (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "paintbrush" => Box::new(BrushPaintTool::default()),
        "symbolSprayer" | "symbolShifter" | "symbolScruncher" | "symbolSizer" | "symbolSpinner" | "symbolStainer" | "symbolScreener"
        | "symbolStyler" => Box::new(SymbolismTool::new(id)),
        _ => return None,
    })
}

/// The values of Method in the Symbolism Tools Options.
pub const METHODS: [&str; 3] = ["userDefined", "average", "random"];

/// The values of each Symbol Sprayer option in the Symbolism Tools Options.
pub const SPRAY_MODES: [&str; 2] = ["average", "userDefined"];

/// The Symbol Sprayer options: (tool option, key in `symbol.spray`'s `attributes`).
pub const SPRAY_ATTRIBUTES: [(&str, &str); 6] = [
    ("sprayScrunch", "scrunch"),
    ("spraySize", "size"),
    ("spraySpin", "spin"),
    ("sprayScreen", "screen"),
    ("sprayStain", "stain"),
    ("sprayStyle", "style"),
];

pub struct SymbolismTool {
    id: &'static str,
    /// The drag's points, each with its pen pressure (0..1).
    points: Vec<(Point, f64)>,
    /// How many points the last preview had ([`previews_at`] thins previews of a long drag).
    previewed: usize,
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
    /// Use Pressure Pen: each point's pen pressure is the intensity there.
    pub use_pressure: bool,
    /// Method of the Scruncher, Sizer, Spinner, Stainer, Screener and Styler: one of [`METHODS`].
    pub method: &'static str,
    /// Show Brush Size and Intensity: the brush outline follows the pointer.
    pub show_brush: bool,
    /// The Symbol Sprayer options, in the order of [`SPRAY_ATTRIBUTES`]: each one of [`SPRAY_MODES`].
    pub spray: [&'static str; 6],
    /// Symbol Sizer: Proportional Resizing.
    pub proportional: bool,
    /// Symbol Sizer: Resizing Affects Density.
    pub affects_density: bool,
}

/// Checks the Symbolism Tools Options in `values` before `tool.setOption` sets any, when `tool` is
/// a symbolism tool, with the tool's own checks ([`Tool::check_option`]): a number out of its
/// range, `method` that is not one of [`METHODS`], a Symbol Sprayer option that is not one of
/// [`SPRAY_MODES`], and `usePressure`, `showBrush`, `proportional` or `affectsDensity` that is not
/// true or false are errors. Ok for every other tool, and for a key the tool does not report
/// (`tool.setOption` refuses that one as an unknown option). Saved settings go to the tool's
/// `set_option` instead, which skips a value it cannot read.
pub fn check_options(tool: &str, values: &serde_json::Map<String, Value>) -> Result<(), String> {
    if !tool.starts_with("symbol") || create(tool).is_none() {
        return Ok(());
    }
    let tool = SymbolismTool::new(tool);
    let known = tool.options();
    values.iter().filter(|(key, _)| known.get(key.as_str()).is_some()).try_for_each(|(key, v)| tool.check_option(key, v))
}

/// `v` as one of `allowed`, when it is one.
fn one_of(v: &Value, allowed: &[&'static str]) -> Option<&'static str> {
    let s = v.as_str()?;
    allowed.iter().copied().find(|a| *a == s)
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
        Self {
            id,
            points: vec![],
            previewed: 0,
            active: false,
            hover: None,
            alt: false,
            diameter: 80.0,
            intensity: 5.0,
            density: 5.0,
            symbol: None,
            style: None,
            use_pressure: false,
            method: "userDefined",
            show_brush: true,
            spray: ["average"; 6],
            proportional: true,
            affects_density: false,
        }
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
        if self.id == "symbolSprayer" {
            let pts = points_json(&self.points.iter().map(|(p, _)| *p).collect::<Vec<_>>());
            let attributes: serde_json::Map<String, Value> =
                SPRAY_ATTRIBUTES.iter().zip(self.spray).map(|((_, key), mode)| (key.to_string(), json!(mode))).collect();
            let mut v = json!({"points": pts, "radius": radius, "density": self.density, "alt": self.alt, "attributes": attributes});
            if let Some(s) = &self.symbol {
                v["name"] = json!(s);
            }
            // Stain: User Defined stains the new instances with the fill color in its own model.
            if self.spray[4] == "userDefined"
                && let Some(c) = cx.paint.fill.color()
            {
                v["color"] = serde_json::to_value(c).unwrap_or(Value::Null);
                v["keepModel"] = json!(true);
            }
            // A style the document no longer has (a deleted one) is left out: the default style applies.
            if let Some(s) = self.style.as_ref().filter(|s| cx.doc.graphic_styles.iter().any(|g| g.name == **s)) {
                v["style"] = json!(s);
            }
            return ("symbol.spray".into(), v);
        }
        let pressure = self.use_pressure;
        let pts = Value::Array(self.points.iter().map(|(p, f)| if pressure { json!([p.x, p.y, f]) } else { json!([p.x, p.y]) }).collect());
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
        if pressure {
            v["usePressure"] = json!(true);
        }
        // The Symbol Shifter has no Method.
        if tool != "shift" {
            v["method"] = json!(self.method);
        }
        if tool == "size" {
            v["proportional"] = json!(self.proportional);
            v["affectsDensity"] = json!(self.affects_density);
        }
        ("symbol.adjust".into(), v)
    }
}

impl SymbolismTool {
    /// Ends the drag: what is committed is the last preview, so the whole drag is shown first.
    fn finish(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.active = false;
        let stale = (self.previewed != self.points.len()).then(|| {
            let (c, v) = self.command(cx);
            Action::Preview(c, v)
        });
        self.points.clear();
        stale.into_iter().chain([Action::Commit]).collect()
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
        let sample = (p, f64::from(ev.pressure).clamp(0.0, 1.0));
        match ev.kind {
            PointerKind::Down => {
                self.points = vec![sample];
                self.previewed = 1;
                self.active = true;
                self.alt = ev.mods.alt;
                let (c, v) = self.command(cx);
                vec![Action::Begin(self.label().into()), Action::Preview(c, v)]
            }
            PointerKind::Drag if self.active => {
                // Sprayer: thin the samples to a fraction of the brush; others: every few pixels.
                let step = if self.id == "symbolSprayer" { (self.diameter * 0.15).max(cx.tol(3.0)) } else { cx.tol(3.0) };
                // With Use Pressure Pen a change of pressure of at least 0.05 is kept where the pen
                // stays (the Sprayer does not use the pressure).
                let pressed = self.use_pressure && self.id != "symbolSprayer";
                let thinned = self.points.last().is_some_and(|(l, f)| l.distance(p) < step && !(pressed && (sample.1 - f).abs() >= 0.05));
                // Each preview carries every point so far: a long drag stops growing and previews less often.
                if self.points.len() >= MAX_STROKE_SAMPLES || thinned {
                    return vec![];
                }
                self.points.push(sample);
                if !previews_at(self.points.len()) {
                    return vec![];
                }
                self.previewed = self.points.len();
                let (c, v) = self.command(cx);
                vec![Action::Preview(c, v)]
            }
            PointerKind::Up if self.active => self.finish(cx),
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
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        if self.active {
            return self.finish(cx);
        }
        vec![]
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        let Some(p) = self.hover.filter(|_| self.show_brush) else { return vec![] };
        let ring = |radius: f64, dashed| Overlay::Path { path: kurbo::Circle::new(p, radius).to_path(0.1), color: FEEDBACK, width: 1.0, dashed };
        let mut v = vec![ring(self.diameter / 2.0, false)];
        // The intensity: a dashed ring that fills the brush at 10. A pen's pressure sets it instead
        // with Use Pressure Pen, so no ring is drawn then. The Symbol Sprayer uses neither, so it
        // draws no ring.
        if self.id != "symbolSprayer" && !self.use_pressure && self.intensity < 10.0 {
            v.push(ring(self.diameter / 2.0 * self.intensity / 10.0, true));
        }
        v
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
        let mut v = json!({
            "diameter": self.diameter,
            "intensity": self.intensity,
            "density": self.density,
            "symbol": self.symbol,
            "style": self.style,
            "usePressure": self.use_pressure,
            "method": self.method,
            "showBrush": self.show_brush,
            "proportional": self.proportional,
            "affectsDensity": self.affects_density,
        });
        for ((key, _), mode) in SPRAY_ATTRIBUTES.iter().zip(self.spray) {
            v[*key] = json!(mode);
        }
        v
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "diameter" => self.diameter = v.as_f64().unwrap_or(self.diameter).clamp(2.0, 2000.0),
            "intensity" => self.intensity = v.as_f64().unwrap_or(self.intensity).clamp(1.0, 10.0),
            "density" => self.density = v.as_f64().unwrap_or(self.density).clamp(1.0, 10.0),
            "symbol" => self.symbol = v.as_str().map(str::to_string),
            "style" => self.style = v.as_str().map(str::to_string),
            "usePressure" => self.use_pressure = v.as_bool().unwrap_or(self.use_pressure),
            "method" => self.method = one_of(v, &METHODS).unwrap_or(self.method),
            "showBrush" => self.show_brush = v.as_bool().unwrap_or(self.show_brush),
            "proportional" => self.proportional = v.as_bool().unwrap_or(self.proportional),
            "affectsDensity" => self.affects_density = v.as_bool().unwrap_or(self.affects_density),
            _ => {
                if let Some(i) = SPRAY_ATTRIBUTES.iter().position(|(k, _)| *k == key) {
                    self.spray[i] = one_of(v, &SPRAY_MODES).unwrap_or(self.spray[i]);
                }
            }
        }
    }
    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        let must_be = |names: &[&str]| format!("`{key}` must be one of {}", names.join(", "));
        match key {
            "diameter" => c::number(key, v, 2.0, 2000.0),
            "intensity" | "density" => c::number(key, v, 1.0, 10.0),
            "symbol" | "style" => c::string_or_null(key, v),
            "usePressure" | "showBrush" | "proportional" | "affectsDensity" => c::boolean(key, v),
            "method" => one_of(v, &METHODS).map(drop).ok_or_else(|| must_be(&METHODS)),
            k if SPRAY_ATTRIBUTES.iter().any(|(a, _)| *a == k) => one_of(v, &SPRAY_MODES).map(drop).ok_or_else(|| must_be(&SPRAY_MODES)),
            _ => Err(c::unknown(key)),
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
    /// The tool's brush, else the document's current brush, skipping a name the document's brush
    /// library no longer has (a deleted brush), so the stroke is drawn without it.
    fn brush_name(&self, cx: &ToolContext) -> Option<String> {
        [self.brush.clone(), vectorcraft_brush::current(cx.doc)].into_iter().flatten().find(|n| vectorcraft_brush::find(cx.doc, n).is_some())
    }
    fn map(&self, cx: &ToolContext, acts: Vec<Action>) -> Vec<Action> {
        if !acts.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "path.freehand")) {
            return acts;
        }
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
    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        if key == "brush" { crate::option_check::string_or_null(key, v) } else { self.inner.check_option(key, v) }
    }
    fn wants_samples(&self) -> bool {
        self.inner.wants_samples()
    }
    fn busy(&self) -> bool {
        self.inner.busy()
    }
    fn readout(&self, cx: &ToolContext) -> Option<crate::Readout> {
        Some(crate::Readout::Brush { name: self.brush_name(cx) })
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
    fn a_long_drag_previews_less_often_and_commits_all_of_it() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolShifter");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        let n = 7 * crate::PREVIEW_ALL_UNTIL + 1; // not a point that previews
        let previews = (1..n).filter(|&i| !t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0 * i as f64, 0.0)).is_empty()).count();
        assert!(previews < n * 3 / 4, "{previews} previews of {n} points");
        let up = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.0, 0.0));
        let last = up.iter().rev().nth(1);
        assert!(matches!(last, Some(Action::Preview(_, v)) if v["points"].as_array().unwrap().len() == n), "{up:?}");
        assert_eq!(up.last(), Some(&Action::Commit));
    }

    #[test]
    fn switching_tools_mid_drag_previews_the_whole_drag_before_committing() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolShifter");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
        let n = 7 * crate::PREVIEW_ALL_UNTIL + 1; // not a point that previews
        for i in 1..n {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0 * i as f64, 0.0));
        }
        let a = t.deactivate(&cx);
        assert!(matches!(a.first(), Some(Action::Preview(_, v)) if v["points"].as_array().unwrap().len() == n), "{a:?}");
        assert_eq!(a.last(), Some(&Action::Commit));
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

    /// The Symbolism Tools Options the tools keep, with what each is before it is set.
    #[test]
    fn the_options_dialog_fields_are_tool_options() {
        let mut t = SymbolismTool::new("symbolSizer");
        let o = t.options();
        assert_eq!((o["usePressure"].clone(), o["method"].clone(), o["showBrush"].clone()), (json!(false), json!("userDefined"), json!(true)));
        assert_eq!((o["proportional"].clone(), o["affectsDensity"].clone()), (json!(true), json!(false)));
        for (key, _) in SPRAY_ATTRIBUTES {
            assert_eq!(o[key], "average", "{key}");
        }
        for (key, v) in [
            ("usePressure", json!(true)),
            ("method", json!("random")),
            ("showBrush", json!(false)),
            ("proportional", json!(false)),
            ("affectsDensity", json!(true)),
            ("sprayStain", json!("userDefined")),
        ] {
            t.set_option(key, &v);
            assert_eq!(t.options()[key], v, "{key}");
        }
        // A value that is none of an option's choices leaves it as it was.
        for (key, v) in [("method", json!("smooth")), ("method", json!(2)), ("sprayStain", json!("random")), ("usePressure", json!("yes"))] {
            let before = t.options();
            t.set_option(key, &v);
            assert_eq!(t.options(), before, "{key}: {v}");
        }
    }

    /// The adjusting tools send the Method (the Shifter has none), the Sizer its two options, and
    /// with Use Pressure Pen each point's pen pressure.
    #[test]
    fn adjust_tools_send_the_method_the_sizer_options_and_the_pen_pressure() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let press = |t: &mut SymbolismTool| {
            let down = PointerEvent { pressure: 0.25, ..PointerEvent::new(PointerKind::Down, 5.0, 5.0) };
            match t.pointer(&cx, &down).swap_remove(1) {
                Action::Preview(_, v) => v,
                other => panic!("{other:?}"),
            }
        };
        let mut sizer = SymbolismTool::new("symbolSizer");
        let v = press(&mut sizer);
        assert_eq!((v["method"].clone(), v["proportional"].clone(), v["affectsDensity"].clone()), (json!("userDefined"), json!(true), json!(false)));
        assert_eq!((v["points"].clone(), v.get("usePressure")), (json!([[5.0, 5.0]]), None));
        sizer.set_option("usePressure", &json!(true));
        sizer.set_option("method", &json!("average"));
        sizer.set_option("affectsDensity", &json!(true));
        let v = press(&mut sizer);
        assert_eq!((v["points"].clone(), v["usePressure"].clone()), (json!([[5.0, 5.0, 0.25]]), json!(true)));
        assert_eq!((v["method"].clone(), v["affectsDensity"].clone()), (json!("average"), json!(true)));
        let mut shifter = SymbolismTool::new("symbolShifter");
        shifter.set_option("method", &json!("random"));
        let v = press(&mut shifter);
        assert!(v.get("method").is_none() && v.get("proportional").is_none(), "{v}");
        let mut spinner = SymbolismTool::new("symbolSpinner");
        spinner.set_option("method", &json!("random"));
        let v = press(&mut spinner);
        assert!(v["method"] == "random" && v.get("proportional").is_none(), "{v}");
    }

    /// With Use Pressure Pen a drag sample where the pen stays is kept when the pressure changes;
    /// without it, or with the same pressure, the drag keeps thinning the samples by distance.
    #[test]
    fn a_pressure_change_where_the_pen_stays_is_kept() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let stroke = |use_pressure: bool, pressures: [f32; 3]| {
            let mut t = SymbolismTool::new("symbolSizer");
            t.set_option("usePressure", &json!(use_pressure));
            let at = |kind, pressure| PointerEvent { pressure, ..PointerEvent::new(kind, 5.0, 5.0) };
            let events =
                [(PointerKind::Down, pressures[0]), (PointerKind::Drag, pressures[1]), (PointerKind::Drag, pressures[2]), (PointerKind::Up, 0.0)];
            // The points of the last preview, the one the drag commits.
            let mut last = Value::Null;
            for (kind, pressure) in events {
                for a in t.pointer(&cx, &at(kind, pressure)) {
                    if let Action::Preview(_, v) = a {
                        last = v["points"].clone();
                    }
                }
            }
            last
        };
        assert_eq!(stroke(true, [0.0, 1.0, 1.0]), json!([[5.0, 5.0, 0.0], [5.0, 5.0, 1.0]]));
        assert_eq!(stroke(true, [0.5, 0.52, 0.5]), json!([[5.0, 5.0, 0.5]]));
        assert_eq!(stroke(false, [0.0, 1.0, 1.0]), json!([[5.0, 5.0]]));
    }

    /// The Sprayer sends its six options as `attributes`, and the fill color only while Stain is
    /// User Defined.
    #[test]
    fn the_sprayer_sends_its_attributes() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolSprayer");
        t.set_option("usePressure", &json!(true));
        let spray = |t: &mut SymbolismTool| match t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 5.0, 5.0)).swap_remove(1) {
            Action::Preview(c, v) if c == "symbol.spray" => v,
            other => panic!("{other:?}"),
        };
        let v = spray(&mut t);
        let average =
            json!({"scrunch": "average", "size": "average", "spin": "average", "screen": "average", "stain": "average", "style": "average"});
        assert_eq!((v["attributes"].clone(), v.get("color"), v["points"].clone()), (average, None, json!([[5.0, 5.0]])));
        t.set_option("sprayStain", &json!("userDefined"));
        t.set_option("spraySpin", &json!("userDefined"));
        let v = spray(&mut t);
        assert_eq!(
            (v["attributes"]["stain"].clone(), v["attributes"]["spin"].clone(), v["attributes"]["size"].clone()),
            (json!("userDefined"), json!("userDefined"), json!("average"))
        );
        assert_eq!((v["color"].clone(), v["keepModel"].clone()), (serde_json::to_value(p.fill.color().unwrap()).unwrap(), json!(true)));
    }

    /// Show Brush Size and Intensity: the brush's outline with a dashed ring for the intensity
    /// inside it; no ring with Use Pressure Pen, at intensity 10 or for the Symbol Sprayer, and
    /// nothing with it off.
    #[test]
    fn show_brush_size_and_intensity_draws_the_brush() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = SymbolismTool::new("symbolScruncher");
        assert!(t.overlays(&cx).is_empty(), "no pointer yet");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 50.0, 50.0));
        let rings = |t: &SymbolismTool| {
            t.overlays(&cx)
                .into_iter()
                .map(|o| match o {
                    Overlay::Path { path, dashed, .. } => (path.bounding_box().width().round(), dashed),
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rings(&t), [(80.0, false), (40.0, true)]);
        t.set_option("intensity", &json!(10));
        assert_eq!(rings(&t), [(80.0, false)]);
        t.set_option("intensity", &json!(2));
        t.set_option("usePressure", &json!(true));
        assert_eq!(rings(&t), [(80.0, false)]);
        t.set_option("showBrush", &json!(false));
        assert!(t.overlays(&cx).is_empty());
        // The Symbol Sprayer uses neither Intensity nor pen pressure: the outline only.
        let mut t = SymbolismTool::new("symbolSprayer");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 50.0, 50.0));
        assert_eq!(rings(&t), [(80.0, false)]);
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

    #[test]
    fn paintbrush_skips_a_brush_the_document_does_not_have() {
        let (mut d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let drag = |d: &vectorcraft_doc::Document, t: &mut BrushPaintTool| {
            let cx = cx(d, &s, &p);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 30.0, 0.0)).swap_remove(1)
        };
        d.unknown.insert("currentBrush".into(), json!("Charcoal"));
        let mut t = BrushPaintTool::default();
        t.set_option("brush", &json!("Deleted"));
        let a = drag(&d, &mut t);
        assert!(matches!(&a, Action::Preview(c, v) if c == "brush.freehand" && v["brush"] == "Charcoal"), "the current brush: {a:?}");
        d.unknown.insert("currentBrush".into(), json!("Also deleted"));
        let a = drag(&d, &mut BrushPaintTool::default());
        assert!(matches!(&a, Action::Preview(c, v) if c == "path.freehand" && v.get("brush").is_none()), "no brush: {a:?}");
    }
}
