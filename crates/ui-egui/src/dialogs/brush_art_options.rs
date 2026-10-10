//! Scatter, Art and Pattern Brush Options: the [`super::brush_options`] of the brushes made of
//! art. Each has the brush's own values, Colorization (Method, and Key Color for Hue Shift, picked
//! from the art's colours) and a live preview of a stroke.
//!
//! Fields (besides `__brush`, `__type` and `name`): `__def` (the brush definition, for the preview),
//! `__colors` (the art's colours, most used first), `colorization` (`none` | `tints` |
//! `tintsAndShades` | `hueShift`), `keyColor` (a colour).
//! Scatter: per value (`size`, `spacing`, `scatter` %, `rotation` °) `…Min`, `…Max` and `…Mode`
//! (`fixed` | `random` | `pressure`), `rotationRelativeTo` (`page` | `path`).
//! Art: `width` (%), `scaleMode` (`proportional` | `stretch` | `betweenGuides`), `guideStart` and
//! `guideEnd` (% of the art's length), `direction` (`leftToRight` …), `flipAlong`, `flipAcross`.
//! Pattern: `scale`, `spacing` (%), `fit` (`stretch` | `addSpace` | `approximate`), `flipAlong`,
//! `flipAcross`, `__tiles` (slot → preview key, for the tiles the brush has).

use std::hash::{Hash, Hasher};

use egui::{Color32, Sense, Stroke, pos2, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_brush::{ArtBrush, ArtScale, Brush, BrushKind, Colorization, Direction, PatternBrush, PatternFit, Scatter, Variation};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Document, Node};

use super::brush_options::{clamped, mode_key, stroke_preview};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// Scatter values: (field, suffix), in [`Scatter::RANGES`] order.
const SCATTER: [(&str, &str); 4] = [("size", "%"), ("spacing", "%"), ("scatter", "%"), ("rotation", "°")];

/// Art brush directions, in the order of their arrows (→ ← ↓ ↑).
const DIRECTIONS: [Direction; 4] = [Direction::LeftToRight, Direction::RightToLeft, Direction::TopToBottom, Direction::BottomToTop];

/// Pattern brush tiles: (field of the definition, slot), in the dialog's order.
const TILES: [(&str, &str); 5] =
    [("outer_corner", "outerCorner"), ("side", "side"), ("inner_corner", "innerCorner"), ("start", "start"), ("end", "end")];

/// The most key colours offered.
const MAX_COLORS: usize = 16;

/// Width of the label column.
const LABEL_W: f32 = 132.0;

/// Width of the stroke preview.
const PREVIEW_W: f32 = 420.0;

pub(super) fn heading(ty: &str) -> &'static str {
    match ty {
        "scatter" => tl!("Scatter Brush Options"),
        "art" => tl!("Art Brush Options"),
        _ => tl!("Pattern Brush Options"),
    }
}

fn min_key(key: &str) -> String {
    format!("{key}Min")
}

fn max_key(key: &str) -> String {
    format!("{key}Max")
}

/// A definition id of a serde enum (`leftToRight`, `addSpace` …).
fn id_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The value of the serde enum `T` that field `key` names, else `T`'s default.
fn parse_field<T: serde::de::DeserializeOwned + Default>(d: &Dialog, key: &str) -> T {
    serde_json::from_value(json!(d.str(key))).unwrap_or_default()
}

// ---------- fields ----------

/// The fields of brush `b` (a scatter, art or pattern brush) whose definition is `def`.
pub(super) fn fields(b: &Brush, def: Value, f: &mut Map<String, Value>) {
    match &b.kind {
        BrushKind::Scatter(s) => {
            for ((key, _), ((lo, hi), mode)) in SCATTER.iter().zip(s.values().into_iter().zip(s.modes())) {
                f.insert(min_key(key), json!(lo));
                f.insert(max_key(key), json!(hi));
                f.insert(mode_key(key), json!(mode.id()));
            }
            f.insert("rotationRelativeTo".into(), json!(if s.rotation_relative_to_path { "path" } else { "page" }));
        }
        BrushKind::Art(a) => {
            let (mode, start, end) = match a.scale {
                ArtScale::Proportional => ("proportional", 0.0, 1.0),
                ArtScale::Stretch => ("stretch", 0.0, 1.0),
                ArtScale::BetweenGuides { start, end } => ("betweenGuides", start, end),
            };
            f.insert("width".into(), json!(a.width));
            f.insert("scaleMode".into(), json!(mode));
            f.insert("guideStart".into(), json!(start * 100.0));
            f.insert("guideEnd".into(), json!(end * 100.0));
            f.insert("direction".into(), json!(id_of(&a.direction)));
            f.insert("flipAlong".into(), json!(a.flip_along));
            f.insert("flipAcross".into(), json!(a.flip_across));
        }
        BrushKind::Pattern(p) => {
            f.insert("scale".into(), json!(p.scale));
            f.insert("spacing".into(), json!(p.spacing));
            f.insert("fit".into(), json!(id_of(&p.fit)));
            f.insert("flipAlong".into(), json!(p.flip_along));
            f.insert("flipAcross".into(), json!(p.flip_across));
            // A preview key per tile the brush has: its art's hash, so a changed tile redraws.
            let tiles: Map<String, Value> = TILES
                .iter()
                .filter_map(|(field, slot)| {
                    let art = def.get(*field).filter(|v| !v.is_null())?;
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    art.to_string().hash(&mut h);
                    Some(((*slot).to_string(), json!(format!("brush-tile:{:x}", h.finish()))))
                })
                .collect();
            f.insert("__tiles".into(), Value::Object(tiles));
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => {}
    }
    let colors = vectorcraft_brush::art_colors(b.art());
    let colorization = b.kind.colorization();
    let key = match colorization {
        Colorization::HueShift { key } => key,
        _ => colors.first().copied().unwrap_or(Color::BLACK),
    };
    f.insert("colorization".into(), json!(colorization.id()));
    f.insert("keyColor".into(), serde_json::to_value(key).unwrap_or_default());
    f.insert("__colors".into(), serde_json::to_value(colors.into_iter().take(MAX_COLORS).collect::<Vec<_>>()).unwrap_or_default());
    f.insert("__def".into(), def);
}

fn key_color(d: &Dialog) -> Color {
    d.fields.get("keyColor").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(Color::BLACK)
}

fn colorization(d: &Dialog) -> Colorization {
    Colorization::parse(&d.str("colorization"), key_color(d)).unwrap_or_default()
}

/// How scatter value `key` varies.
fn scatter_mode(d: &Dialog, key: &str) -> Variation {
    Variation::parse(&d.str(&mode_key(key))).unwrap_or_default()
}

/// `brush.options` params from the fields.
pub(super) fn params(d: &Dialog) -> Value {
    let mut p = match d.str("__type").as_str() {
        "scatter" => {
            let mut p = json!({ "rotation_relative_to_path": d.str("rotationRelativeTo") == "path" });
            for ((key, _), range) in SCATTER.iter().zip(Scatter::RANGES) {
                p[*key] = json!([clamped(d, &min_key(key), range), clamped(d, &max_key(key), range)]);
            }
            p["modes"] = json!(SCATTER.map(|(key, _)| scatter_mode(d, key).id()));
            p
        }
        "art" => {
            let scale = match d.str("scaleMode").as_str() {
                "proportional" => ArtScale::Proportional,
                "betweenGuides" => ArtScale::BetweenGuides {
                    start: clamped(d, "guideStart", (0.0, 100.0)) / 100.0,
                    end: clamped(d, "guideEnd", (0.0, 100.0)) / 100.0,
                },
                _ => ArtScale::Stretch,
            };
            json!({
                "width": clamped(d, "width", ArtBrush::WIDTH_RANGE),
                "scale": serde_json::to_value(scale).unwrap_or_default(),
                "direction": id_of(&parse_field::<Direction>(d, "direction")),
                "flip_along": d.bool("flipAlong"),
                "flip_across": d.bool("flipAcross"),
            })
        }
        _ => json!({
            "scale": clamped(d, "scale", PatternBrush::SCALE_RANGE),
            "spacing": clamped(d, "spacing", PatternBrush::SPACING_RANGE),
            "fit": id_of(&parse_field::<PatternFit>(d, "fit")),
            "flip_along": d.bool("flipAlong"),
            "flip_across": d.bool("flipAcross"),
        }),
    };
    p["colorization"] = serde_json::to_value(colorization(d)).unwrap_or_default();
    p
}

// ---------- body ----------

pub(super) fn body(ui: &mut egui::Ui, d: &mut Dialog) {
    match d.str("__type").as_str() {
        "scatter" => scatter_body(ui, d),
        "art" => art_body(ui, d),
        _ => pattern_body(ui, d),
    }
    ui.add_space(6.0);
    colorization_body(ui, d);
    ui.add_space(10.0);
    // The brush as OK would make it, on a sample stroke.
    let mut def = d.fields.get("__def").cloned().unwrap_or_default();
    if let (Some(o), Value::Object(p)) = (def.as_object_mut(), params(d)) {
        o.extend(p);
    }
    let ty = d.str("__type");
    stroke_preview(ui, &ty, def, vec2(PREVIEW_W, 64.0));
}

/// A whole-number field (`suffix`: % or °) bound to `d.fields[key]`, kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, range: (f64, f64), suffix: &str, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(v) = widgets::range_field(ui, ("brush-art-value", key), clamped(d, key, range), range.0..=range.1, suffix, 0, 72.0) {
            d.fields.insert(key.into(), json!(v));
        }
    });
}

/// A row of radio buttons bound to the string field `key`: (value, label).
fn radios(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(&str, &str)]) {
    let cur = d.str(key);
    for (value, label) in options {
        if widgets::radio(ui, label, cur == *value, true) {
            d.fields.insert(key.into(), json!(value));
        }
    }
}

fn flips(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::label_row(ui, tl!("Flip:"), LABEL_W, |ui| {
        super::form::check(ui, d, "flipAlong", tl!("Flip Along"));
        ui.add_space(12.0);
        super::form::check(ui, d, "flipAcross", tl!("Flip Across"));
    });
}

fn scatter_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let labels = [tl!("Size:"), tl!("Spacing:"), tl!("Scatter:"), tl!("Rotation:")];
    let modes = Variation::ALL.map(Variation::label);
    egui::Grid::new("brush-options-scatter").num_columns(4).spacing([8.0, 8.0]).show(ui, |ui| {
        for (((key, suffix), range), text) in SCATTER.into_iter().zip(Scatter::RANGES).zip(labels) {
            super::swatch_options::label(ui, text);
            let mode = scatter_mode(d, key);
            for (field, enabled) in [(min_key(key), true), (max_key(key), mode != Variation::Fixed)] {
                ui.add_enabled_ui(enabled, |ui| {
                    if let Some(v) =
                        widgets::range_field(ui, ("brush-scatter", field.as_str()), clamped(d, &field, range), range.0..=range.1, suffix, 0, 72.0)
                    {
                        d.fields.insert(field.clone(), json!(v));
                    }
                });
            }
            if let Some(m) = widgets::dropdown(ui, ("brush-scatter-mode", key), mode.label(), &modes, 104.0).and_then(|i| Variation::ALL.get(i)) {
                d.fields.insert(mode_key(key), json!(m.id()));
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    let relative = [("page", tl!("Page")), ("path", tl!("Path"))];
    super::form::choice(ui, d, "rotationRelativeTo", tl!("Rotation relative to:"), (LABEL_W, 104.0), &relative);
}

fn art_body(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::label_row(ui, tl!("Width:"), LABEL_W, |ui| number(ui, d, "width", ArtBrush::WIDTH_RANGE, "%", true));
    ui.add_space(4.0);
    widgets::section_header(ui, tl!("Brush Scale Options"));
    let scales = [
        ("proportional", tl!("Scale Proportionately")),
        ("stretch", tl!("Stretch to Fit Stroke Length")),
        ("betweenGuides", tl!("Stretch Between Guides")),
    ];
    radios(ui, d, "scaleMode", &scales);
    let guides = d.str("scaleMode") == "betweenGuides";
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        widgets::label_row(ui, tl!("Start:"), 40.0, |ui| number(ui, d, "guideStart", (0.0, 100.0), "%", guides));
        ui.add_space(8.0);
        widgets::label_row(ui, tl!("End:"), 34.0, |ui| number(ui, d, "guideEnd", (0.0, 100.0), "%", guides));
    });
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Direction:"), LABEL_W, |ui| {
        let tips = [tl!("Left to Right"), tl!("Right to Left"), tl!("Top to Bottom"), tl!("Bottom to Top")];
        let cur = parse_field::<Direction>(d, "direction");
        for (dir, tip) in DIRECTIONS.into_iter().zip(tips) {
            if arrow_button(ui, dir, dir == cur, tip) {
                d.fields.insert("direction".into(), json!(id_of(&dir)));
            }
        }
    });
    flips(ui, d);
}

/// A toggle button showing an arrow pointing the way `dir` runs. Whether it was clicked.
fn arrow_button(ui: &mut egui::Ui, dir: Direction, selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(r, 3.0, t.row_selected);
    } else if resp.hovered() {
        p.rect_filled(r, 3.0, t.hover);
    }
    p.rect_stroke(r, 3.0, Stroke::new(1.0, if selected { t.accent } else { t.input_border }), egui::StrokeKind::Inside);
    let u = match dir {
        Direction::LeftToRight => vec2(1.0, 0.0),
        Direction::RightToLeft => vec2(-1.0, 0.0),
        Direction::TopToBottom => vec2(0.0, 1.0),
        Direction::BottomToTop => vec2(0.0, -1.0),
    };
    let (c, n) = (r.center(), vec2(-u.y, u.x));
    let (tail, tip_at) = (c - u * 7.0, c + u * 7.0);
    let stroke = Stroke::new(1.5, if selected { t.text_strong } else { t.text });
    p.line_segment([tail, tip_at], stroke);
    p.line_segment([tip_at, tip_at - u * 4.0 + n * 4.0], stroke);
    p.line_segment([tip_at, tip_at - u * 4.0 - n * 4.0], stroke);
    resp.on_hover_text(tip).clicked()
}

fn pattern_body(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::label_row(ui, tl!("Scale:"), LABEL_W, |ui| number(ui, d, "scale", PatternBrush::SCALE_RANGE, "%", true));
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| number(ui, d, "spacing", PatternBrush::SPACING_RANGE, "%", true));
    ui.add_space(6.0);
    tiles(ui, d);
    ui.add_space(6.0);
    flips(ui, d);
    widgets::section_header(ui, tl!("Fit"));
    let fits = [("stretch", tl!("Stretch to fit")), ("addSpace", tl!("Add space to fit")), ("approximate", tl!("Approximate path"))];
    radios(ui, d, "fit", &fits);
}

/// The pattern's five tiles, each with its art (or None when the brush has no such tile).
fn tiles(ui: &mut egui::Ui, d: &Dialog) {
    let t = Tokens::get(ui.ctx());
    let names = [tl!("Outer Corner Tile"), tl!("Side Tile"), tl!("Inner Corner Tile"), tl!("Start Tile"), tl!("End Tile")];
    let keys = d.fields.get("__tiles");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for ((field, slot), name) in TILES.into_iter().zip(names) {
            ui.vertical(|ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(76.0, 44.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, Color32::WHITE);
                let key = keys.and_then(|k| k.get(slot)).and_then(Value::as_str);
                let tex = key.and_then(|key| {
                    widgets::doc_preview(ui, key, r.size(), |w, h| {
                        let art: Node = serde_json::from_value(d.fields.get("__def")?.get(field)?.clone()).ok()?;
                        tile_doc(&art, w, h)
                    })
                });
                match tex {
                    Some(tex) => {
                        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            tl!("None"),
                            egui::FontId::proportional(11.5),
                            Color32::from_gray(140),
                        );
                    }
                }
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
                resp.on_hover_text(name);
                ui.add_sized(vec2(76.0, 14.0), egui::Label::new(egui::RichText::new(name).size(10.5).color(t.text_dim)).truncate());
            });
        }
    });
}

/// A document of `w` × `h` showing `art` scaled to fit, centred.
fn tile_doc(art: &Node, w: f64, h: f64) -> Option<Document> {
    let b = art.visual_bounds().or_else(|| art.geometric_bounds())?;
    let k = ((w - 8.0) / b.width().max(1e-6)).min((h - 8.0) / b.height().max(1e-6));
    if !k.is_finite() || k <= 0.0 {
        return None;
    }
    let mut doc = Document::new(w, h);
    let mut n = doc.reid(art);
    let xf = vectorcraft_geom::Affine::translate((w / 2.0, h / 2.0))
        * vectorcraft_geom::Affine::scale(k)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    n.transform(xf, true);
    let l = doc.layers.first()?.id;
    doc.insert(Some(l), 0, n).ok()?;
    Some(doc)
}

fn colorization_body(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::section_header(ui, tl!("Colorization"));
    let methods = [tl!("None"), tl!("Tints"), tl!("Tints and Shades"), tl!("Hue Shift")];
    let cur = d.str("colorization");
    let at = Colorization::IDS.iter().position(|id| *id == cur).unwrap_or(0);
    widgets::label_row(ui, tl!("Method:"), LABEL_W, |ui| {
        if let Some(id) = widgets::dropdown_names(ui, "brush-colorization", methods.get(at).copied().unwrap_or_default(), &methods, 150.0)
            .and_then(|i| Colorization::IDS.get(i))
        {
            d.fields.insert("colorization".into(), json!(id));
        }
    });
    // Hue Shift's key colour: the art colour that becomes the stroke colour.
    let colors: Vec<Color> = d.fields.get("__colors").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let hue_shift = cur == "hueShift";
    let key = key_color(d);
    widgets::label_row(ui, tl!("Key Color:"), LABEL_W, |ui| {
        ui.add_enabled_ui(hue_shift, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for c in colors {
                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                widgets::swatch_tile(ui, r, &Paint::solid(c), hue_shift && c == key, resp.hovered());
                if resp.on_hover_text(c.to_hex()).clicked() {
                    d.fields.insert("keyColor".into(), serde_json::to_value(c).unwrap_or_default());
                }
            }
        });
    });
}
