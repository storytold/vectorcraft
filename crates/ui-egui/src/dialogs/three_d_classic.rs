//! The own controls of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) in the
//! compact 3D dialog: the extrude and bevel options, or the revolve options, on the Object tab
//! (Rotate (Classic) has none), and the surface with its lights, highlight, blend steps, shading
//! color and Preserve Spot Colors on the Lighting tab. Position (presets, rotation, perspective) is the Rotation tab every 3D
//! effect shares.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{CLASSIC_EXTRUDE, CLASSIC_REVOLVE, CLASSIC_ROTATE, MAP_ART, MAP_INVISIBLE, MAP_SHADE, MapSurface, SurfaceMap};

use super::revolve::{c3, choice, heading, label, light_list, light_record, number, reset_dot, selected_light, store_selected, toggle};
use crate::state::Dialog;

/// Depth and cap, then the bevel: its shape, its height and whether it is added around the shape
/// or carved into it.
pub(super) fn object_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, c3("Bevel"));
    choice(ui, d, "bevel", tl!("Bevel Shape"), &options("bevel"));
    ui.add_enabled_ui(d.str("bevel") != "none", |ui| {
        number(ui, d, "bevelHeight", tl!("Height"), (1.0, 100.0), "", 1, Some(unit));
        toggle(ui, d, "bevelInside", tl!("Bevel Inside"), false);
    });
    map_art_button(ui, d);
}

/// The field of [`Dialog::fields`] that is true while the dialog shows Map Art in place of its
/// tabs. It is UI state and never enters the effect's parameters.
pub(super) const MAP_ART_OPEN: &str = "__mapArt";
/// The field that holds the number of the surface Map Art shows.
const MAP_SURFACE: &str = "__mapSurface";

/// Map Art…, which opens [`map_art_controls`]. The 3D and Materials panel has no such button.
fn map_art_button(ui: &mut egui::Ui, d: &mut Dialog) {
    if d.bool("__panel") {
        return;
    }
    ui.add_space(10.0);
    if ui.button(tl!("Map Art…")).clicked() {
        d.fields.insert(MAP_ART_OPEN.into(), json!(true));
    }
}

/// Is Map Art open in the dialog?
pub(super) fn map_art_open(d: &Dialog) -> bool {
    d.bool(MAP_ART_OPEN) && vectorcraft_effects::takes_map_art(&d.str("__effect"))
}

/// The numbered surfaces of the first selected object with the dialog's effect and settings
/// ([`vectorcraft_effects::map_surfaces_with`]), kept until the object, the document or a setting
/// that shapes the solid changes.
fn surfaces(app: &crate::VectorcraftApp, ctx: &egui::Context, d: &Dialog) -> Option<std::sync::Arc<Vec<MapSurface>>> {
    use std::hash::{Hash, Hasher};
    let st = app.session.active()?;
    let id = *st.selection.objects.first()?;
    let effect = d.str("__effect");
    // The maps themselves do not change the surfaces.
    let mut p = super::form::params(d);
    if let Some(m) = p.as_object_mut() {
        for key in [MAP_ART, MAP_SHADE, MAP_INVISIBLE] {
            m.remove(key);
        }
    }
    let item = d.fields.get("__item").and_then(Value::as_u64).map(|i| i as usize);
    // The effect being edited, when the dialog edits one already on the object.
    let index = d.fields.get("__index").and_then(Value::as_u64).map(|i| i as usize);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (id.0, item, index, &effect, p.to_string(), st.revision).hash(&mut hasher);
    let key = hasher.finish();
    let cache = egui::Id::unique("map-art-surfaces");
    if let Some((at, kept)) = ctx.data(|m| m.get_temp::<(u64, std::sync::Arc<Vec<MapSurface>>)>(cache))
        && at == key
    {
        return Some(kept);
    }
    let listed = std::sync::Arc::new(vectorcraft_effects::map_surfaces_with(Some(&st.doc), st.doc.node(id)?, (item, index), &effect, &p)?);
    ctx.data_mut(|m| m.insert_temp(cache, (key, listed.clone())));
    Some(listed)
}

/// The dialog's maps, read as the effect reads them.
fn maps_of(d: &Dialog) -> Vec<SurfaceMap> {
    vectorcraft_effects::surface_maps(&json!({ MAP_ART: d.fields.get(MAP_ART).cloned().unwrap_or(Value::Null) }))
}

/// The surface Map Art shows, from 1, among `count` surfaces.
fn shown_surface(d: &Dialog, count: usize) -> usize {
    (d.f64(MAP_SURFACE, 1.0).round().max(1.0) as usize).min(count.max(1))
}

/// Map Art: the surface (first, previous, its number, next, last, and whether the viewer sees
/// it), the symbol mapped onto it, the surface laid out flat with the art's box to move, scale and
/// rotate, the same values as numbers, Scale to Fit, Clear, Clear All, Shade Artwork and Invisible
/// Geometry. Edits the effect's `mapArt`, `mapShade` and `mapInvisible` in `d.fields`, which the
/// dialog previews and applies with its other settings. Done, beside the heading, goes back to the
/// effect's options.
pub(super) fn map_art_controls(app: &crate::VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        heading(ui, tl!("Map Art"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Done")).clicked() {
                d.fields.remove(MAP_ART_OPEN);
            }
        });
    });
    let listed = surfaces(app, ui.ctx(), d).filter(|s| !s.is_empty());
    let Some(listed) = listed else { return };
    let count = listed.len();
    let mut current = shown_surface(d, count);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        label(ui, tl!("Surface"));
        let step = |ui: &mut egui::Ui, text: &str, tip: &str, enabled: bool| {
            let r = ui.add_enabled(enabled, egui::Button::new(text).min_size(egui::vec2(26.0, 24.0)));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
            r.on_hover_text(tip).clicked()
        };
        let mut next = None;
        if step(ui, "|<", tl!("First Surface"), current > 1) {
            next = Some(1);
        }
        if step(ui, "<", tl!("Previous Surface"), current > 1) {
            next = Some(current - 1);
        }
        let mut typed = current;
        let field = ui.add_sized([44.0, 24.0], egui::DragValue::new(&mut typed).range(1..=count).speed(0.1));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, tl!("Surface")));
        if field.changed() {
            next = Some(typed);
        }
        ui.label(format!("/ {count}"));
        if step(ui, ">", tl!("Next Surface"), current < count) {
            next = Some(current + 1);
        }
        if step(ui, ">|", tl!("Last Surface"), current < count) {
            next = Some(count);
        }
        if let Some(n) = next {
            current = n.clamp(1, count);
        }
        // A light mark for a surface the viewer sees, a dark one for a hidden surface.
        let seen = listed.get(current - 1).is_some_and(|s| s.visible);
        let (mark, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(mark, 2.0, if seen { egui::Color32::from_gray(205) } else { egui::Color32::from_gray(90) });
        if !seen {
            crate::widgets::dim_label(ui, "Hidden");
        }
    });
    d.fields.insert(MAP_SURFACE.into(), json!(current));
    let Some(surface) = listed.get(current - 1) else { return };
    let maps = maps_of(d);
    let map = maps.iter().find(|m| m.surface == current).cloned();
    let mut edited = map.clone();
    let names: Vec<String> = app.session.active().map(|st| st.doc.symbols.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Symbol"));
        let none = tl!("None");
        let options: Vec<&str> = std::iter::once(none).chain(names.iter().map(String::as_str)).collect();
        let shown = map.as_ref().map_or(none, |m| m.symbol.as_str());
        if let Some(k) = crate::widgets::dropdown_names(ui, "map-art-symbol", shown, &options, 190.0) {
            edited = k.checked_sub(1).and_then(|k| names.get(k)).map(|name| match &map {
                Some(m) => SurfaceMap { symbol: name.clone(), ..m.clone() },
                None => SurfaceMap { surface: current, symbol: name.clone(), x: 0.0, y: 0.0, scale_x: 100.0, scale_y: 100.0, rotation: 0.0 },
            });
        }
    });
    ui.add_space(6.0);
    let art = edited.as_ref().and_then(|m| app.session.active().and_then(|st| vectorcraft_effects::graphic_bounds(&st.doc, &m.symbol)));
    flat_view(app, ui, surface, &mut edited, art);
    ui.add_space(6.0);
    ui.add_enabled_ui(edited.is_some(), |ui| {
        let shown = edited.clone().unwrap_or(SurfaceMap {
            surface: current,
            symbol: String::new(),
            x: 0.0,
            y: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            rotation: 0.0,
        });
        let unit = app.session.general_unit();
        let mut fields = Dialog { kind: "mapArt".into(), fields: Default::default() };
        fields.fields.insert("__defaults".into(), json!({"mapX": 0.0, "mapY": 0.0, "mapScaleX": 100.0, "mapScaleY": 100.0, "mapRotation": 0.0}));
        for (key, value) in
            [("mapX", shown.x), ("mapY", shown.y), ("mapScaleX", shown.scale_x), ("mapScaleY", shown.scale_y), ("mapRotation", shown.rotation)]
        {
            fields.fields.insert(key.into(), json!(value));
        }
        let offsets = (-vectorcraft_effects::GRAPHIC_MAX_OFFSET, vectorcraft_effects::GRAPHIC_MAX_OFFSET);
        number(ui, &mut fields, "mapX", "X", offsets, "", 1, Some(unit));
        number(ui, &mut fields, "mapY", "Y", offsets, "", 1, Some(unit));
        number(ui, &mut fields, "mapScaleX", tl!("Horizontal Scale"), vectorcraft_effects::GRAPHIC_SCALE_RANGE, "%", 1, None);
        number(ui, &mut fields, "mapScaleY", tl!("Vertical Scale"), vectorcraft_effects::GRAPHIC_SCALE_RANGE, "%", 1, None);
        number(ui, &mut fields, "mapRotation", tl!("Rotation"), (-360.0, 360.0), "°", 1, None);
        if let Some(m) = edited.as_mut() {
            m.x = fields.f64("mapX", m.x);
            m.y = fields.f64("mapY", m.y);
            m.scale_x = fields.f64("mapScaleX", m.scale_x);
            m.scale_y = fields.f64("mapScaleY", m.scale_y);
            m.rotation = fields.f64("mapRotation", m.rotation);
        }
    });
    let mut clear_all = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if ui.add_enabled(edited.is_some() && art.is_some(), egui::Button::new(tl!("Scale to Fit"))).clicked()
            && let (Some(m), Some(art)) = (edited.as_mut(), art)
        {
            let (scale_x, scale_y) = m.fitting_scales(art, (surface.width, surface.height)).unwrap_or((m.scale_x, m.scale_y));
            *m = SurfaceMap { x: 0.0, y: 0.0, scale_x, scale_y, ..m.clone() };
        }
        if ui.add_enabled(edited.is_some(), egui::Button::new(tl!("Clear"))).clicked() {
            edited = None;
        }
        clear_all = ui.add_enabled(!maps.is_empty(), egui::Button::new(tl!("Clear All"))).clicked();
    });
    ui.add_space(4.0);
    toggle(ui, d, MAP_SHADE, tl!("Shade Artwork"), false);
    toggle(ui, d, MAP_INVISIBLE, tl!("Invisible Geometry"), false);
    if clear_all {
        d.fields.insert(MAP_ART.into(), json!([]));
    } else if edited != map {
        let raw = json!({ MAP_ART: d.fields.get(MAP_ART).cloned().unwrap_or(Value::Null) });
        let next = vectorcraft_effects::with_surface_map(&raw, current, edited.as_ref());
        d.fields.insert(MAP_ART.into(), next.get(MAP_ART).cloned().unwrap_or_else(|| json!([])));
    }
}

/// What a drag in the flat view changes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Move,
    /// A handle of the art's box: -1, 0 or 1 across and down, from its center.
    Scale(f32, f32),
    Rotate,
}

/// A drag in the flat view: what it changes, the map as it was when the drag began and where the
/// pointer was then.
#[derive(Clone, Debug)]
struct MapDrag {
    grab: Grab,
    from: SurfaceMap,
    origin: egui::Pos2,
}

/// The surface laid out flat, as large as fits the view, in a light gray when the viewer sees it
/// and a dark gray when it is hidden, with the box of the mapped art `art` (its bounds at its own
/// size) on it: the art's thumbnail while it is not turned, its outline and its eight handles. A
/// drag inside the box moves the art, a drag of a handle scales it about its center, and a drag
/// just outside the box turns it.
fn flat_view(
    app: &crate::VectorcraftApp,
    ui: &mut egui::Ui,
    surface: &MapSurface,
    map: &mut Option<SurfaceMap>,
    art: Option<vectorcraft_geom::Rect>,
) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (view, response) = ui.allocate_exact_size(egui::vec2(ui.available_width().clamp(120.0, 330.0), 150.0), egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Surface")));
    ui.painter().rect_filled(view, 3.0, t.input);
    ui.painter().rect_stroke(view, 3.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let (width, height) = (surface.width.max(1e-6), surface.height.max(1e-6));
    // Screen points for each point of the surface.
    let scale = ((f64::from(view.width()) - 28.0) / width).min((f64::from(view.height()) - 28.0) / height).max(1e-6);
    let flat = egui::Rect::from_center_size(view.center(), egui::vec2((width * scale) as f32, (height * scale) as f32));
    let painter = ui.painter().with_clip_rect(view.intersect(ui.clip_rect()));
    painter.rect_filled(flat, 0.0, if surface.visible { egui::Color32::from_gray(205) } else { egui::Color32::from_gray(90) });
    let drag_id = egui::Id::unique("map-art-drag");
    let (Some(m), Some(art)) = (map.as_mut(), art) else {
        ui.data_mut(|d| d.remove::<MapDrag>(drag_id));
        return;
    };
    let box_of = |m: &SurfaceMap| {
        let center = view.center() + egui::vec2((m.x * scale) as f32, (m.y * scale) as f32);
        let half = egui::vec2((art.width() * m.scale_x / 100.0 * scale / 2.0) as f32, (art.height() * m.scale_y / 100.0 * scale / 2.0) as f32);
        // Counterclockwise on a screen whose y runs down.
        let (sin, cos) = (-m.rotation.to_radians() as f32).sin_cos();
        (center, half, sin, cos)
    };
    let (center, half, sin, cos) = box_of(m);
    let handles: Vec<(f32, f32)> =
        [-1.0f32, 0.0, 1.0].iter().flat_map(|y| [-1.0f32, 0.0, 1.0].map(|x| (x, *y))).filter(|h| *h != (0.0, 0.0)).collect();
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        // The pointer in the box's own frame.
        let local = egui::vec2((origin - center).x * cos + (origin - center).y * sin, -(origin - center).x * sin + (origin - center).y * cos);
        let handle = handles.iter().copied().find(|(x, y)| (local - egui::vec2(x * half.x, y * half.y)).length() <= 7.0);
        let grab = if let Some((x, y)) = handle {
            Some(Grab::Scale(x, y))
        } else if local.x.abs() <= half.x && local.y.abs() <= half.y {
            Some(Grab::Move)
        } else if local.x.abs() <= half.x + 18.0 && local.y.abs() <= half.y + 18.0 {
            Some(Grab::Rotate)
        } else {
            None
        };
        match grab {
            Some(grab) => {
                ui.data_mut(|d| d.insert_temp(drag_id, MapDrag { grab, from: m.clone(), origin }));
            }
            None => ui.data_mut(|d| d.remove::<MapDrag>(drag_id)),
        }
    }
    if let (Some(drag), Some(p)) = (ui.data(|d| d.get_temp::<MapDrag>(drag_id)), response.interact_pointer_pos().filter(|_| response.dragged())) {
        let (from_center, _, from_sin, from_cos) = box_of(&drag.from);
        let round = |v: f64, places: i32| (v * 10f64.powi(places)).round() / 10f64.powi(places);
        let limit = vectorcraft_effects::GRAPHIC_MAX_OFFSET;
        let (lo, hi) = vectorcraft_effects::GRAPHIC_SCALE_RANGE;
        match drag.grab {
            Grab::Move => {
                m.x = round(drag.from.x + f64::from(p.x - drag.origin.x) / scale, 2).clamp(-limit, limit);
                m.y = round(drag.from.y + f64::from(p.y - drag.origin.y) / scale, 2).clamp(-limit, limit);
            }
            Grab::Scale(x, y) => {
                let v = p - from_center;
                let local = egui::vec2(v.x * from_cos + v.y * from_sin, -v.x * from_sin + v.y * from_cos);
                if x != 0.0 && art.width() > 0.0 {
                    m.scale_x = round(f64::from(local.x.abs()) * 2.0 / (art.width() * scale) * 100.0, 2).clamp(lo, hi);
                }
                if y != 0.0 && art.height() > 0.0 {
                    m.scale_y = round(f64::from(local.y.abs()) * 2.0 / (art.height() * scale) * 100.0, 2).clamp(lo, hi);
                }
            }
            Grab::Rotate => {
                let angle = |q: egui::Pos2| f64::from((q - from_center).y.atan2((q - from_center).x)).to_degrees();
                // The screen's angles run clockwise.
                let rotation = drag.from.rotation - (angle(p) - angle(drag.origin));
                m.rotation = round((rotation + 180.0).rem_euclid(360.0) - 180.0, 1);
            }
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<MapDrag>(drag_id));
    }
    let (center, half, sin, cos) = box_of(m);
    let turned_now = |v: egui::Vec2| egui::vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    if m.rotation.abs() < 0.01
        && let Some(st) = app.session.active()
    {
        let keys = crate::panels::symbols::document_symbols(st);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(view));
        inner.set_clip_rect(view.intersect(ui.clip_rect()));
        crate::panels::symbols::chip(&inner, &st.doc, &keys, egui::Rect::from_center_size(center, half * 2.0), &m.symbol);
    }
    let corners: Vec<egui::Pos2> = [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|(x, y)| center + turned_now(egui::vec2(x * half.x, y * half.y)))
        .collect();
    painter.add(egui::Shape::closed_line(corners, egui::Stroke::new(1.0, t.accent)));
    for (x, y) in handles {
        let at = center + turned_now(egui::vec2(x * half.x, y * half.y));
        let handle = egui::Rect::from_center_size(at, egui::vec2(6.0, 6.0));
        painter.rect_filled(handle, 0.0, egui::Color32::WHITE);
        painter.rect_stroke(handle, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
}

/// While Map Art is open with Preview on, the surface it shows, outlined in red on the artboard.
pub(super) fn surface_outline(app: &crate::VectorcraftApp, ctx: &egui::Context, d: &Dialog) {
    if !map_art_open(d) || !d.bool("preview") {
        return;
    }
    let (Some(listed), Some(xf)) = (surfaces(app, ctx, d), app.canvas_xf()) else { return };
    let Some(surface) = listed.get(shown_surface(d, listed.len()).saturating_sub(1)) else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("map-art-outline"))).with_clip_rect(xf.rect);
    for [a, b] in &surface.outline {
        painter.line_segment([xf.to_screen(*a), xf.to_screen(*b)], egui::Stroke::new(1.5, egui::Color32::RED));
    }
}

/// Revolve (Classic)'s angle, cap and offset, and the edge of the profile's bounds the axis is at.
pub(super) fn revolve_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "angle", tl!("Angle"), (0.0, 360.0), "°", 1, None);
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    number(ui, d, "offset", tl!("Offset"), (0.0, 1000.0), "", 1, Some(unit));
    let sides: Vec<(Value, &str)> = super::effect::own_choices(CLASSIC_REVOLVE, "edge", false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (json!(value), label))
        .collect();
    choice(ui, d, "edge", tl!("Axis side"), &sides);
    map_art_button(ui, d);
}

/// The surface, then for Diffuse and Plastic the lights, Ambient Light, the highlight (Plastic
/// only, and not for Rotate (Classic), which has no Plastic surface), Blend Steps, Shading Color and
/// Preserve Spot Colors (unavailable while Shading Color is Custom); Draw Hidden Faces last, for Extrude & Bevel (Classic) (Revolve (Classic) draws every face, and
/// Rotate (Classic)'s plane is seen from either side).
pub(super) fn surface_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    let id = d.str("__effect");
    let effect = if id == CLASSIC_ROTATE { CLASSIC_ROTATE } else { CLASSIC_EXTRUDE };
    choice(ui, d, "surface", tl!("Surface"), &options_of(effect, "surface"));
    let surface = d.str("surface");
    ui.add_enabled_ui(surface == "diffuse" || surface == "plastic", |ui| {
        light_list(ui, d);
        let mut selected = selected_light(d);
        let before = light_record(&selected);
        number(ui, &mut selected, "lightIntensity", tl!("Light Intensity"), (0.0, 100.0), "%", 1, None);
        number(ui, &mut selected, "lightRotation", tl!("Rotation"), (-180.0, 180.0), "°", 1, None);
        number(ui, &mut selected, "lightHeight", tl!("Height"), (0.0, 90.0), "°", 1, None);
        toggle(ui, &mut selected, "lightBehind", tl!("Light behind the object"), false);
        if light_record(&selected) != before {
            store_selected(d, &selected);
        }
        number(ui, d, "ambient", tl!("Ambient Light"), (0.0, 100.0), "%", 1, None);
        if id != CLASSIC_ROTATE {
            ui.add_enabled_ui(surface == "plastic", |ui| {
                number(ui, d, "highlightIntensity", tl!("Highlight Intensity"), (0.0, 100.0), "%", 1, None);
                number(ui, d, "highlightSize", tl!("Highlight Size"), (0.0, 100.0), "%", 1, None);
            });
        }
        number(ui, d, "blendSteps", tl!("Blend Steps"), (1.0, 256.0), "", 0, None);
        choice(ui, d, "shadingColor", tl!("Shading Color"), &options_of(effect, "shadingColor"));
        let custom = d.str("shadingColor") == "custom";
        if custom {
            shading_custom(ui, d);
        }
        ui.add_enabled_ui(!custom, |ui| toggle(ui, d, "preserveSpotColors", tl!("Preserve Spot Colors"), false));
    });
    if id == CLASSIC_EXTRUDE {
        toggle(ui, d, "drawHiddenFaces", tl!("Draw Hidden Faces"), false);
    }
}

/// The values of Extrude & Bevel (Classic)'s `key` as the dropdowns list them, (value, label).
fn options(key: &str) -> Vec<(Value, &'static str)> {
    options_of(CLASSIC_EXTRUDE, key)
}

/// The values of classic 3D effect `effect`'s `key` as the dropdowns list them, (value, label).
fn options_of(effect: &str, key: &str) -> Vec<(Value, &'static str)> {
    super::effect::own_choices(effect, key, false).unwrap_or_default().into_iter().map(|(label, value)| (json!(value), label)).collect()
}

/// The custom shading color: a color button, its hex value and its reset dot.
fn shading_custom(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Color"));
        let color = d
            .fields
            .get("shadingCustom")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 7 && s.starts_with('#'))
            .and_then(vectorcraft_color::Color::from_hex)
            .unwrap_or(vectorcraft_color::Color::BLACK);
        let mut rgb = color.to_rgb_uncalibrated().map(|c| (c * 255.0).round() as u8);
        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(tl!("Shading Color")).changed() {
            d.fields.insert("shadingCustom".into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
        }
        ui.label(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shadingCustom", rgb != [0; 3], "#000000") {
            d.fields.insert("shadingCustom".into(), json!("#000000"));
        }
    });
}
