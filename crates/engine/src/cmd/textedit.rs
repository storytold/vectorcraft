//! Rich text editing commands: range edits and range styling (what the Type tool and the
//! Character panel use while editing), creating area / on-path type from a path, and Fit Headline.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, CharStyle, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::Affine;
use vectorcraft_text::edit;

use super::stroke::StrokeChange;
use super::typecmd::refresh_bounds;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "text.editRange",
            "Edit Text",
            [],
            None,
            "{id, start: byte, end: byte, insert?: string, runs?: [{text, style}]} replace bytes start..end of the plain text (inserted text takes the replaced text's style unless styled `runs` are given) → {id, caret}",
            has_doc,
            edit_range
        ),
        cmd!(
            "text.setRangeStyle",
            "Character",
            [],
            None,
            "{id? (default: the one selected type object), start?: byte, end?: byte (default: all text), font?, style?, size?: pt, leading?: pt|\"auto\", tracking?, kerning?: 1/1000 em|\"auto\", baselineShift?: pt, hScale?: %, vScale?: %, rotation?: deg, fill?: colour|\"none\", stroke?: colour|\"none\", strokeWidth?: pt, strokeOptions?: {weight?, cap?, join?, miterLimit?, dash?, dashOffset?, alignDashes?} (as stroke.set: the character stroke), underline?, strikethrough?, allCaps?: bool, smallCaps?: bool, position?: \"normal\"|\"superscript\"|\"subscript\" (sizes from Document Setup), features?: [\"dlig\", \"-liga\", …], charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\", proportionalMetrics?: bool (full-width glyphs on the font's proportional widths: `palt` in horizontal type, `vpal` in vertical type)} style a character range (size/leading/baselineShift are document points and hScale includes the object transform; runs are split at the range ends) → {id, runs}",
            has_doc,
            set_range_style
        ),
        cmd!(
            query "text.getRange",
            "Get Text Range",
            [],
            None,
            "{id, start?, end?, effective?: bool = false} → {text, runs: [{text, style}], length}; effective uses document-point size/leading/baseline shift and transformed horizontal scale; default returns stored local styles for rich-text copying",
            has_doc,
            get_range
        ),
        cmd!(
            "text.createInPath",
            "Area / Path Type",
            [],
            None,
            "{path: id, mode: \"area\"|\"onPath\", text?: \"\", vertical?: bool = false, at?: [x, y] (on-path start: nearest point), size?, font?, style?, color? (as text.create), fit?: none|autoHeight|shrinkText, fitMinPercent? (area: as text.areaOptions; default autoHeight when the autoSizeAreaType preference is on), placeholder?: bool (placeholder text instead, as text.create), leadingModel?, charAlign? (as text.create)} turn a path into an area-type frame or a type-on-a-path baseline (the path's paint is dropped) → {id}",
            has_doc,
            create_in_path
        ),
        cmd!(
            "type.fitHeadline",
            "Fit Headline",
            ["Type"],
            None,
            "{ids?} track the first line of area type so it fills the frame width → {ids, tracking}",
            has_selection,
            fit_headline
        ),
        cmd!(
            "type.step",
            "Step Type",
            [],
            None,
            "{attribute: \"size\"|\"leading\"|\"tracking\"|\"kerning\"|\"baselineShift\", by?: steps (1; negative steps down), id?, start?: byte, end?: byte} step type by the Preferences › Type increments (size and leading: typeSizeIncrement, pt; tracking and kerning: trackingIncrement, 1/1000 em; baseline shift: baselineShiftIncrement, pt), what the Type tool's Alt+arrows do (Cmd/Ctrl too: five steps; ←/→ kerning at the caret or tracking of the selection, ↑/↓ leading, Shift+↑/↓ baseline shift). The range of `id`, else the text the Type tool has selected (at a caret: kerning steps the character before it, leading its paragraph), else every selected type object → {ids}",
            has_doc,
            type_step
        ),
        cmd!(
            "type.size.increase",
            "Increase Font Size",
            [],
            Some("Cmd+Shift+."),
            "{by?: steps (1)} type.step {attribute: size}",
            has_selection,
            |s, p| type_step(s, &json!({"attribute": "size", "by": f64_or(p, "by", 1.0)}))
        ),
        cmd!(
            "type.size.decrease",
            "Decrease Font Size",
            [],
            Some("Cmd+Shift+,"),
            "{by?: steps (1)} type.step {attribute: size, by: -by}",
            has_selection,
            |s, p| type_step(s, &json!({"attribute": "size", "by": -f64_or(p, "by", 1.0)}))
        ),
        cmd!(
            "text.discardEmpty",
            "Discard Empty Type",
            [],
            None,
            "{id} remove text object `id` if it has no characters (what the Type tool does with point type it placed when editing ends). When nothing but that text changed since the step that created it, the steps since are dropped instead, leaving no trace in the history → {removed}",
            has_doc,
            discard_empty
        ),
    ]
}

fn text_mut(d: &mut vectorcraft_doc::Document, id: NodeId) -> Option<&mut TextObject> {
    match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

fn text_ref(s: &Session, id: NodeId) -> Result<TextObject> {
    match s.doc()?.doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Ok((**t).clone()),
        Some(_) => Err(EngineError::Other(format!("node {} is not text", id.0))),
        None => Err(EngineError::NoNode(id)),
    }
}

/// Replace a type object's whole content: the new text takes the replaced text's
/// style and paragraph styles follow, exactly as `text.editRange` over the full
/// range does. Dataset apply and friends share this so Variables edits behave
/// like Type tool edits.
pub(crate) fn set_plain_text(d: &mut vectorcraft_doc::Document, id: NodeId, text: &str) -> Result<()> {
    let len = match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => edit::runs_len(&t.runs),
        Some(_) => return Err(EngineError::Other(format!("node {} is not text", id.0))),
        None => return Err(EngineError::NoNode(id)),
    };
    let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
    t.splice_paras(0, len, text);
    edit::replace_range(&mut t.runs, 0, len, text);
    refresh_bounds(t);
    Ok(())
}

fn byte_param(p: &Value, k: &str) -> Option<usize> {
    p.get(k).and_then(Value::as_u64).map(|v| v as usize)
}

fn range_of(p: &Value, len: usize) -> (usize, usize) {
    let a = byte_param(p, "start").unwrap_or(0).min(len);
    let b = byte_param(p, "end").unwrap_or(len).min(len);
    (a.min(b), a.max(b))
}

fn edit_range(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.editRange";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let t = text_ref(s, id)?;
    let len = edit::runs_len(&t.runs);
    let (a, b) = range_of(p, len);
    let styled: Option<Vec<TextRun>> = match p.get("runs") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| bad(C, format!("bad `runs`: {e}")))?),
    };
    let insert = str_param(p, "insert").unwrap_or("").to_string();
    let caret = s.edit("Typing", |d, _| {
        let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
        // Paragraph styles follow the edit: a split paragraph's new one continues its style, a
        // merge keeps the first paragraph's.
        let inserted: String = match &styled {
            Some(r) => r.iter().map(|r| r.text.as_str()).collect(),
            None => insert.clone(),
        };
        t.splice_paras(a, b, &inserted);
        let caret = match &styled {
            Some(r) => edit::replace_range_styled(&mut t.runs, a, b, r),
            None => edit::replace_range(&mut t.runs, a, b, &insert),
        };
        refresh_bounds(t);
        Ok(caret)
    })?;
    Ok(json!({"id": id.0, "caret": caret}))
}

fn get_range(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("text.getRange", "missing `id`"))?;
    let t = text_ref(s, id)?;
    let len = edit::runs_len(&t.runs);
    let (a, b) = range_of(p, len);
    let mut runs = edit::slice_runs(&t.runs, a, b);
    if p.get("effective").and_then(Value::as_bool).unwrap_or(false) {
        for run in &mut runs {
            run.style = t.effective_char_style(&run.style).ok_or_else(|| bad("text.getRange", "text transform is collapsed or unrepresentable"))?;
        }
    }
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    Ok(json!({"text": text, "runs": runs, "length": len}))
}

/// Character attribute changes parsed from params (shared by range and whole-object styling).
#[derive(Default)]
pub(crate) struct CharChange {
    font: Option<String>,
    style: Option<String>,
    size: Option<f64>,
    leading: Option<Option<f64>>,
    tracking: Option<f64>,
    kerning: Option<Option<f64>>,
    baseline_shift: Option<f64>,
    h_scale: Option<f64>,
    v_scale: Option<f64>,
    rotation: Option<f64>,
    fill: Option<Paint>,
    stroke: Option<Paint>,
    /// Weight (`strokeWidth`), cap, join, miter limit and dashes of the character stroke.
    stroke_opts: StrokeChange,
    underline: Option<bool>,
    strikethrough: Option<bool>,
    all_caps: Option<bool>,
    features: Option<Vec<String>>,
    position: Option<vectorcraft_doc::CharPosition>,
    small_caps: Option<Option<f64>>,
    char_align: Option<vectorcraft_doc::CharAlign>,
    proportional_metrics: Option<bool>,
}

/// `features: ["dlig", "-liga", …]` → the canonical tag list (differences from the defaults).
pub(crate) fn features_param(p: &Value, cmd: &str) -> Result<Option<Vec<String>>> {
    let Some(v) = p.get("features").filter(|v| !v.is_null()) else { return Ok(None) };
    let tags: Vec<&str> = v.as_array().ok_or_else(|| bad(cmd, "features must be a list of tags"))?.iter().filter_map(Value::as_str).collect();
    if let Some(t) = tags.iter().find(|t| !vectorcraft_text::OtFeatures::known_tag(t)) {
        return Err(bad(cmd, format!("unknown OpenType feature `{t}` (liga, calt, dlig, smcp, frac, onum, tnum, ordn, swsh; prefix - to turn off)")));
    }
    Ok(Some(vectorcraft_text::OtFeatures::default().with_tags(tags).to_tags()))
}

/// `charAlign: "romanBaseline"|"emBoxTop"|"emBoxCenter"|"emBoxBottom"|"icfTop"|"icfBottom"` (Character Alignment).
pub(crate) fn char_align_param(p: &Value, cmd: &str) -> Result<Option<vectorcraft_doc::CharAlign>> {
    use vectorcraft_doc::CharAlign;
    let Some(v) = p.get("charAlign").filter(|v| !v.is_null()) else { return Ok(None) };
    Ok(Some(match v.as_str() {
        Some("romanBaseline") => CharAlign::RomanBaseline,
        Some("emBoxTop") => CharAlign::EmBoxTop,
        Some("emBoxCenter") => CharAlign::EmBoxCenter,
        Some("emBoxBottom") => CharAlign::EmBoxBottom,
        Some("icfTop") => CharAlign::IcfTop,
        Some("icfBottom") => CharAlign::IcfBottom,
        _ => {
            return Err(bad(
                cmd,
                "`charAlign` must be \"romanBaseline\", \"emBoxTop\", \"emBoxCenter\", \"emBoxBottom\", \"icfTop\" or \"icfBottom\"",
            ));
        }
    }))
}

fn paint_param(p: &Value, k: &str, cmd: &str) -> Result<Option<Paint>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) if n.eq_ignore_ascii_case("none") => Ok(Some(Paint::None)),
        Some(v) => Ok(Some(Paint::solid(color_value(v).ok_or_else(|| bad(cmd, format!("bad {k} colour")))?))),
    }
}

fn auto_or_num(p: &Value, k: &str, cmd: &str) -> Result<Option<Option<f64>>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => Ok(Some(None)),
        Some(v) => Ok(Some(Some(v.as_f64().ok_or_else(|| bad(cmd, format!("{k} must be a number or \"auto\"")))?))),
    }
}

impl CharChange {
    /// `setup` gives the superscript, subscript and small caps proportions.
    pub(crate) fn parse(p: &Value, cmd: &str, setup: &vectorcraft_doc::DocSetup) -> Result<Self> {
        let num = |k: &str| p.get(k).and_then(Value::as_f64);
        let (position, small_caps) = super::docsetup::script_params(p, setup, cmd)?;
        let flag = |k: &str| p.get(k).and_then(Value::as_bool);
        let mut stroke_opts = match p.get("strokeOptions") {
            None | Some(Value::Null) => StrokeChange::default(),
            Some(o) if o.is_object() => StrokeChange::parse(o, cmd)?,
            Some(_) => return Err(bad(cmd, "strokeOptions must be an object of stroke.set options")),
        };
        if let Some(w) = num("strokeWidth") {
            stroke_opts.weight = Some(w.clamp(0.0, 1000.0));
        }
        let c = Self {
            font: str_param(p, "font").map(str::to_string),
            style: str_param(p, "style").map(str::to_string),
            size: num("size"),
            leading: auto_or_num(p, "leading", cmd)?,
            tracking: num("tracking"),
            kerning: auto_or_num(p, "kerning", cmd)?,
            baseline_shift: num("baselineShift"),
            h_scale: num("hScale"),
            v_scale: num("vScale"),
            rotation: num("rotation"),
            fill: paint_param(p, "fill", cmd)?,
            stroke: paint_param(p, "stroke", cmd)?,
            stroke_opts,
            underline: flag("underline"),
            strikethrough: flag("strikethrough"),
            all_caps: flag("allCaps"),
            features: features_param(p, cmd)?,
            position,
            small_caps,
            char_align: char_align_param(p, cmd)?,
            proportional_metrics: flag("proportionalMetrics"),
        };
        if c.size.is_some_and(|v| v <= 0.0) {
            return Err(bad(cmd, "size must be positive"));
        }
        Ok(c)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.font.is_none()
            && self.style.is_none()
            && self.size.is_none()
            && self.leading.is_none()
            && self.tracking.is_none()
            && self.kerning.is_none()
            && self.baseline_shift.is_none()
            && self.h_scale.is_none()
            && self.v_scale.is_none()
            && self.rotation.is_none()
            && self.fill.is_none()
            && self.stroke.is_none()
            && self.stroke_opts.is_empty()
            && self.underline.is_none()
            && self.strikethrough.is_none()
            && self.all_caps.is_none()
            && self.features.is_none()
            && self.position.is_none()
            && self.small_caps.is_none()
            && self.char_align.is_none()
            && self.proportional_metrics.is_none()
    }

    fn localize(&mut self, t: &TextObject, cmd: &str) -> Result<()> {
        use super::typecmd::local_type_value;
        self.size = self.size.map(|v| local_type_value(t, v, (0.1, 1296.0), false, cmd)).transpose()?;
        self.leading = self.leading.map(|v| v.map(|v| local_type_value(t, v, (0.1, 5000.0), false, cmd)).transpose()).transpose()?;
        self.baseline_shift = self.baseline_shift.map(|v| local_type_value(t, v, (-1296.0, 1296.0), false, cmd)).transpose()?;
        self.h_scale = self.h_scale.map(|v| local_type_value(t, v, (1.0, 10000.0), true, cmd)).transpose()?;
        Ok(())
    }

    pub(crate) fn apply(&self, st: &mut CharStyle) {
        if let Some(f) = &self.font {
            st.font_family = f.clone();
            // Keep the style when the new family has it, else its closest match.
            if self.style.is_none() {
                let styles = vectorcraft_text::FontDb::global().styles(f);
                if !styles.is_empty()
                    && !styles.iter().any(|s| s.eq_ignore_ascii_case(&st.font_style))
                    && let Some(face) = vectorcraft_text::FontDb::global().face(f, &st.font_style)
                {
                    st.font_style = face.style.clone();
                }
            }
        }
        if let Some(v) = &self.style {
            st.font_style = v.clone();
        }
        // Another font: none of the old one's versions.
        if self.font.is_some() || self.style.is_some() {
            st.font_version = None;
        }
        if let Some(v) = self.size {
            st.size = v;
        }
        if let Some(v) = self.leading {
            st.leading = v;
        }
        if let Some(v) = self.tracking {
            st.tracking = v.clamp(-1000.0, 10000.0);
        }
        if let Some(v) = self.kerning {
            st.kerning = v.map(|k| k.clamp(-1000.0, 10000.0));
        }
        if let Some(v) = self.baseline_shift {
            st.baseline_shift = v;
        }
        if let Some(v) = self.h_scale {
            st.h_scale = v;
        }
        if let Some(v) = self.v_scale {
            st.v_scale = v.clamp(1.0, 10000.0);
        }
        if let Some(v) = self.rotation {
            st.rotation = (v + 180.0).rem_euclid(360.0) - 180.0;
        }
        if let Some(v) = &self.fill {
            st.fill = v.clone();
        }
        if let Some(v) = &self.stroke {
            st.stroke = v.clone();
        }
        if !self.stroke_opts.is_empty() {
            self.stroke_opts.apply_char(st);
        }
        if let Some(v) = self.underline {
            st.underline = v;
        }
        if let Some(v) = self.strikethrough {
            st.strikethrough = v;
        }
        if let Some(v) = self.all_caps {
            st.all_caps = v;
        }
        if let Some(v) = &self.features {
            st.features = v.clone();
        }
        if let Some(v) = self.position {
            st.position = v;
        }
        if let Some(v) = self.small_caps {
            st.small_caps = v;
        }
        if let Some(v) = self.char_align {
            st.char_align = v;
        }
        if let Some(v) = self.proportional_metrics {
            st.proportional_metrics = v;
        }
    }
}

/// Type › Enable Missing Glyph Protection: after a font change, the characters of `runs` their new
/// font has no glyph for, but their font `before` the change had, keep that font.
pub(crate) fn protect_missing_glyphs(before: &[TextRun], runs: &mut Vec<TextRun>) {
    fn chars(runs: &[TextRun]) -> impl Iterator<Item = (usize, char, &CharStyle)> {
        runs.iter()
            .scan(0, |at, r| {
                let start = *at;
                *at += r.text.len();
                Some(r.text.char_indices().map(move |(i, c)| (start + i, c, &r.style)))
            })
            .flatten()
    }
    let db = vectorcraft_text::FontDb::global();
    let mut faces = std::collections::HashMap::new();
    let mut covers = |st: &CharStyle, c: char| {
        let face = faces
            .entry((st.font_family.clone(), st.font_style.clone(), st.font_version.clone()))
            .or_insert_with(|| db.face_version(&st.font_family, &st.font_style, st.font_version.as_deref()));
        face.as_ref().map(|f| f.covers(c))
    };
    // Byte ranges that keep a font (family, style).
    let mut keep: Vec<(usize, usize, &CharStyle)> = vec![];
    for ((at, c, old), (_, _, new)) in chars(before).zip(chars(runs)) {
        let changed = (&old.font_family, &old.font_style) != (&new.font_family, &new.font_style);
        if !changed || c.is_whitespace() || c.is_control() || covers(new, c) != Some(false) || covers(old, c) != Some(true) {
            continue;
        }
        let end = at + c.len_utf8();
        match keep.last_mut() {
            Some((_, e, st)) if *e == at && (&st.font_family, &st.font_style) == (&old.font_family, &old.font_style) => *e = end,
            _ => keep.push((at, end, old)),
        }
    }
    for (a, b, old) in keep {
        edit::style_range(runs, a, b, |st| {
            st.font_family.clone_from(&old.font_family);
            st.font_style.clone_from(&old.font_style);
        });
    }
}

fn set_range_style(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.setRangeStyle";
    // The type object given, else the one selected (#785).
    let id = match p.get("id").filter(|v| !v.is_null()) {
        Some(v) => v.as_u64().map(NodeId).ok_or_else(|| bad(C, format!("`id` must be an object id, not {v}")))?,
        None => match super::typecmd::text_targets(s, &json!({}), C)?.as_slice() {
            [one] => *one,
            _ => return Err(bad(C, "missing `id`: give one, or select one type object")),
        },
    };
    let mut change = CharChange::parse(p, C, &s.doc()?.doc.setup)?;
    if change.is_empty() {
        return Err(bad(C, "nothing to change"));
    }
    let protect = s.prefs.missing_glyph_protection && (change.font.is_some() || change.style.is_some());
    let t = text_ref(s, id)?;
    change.localize(&t, C)?;
    let (a, b) = range_of(p, edit::runs_len(&t.runs));
    let n = s.edit("Character", |d, _| {
        let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
        if a == b && !t.plain_text().is_empty() {
            // An empty range inside text styles nothing (Illustrator keeps it for the next typing).
            return Ok(t.runs.len());
        }
        let before = protect.then(|| t.runs.clone());
        edit::style_range(&mut t.runs, a, b, |st| change.apply(st));
        if let Some(before) = before {
            protect_missing_glyphs(&before, &mut t.runs);
        }
        refresh_bounds(t);
        Ok(t.runs.len())
    })?;
    Ok(json!({"id": id.0, "runs": n}))
}

fn create_in_path(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.createInPath";
    let pid = id_param(p, "path").ok_or_else(|| bad(C, "missing `path` (a path object id)"))?;
    let on_path = match str_param(p, "mode").unwrap_or("area") {
        "area" => false,
        "onPath" | "path" => true,
        o => return Err(bad(C, format!("mode must be area|onPath, got {o}"))),
    };
    let node = s.doc()?.doc.node(pid).cloned().ok_or(EngineError::NoNode(pid))?;
    let NodeKind::Path { path, .. } = &node.kind else { return Err(bad(C, "`path` must be a path object")) };
    if path.is_empty() {
        return Err(bad(C, "the path is empty"));
    }
    if !on_path && !path.is_closed() && path.bounds().is_none_or(|b| b.width() < 1.0 || b.height() < 1.0) {
        return Err(bad(C, "area type needs a path that encloses an area"));
    }
    let style = super::create::new_type_style(s, p);
    let text = str_param(p, "text").unwrap_or("").to_string();
    let start = match point_param(p, "at") {
        Some(at) if on_path => vectorcraft_geom::ArcPath::new(path).fraction_at(at).unwrap_or(0.0),
        _ => 0.0,
    };
    let kind = if on_path { TextKind::OnPath { path: path.clone(), start, end: None } } else { TextKind::Area { frame: path.clone() } };
    let mut t = TextObject {
        vertical: p.get("vertical").and_then(Value::as_bool).unwrap_or(false),
        kind,
        xf: Affine::IDENTITY,
        runs: vec![TextRun { text, style, inline: None }],
        para: super::create::new_type_para(),
        paras: Vec::new(),
        area: Default::default(),
        path_effect: Default::default(),
        path_align: Default::default(),
        path_spacing: 0.0,
        wrap: Vec::new(),
        cached_bounds: None,
        cached_baselines: Vec::new(),
    };
    super::create::new_type_alignment(s, p, C, &mut t)?;
    if !on_path {
        t.area.fit = super::create::new_area_fit(s, p, C)?;
    }
    if bool_or(p, "placeholder", false) {
        super::typemenu::fill_with_placeholder(&mut t);
    } else {
        refresh_bounds(&mut t);
    }
    let id = s.edit(if on_path { "Type on a Path" } else { "Area Type" }, |d, sel| {
        let (par, idx, _) = d.position(pid).ok_or(EngineError::NoNode(pid))?;
        d.remove(pid)?;
        let id = d.alloc_id();
        let mut n = Node::new(id, NodeKind::Text(Box::new(t)));
        n.appearance = Appearance::default();
        n.opacity = node.opacity;
        n.name = node.name.clone();
        d.insert(par, idx, n)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({"id": id.0}))
}

/// What `type.step` changes.
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Size,
    Leading,
    Tracking,
    Kerning,
    BaselineShift,
}

/// The Type tool's selected text while it edits (its typing session ends first: one undo step).
pub(crate) fn editing_range(s: &mut Session) -> Result<Option<(NodeId, usize, usize)>> {
    let o = s.tool_options();
    let Some(id) = o.get("editing").and_then(Value::as_u64).map(NodeId) else { return Ok(None) };
    if super::edit::typing_in_progress(s) {
        s.set_tool_option("commitTyping", &Value::Bool(true));
        s.commit_interaction()?;
    }
    let at = |k: &str| byte_param(&o, k).unwrap_or(0);
    Ok(Some((id, at("start"), at("end"))))
}

fn type_step(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.step";
    let step = match str_param(p, "attribute") {
        Some("size") => Step::Size,
        Some("leading") => Step::Leading,
        Some("tracking") => Step::Tracking,
        Some("kerning") => Step::Kerning,
        Some("baselineShift") => Step::BaselineShift,
        _ => return Err(bad(C, "attribute must be size|leading|tracking|kerning|baselineShift")),
    };
    let by = f64_or(p, "by", 1.0);
    if !by.is_finite() {
        return Err(bad(C, "by must be a number"));
    }
    let prefs = &s.prefs;
    let inc = by.clamp(-1000.0, 1000.0)
        * match step {
            Step::Size | Step::Leading => prefs.type_size_increment,
            Step::Tracking | Step::Kerning => prefs.tracking_increment,
            Step::BaselineShift => prefs.baseline_shift_increment,
        };
    // (text, its range; None: all of it).
    let targets: Vec<(NodeId, Option<(usize, usize)>)> = match id_param(p, "id") {
        Some(id) => vec![(id, Some(range_of(p, edit::runs_len(&text_ref(s, id)?.runs))))],
        None => match editing_range(s)? {
            Some((id, a, b)) => vec![(id, Some((a.min(b), a.max(b))))],
            None => super::typecmd::text_targets(s, p, C)?.into_iter().map(|id| (id, None)).collect(),
        },
    };
    // The ranges stepped. At a caret: kerning is the space after the character before it, leading
    // the paragraph's; the rest wait for a selection.
    let mut ranges = vec![];
    for (id, range) in targets {
        let text = text_ref(s, id)?.plain_text();
        let (a, b) = range.map_or((0, text.len()), |(a, b)| (a.min(text.len()), b.min(text.len())));
        let (a, b) = match step {
            _ if a < b => (a, b),
            Step::Kerning => (edit::prev_char(&text, a), a),
            Step::Leading => {
                let para = edit::paragraph_at(&text, a);
                (para.start, para.end)
            }
            _ => continue,
        };
        if a < b {
            ranges.push((id, a, b));
        }
    }
    if !ranges.is_empty() {
        s.edit("Character", |d, _| {
            for &(id, a, b) in &ranges {
                let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
                let points = if matches!(step, Step::Size | Step::Leading | Step::BaselineShift) {
                    t.style_scale().ok_or_else(|| bad(C, "text transform is collapsed or unrepresentable"))?.points
                } else {
                    1.0
                };
                let mut runs = edit::slice_runs(&t.runs, a, b);
                for run in &mut runs {
                    let st = &mut run.style;
                    match step {
                        Step::Size => st.size = super::typecmd::local_type_value(t, st.size * points + inc, (0.1, 1296.0), false, C)?,
                        Step::Leading => {
                            st.leading = Some(super::typecmd::local_type_value(t, st.effective_leading() * points + inc, (0.1, 5000.0), false, C)?);
                        }
                        Step::Tracking => st.tracking = (st.tracking + inc).clamp(-1000.0, 10000.0),
                        Step::Kerning => st.kerning = Some((st.kerning.unwrap_or(0.0) + inc).clamp(-1000.0, 10000.0)),
                        Step::BaselineShift => {
                            st.baseline_shift = super::typecmd::local_type_value(t, st.baseline_shift * points + inc, (-1296.0, 1296.0), false, C)?;
                        }
                    }
                }
                edit::replace_range_styled(&mut t.runs, a, b, &runs);
                refresh_bounds(t);
            }
            Ok(())
        })?;
    }
    Ok(json!({ "ids": ranges.iter().map(|(id, ..)| id.0).collect::<Vec<_>>() }))
}

/// Tracking (1/1000 em) that makes the first paragraph of `t` exactly `target` wide, if it fits
/// on one line at all.
fn headline_tracking(t: &TextObject, target: f64) -> Option<f64> {
    let plain = t.plain_text();
    let end = plain.find('\n').unwrap_or(plain.len());
    if end == 0 {
        return None;
    }
    let head = edit::slice_runs(&t.runs, 0, end);
    let measure = |tr: f64| {
        let mut h = TextObject {
            vertical: t.vertical,
            kind: TextKind::Point,
            xf: Affine::IDENTITY,
            runs: head.clone(),
            para: Default::default(),
            paras: Vec::new(),
            area: Default::default(),
            path_effect: Default::default(),
            path_align: Default::default(),
            path_spacing: 0.0,
            wrap: Vec::new(),
            cached_bounds: None,
            cached_baselines: Vec::new(),
        };
        for r in &mut h.runs {
            r.style.tracking = tr;
        }
        let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &h);
        l.lines.first().map_or(0.0, |li| li.x1 - li.x0)
    };
    // Width is linear in tracking: w(tr) = w0 + tr * slope.
    let (w0, w1) = (measure(0.0), measure(100.0));
    let slope = (w1 - w0) / 100.0;
    if slope <= 1e-9 {
        return None;
    }
    // A hair under the target so the line doesn't wrap.
    Some(((target - w0) / slope - 0.05).clamp(-1000.0, 10000.0))
}

fn fit_headline(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.fitHeadline";
    let ids: Vec<NodeId> = {
        let st = s.doc()?;
        targets(s, p)?
            .into_iter()
            .filter(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Area { .. })))
            .collect()
    };
    if ids.is_empty() {
        return Err(bad(C, "select area type"));
    }
    let mut applied = vec![];
    s.edit("Fit Headline", |d, _| {
        for id in &ids {
            let Some(t) = text_mut(d, *id) else { continue };
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let Some(first) = lay.lines.first() else { continue };
            let target = first.avail.1 - first.avail.0;
            let Some(tr) = headline_tracking(t, target) else { continue };
            let end = t.plain_text().find('\n').unwrap_or(t.plain_text().len());
            edit::style_range(&mut t.runs, 0, end, |st| st.tracking = tr);
            refresh_bounds(t);
            applied.push(tr);
        }
        Ok(())
    })?;
    Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "tracking": applied}))
}

fn discard_empty(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.discardEmpty";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    if edit::runs_len(&text_ref(s, id)?.runs) > 0 {
        return Ok(json!({"removed": false}));
    }
    // The newest step whose document lacks the text created it: when the document now is that one
    // plus the text (and its id), roll back to it without a trace.
    let st = s.doc_mut()?;
    if st.interaction.is_none()
        && let Some(k) = st.history.undo.iter().rposition(|e| e.doc.node(id).is_none())
        && let Some(created) = st.history.undo.get(k)
    {
        let mut now = (*st.doc).clone();
        let mut before = (*created.doc).clone();
        before.alloc_id();
        if now.remove(id).is_ok() && now == before {
            st.doc = created.doc.clone();
            st.history.undo.truncate(k);
            st.selection.prune(&st.doc);
            st.revision += 1;
            return Ok(json!({"removed": true}));
        }
    }
    s.edit("Discard Empty Type", |d, _| d.remove(id).map(|_| ()).map_err(|_| EngineError::NoNode(id)))?;
    Ok(json!({"removed": true}))
}
