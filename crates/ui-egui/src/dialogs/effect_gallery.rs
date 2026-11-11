//! Effect → Effect Gallery: browse Photoshop-style raster filters by category, build a stack, and
//! apply it as separate appearance effects in one undo step.
//!
//! Each filter has a thumbnail, and the dialog has its own preview, as the
//! [Filter Gallery](https://helpx.adobe.com/photoshop/desktop/effects-filters/get-started-with-filters/filter-gallery.html)
//! describes. Both are an original drawing made for VectorCraft (a small landscape), passed through
//! the filter. The thumbnails use each filter's defaults. The preview uses the visible stack, or
//! the filter being browsed when the stack is empty. The canvas preview and the one-undo OK are
//! unchanged.

use std::collections::HashMap;

use egui;
use serde_json::{Map, Value, json};
use vectorcraft_effects::{EffectInfo, PIXEL_EFFECTS, PixelSpace};
use vectorcraft_geom::{Affine, Point};

use super::{DialogSpec, effect, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// Thumbnail of one filter, in pixels. The drawing is original; the filter is this app's.
const THUMB: usize = 36;
/// In-dialog preview, in pixels, drawn at twice this size.
const PREVIEW_W: usize = 120;
const PREVIEW_H: usize = 84;

/// Dialog kind for the Effect Gallery.
pub const KIND: &str = "effectGallery";

/// Raster effect submenus in the gallery (the six Photoshop-style folders it holds).
const CATEGORIES: [(&str, &[&str]); 6] = [
    ("Artistic", &["Effect", "Artistic"]),
    ("Brush Strokes", &["Effect", "Brush Strokes"]),
    ("Distort", &["Effect", "Distort"]),
    ("Sketch", &["Effect", "Sketch"]),
    ("Stylize", &["Effect", "Stylize"]),
    ("Texture", &["Effect", "Texture"]),
];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Effect Gallery").into(),
    body,
    confirm,
    preview: true,
    min_width: 720.0,
    max_width: Some(820.0),
    inputs: |_| &["browseEffect", "browseParams"],
    ..DialogSpec::FORM
};

#[derive(Clone)]
struct StackEntry {
    effect: String,
    visible: bool,
    params: Map<String, Value>,
}

/// Open the gallery on the current selection.
pub fn open(app: &mut VectorcraftApp, _p: &Value) -> Result<Value, String> {
    if app.session.active().is_none_or(|s| s.selection.is_empty()) {
        return Err(tl!("Select artwork to apply effects").into());
    }
    let selected = effects_in_category(0).first().map(|e| e.id).unwrap_or("");
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({ "category": 0, "selected": selected, "stack": [], "stackSelected": 0, "stackFocus": false, "preview": true }),
    ));
    Ok(Value::Null)
}

fn effects_in_category(cat: usize) -> Vec<EffectInfo> {
    let path = CATEGORIES.get(cat).map(|(_, p)| *p).unwrap_or(CATEGORIES[0].1);
    let mut out: Vec<EffectInfo> =
        vectorcraft_effects::effect_catalog().into_iter().filter(|e| e.menu == path && PIXEL_EFFECTS.contains(&e.id)).collect();
    out.sort_by(|a, b| a.label.cmp(b.label));
    out
}

fn default_params(id: &str) -> Map<String, Value> {
    match vectorcraft_effects::effect_info(id).map(|e| e.defaults) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn merged_params(id: &str, stored: &Map<String, Value>) -> Map<String, Value> {
    match vectorcraft_effects::merged_params(id, &Value::Object(stored.clone())) {
        Value::Object(m) => m,
        _ => default_params(id),
    }
}

fn stack_of(d: &Dialog) -> Vec<StackEntry> {
    d.fields
        .get("stack")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let effect = row.get("effect").and_then(Value::as_str)?;
                    let visible = row.get("visible").and_then(Value::as_bool).unwrap_or(true);
                    let params = row
                        .get("params")
                        .and_then(Value::as_object)
                        .cloned()
                        .map(|m| merged_params(effect, &m))
                        .unwrap_or_else(|| default_params(effect));
                    Some(StackEntry { effect: effect.to_string(), visible, params })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn write_stack(d: &mut Dialog, stack: &[StackEntry]) {
    let rows: Vec<Value> = stack.iter().map(|e| json!({ "effect": e.effect, "visible": e.visible, "params": e.params })).collect();
    d.fields.insert("stack".into(), Value::Array(rows));
}

fn category_ix(d: &Dialog) -> usize {
    d.fields.get("category").and_then(Value::as_u64).unwrap_or(0) as usize
}

fn selected_id(d: &Dialog) -> String {
    d.str("selected")
}

fn set_selected(d: &mut Dialog, id: &str) {
    d.fields.insert("selected".into(), json!(id));
}

fn stack_focus(d: &Dialog) -> bool {
    d.fields.get("stackFocus").and_then(Value::as_bool).unwrap_or(false)
}

fn set_stack_focus(d: &mut Dialog, on: bool) {
    d.fields.insert("stackFocus".into(), json!(on));
}

fn browse_params(d: &Dialog) -> Map<String, Value> {
    let effect = d.fields.get("browseEffect").and_then(Value::as_str).unwrap_or("");
    let stored = d.fields.get("browseParams").and_then(Value::as_object).cloned().unwrap_or_default();
    if effect.is_empty() { stored } else { merged_params(effect, &stored) }
}

fn set_browse_params(d: &mut Dialog, effect: &str, params: Map<String, Value>) {
    d.fields.insert("browseEffect".into(), json!(effect));
    d.fields.insert("browseParams".into(), Value::Object(params));
}

/// The effect whose parameters the panel edits, and its values.
fn param_target(d: &Dialog, stack: &[StackEntry], stack_sel: usize, browse: &str) -> (String, Map<String, Value>) {
    if stack_focus(d) {
        let entry = stack.get(stack_sel);
        if let Some(e) = entry {
            return (e.effect.clone(), e.params.clone());
        }
    }
    let id = browse.to_string();
    let params = if d.fields.get("browseEffect").and_then(Value::as_str) == Some(browse) { browse_params(d) } else { default_params(browse) };
    (id, params)
}

fn write_param_target(d: &mut Dialog, stack: &mut [StackEntry], stack_sel: usize, browse: &str, effect: &str, params: Map<String, Value>) {
    if stack_focus(d) {
        if let Some(e) = stack.get_mut(stack_sel)
            && e.effect == effect
        {
            e.params = params;
        }
        return;
    }
    if browse == effect {
        set_browse_params(d, effect, params);
    }
}

/// Reuse the live effect dialog's fields for `effect_id` and `params`.
fn effect_param_fields(app: &mut VectorcraftApp, ui: &mut egui::Ui, effect_id: &str, params: &mut Map<String, Value>) {
    if effect_id.is_empty() {
        return;
    }
    let label =
        vectorcraft_effects::effect_info(effect_id).map(|e| e.label.trim_end_matches('…').to_string()).unwrap_or_else(|| effect_id.to_string());
    let mut fields = params.clone();
    fields.insert("__effect".into(), json!(effect_id));
    fields.insert("__label".into(), json!(label));
    fields.insert("preview".into(), json!(false));
    let mut temp = Dialog::new("effect", Value::Object(fields));
    (effect::SPEC.body)(app, ui, &mut temp);
    if let Value::Object(m) = form::params(&temp) {
        *params = m;
    }
    if let Value::Object(m) = vectorcraft_effects::upgrade_params(effect_id, &Value::Object(params.clone())) {
        *params = m;
    }
}

fn stack_batch_commands(stack: &[StackEntry]) -> Vec<Value> {
    let mut commands = Vec::with_capacity(stack.len() * 2);
    for (i, entry) in stack.iter().enumerate() {
        let params = Value::Object(entry.params.clone());
        commands.push(json!({ "command": "effect.apply", "params": { "effect": entry.effect, "params": params } }));
        if !entry.visible {
            commands.push(json!({ "command": "effect.setParams", "params": { "index": i, "visible": false } }));
        }
    }
    commands
}

fn stack_batch_params(stack: &[StackEntry]) -> Value {
    json!({ "label": "Effect Gallery", "commands": stack_batch_commands(stack) })
}

/// Live canvas preview of the stack (`command.batch` would settle the dialog interaction each frame).
fn preview_stack_on_canvas(app: &mut VectorcraftApp, stack: &[StackEntry]) {
    if app.session.in_interaction() {
        let _ = app.session.cancel_interaction();
    }
    let _ = app.session.begin_interaction("Effect Gallery");
    for (i, entry) in stack.iter().enumerate() {
        let params = Value::Object(entry.params.clone());
        if app.run("effect.apply", json!({ "effect": entry.effect, "params": params })).is_err() {
            let _ = app.session.cancel_interaction();
            return;
        }
        if !entry.visible && app.run("effect.setParams", json!({ "index": i, "visible": false })).is_err() {
            let _ = app.session.cancel_interaction();
            return;
        }
    }
}

fn stack_preview_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, stack: &[StackEntry]) {
    const LAST: &str = super::form::PREVIEWED;
    ui.add_space(8.0);
    let mut on = d.bool("preview");
    if widgets::check(ui, tl!("Preview"), on, true) {
        on = !on;
        d.fields.insert("preview".into(), json!(on));
    }
    if !on {
        if d.fields.remove(LAST).is_some() {
            let _ = app.session.cancel_interaction();
        }
        return;
    }
    if stack.is_empty() {
        if d.fields.remove(LAST).is_some() {
            let _ = app.session.cancel_interaction();
        }
        return;
    }
    let batch = stack_batch_params(stack);
    if d.fields.get(LAST) != Some(&batch) || !app.session.in_interaction() {
        preview_stack_on_canvas(app, stack);
        d.fields.insert(LAST.into(), batch);
    }
}

fn effect_label(id: &str) -> String {
    vectorcraft_effects::effect_info(id).map(|e| tl!(e.label).trim_end_matches('…').to_string()).unwrap_or_else(|| id.to_string())
}

/// Where the preview rasters sit: one document point per pixel, origin at the top-left.
fn preview_space(w: usize, h: usize) -> PixelSpace {
    let (wf, hf) = (w as f64, h as f64);
    PixelSpace {
        to_doc: Affine::IDENTITY,
        px: 1.0,
        center: Point::new(wf / 2.0, hf / 2.0),
        channels: vectorcraft_effects::pixel::Channels::Rgb,
        line: 1.0,
        extent: 0.5 * wf.hypot(hf),
    }
}

fn byte(v: f32) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    v.round().clamp(0.0, 255.0) as u8
}

/// An original landscape for the gallery (sky, sun, hills, a house, a river). Not a copied picture.
fn sample_drawing(w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
    if w == 0 || h == 0 || out.len() != w * h * 4 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let xf = x as f32 / w as f32;
            let yf = y as f32 / h as f32;
            let mut rgb = [90.0 + 90.0 * (1.0 - yf), 150.0 + 50.0 * (1.0 - yf), 200.0 + 40.0 * yf];
            let (dx, dy) = (xf - 0.78, yf - 0.22);
            if dx * dx + dy * dy < 0.012 {
                rgb = [252.0, 206.0, 64.0];
            }
            let hill = 0.58 + 0.08 * (xf * std::f32::consts::TAU).sin();
            if yf > hill {
                rgb = [46.0, 110.0 + 40.0 * (1.0 - (yf - hill).clamp(0.0, 1.0)), 52.0];
            }
            if (0.16..0.40).contains(&xf) && (0.46..0.70).contains(&yf) {
                rgb = [196.0, 64.0, 48.0];
            }
            let roof = 0.46 - (xf - 0.28).abs() * 1.15;
            if (0.12..0.44).contains(&xf) && yf > roof && yf < 0.48 {
                rgb = [92.0, 48.0, 36.0];
            }
            if (0.22..0.32).contains(&xf) && (0.52..0.62).contains(&yf) {
                rgb = [255.0, 236.0, 150.0];
            }
            if yf > 0.80 && (yf * 18.0 + xf * 9.0).sin() > -0.15 {
                rgb = [36.0, 78.0, 196.0];
            }
            let i = (y * w + x) * 4;
            out[i] = byte(rgb[0]);
            out[i + 1] = byte(rgb[1]);
            out[i + 2] = byte(rgb[2]);
            out[i + 3] = 255;
        }
    }
    out
}

/// Run one Photoshop-style filter on a premultiplied RGBA buffer. Leaves it unchanged when `id`
/// is not one of those filters or the buffer is the wrong size.
fn apply_pixel(id: &str, params: &Map<String, Value>, data: &mut [u8], w: usize, h: usize) {
    let Some(n) = w.checked_mul(h).and_then(|n| n.checked_mul(4)) else { return };
    if n != data.len() || w == 0 || h == 0 {
        return;
    }
    let effect = vectorcraft_doc::Effect { id: id.to_string(), params: Value::Object(params.clone()), visible: true };
    let Some(vectorcraft_effects::RasterFx::Pixel(fx)) = vectorcraft_effects::raster_effects(std::slice::from_ref(&effect)).into_iter().next() else {
        return;
    };
    fx.apply(data, w, h, &preview_space(w, h));
}

fn filtered(id: &str, params: &Map<String, Value>, w: usize, h: usize) -> Vec<u8> {
    let mut data = sample_drawing(w, h);
    apply_pixel(id, params, &mut data, w, h);
    data
}

/// The drawing with `jobs` applied in order (the visible stack, or the filter being browsed).
fn preview_rgba(jobs: &[(String, Map<String, Value>)], w: usize, h: usize) -> Vec<u8> {
    let mut data = sample_drawing(w, h);
    for (id, params) in jobs {
        apply_pixel(id, params, &mut data, w, h);
    }
    data
}

fn thumb_pixels(id: &str) -> Vec<u8> {
    static CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Vec<u8>>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
    if let Some(px) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() {
        return px;
    }
    // The filter runs outside the lock: a thumbnail is a few thousand pixels, and two threads
    // computing the same one just keep one copy.
    let px = filtered(id, &default_params(id), THUMB, THUMB);
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), px.clone());
    px
}

/// Straight RGBA for egui, from the filters' premultiplied bytes.
fn color_image(data: &[u8], w: usize, h: usize) -> egui::ColorImage {
    let mut pixels = Vec::with_capacity(w.saturating_mul(h));
    let (rows, _) = data.as_chunks::<4>();
    for p in rows {
        let a = u32::from(p[3]);
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else if a == 255 {
            (p[0], p[1], p[2])
        } else {
            let s = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
            (s(p[0]), s(p[1]), s(p[2]))
        };
        pixels.push(egui::Color32::from_rgba_unmultiplied(r, g, b, p[3]));
    }
    if pixels.len() != w.saturating_mul(h) {
        return egui::ColorImage::filled([w.max(1), h.max(1)], egui::Color32::TRANSPARENT);
    }
    egui::ColorImage::new([w, h], pixels)
}

fn thumb_tex(ctx: &egui::Context, id: &str) -> egui::TextureHandle {
    let key = egui::Id::unique("effect-gallery-thumbs");
    if let Some(tex) = ctx.data(|d| d.get_temp::<HashMap<String, egui::TextureHandle>>(key).and_then(|m| m.get(id).cloned())) {
        return tex;
    }
    let tex = ctx.load_texture(format!("effect-gallery-thumb-{id}"), color_image(&thumb_pixels(id), THUMB, THUMB), egui::TextureOptions::NEAREST);
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(key, HashMap::new).insert(id.to_string(), tex.clone());
    });
    tex
}

/// What the in-dialog preview shows: the visible stack, or the browsed filter when nothing is stacked.
fn preview_jobs(d: &Dialog, stack: &[StackEntry], browse: &str) -> Vec<(String, Map<String, Value>)> {
    if !stack.is_empty() {
        return stack.iter().filter(|e| e.visible).map(|e| (e.effect.clone(), e.params.clone())).collect();
    }
    if browse.is_empty() {
        return Vec::new();
    }
    let params = if d.fields.get("browseEffect").and_then(Value::as_str) == Some(browse) { browse_params(d) } else { default_params(browse) };
    vec![(browse.to_string(), params)]
}

fn preview_key(jobs: &[(String, Map<String, Value>)]) -> String {
    let rows: Vec<Value> = jobs.iter().map(|(id, params)| json!({ "effect": id, "params": params })).collect();
    serde_json::to_string(&Value::Array(rows)).unwrap_or_default()
}

fn preview_tex(ctx: &egui::Context, key: &str, rgba: &[u8]) -> egui::TextureHandle {
    let id = egui::Id::unique("effect-gallery-preview");
    if let Some(tex) = ctx.data(|d| d.get_temp::<(String, egui::TextureHandle)>(id).filter(|(k, _)| k == key).map(|(_, tex)| tex)) {
        return tex;
    }
    let tex = ctx.load_texture("effect-gallery-preview", color_image(rgba, PREVIEW_W, PREVIEW_H), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (key.to_string(), tex.clone())));
    tex
}

fn preview_area(ui: &mut egui::Ui, d: &Dialog, stack: &[StackEntry], browse: &str) {
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Preview"));
    let jobs = preview_jobs(d, stack, browse);
    let key = preview_key(&jobs);
    let rgba = preview_rgba(&jobs, PREVIEW_W, PREVIEW_H);
    let tex = preview_tex(ui.ctx(), &key, &rgba);
    #[cfg(test)]
    PREVIEWS_PAINTED.with(|c| c.set(c.get() + 1));
    let (img_w, img_h) = (PREVIEW_W as f32 * 2.0, PREVIEW_H as f32 * 2.0);
    ui.horizontal(|ui| {
        let spare = (ui.available_width() - img_w).max(0.0);
        ui.add_space(spare / 2.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(img_w, img_h), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, t.panel_darker);
        let inner = rect.shrink(4.0);
        ui.painter().image(tex.id(), inner, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    });
    let caption = jobs.iter().map(|(id, _)| effect_label(id)).collect::<Vec<_>>().join(", ");
    if !caption.is_empty() {
        ui.label(egui::RichText::new(caption).color(t.text_dim));
    }
    ui.add_space(8.0);
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let cat = category_ix(d);
    let effects = effects_in_category(cat);
    let mut sel = selected_id(d);
    if !effects.iter().any(|e| e.id == sel) {
        sel = effects.first().map(|e| e.id.to_string()).unwrap_or_default();
        set_selected(d, &sel);
    }

    let mut stack = stack_of(d);
    let mut stack_sel = d.fields.get("stackSelected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if stack_sel >= stack.len() && !stack.is_empty() {
        stack_sel = stack.len() - 1;
    }
    preview_area(ui, d, &stack, &sel);

    let (param_effect, mut param_values) = param_target(d, &stack, stack_sel, &sel);
    if !param_effect.is_empty() {
        widgets::subheader(ui, tl!("Settings"));
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            effect_param_fields(app, ui, &param_effect, &mut param_values);
        });
        write_param_target(d, &mut stack, stack_sel, &sel, &param_effect, param_values);
    }

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Categories"));
            ui.set_width(120.0);
            for (i, (name, _)) in CATEGORIES.iter().enumerate() {
                let count = effects_in_category(i).len();
                if count == 0 {
                    continue;
                }
                let label = tl!(name);
                if ui.add(egui::Button::selectable(cat == i, label)).clicked() {
                    d.fields.insert("category".into(), json!(i));
                    let first = effects_in_category(i).first().map(|e| e.id).unwrap_or("");
                    set_selected(d, first);
                    set_stack_focus(d, false);
                }
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Filters"));
            ui.set_width(248.0);
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for e in &effects {
                    let picked = e.id == sel;
                    let name = tl!(e.label).trim_end_matches('…').to_string();
                    let image = egui::Image::from_texture(&thumb_tex(ui.ctx(), e.id)).fit_to_exact_size(egui::vec2(THUMB as f32, THUMB as f32));
                    #[cfg(test)]
                    THUMBS_PAINTED.with(|c| c.set(c.get() + 1));
                    let resp = ui.add(egui::Button::selectable(picked, (image, name)).min_size(egui::vec2(200.0, THUMB as f32 + 4.0)));
                    if resp.clicked() {
                        set_selected(d, e.id);
                        set_stack_focus(d, false);
                    }
                    if resp.double_clicked() {
                        set_selected(d, e.id);
                        set_stack_focus(d, false);
                        add_selected(d, e.id);
                        stack = stack_of(d);
                    }
                }
            });
            if widgets::flat_button(ui, tl!("Add to Stack"), 120.0).clicked() && !sel.is_empty() {
                add_selected(d, &sel);
                stack = stack_of(d);
            }
        });
    });

    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Applied Filters"));
    let mut stack_sel = d.fields.get("stackSelected").and_then(Value::as_u64).unwrap_or(0) as usize;
    if stack_sel >= stack.len() && !stack.is_empty() {
        stack_sel = stack.len() - 1;
    }
    enum StackOp {
        Toggle(usize),
        Up(usize),
        Down(usize),
        Remove(usize),
        Select(usize),
    }
    let mut op = None;
    let stack_len = stack.len();
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        if stack.is_empty() {
            ui.label(egui::RichText::new(tl!("Add filters from the list above")).color(t.text_dim));
            return;
        }
        for (i, entry) in stack.iter().enumerate().take(stack_len) {
            ui.horizontal(|ui| {
                let eye = if entry.visible { "eye" } else { "eye-off" };
                if widgets::icon_button(ui, eye, tl!("Click to toggle visibility"), false, 22.0).clicked() {
                    op = Some(StackOp::Toggle(i));
                }
                let name = effect_label(&entry.effect);
                if ui.add(egui::Button::selectable(stack_sel == i, name)).clicked() {
                    op = Some(StackOp::Select(i));
                }
                if widgets::icon_button_enabled(ui, "dc-arrow-up", tl!("Move Up"), false, i > 0, 22.0).clicked() {
                    op = Some(StackOp::Up(i));
                }
                if widgets::icon_button_enabled(ui, "dc-arrow-down", tl!("Move Down"), false, i + 1 < stack_len, 22.0).clicked() {
                    op = Some(StackOp::Down(i));
                }
                if widgets::flat_button(ui, tl!("Remove"), 64.0).clicked() {
                    op = Some(StackOp::Remove(i));
                }
            });
        }
    });
    match op {
        Some(StackOp::Toggle(i)) if stack.get(i).is_some() => stack[i].visible = !stack[i].visible,
        Some(StackOp::Select(i)) => {
            stack_sel = i;
            set_stack_focus(d, true);
        }
        Some(StackOp::Up(i)) if i > 0 => {
            stack.swap(i, i - 1);
            stack_sel = i - 1;
        }
        Some(StackOp::Down(i)) if i + 1 < stack.len() => {
            stack.swap(i, i + 1);
            stack_sel = i + 1;
        }
        Some(StackOp::Remove(i)) if i < stack.len() => {
            stack.remove(i);
            if stack_sel >= stack.len() && !stack.is_empty() {
                stack_sel = stack.len() - 1;
            }
        }
        _ => {}
    }
    write_stack(d, &stack);
    d.fields.insert("stackSelected".into(), json!(stack_sel));
    stack_preview_row(app, ui, d, &stack);
    false
}

fn add_selected(d: &mut Dialog, id: &str) {
    let params =
        if !stack_focus(d) && d.fields.get("browseEffect").and_then(Value::as_str) == Some(id) { browse_params(d) } else { default_params(id) };
    let mut stack = stack_of(d);
    stack.push(StackEntry { effect: id.to_string(), visible: true, params });
    write_stack(d, &stack);
    d.fields.insert("stackSelected".into(), json!(stack.len().saturating_sub(1)));
    set_stack_focus(d, true);
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let stack = stack_of(d);
    if stack.is_empty() {
        if app.session.in_interaction() {
            let _ = app.session.cancel_interaction();
        }
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let batch = stack_batch_params(&stack);
    if d.bool("preview") && app.session.in_interaction() {
        return super::form::commit_preview(app, "command.batch", batch);
    }
    let r = app.run("command.batch", batch);
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
thread_local! {
    static THUMBS_PAINTED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static PREVIEWS_PAINTED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn stack_preview_is_one_interaction() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("stack".into(), json!([{ "effect": "artistic.cutout", "visible": true, "params": { "levels": 3 } }]));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the stack previews on the canvas");
        crate::dialogs::cancel(&mut app);
        assert!(!app.session.in_interaction());
        assert!(app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.is_empty());
    }

    #[test]
    fn stack_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(
            "stack".into(),
            json!([
                { "effect": "artistic.cutout", "visible": true, "params": { "levels": 3 } },
                { "effect": "stylize.glowingEdges", "visible": false, "params": { "edgeWidth": 2 } }
            ]),
        );
        frame(&mut app);
        let steps = app.session.active().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        let n = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        assert_eq!(n.appearance.effects.len(), 2);
        assert_eq!(n.appearance.effects[0].id, "artistic.cutout");
        assert_eq!(n.appearance.effects[0].params.get("levels").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(n.appearance.effects[1].id, "stylize.glowingEdges");
        assert!(!n.appearance.effects[1].visible);
        assert_eq!(st.history.undo.len(), steps + 1);
    }

    #[test]
    fn gallery_shows_thumbnails_and_an_in_dialog_preview() {
        let plain = super::sample_drawing(super::THUMB, super::THUMB);
        let mut seen = Vec::new();
        for cat in 0..super::CATEGORIES.len() {
            for e in super::effects_in_category(cat) {
                let thumb = super::thumb_pixels(e.id);
                assert_eq!(thumb.len(), super::THUMB * super::THUMB * 4, "{}", e.id);
                assert_ne!(thumb, plain, "{} thumbnail is the untouched drawing", e.id);
                seen.push(thumb);
            }
        }
        assert!(seen.len() > 2);
        assert_ne!(seen[0], seen[1], "two filters share one thumbnail");

        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        super::THUMBS_PAINTED.with(|c| c.set(0));
        super::PREVIEWS_PAINTED.with(|c| c.set(0));
        frame(&mut app);
        let thumbs = super::THUMBS_PAINTED.with(|c| c.get());
        let previews = super::PREVIEWS_PAINTED.with(|c| c.get());
        let artistic = super::effects_in_category(0).len() as u32;
        assert!(thumbs >= artistic, "painted {thumbs} thumbnails, Artistic has {artistic}");
        assert!(previews >= 1, "the dialog has a preview area");

        let d = app.ui.dialog.as_ref().unwrap();
        let stack = super::stack_of(d);
        let jobs = super::preview_jobs(d, &stack, &super::selected_id(d));
        assert_eq!(jobs.len(), 1, "an empty stack previews the filter being browsed");
        let shown = super::preview_rgba(&jobs, super::PREVIEW_W, super::PREVIEW_H);
        assert_ne!(shown, super::sample_drawing(super::PREVIEW_W, super::PREVIEW_H));

        let soft = super::filtered("artistic.cutout", &super::default_params("artistic.cutout"), super::PREVIEW_W, super::PREVIEW_H);
        let hard = super::filtered(
            "artistic.cutout",
            &serde_json::from_value(json!({"levels": 8, "edgeSimplicity": 0, "edgeFidelity": 3})).unwrap(),
            super::PREVIEW_W,
            super::PREVIEW_H,
        );
        assert_ne!(soft, hard, "the preview follows the filter's settings");
    }
}
