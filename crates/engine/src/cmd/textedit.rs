//! Rich text editing commands: range edits and range styling (what the Type tool and the
//! Character panel use while editing), creating area / on-path type from a path, and Fit Headline.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, CharStyle, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, Point, Vec2};
use vectorcraft_text::edit;

use super::stroke::StrokeChange;
use super::typecmd::refresh_bounds;
use super::{
    CommandSpec, EngineError, Result, Session, bad, bool_or, check_numbers, check_switches, cmd, color_value, has_doc, has_selection, id_param,
    index, number_or, point_param, str_param, switch_or, targets,
};

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
            "text.startList",
            "Bullets and Numbering",
            [],
            None,
            "{id, at?: byte (default: the end of the text)} start a list from typed characters, what the Type tool runs when a space is typed on its own: when the text from the start of the paragraph that `at` is in up to `at` is \"1. \" or \"1) \" (a numbered list in 1, 2, 3), \"A. \" or \"A) \" (a numbered list in A, B, C), or \"* \", \"# \", \"> \" or \"+ \" (a bulleted list with the filled dot), and the paragraph is in no list, those characters are removed and the paragraph joins that list as one undo step (left and first-line indents of 0 become 18 and -18, as text.setFormat sets them); otherwise nothing changes → {id, started: bool, caret: byte (the paragraph's start when started, else `at`)}",
            has_doc,
            start_list
        ),
        cmd!(
            "text.setRangeStyle",
            "Character",
            [],
            None,
            "{id? (default: the one selected type object), start?: byte, end?: byte (default: all text), font?, style?, size?: pt, leading?: pt|\"auto\", tracking?, kerning?: 1/1000 em|\"auto\", baselineShift?: pt, hScale?: %, vScale?: %, rotation?: deg, fill?: colour|\"none\", stroke?: colour|\"none\", keepModel?: bool (fill and stroke, as paint.setFill), strokeWidth?: pt, strokeOptions?: {weight?, cap?, join?, miterLimit?, align?, dash?, dashOffset?, alignDashes?} (as stroke.set: the character stroke), underline?, strikethrough?, allCaps?: bool, smallCaps?: bool, position?: \"normal\"|\"superscript\"|\"subscript\" (sizes from Document Setup), features?: [\"dlig\", \"-liga\", …], charAlign?: \"romanBaseline\"|\"emBoxTop\"|\"emBoxCenter\"|\"emBoxBottom\"|\"icfTop\"|\"icfBottom\", proportionalMetrics?: bool (full-width glyphs on the font's proportional widths: `palt` in horizontal type, `vpal` in vertical type), tateChuYoko?: bool (Character panel menu › Tate-chu-yoko, with Show East Asian Options: in vertical type those characters stand side by side across one em of the column; kept on the characters and ignored in horizontal type), noBreak?: bool (Character panel menu › No Break: these characters stay on one line, including across a space inside the run), charDirection?: \"default\"|\"leftToRight\"|\"rightToLeft\" (Character panel menu › Character Direction: the characters are set left to right or right to left inside their paragraph, as a Unicode bidi embedding; default: as the bidi algorithm sets them), digits?: \"default\"|\"arabic\"|\"hindi\"|\"farsi\" (Character panel menu › Digits: the numeral system every digit is drawn in, whatever was typed), fontAxes?: {\"wght\": 650, …} (Character panel › Variable Font: the setting of each variation axis named, in the axis's units and clamped to its range; null: back to the style's own setting; other axes keep theirs; tags the font has no axis for are left out; a font or style change clears them; text.fontList {family, style} lists the axes)} style a character range (size/leading/baselineShift are document points and hScale includes the object transform; runs are split at the range ends) → {id, runs}",
            has_doc,
            set_range_style
        ),
        cmd!(
            "text.touchType",
            "Touch Type",
            [],
            None,
            "{id, start: byte (a character of it), dx?: pt, dy?: pt, hScale?: %, vScale?: %, rotation?: deg} change one character (a ligature, one glyph for several characters, as a whole) as a Touch Type tool drag does, as one undo step: dx moves it along its line in reading order by changing the kerning of the character before it on the line and its own kerning, and the characters after it stay where they are (an error for the first character of a line and for a right-to-left character; limited to what kerning from -1000 to 10000 can give); dy raises it (baseline shift, along its own up axis when it is rotated); dx and dy are document points; hScale (including the object transform), vScale and rotation set its scales and rotation as text.setRangeStyle does. The kerning of the two characters also keeps the character and the ones after it where they were along the line when the font kerned them with their neighbors; a wider or narrower character moves the ones after it by the change in its width → {id, start, end} (the character's bytes)",
            has_doc,
            touch_type
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
            "{path: id, mode: \"area\"|\"onPath\", text?: \"\", vertical?: bool = false, at?: [x, y] (on-path start: nearest point), size?, font?, style?, color?, keepModel? (as text.create), fit?: none|autoHeight|shrinkText, fitMinPercent? (area: as text.areaOptions; default autoHeight when the autoSizeAreaType preference is on), placeholder?: bool (placeholder text instead, as text.create), justify?, leadingModel?, charAlign? (as text.create)} turn a path into an area-type frame or a type-on-a-path baseline (the path's paint is dropped) → {id, bounds: {x, y, width, height} (as text.create), overflow?: [id] (the text doesn't fit; left out when it fits)}",
            has_doc,
            create_in_path
        ),
        cmd!(
            "type.fitHeadline",
            "Fit Headline",
            ["Type"],
            None,
            "{ids?|id?} track the first line of area type so it fills the frame width → {ids, tracking}",
            has_selection,
            fit_headline
        ),
        cmd!(
            "type.step",
            "Step Type",
            [],
            None,
            "{attribute: \"size\"|\"leading\"|\"tracking\"|\"kerning\"|\"baselineShift\", by?: steps (1; negative steps down), id?, start?: byte, end?: byte, ids?: [id…] (instead of id)} step type by the Preferences › Type increments (size and leading: typeSizeIncrement, pt; tracking and kerning: trackingIncrement, 1/1000 em; baseline shift: baselineShiftIncrement, pt), what the Type tool's Alt+arrows do (Cmd/Ctrl too: five steps; horizontal: ←/→ kerning at the caret or tracking of the selection, ↑/↓ leading, Shift+↑/↓ baseline shift; vertical: ↓/↑ with or without Shift kerning or tracking, ←/→ leading, Shift+→/← baseline shift; mixed selected text objects use the horizontal mapping). The range of `id`, else the type objects in `ids`, else the text the Type tool has selected (at a caret: kerning steps the character before it, leading its paragraph), else every selected type object → {ids}",
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
            |s, p| type_step(s, &json!({"attribute": "size", "by": number_or(p, "by", 1.0, "type.size.increase")?}))
        ),
        cmd!(
            "type.size.decrease",
            "Decrease Font Size",
            [],
            Some("Cmd+Shift+,"),
            "{by?: steps (1)} type.step {attribute: size, by: -by}",
            has_selection,
            |s, p| type_step(s, &json!({"attribute": "size", "by": -number_or(p, "by", 1.0, "type.size.decrease")?}))
        ),
        cmd!(
            "type.size.increaseByFive",
            "Increase Font Size by Five Steps",
            [],
            Some("Cmd+Alt+Shift+."),
            "{} type.step {attribute: size, by: 5}: five times the Size/Leading increment (typeSizeIncrement)",
            has_selection,
            |s, _| type_step(s, &json!({"attribute": "size", "by": 5.0}))
        ),
        cmd!(
            "type.size.decreaseByFive",
            "Decrease Font Size by Five Steps",
            [],
            Some("Cmd+Alt+Shift+,"),
            "{} type.step {attribute: size, by: -5}: five times the Size/Leading increment (typeSizeIncrement)",
            has_selection,
            |s, _| type_step(s, &json!({"attribute": "size", "by": -5.0}))
        ),
        cmd!(
            "type.resetTracking",
            "Reset Tracking and Kerning",
            [],
            Some("Cmd+Alt+Q"),
            "{ids?: [id…]} set tracking to 0 and kerning to Auto (the font's own kerning, which the Character panel shows as 0) in all the text of the type objects in `ids`, else in the text the Type tool has selected (at a caret: only the kerning of the character before it), else in every selected type object, as one undo step → {ids}",
            has_selection,
            reset_tracking
        ),
        cmd!(
            "type.resetHorizontalScale",
            "Reset Horizontal Scale",
            [],
            Some("Cmd+Shift+X"),
            "{ids?: [id…]} set the horizontal scale to 100% (as the Character panel shows it, the object's own transform included) in all the text of the type objects in `ids`, else in the text the Type tool has selected (nothing at a caret), else in every selected type object, as one undo step → {ids}",
            has_selection,
            reset_horizontal_scale
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

/// The byte offset that `k` holds, read with [`index`].
fn byte_param(p: &Value, k: &str) -> Option<usize> {
    p.get(k).and_then(Value::as_u64).map(index)
}

/// The bytes `start..end` of `p` in text of `len` bytes, clamped and ordered (without them, all of
/// it). A `start` or `end` that is not a number or null, such as `"2"`, is an error.
fn range_of(p: &Value, len: usize, cmd: &str) -> Result<(usize, usize)> {
    check_numbers(p, &["start", "end"], cmd)?;
    let a = byte_param(p, "start").unwrap_or(0).min(len);
    let b = byte_param(p, "end").unwrap_or(len).min(len);
    Ok((a.min(b), a.max(b)))
}

fn edit_range(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.editRange";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let t = text_ref(s, id)?;
    let len = edit::runs_len(&t.runs);
    let (a, b) = range_of(p, len, C)?;
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
        // Ruby over the edited text moves with it (its ranges are plain-text bytes).
        t.ruby_after_replace(a, b, inserted.len());
        refresh_bounds(t);
        Ok(caret)
    })?;
    Ok(json!({"id": id.0, "caret": caret}))
}

fn start_list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.startList";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    check_numbers(p, &["at"], C)?;
    let t = text_ref(s, id)?;
    let len = edit::runs_len(&t.runs);
    let at = byte_param(p, "at").unwrap_or(len).min(len);
    let Some((start, list)) = t.typed_list_at(at) else {
        return Ok(json!({"id": id.0, "started": false, "caret": at}));
    };
    s.edit("Bullets and Numbering", |d, _| {
        let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
        t.splice_paras(start, at, "");
        edit::replace_range(&mut t.runs, start, at, "");
        t.ruby_after_replace(start, at, 0);
        let i = t.paragraphs_in(start, start).start;
        t.edit_paras(Some(i..i + 1), |para| {
            para.list = Some(list);
            // A paragraph with no left or first-line indent gets the hanging indent.
            if para.left_indent == 0.0 && para.first_line_indent == 0.0 {
                para.left_indent = vectorcraft_doc::text::LIST_HANGING_INDENT;
                para.first_line_indent = -vectorcraft_doc::text::LIST_HANGING_INDENT;
            }
        });
        refresh_bounds(t);
        Ok(())
    })?;
    Ok(json!({"id": id.0, "started": true, "caret": start}))
}

fn get_range(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("text.getRange", "missing `id`"))?;
    let t = text_ref(s, id)?;
    let len = edit::runs_len(&t.runs);
    let (a, b) = range_of(p, len, "text.getRange")?;
    let mut runs = edit::slice_runs(&t.runs, a, b);
    if switch_or(p, "effective", false, "text.getRange")? {
        for run in &mut runs {
            run.style = t.effective_char_style(&run.style).ok_or_else(|| bad("text.getRange", "text transform is collapsed or unrepresentable"))?;
        }
    }
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    Ok(json!({"text": text, "runs": runs, "length": len}))
}

/// Keep `st`'s font style when its family has it, else take the family's closest one: a family
/// whose styles are numbered (W3, W6) has no Regular (#4128). A family that isn't installed keeps
/// the style as it is.
pub(crate) fn fit_font_style(st: &mut CharStyle) {
    let db = vectorcraft_text::FontDb::global();
    let styles = db.styles(&st.font_family);
    if !styles.is_empty()
        && !styles.iter().any(|s| s.eq_ignore_ascii_case(&st.font_style))
        && let Some(face) = db.face(&st.font_family, &st.font_style)
    {
        st.font_style = face.style.clone();
    }
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
    char_direction: Option<Option<vectorcraft_doc::ParaDirection>>,
    proportional_metrics: Option<bool>,
    tate_chu_yoko: Option<bool>,
    no_break: Option<bool>,
    digits: Option<vectorcraft_doc::Digits>,
    font_axes: Option<AxisChange>,
}

/// `charDirection: "default"|"leftToRight"|"rightToLeft"` → the Character Direction to set (`None`
/// inside: Default).
pub(crate) fn char_direction_param(p: &Value, cmd: &str) -> Result<Option<Option<vectorcraft_doc::ParaDirection>>> {
    use vectorcraft_doc::ParaDirection;
    let Some(v) = p.get("charDirection").filter(|v| !v.is_null()) else { return Ok(None) };
    Ok(Some(match v.as_str() {
        Some("default") => None,
        Some("leftToRight") => Some(ParaDirection::LeftToRight),
        Some("rightToLeft") => Some(ParaDirection::RightToLeft),
        _ => return Err(bad(cmd, "`charDirection` must be \"default\", \"leftToRight\" or \"rightToLeft\"")),
    }))
}

/// `digits: "default"|"arabic"|"hindi"|"farsi"` → the numeral system the digits are drawn in.
pub(crate) fn digits_param(p: &Value, cmd: &str) -> Result<Option<vectorcraft_doc::Digits>> {
    use vectorcraft_doc::Digits;
    let Some(v) = p.get("digits").filter(|v| !v.is_null()) else { return Ok(None) };
    Ok(Some(match v.as_str() {
        Some("default") => Digits::Default,
        Some("arabic") => Digits::Arabic,
        Some("hindi") => Digits::Hindi,
        Some("farsi") => Digits::Farsi,
        _ => return Err(bad(cmd, "`digits` must be \"default\", \"arabic\", \"hindi\" or \"farsi\"")),
    }))
}

/// Variable Font settings to change: by axis tag, a value in the axis's units, or `None` to go
/// back to the style's own setting of that axis.
pub(crate) type AxisChange = std::collections::BTreeMap<String, Option<f64>>;

/// `fontAxes: {"wght": 650, "wdth": null, …}` → the Variable Font settings to change. A tag is four
/// printable ASCII characters (`wght`); a value is a number, or null.
pub(crate) fn font_axes_param(p: &Value, cmd: &str) -> Result<Option<AxisChange>> {
    let Some(v) = p.get("fontAxes").filter(|v| !v.is_null()) else { return Ok(None) };
    let obj = v.as_object().ok_or_else(|| bad(cmd, "`fontAxes` must be an object of axis tags and values, such as {\"wght\": 650}"))?;
    let mut out = AxisChange::new();
    for (tag, value) in obj {
        if tag.len() != 4 || !tag.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Err(bad(cmd, format!("`{tag}` is not an axis tag: a tag is four characters, such as \"wght\"")));
        }
        let value = match value {
            Value::Null => None,
            v => Some(v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("the value of axis `{tag}` must be a number or null")))?),
        };
        out.insert(tag.clone(), value);
    }
    Ok(Some(out))
}

/// Change the Variable Font settings of `st` by `change`. With its font available, a value is
/// clamped to its axis's range, a tag its font has no axis for is left out, and a value equal to
/// the style's own setting is removed; with the font missing, the values are kept as given.
pub(crate) fn change_font_axes(st: &mut CharStyle, change: &AxisChange) {
    let db = vectorcraft_text::FontDb::global();
    let face = match db.resolve(&st.font_family, &st.font_style) {
        Some((f, m)) if m != vectorcraft_text::FontMatch::Missing => {
            Some(db.face_version(&st.font_family, &st.font_style, st.font_version.as_deref()).unwrap_or(f))
        }
        _ => None,
    };
    let axes = face.as_ref().map(|f| f.axes()).unwrap_or_default();
    for (tag, value) in change {
        let Some(v) = value else {
            st.font_axes.remove(tag);
            continue;
        };
        let Some(face) = &face else {
            st.font_axes.insert(tag.clone(), *v);
            continue;
        };
        let Some(axis) = axes.iter().find(|a| a.tag.as_slice() == tag.as_bytes()) else { continue };
        let v = v.clamp(f64::from(axis.min.min(axis.max)), f64::from(axis.max.max(axis.min)));
        if face.axis_value(axis.tag).is_some_and(|own| f64::from(own) == v) {
            st.font_axes.remove(tag);
        } else {
            st.font_axes.insert(tag.clone(), v);
        }
    }
}

/// `features: ["dlig", "-liga", …]` → the canonical tag list (differences from the defaults).
pub(crate) fn features_param(p: &Value, cmd: &str) -> Result<Option<Vec<String>>> {
    let Some(v) = p.get("features").filter(|v| !v.is_null()) else { return Ok(None) };
    let tags: Vec<&str> = v.as_array().ok_or_else(|| bad(cmd, "features must be a list of tags"))?.iter().filter_map(Value::as_str).collect();
    if let Some(t) = tags.iter().find(|t| !vectorcraft_text::OtFeatures::known_tag(t)) {
        return Err(bad(
            cmd,
            format!(
                "unknown OpenType feature `{t}` (liga, calt, dlig, smcp, frac, onum, tnum, ordn, swsh, ss01–ss20, salt, titl, pnum, lnum, sups, subs; prefix - to turn off)"
            ),
        ));
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
        check_numbers(p, &["size", "tracking", "baselineShift", "hScale", "vScale", "rotation", "strokeWidth"], cmd)?;
        check_switches(p, &["underline", "strikethrough", "allCaps", "proportionalMetrics", "tateChuYoko", "noBreak"], cmd)?;
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
            char_direction: char_direction_param(p, cmd)?,
            proportional_metrics: flag("proportionalMetrics"),
            tate_chu_yoko: flag("tateChuYoko"),
            no_break: flag("noBreak"),
            digits: digits_param(p, cmd)?,
            font_axes: font_axes_param(p, cmd)?,
        };
        if c.size.is_some_and(|v| v <= 0.0) {
            return Err(bad(cmd, "size must be positive"));
        }
        Ok(c)
    }

    /// The `fill` and `stroke` colors as the document stores them: an RGB color in a CMYK
    /// document becomes CMYK through the color settings ([`super::paint::new_color`]).
    fn store_colors(&mut self, s: &Session, p: &Value) {
        for paint in [&mut self.fill, &mut self.stroke].into_iter().flatten() {
            if let Paint::Solid { color, .. } = paint {
                *color = super::paint::new_color(s, p, *color);
            }
        }
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
            && self.char_direction.is_none()
            && self.proportional_metrics.is_none()
            && self.tate_chu_yoko.is_none()
            && self.no_break.is_none()
            && self.digits.is_none()
            && self.font_axes.is_none()
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
                fit_font_style(st);
            }
        }
        if let Some(v) = &self.style {
            st.font_style = v.clone();
        }
        // Another font: none of the old one's versions or Variable Font settings.
        if self.font.is_some() || self.style.is_some() {
            st.font_version = None;
            st.font_axes.clear();
        }
        if let Some(v) = &self.font_axes {
            change_font_axes(st, v);
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
        if let Some(v) = self.char_direction {
            st.char_direction = v;
        }
        if let Some(v) = self.proportional_metrics {
            st.proportional_metrics = v;
        }
        if let Some(v) = self.tate_chu_yoko {
            st.tate_chu_yoko = v;
        }
        if let Some(v) = self.no_break {
            st.no_break = v;
        }
        if let Some(v) = self.digits {
            st.digits = v;
        }
    }
}

/// Type › Enable Missing Glyph Protection: after a font change, the characters of `runs` their new
/// font has no glyph for, but their font `before` the change had, keep that font: its family,
/// style, version and Variable Font settings.
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
    // Byte ranges that keep a font (family, style, version and Variable Font settings).
    let font = |st: &CharStyle| (st.font_family.clone(), st.font_style.clone(), st.font_version.clone(), st.font_axes.clone());
    let mut keep: Vec<(usize, usize, &CharStyle)> = vec![];
    for ((at, c, old), (_, _, new)) in chars(before).zip(chars(runs)) {
        let changed = (&old.font_family, &old.font_style) != (&new.font_family, &new.font_style);
        if !changed || c.is_whitespace() || c.is_control() || covers(new, c) != Some(false) || covers(old, c) != Some(true) {
            continue;
        }
        let end = at + c.len_utf8();
        match keep.last_mut() {
            Some((_, e, st)) if *e == at && font(st) == font(old) => *e = end,
            _ => keep.push((at, end, old)),
        }
    }
    for (a, b, old) in keep {
        edit::style_range(runs, a, b, |st| {
            st.font_family.clone_from(&old.font_family);
            st.font_style.clone_from(&old.font_style);
            st.font_version.clone_from(&old.font_version);
            st.font_axes.clone_from(&old.font_axes);
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
    super::paint::check_keep_model(p, C)?;
    change.store_colors(s, p);
    if change.is_empty() {
        return Err(bad(C, "nothing to change"));
    }
    let protect = s.prefs.missing_glyph_protection && (change.font.is_some() || change.style.is_some());
    let t = text_ref(s, id)?;
    change.localize(&t, C)?;
    let (a, b) = range_of(p, edit::runs_len(&t.runs), C)?;
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

/// The em a laid-out glyph's kerning and tracking are measured in, in text space: the size it
/// was shaped at (superscript, small caps and Shrink Text to Fit included).
fn glyph_em(t: &TextObject, g: &vectorcraft_text::PositionedGlyph) -> Option<f64> {
    let face = vectorcraft_text::FontDb::global().face_by_id(g.font_id)?;
    let h_scale = edit::slice_runs(&t.runs, g.byte, g.byte + g.len).first()?.style.h_scale / 100.0;
    let [a, b, ..] = g.xf.as_coeffs();
    Some(a.hypot(b) * face.units_per_em() / h_scale).filter(|em| em.is_finite() && *em > 0.0)
}

fn touch_type(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.touchType";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    check_numbers(p, &["start"], C)?;
    let start = byte_param(p, "start").ok_or_else(|| bad(C, "missing `start` (the byte offset of a character)"))?;
    if !["dx", "dy", "hScale", "vScale", "rotation"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        return Err(bad(C, "nothing to change: give dx, dy, hScale, vScale or rotation"));
    }
    let num = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(0.0),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(C, format!("`{k}` must be a number"))),
    };
    let (dx, dy) = (num("dx")?, num("dy")?);
    let look = json!({"hScale": p.get("hScale"), "vScale": p.get("vScale"), "rotation": p.get("rotation")});
    let mut change = CharChange::parse(&look, C, &s.doc()?.doc.setup)?;
    let t = text_ref(s, id)?;
    change.localize(&t, C)?;
    let points = t.style_scale().ok_or_else(|| bad(C, "text transform is collapsed or unrepresentable"))?.points;
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| g.byte <= start && start < g.byte + g.len).ok_or_else(|| bad(C, format!("no character at byte {start}")))?;
    let (a, b) = (g.byte, g.byte + g.len);
    // The line's direction in reading order.
    let d = Vec2::new(g.angle.cos(), g.angle.sin());
    let along = if g.rtl { -d } else { d };
    let linear = |m: Affine, v: Vec2| m * (Point::ZERO + v) - m * Point::ZERO;
    // Document points per text-space point along `v`.
    let doc_len = |v: Vec2| linear(t.xf, v).hypot() / v.hypot();
    let limit = 1296.0 / points;
    if dx != 0.0 && g.rtl {
        return Err(bad(C, "a right-to-left character moves only up and down (dy)"));
    }
    // A list marker's glyphs (length 0) stand for no character.
    let prev = lay.glyphs.iter().find(|h| h.line == g.line && h.len > 0 && h.byte + h.len == a);
    let pr = prev.map(|h| (h.byte, h.byte + h.len));
    let em = glyph_em(&t, g).ok_or_else(|| bad(C, "the character's size can't be measured"))?;
    let em_prev = match prev {
        Some(h) => Some(glyph_em(&t, h).ok_or_else(|| bad(C, "the character's size can't be measured"))?),
        None if dx != 0.0 => return Err(bad(C, "the first character of a line has no character before it to kern: it moves only up and down (dy)")),
        None => None,
    };
    let kerning_in = |(x, y): (usize, usize)| edit::slice_runs(&t.runs, x, y).first().and_then(|r| r.style.kerning).unwrap_or(0.0);
    // The move along the line, in text space, limited to what kerning from -1000 to 10000 (1/1000
    // em) on the character before it and on the character itself can give.
    let mut moved = dx / doc_len(along);
    if let (Some(range), Some(ep)) = (pr, em_prev) {
        let (kp, kc) = (kerning_in(range), kerning_in((a, b)));
        let hi = ((10000.0 - kp) * ep).min((kc + 1000.0) * em) / 1000.0;
        let lo = ((-1000.0 - kp) * ep).max((kc - 10000.0) * em) / 1000.0;
        if lo <= hi {
            moved = moved.clamp(lo, hi);
        }
    }
    // The character's scales, rotation and baseline shift, and kerning added to the character
    // before it (`before`) and to the character itself (`own`), in 1/1000 em.
    let touch = |t: &mut TextObject, before: Option<f64>, own: Option<f64>, raise: f64| {
        for (range, by) in [(pr, before), (Some((a, b)), own)] {
            if let (Some((x, y)), Some(by)) = (range, by) {
                edit::style_range(&mut t.runs, x, y, |st| st.kerning = Some((st.kerning.unwrap_or(0.0) + by).clamp(-1000.0, 10000.0)));
            }
        }
        edit::style_range(&mut t.runs, a, b, |st| {
            if raise != 0.0 {
                st.baseline_shift = (st.baseline_shift + raise).clamp(-limit, limit);
            }
            change.apply(st);
        });
    };
    // The baseline shift that raises the character `dy` document points. Baseline shift moves it
    // along its own up axis, turned with its rotation (the new one, when the call sets it), by a
    // length that the object transform, Shrink Text to Fit and a path effect such as Skew scale:
    // two layouts with the call's other changes and shifts one step apart, both within the range,
    // measure that length. Neither shift is one that a run already has, so the character's run
    // splits off alike in both.
    let raise = if dy == 0.0 {
        0.0
    } else {
        let shift = edit::slice_runs(&t.runs, a, b).first().map_or(0.0, |r| r.style.baseline_shift);
        let from = shift.clamp(-limit, limit);
        let free = |v: f64| v.abs() <= limit && t.runs.iter().all(|r| r.style.baseline_shift != v);
        // A step of a quarter of the range at most leaves room for two steps up or down.
        let unit = (limit / 4.0).min(1.0);
        let step =
            [1.0, -1.0, 0.75, -0.75].into_iter().map(|f| f * unit).find(|p| free(from + p) && free(from + 2.0 * p)).unwrap_or(-unit.copysign(from));
        // The character's pen with its baseline shift set to `to`.
        let pen_at = |to: f64| {
            let mut trial = t.clone();
            touch(&mut trial, None, None, to - shift);
            let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &trial);
            l.glyphs.iter().find(|h| h.byte == a && h.len > 0).map(|h| h.xf * Point::ZERO)
        };
        let per_step = match (pen_at(from + step), pen_at(from + 2.0 * step)) {
            (Some(p), Some(q)) => linear(t.xf, q - p).hypot() / step.abs(),
            _ => 0.0,
        };
        dy / per_step
    };
    if !raise.is_finite() {
        return Err(bad(C, "the character's size can't be measured"));
    }
    // Where a glyph starts along the line.
    let pen = |h: &vectorcraft_text::PositionedGlyph| h.origin.to_vec2().dot(along);
    // How much further the character's new scales advance it along the line: the character laid
    // out alone, in point type of the same orientation, before and after the change.
    let grown = {
        let mut alone = TextObject { kind: TextKind::Point, runs: edit::slice_runs(&t.runs, a, b), paras: Vec::new(), ..t.clone() };
        let advance = |o: &TextObject| {
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), o).glyphs.iter().filter(|h| h.len > 0).map(|h| h.advance).sum::<f64>()
        };
        let old = advance(&alone);
        edit::style_range(&mut alone.runs, 0, b - a, |st| change.apply(st));
        (advance(&alone) - old) * lay.fit_scale
    };
    // The character lands `moved` further on, and the character after it on its line keeps its
    // distance from it, wider or narrower only by the character's new scales.
    let goal = pen(g) + moved;
    let next_goal = lay.glyphs.iter().find(|h| h.byte == b && h.len > 0 && h.line == g.line).map(|h| pen(h) + grown);
    // How far the character and the character after it on its line miss their places, along the
    // line, with the kerning `k` (before, own) added.
    let miss = |k: (Option<f64>, Option<f64>)| {
        let mut trial = t.clone();
        touch(&mut trial, k.0, k.1, raise);
        let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &trial);
        let h = l.glyphs.iter().find(|h| h.byte == a && h.len > 0)?;
        let next = match (l.glyphs.iter().find(|n| n.byte == b && n.len > 0 && n.line == h.line), next_goal) {
            (Some(n), Some(at)) => pen(n) - at,
            _ => 0.0,
        };
        Some((pen(h) - goal, next))
    };
    let placed = |m: (f64, f64)| m.0.abs() <= 1e-9 && m.1.abs() <= 1e-9;
    let mut k = if moved != 0.0 { (em_prev.map(|ep| moved / ep * 1000.0), Some(-moved / em * 1000.0)) } else { (None, None) };
    // A manual kerning value turns off the font's kerning for its character, each style run is
    // shaped apart, and centered, right-aligned and justified lines move as their width changes, so
    // the first values can miss. Layouts then measure what each kerning value does to the two
    // characters, and the values are solved for from that (at most six layouts).
    if !g.rtl && miss(k).is_some_and(|m| !placed(m)) {
        k = (pr.map(|_| k.0.unwrap_or(0.0)), Some(k.1.unwrap_or(0.0)));
        if let Some(mut m) = miss(k) {
            const STEP: f64 = 10.0;
            let column = |dk: (f64, f64)| miss((k.0.map(|v| v + dk.0), k.1.map(|v| v + dk.1))).map(|n| ((n.0 - m.0) / STEP, (n.1 - m.1) / STEP));
            let by_before = if k.0.is_some() { column((STEP, 0.0)) } else { None };
            let by_own = column((0.0, STEP));
            // The kerning changes that cancel the misses `m`.
            let solve = |m: (f64, f64)| match (by_before, by_own) {
                (Some(p), Some(q)) if (p.0 * q.1 - q.0 * p.1).abs() > 1e-12 => {
                    let det = p.0 * q.1 - q.0 * p.1;
                    ((q.0 * m.1 - m.0 * q.1) / det, (m.0 * p.1 - p.0 * m.1) / det)
                }
                (Some(p), _) if p.0.abs() > 1e-12 => (-m.0 / p.0, 0.0),
                (_, Some(q)) if q.1.abs() > 1e-12 => (0.0, -m.1 / q.1),
                _ => (0.0, 0.0),
            };
            for _ in 0..2 {
                if placed(m) {
                    break;
                }
                let (dp, dq) = solve(m);
                k = (k.0.map(|v| v + dp), k.1.map(|v| v + dq));
                match miss(k) {
                    Some(n) => m = n,
                    None => break,
                }
            }
        }
    }
    s.edit("Touch Type", |d, _| {
        let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
        touch(t, k.0, k.1, raise);
        refresh_bounds(t);
        Ok(())
    })?;
    Ok(json!({"id": id.0, "start": a, "end": b}))
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
    super::paint::check_keep_model(p, C)?;
    check_numbers(p, &["size"], C)?;
    check_switches(p, &["vertical", "placeholder"], C)?;
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
        ruby: Vec::new(),
        para: super::create::new_type_para(),
        paras: Vec::new(),
        first_para_continues: false,
        area: Default::default(),
        path_effect: Default::default(),
        path_align: Default::default(),
        path_spacing: 0.0,
        wrap: Vec::new(),
        optical_margin: false,
        system_layout: false,
        fractional_widths: true,
        list_count: 0,
        cached_bounds: None,
        cached_baselines: Vec::new(),
    };
    super::create::new_type_alignment(s, p, C, &mut t)?;
    if !on_path {
        t.area.fit = super::create::new_area_fit(s, p, C)?;
    }
    t.system_layout = s.doc()?.doc.setup.system_layout;
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
        n.appearance = Appearance::default().into();
        n.opacity = node.opacity;
        n.name = node.name.clone();
        d.insert(par, idx, n)?;
        sel.set([id]);
        Ok(id)
    })?;
    let made = super::typecmd::with_bounds(s, Some(id), json!({"id": id.0}));
    Ok(super::typecmd::with_overflow(s, &[id], made))
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

/// The text ranges a type shortcut changes: the range of `id`, else all the text of the type
/// objects in `ids`, else the text the Type tool has selected, else all the text of every selected
/// type object. At a caret, `at_caret` gives the range from the plain text and the caret's byte
/// (None: nothing changes there). Each range comes with whether it came from a caret.
fn type_ranges(
    s: &mut Session,
    p: &Value,
    c: &str,
    at_caret: impl Fn(&str, usize) -> Option<(usize, usize)>,
) -> Result<Vec<(NodeId, usize, usize, bool)>> {
    // (text, its range; None: all of it).
    let targets: Vec<(NodeId, Option<(usize, usize)>)> = match id_param(p, "id") {
        Some(id) => vec![(id, Some(range_of(p, edit::runs_len(&text_ref(s, id)?.runs), c)?))],
        None => {
            let editing = editing_range(s)?.filter(|_| p.get("ids").filter(|v| !v.is_null()).is_none());
            match editing {
                Some((id, a, b)) => vec![(id, Some((a.min(b), a.max(b))))],
                None => super::typecmd::text_targets(s, p, c)?.into_iter().map(|id| (id, None)).collect(),
            }
        }
    };
    let mut ranges = vec![];
    for (id, range) in targets {
        let text = text_ref(s, id)?.plain_text();
        let (a, b) = range.map_or((0, text.len()), |(a, b)| (a.min(text.len()), b.min(text.len())));
        let (a, b, caret) = if a < b {
            (a, b, false)
        } else {
            match at_caret(&text, a) {
                Some((a, b)) => (a, b, true),
                None => continue,
            }
        };
        if a < b {
            ranges.push((id, a, b, caret));
        }
    }
    Ok(ranges)
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
    let by = number_or(p, "by", 1.0, C)?;
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
    // The ranges stepped. At a caret: kerning is the space after the character before it, leading
    // the paragraph's; the rest wait for a selection.
    let ranges = type_ranges(s, p, C, |text, a| match step {
        Step::Kerning => Some((edit::prev_char(text, a), a)),
        Step::Leading => {
            let para = edit::paragraph_at(text, a);
            Some((para.start, para.end))
        }
        _ => None,
    })?;
    if !ranges.is_empty() {
        s.edit("Character", |d, _| {
            for &(id, a, b, _) in &ranges {
                let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
                let points = if matches!(step, Step::Size | Step::Leading | Step::BaselineShift) {
                    t.style_scale().ok_or_else(|| bad(C, "text transform is collapsed or unrepresentable"))?.points
                } else {
                    1.0
                };
                let mut runs = edit::slice_runs(&t.runs, a, b);
                // Auto leading steps from the leading its paragraph's Justification › Auto Leading
                // sets, so each run covers one paragraph and knows which.
                let mut paras = vec![];
                if matches!(step, Step::Leading) {
                    let plain = t.plain_text();
                    let mut para = plain.get(..a).map_or(0, |s| s.bytes().filter(|&c| c == b'\n').count());
                    let mut split = Vec::with_capacity(runs.len());
                    for run in runs {
                        if run.inline.is_some() || !run.text.contains('\n') {
                            paras.push(para);
                            para += run.text.bytes().filter(|&c| c == b'\n').count();
                            split.push(run);
                            continue;
                        }
                        for piece in run.text.split_inclusive('\n') {
                            paras.push(para);
                            para += usize::from(piece.ends_with('\n'));
                            split.push(TextRun::new(piece, run.style.clone()));
                        }
                    }
                    runs = split;
                }
                for (i, run) in runs.iter_mut().enumerate() {
                    let st = &mut run.style;
                    match step {
                        Step::Size => st.size = super::typecmd::local_type_value(t, st.size * points + inc, (0.1, 1296.0), false, C)?,
                        Step::Leading => {
                            let leading = t.para_at(paras.get(i).copied().unwrap_or(0)).effective_leading(st);
                            st.leading = Some(super::typecmd::local_type_value(t, leading * points + inc, (0.1, 5000.0), false, C)?);
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

/// Cmd/Ctrl+Alt+Q: tracking 0 and kerning Auto (0 in the Character panel) in the type objects in
/// `ids`, else in the Type tool's selected text, else in every selected type object. At a caret,
/// only the kerning of the character before it changes, as Alt+←/→ in horizontal type or Alt+↓/↑ in vertical type kerns there.
fn reset_tracking(s: &mut Session, p: &Value) -> Result<Value> {
    let ranges = type_ranges(s, p, "type.resetTracking", |text, a| Some((edit::prev_char(text, a), a)))?;
    if !ranges.is_empty() {
        s.edit("Reset Tracking and Kerning", |d, _| {
            for &(id, a, b, caret) in &ranges {
                let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
                edit::style_range(&mut t.runs, a, b, |st| {
                    st.kerning = None;
                    if !caret {
                        st.tracking = 0.0;
                    }
                });
                refresh_bounds(t);
            }
            Ok(())
        })?;
    }
    Ok(json!({ "ids": ranges.iter().map(|(id, ..)| id.0).collect::<Vec<_>>() }))
}

/// Cmd/Ctrl+Shift+X: horizontal scale 100% (the Character panel's value, which includes the
/// object's own transform) in the type objects in `ids`, else in the Type tool's selected text,
/// else in every selected type object. At a caret the command leaves the text as it is.
fn reset_horizontal_scale(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.resetHorizontalScale";
    let ranges = type_ranges(s, p, C, |_, _| None)?;
    if !ranges.is_empty() {
        s.edit("Reset Horizontal Scale", |d, _| {
            for &(id, a, b, _) in &ranges {
                let t = text_mut(d, id).ok_or(EngineError::NoNode(id))?;
                let h_scale = super::typecmd::local_type_value(t, 100.0, (1.0, 10000.0), true, C)?;
                edit::style_range(&mut t.runs, a, b, |st| st.h_scale = h_scale);
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
            // Ruby over the counted part; the layout clamps it to `head`.
            ruby: t.ruby.clone(),
            para: Default::default(),
            paras: Vec::new(),
            first_para_continues: false,
            area: Default::default(),
            path_effect: Default::default(),
            path_align: Default::default(),
            path_spacing: 0.0,
            wrap: Vec::new(),
            optical_margin: false,
            system_layout: t.system_layout,
            fractional_widths: t.fractional_widths,
            list_count: 0,
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
        && let Some(k) = st.history.undo.iter().rposition(|e| e.doc.node_uncached(id).is_none())
        && let Some(created) = st.history.undo.get(k)
    {
        let mut now = (*st.doc).clone();
        let mut before = (*created.doc).clone();
        before.alloc_id();
        if now.remove(id).is_ok() && now == before {
            st.doc = created.doc.clone();
            st.history.undo.truncate(k);
            st.selection.prune(&st.doc);
            super::globaledit::follow_history(st);
            st.revision += 1;
            return Ok(json!({"removed": true}));
        }
    }
    s.edit("Discard Empty Type", |d, _| d.remove(id).map(|_| ()).map_err(|_| EngineError::NoNode(id)))?;
    Ok(json!({"removed": true}))
}
