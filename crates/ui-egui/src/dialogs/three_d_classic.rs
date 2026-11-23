//! The own rows of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) in the 3D dialog: the
//! extrude and bevel rows or the revolve rows of the Object page (Rotate (Classic) has none), the Lighting page's
//! surface rows, and the Map Art page with its surface outline on the artboard. Position (presets, rotation,
//! perspective) is the page every 3D effect shares; the classic effects add the track cube to it ([`super::track_cube`]).

use craft::behavior::CanvasView;
use craft::behavior::geometry::quad::{BoxAllow, BoxRules, BoxState, Parts, ScaleRule};
use craft::behavior::kurbo::{BezPath, Point, Rect, Size, Vec2};
use craft::draw::ui::transform::{RotateZone, TransformBox};
use craft::theme::Srgba;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas::Canvas;
use craft::ui::canvas::{CanvasSpace, canvas_space};
use craft::ui::canvas_controls::SelectionOutline;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Picture;
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::DataColor;
use craft::ui::text::NumberField;
use craft::ui::value_drag::SliderScale;
use egui::{Color32, Response, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{
    CLASSIC_EXTRUDE, CLASSIC_ROTATE, GRAPHIC_MAX_OFFSET, GRAPHIC_SCALE_RANGE, MAP_ART, MAP_INVISIBLE, MAP_SHADE, MAX_MAPS, SurfaceMap,
};

use super::revolve::{Place, bevel_row, c3, light_editor};
use super::three_d::{Data, with_fields};
use crate::VectorcraftApp;

/// The field that holds the number of the surface Map Art shows.
const MAP_SURFACE: &str = "__mapSurface";
/// The dialog's page field and the value that shows Map Art.
const PAGE: &str = "__page";
const MAP_PAGE: &str = "mapArt";
/// The field of the custom shading colour, `#rrggbb` text.
const SHADING_CUSTOM: &str = "shadingCustom";
/// Keys of the map rows; they hold no value of their own: `set_map_value` writes the number into `mapArt`.
const MAP_KEYS: [&str; 5] = ["mapX", "mapY", "mapScaleX", "mapScaleY", "mapRotation"];
const MAP_SYMBOL: &str = "mapSymbol";
/// The Map Art page's rows of the shown surface's map: views of `mapArt`, never effect parameters.
pub(super) const MAP_ROWS: [&str; 6] = [MAP_SYMBOL, MAP_KEYS[0], MAP_KEYS[1], MAP_KEYS[2], MAP_KEYS[3], MAP_KEYS[4]];

/// The unit of the general-unit length rows.
fn length_unit(unit: Unit) -> UnitKind {
    crate::canvas::rulers::unit_kind(unit)
}

/// Where the rows show: the 3D and Materials panel marks its fields with `__panel`.
fn place_of(fields: &DialogFields) -> Place {
    if fields.bool("__panel") == Some(true) { Place::Panel } else { Place::Dialog }
}

/// The values of classic 3D effect `effect`'s `key` as the choice rows list them, (value, label).
fn options_of(effect: &str, key: &str) -> Vec<(&'static str, &'static str)> {
    super::effect_params::own_choices(effect, key, false).unwrap_or_default().into_iter().map(|(label, value)| (value, label)).collect()
}

/// `options_of` as the options of a choice row.
fn choice_row(effect: &str, key: &str, label: &'static str) -> Param {
    let options = options_of(effect, key).into_iter().map(|(value, label)| (value.to_string(), label.into())).collect();
    let default = vectorcraft_effects::default_params(effect).and_then(|p| p.get(key).and_then(Value::as_str).map(str::to_string));
    Param::new(key, label, ParamKind::choice(options, default)).resettable()
}

/// A slider row over `range` with the effect's default.
fn range_row(effect: &str, key: &str, label: &'static str, range: (f64, f64), unit: Option<UnitKind>, decimals: u32) -> Param {
    let default = vectorcraft_effects::default_params(effect).and_then(|p| p.get(key).and_then(Value::as_f64)).unwrap_or(range.0);
    let row = Param::new(key, label, ParamKind::range(range.0, range.1, default, SliderScale::Linear)).decimals(decimals).resettable();
    match unit {
        Some(unit) => row.unit(unit),
        None => row,
    }
}

/// A Boolean row with the effect's default.
fn toggle_row(effect: &str, key: &str, label: &'static str) -> Param {
    let default = vectorcraft_effects::default_params(effect).and_then(|p| p.get(key).and_then(Value::as_bool)).unwrap_or(false);
    Param::new(key, label, ParamKind::bool(default)).resettable()
}

/// The Cap row: its two ids are the JSON booleans the effect stores.
fn cap_row(effect: &str) -> Param {
    let default = vectorcraft_effects::default_params(effect).and_then(|p| p.get("cap").and_then(Value::as_bool)).unwrap_or(true);
    Param::new(
        "cap",
        c3("Cap"),
        ParamKind::radio(vec![("true".into(), c3("Solid").into()), ("false".into(), c3("Hollow").into())], Some(default.to_string())),
    )
    .resettable()
}

/// Map Art…, which shows the Map Art page. The 3D and Materials panel has no such button.
fn map_art_button(fields: &DialogFields) -> Option<Param> {
    (place_of(fields) == Place::Dialog).then(|| {
        Param::custom("__mapArtOpen", "", |ui, fields, _| {
            let button = Button::new(tl!("Map Art…")).show(ui);
            if button.clicked() {
                fields.set(PAGE, MAP_PAGE);
            }
            button.response
        })
        .full_width()
    })
}

/// Extrude & Bevel (Classic)'s Object page: depth and cap, then the bevel: its shape (a custom one included), its
/// height and whether it is added around the shape or carved into it.
pub(crate) fn object_params(data: &Data, fields: &DialogFields, effect: &str) -> Vec<Param> {
    let unit = Some(length_unit(data.unit));
    let shaped: Vec<&str> = options_of(effect, "bevel").into_iter().map(|(value, _)| value).filter(|v| *v != "none").collect();
    let mut rows = vec![
        Param::section(tl!("Object")),
        range_row(effect, "depth", tl!("Depth"), (0.0, 2000.0), unit, 1),
        cap_row(effect),
        Param::section(c3("Bevel")),
        bevel_row(data, CLASSIC_EXTRUDE, place_of(fields)),
        range_row(effect, "bevelHeight", tl!("Height"), (1.0, 100.0), unit, 1).enabled_when_in("bevel", shaped.clone()),
        toggle_row(effect, "bevelInside", tl!("Bevel Inside")).enabled_when_in("bevel", shaped),
    ];
    rows.extend(map_art_button(fields));
    rows
}

/// Revolve (Classic)'s angle, cap and offset, and the edge of the profile's bounds the axis is at.
pub(crate) fn revolve_params(data: &Data, fields: &DialogFields, effect: &str) -> Vec<Param> {
    let mut rows = vec![
        Param::section(tl!("Object")),
        range_row(effect, "angle", tl!("Angle"), (0.0, 360.0), Some(UnitKind::Degrees), 1),
        cap_row(effect),
        range_row(effect, "offset", tl!("Offset"), (0.0, 1000.0), Some(length_unit(data.unit)), 1),
        choice_row(effect, "edge", tl!("Axis side")),
    ];
    rows.extend(map_art_button(fields));
    rows
}

/// The surface, then for Diffuse and Plastic the lights, Ambient Light, the highlight (Plastic only, and not for
/// Rotate (Classic), which has no Plastic surface), Blend Steps, Shading Color and Preserve Spot Colors (unavailable
/// while Shading Color is Custom); Draw Hidden Faces last, for Extrude & Bevel (Classic) (Revolve (Classic) draws
/// every face, and Rotate (Classic)'s plane is seen from either side).
pub(crate) fn surface_params(data: &Data, fields: &DialogFields, effect: &str) -> Vec<Param> {
    let options = if effect == CLASSIC_ROTATE { CLASSIC_ROTATE } else { CLASSIC_EXTRUDE };
    let lit = ["diffuse", "plastic"];
    let is_lit = fields.text("surface").is_some_and(|s| lit.contains(&s));
    let custom = fields.text("shadingColor") == Some("custom");
    let unlit_off = |row: Param| row.enabled_when_in("surface", lit);
    let mut rows = vec![
        choice_row(options, "surface", tl!("Surface")),
        light_editor(data, place_of(fields), true),
        unlit_off(range_row(effect, "ambient", tl!("Ambient Light"), (0.0, 100.0), Some(UnitKind::Percent), 1)),
    ];
    if effect != CLASSIC_ROTATE {
        rows.push(
            range_row(effect, "highlightIntensity", tl!("Highlight Intensity"), (0.0, 100.0), Some(UnitKind::Percent), 1)
                .enabled_when("surface", "plastic"),
        );
        rows.push(
            range_row(effect, "highlightSize", tl!("Highlight Size"), (0.0, 100.0), Some(UnitKind::Percent), 1).enabled_when("surface", "plastic"),
        );
    }
    rows.push(unlit_off(range_row(effect, "blendSteps", tl!("Blend Steps"), (1.0, 256.0), None, 0)));
    rows.push(unlit_off(choice_row(options, "shadingColor", tl!("Shading Color"))));
    if custom {
        rows.push(shading_custom());
    }
    // One condition per row: while the surface is flat the row is off whatever the shading colour is.
    let spot = toggle_row(effect, "preserveSpotColors", tl!("Preserve Spot Colors"));
    let not_custom: Vec<&str> = options_of(options, "shadingColor").into_iter().map(|(v, _)| v).filter(|v| *v != "custom").collect();
    rows.push(if is_lit { spot.enabled_when_in("shadingColor", not_custom) } else { spot.enabled_when_in("shadingColor", Vec::<&str>::new()) });
    if effect == CLASSIC_EXTRUDE {
        rows.push(toggle_row(effect, "drawHiddenFaces", tl!("Draw Hidden Faces")));
    }
    rows
}

/// The custom shading colour as the `#rrggbb` text it is stored in; black when missing or malformed.
fn shading_color(fields: &DialogFields) -> Srgba {
    fields.text(SHADING_CUSTOM).filter(|s| s.len() == 7).and_then(Srgba::from_hex).unwrap_or(Srgba::rgb(0.0, 0.0, 0.0))
}

/// The custom shading colour: a swatch button over the host's colour picker.
fn shading_custom() -> Param {
    Param::custom(SHADING_CUSTOM, tl!("Color"), |ui, fields, cx: &mut CustomContext| {
        // The picker answers with a colour; the effect keeps text.
        if let Some(rgb) = fields.color(SHADING_CUSTOM) {
            fields.set(SHADING_CUSTOM, Srgba::from_rgb_array(rgb).to_hex());
        }
        let color = shading_color(fields);
        let button = Button::new(color.to_hex().to_uppercase()).show(ui); // PLAN: assumed a swatch icon via IconSpec::Swatch
        if button.clicked() {
            cx.pick_color(SHADING_CUSTOM, color);
        }
        button.response
    })
    .opens_with("#000000")
}

/// The dialog's maps, read as the effect reads them.
fn maps_of(fields: &DialogFields) -> Vec<SurfaceMap> {
    vectorcraft_effects::surface_maps(&json!({ MAP_ART: json_field(fields, MAP_ART) }))
}

/// The JSON a field holds, null when it has none.
fn json_field(fields: &DialogFields, key: &str) -> Value {
    match fields.get(key) {
        Some(FieldValue::Json(value)) => value.clone(),
        _ => Value::Null,
    }
}

/// The surface number Map Art shows, from 1.
fn surface_number(fields: &DialogFields) -> usize {
    fields.number(MAP_SURFACE).unwrap_or(1.0).round().max(1.0) as usize
}

/// The surface Map Art shows, from 1, among `count` surfaces.
fn shown_surface(fields: &DialogFields, count: usize) -> usize {
    surface_number(fields).min(count.max(1))
}

/// Whether a surface can take a map: with every map the object takes in use, one without a map cannot get one.
fn has_room(maps: &[SurfaceMap], has_map: bool) -> bool {
    has_map || maps.len() < MAX_MAPS
}

/// Writes `map` as surface `surface`'s map (none clears it) into the dialog's `mapArt`.
fn store_map(fields: &mut DialogFields, surface: usize, map: Option<&SurfaceMap>) {
    let raw = json!({ MAP_ART: json_field(fields, MAP_ART) });
    let next = vectorcraft_effects::with_surface_map(&raw, surface, map);
    fields.set(MAP_ART, FieldValue::Json(next.get(MAP_ART).cloned().unwrap_or_else(|| json!([]))));
}

/// A map with the defaults of a new one.
fn new_map(surface: usize, symbol: String) -> SurfaceMap {
    SurfaceMap { surface, symbol, x: 0.0, y: 0.0, scale_x: 100.0, scale_y: 100.0, rotation: 0.0 }
}

/// `v` rounded to `places` decimals and kept within `lo..=hi`.
fn round_clamp(v: f64, places: i32, lo: f64, hi: f64) -> f64 {
    let f = 10f64.powi(places);
    ((v * f).round() / f).clamp(lo, hi)
}

/// What a map row or the symbol select sets: the one place a typed value reaches `mapArt`.
fn set_map_value(fields: &mut DialogFields, key: &str, value: Value) -> Result<(), String> {
    let surface = surface_number(fields);
    let maps = maps_of(fields);
    let old = maps.iter().find(|m| m.surface == surface).cloned();
    let edited = if key == MAP_SYMBOL {
        match value.as_str().filter(|s| !s.is_empty()) {
            None => None,
            Some(name) if has_room(&maps, old.is_some()) => {
                Some(old.map_or_else(|| new_map(surface, name.to_string()), |m| SurfaceMap { symbol: name.to_string(), ..m }))
            }
            Some(_) => return Err("every map is in use".into()),
        }
    } else {
        let n = value.as_f64().filter(|n| n.is_finite()).ok_or("not a number")?;
        let mut m = old.ok_or("the surface has no map")?;
        match key {
            "mapX" => m.x = round_clamp(n, 2, -GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
            "mapY" => m.y = round_clamp(n, 2, -GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
            "mapScaleX" => m.scale_x = round_clamp(n, 2, GRAPHIC_SCALE_RANGE.0, GRAPHIC_SCALE_RANGE.1),
            "mapScaleY" => m.scale_y = round_clamp(n, 2, GRAPHIC_SCALE_RANGE.0, GRAPHIC_SCALE_RANGE.1),
            "mapRotation" => m.rotation = n.clamp(-360.0, 360.0),
            _ => return Err(format!("no map value {key}")),
        }
        Some(m)
    };
    store_map(fields, surface, edited.as_ref());
    Ok(())
}

/// A step button of the surface navigator.
fn step(ui: &mut Ui, icon: &'static str, tip: &'static str, enabled: bool) -> bool {
    IconButton::new(craft::behavior::IconId::new(icon)).label(tip).tooltip(tip).enabled(enabled).show(ui).clicked()
}

/// The surface navigator: first, previous, the number, next, last, and "Hidden" for a surface the viewer cannot see.
fn navigator(count: usize, seen: Vec<bool>) -> Param {
    Param::custom(MAP_SURFACE, tl!("Surface"), move |ui, fields, _| {
        let current = shown_surface(fields, count);
        let mut next = None;
        let row = ui.horizontal(|ui| {
            for (icon, tip, target, enabled) in [
                ("chevrons_left", tl!("First Surface"), 1, current > 1),
                ("chevron_left", tl!("Previous Surface"), current.saturating_sub(1), current > 1),
            ] {
                if step(ui, icon, tip, enabled) {
                    next = Some(target);
                }
            }
            let mut typed = Some(current as f64);
            NumberField::new(tl!("Surface"), Bind::bound(&mut typed)).range(1.0..=count as f64).unit(format!("/ {count}")).show(ui);
            if let Some(n) = typed.filter(|n| n.is_finite()).map(|n| n.round() as usize).filter(|n| *n != current) {
                next = Some(n);
            }
            for (icon, tip, target, enabled) in [
                ("chevron_right", tl!("Next Surface"), current + 1, current < count),
                ("chevrons_right", tl!("Last Surface"), count, current < count),
            ] {
                if step(ui, icon, tip, enabled) {
                    next = Some(target);
                }
            }
            if !seen.get(current - 1).copied().unwrap_or(true) {
                ui.label(tl!("Hidden"));
            }
        });
        let shown = next.map_or(current, |n| n.clamp(1, count));
        if fields.number(MAP_SURFACE) != Some(shown as f64) {
            fields.set(MAP_SURFACE, shown as f64);
        }
        row.response
    })
}

/// What the flat view needs of the surface and the art, owned by its row.
#[derive(Clone)]
struct Flat {
    width: f64,
    height: f64,
    visible: bool,
    /// The art's bounds at its own size.
    art: Option<(f64, f64)>,
    doc: Option<std::sync::Arc<vectorcraft_doc::Document>>,
    revision: u64,
}

/// The art's box at 100 %, centred on the surface's centre, which is the flat view's origin.
fn art_rect(art: (f64, f64)) -> Rect {
    Rect::from_center_size(Point::ZERO, Size::new(art.0, art.1))
}

/// The box `map` puts the art in on the flat surface: its scales and turn about the centre, moved by the offset.
/// `Parts::angle` runs clockwise on the screen and the map's rotation counterclockwise.
fn map_box(map: &SurfaceMap, rect: Rect) -> BoxState {
    let parts = Parts::new(map.scale_x / 100.0, map.scale_y / 100.0, -map.rotation, 0.0, 0.0);
    BoxState::from_rect(rect).with_parts(rect, 0.0, parts).moved(Vec2::new(map.x, map.y))
}

/// `base` with the numbers `state` stands for, each kept to its clamp.
fn map_of_box(state: &BoxState, rect: Rect, base: &SurfaceMap) -> SurfaceMap {
    let parts = state.parts(rect, 0.0);
    let centre = state.quad[0].midpoint(state.quad[2]);
    let (lo, hi) = GRAPHIC_SCALE_RANGE;
    SurfaceMap {
        x: round_clamp(centre.x, 2, -GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
        y: round_clamp(centre.y, 2, -GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
        scale_x: round_clamp(parts.w * 100.0, 2, lo, hi),
        scale_y: round_clamp(parts.h * 100.0, 2, lo, hi),
        rotation: round_clamp((-parts.angle + 180.0).rem_euclid(360.0) - 180.0, 1, -180.0, 180.0),
        ..base.clone()
    }
}

/// The id of Map Art's Done, in the footer as Reset all is on the other pages.
pub(crate) const MAP_DONE: &str = "mapDone";

/// Map Art's Done, in the footer so it stays in view however long the page; none on the other pages.
pub(crate) fn map_done_button(fields: &DialogFields) -> Option<(String, FooterButton)> {
    (fields.text(PAGE) == Some(MAP_PAGE)).then(|| (MAP_DONE.to_owned(), FooterButton::new(tl!("Done"), DialogRole::Other)))
}

/// Done's press: back to the effect's first page.
pub(crate) fn map_done(dialog: &mut HostedParamDialog, effect: &str) {
    dialog.set_field(PAGE, FieldValue::from(if effect == CLASSIC_ROTATE { "rotation" } else { "object" }));
}

/// The surface laid out flat, as large as fits the row, in a light gray when the viewer sees it and a dark gray when
/// it is hidden, with the box of the mapped art on it: a free transform box that moves, scales and turns the art.
fn flat_view(ui: &mut Ui, fields: &mut DialogFields, flat: &Flat) -> Response {
    let surface = surface_number(fields);
    let map = maps_of(fields).into_iter().find(|m| m.surface == surface);
    let size = vec2(ui.available_width().max(ui.spacing().slider_width), ui.spacing().slider_width); // PLAN: tokens.size.plot
    let (room, response) = ui.allocate_exact_size(size, Sense::hover());
    let (width, height) = (flat.width.max(1e-6), flat.height.max(1e-6));
    let margin = f64::from(ui.spacing().item_spacing.x) * 2.0;
    let zoom = ((f64::from(room.width()) - margin) / width).min((f64::from(room.height()) - margin) / height).max(1e-6);
    let view = CanvasView::new(room.into(), Point::ZERO, zoom);
    let rect = flat.art.map(art_rect);
    let mut edited = map.clone();
    Canvas::new(tl!("Surface")).view(view).show_at(ui, room, |ui, body| {
        let corner = |x: f64, y: f64| view.to_screen(Point::new(x, y));
        let page = egui::Rect::from_two_pos(corner(-width / 2.0, -height / 2.0), corner(width / 2.0, height / 2.0));
        let fill = if flat.visible { Color32::from_gray(205) } else { Color32::from_gray(90) };
        if let Some(space) = canvas_space(&body.env) {
            space.paint_picture(ui.ctx(), vec![egui::Shape::rect_filled(page, 0.0, fill)]);
        }
        let (Some(m), Some(rect)) = (map.as_ref(), rect) else { return };
        let mut state = map_box(m, rect);
        if m.rotation.abs() < 0.01
            && let (Some(doc), Some((w, h))) = (flat.doc.as_ref(), flat.art)
        {
            let keys = crate::panels::symbols::document_symbols(doc, flat.revision);
            let at = egui::Rect::from_two_pos(view.to_screen(state.quad[0]), view.to_screen(state.quad[2]));
            let art = vectorcraft_geom::Rect::new(0.0, 0.0, w, h);
            let paint = |cx: &ContentPainterCx<'_>, rect: egui::Rect| crate::panels::symbols::paint_symbol_art(cx, rect, doc, &keys, &m.symbol, art);
            // Decorative: the Art box below is the surface's accessible control.
            Picture::painter("", ThumbAspect::SQUARE, paint).show_at(ui, at);
        }
        let rules = BoxRules::default().with_scale(ScaleRule::Free).with_aspect_linked(false).with_allow(BoxAllow::new(true, false, false));
        TransformBox::new(tl!("Art box"), Bind::bound(&mut state), rect, 0.0)
            .rules(rules)
            .reference_point(false)
            .rotate_zone(RotateZone::border(18.0))
            .show_at(ui, body.rect);
        let moved = map_of_box(&state, rect, m);
        if moved != *m {
            edited = Some(moved);
        }
    });
    if edited != map {
        store_map(fields, surface, edited.as_ref());
    }
    response
}

/// Scale to Fit, Clear and Clear All.
fn map_actions(flat: Flat, has_map: bool, any_map: bool) -> Param {
    Param::custom("__mapActions", "", move |ui, fields, _| {
        let surface = surface_number(fields);
        let map = maps_of(fields).into_iter().find(|m| m.surface == surface);
        ui.horizontal(|ui| {
            let fit = Button::new(tl!("Scale to Fit")).enabled(has_map && flat.art.is_some()).show(ui);
            if let (true, Some(m), Some(art)) = (fit.clicked(), map.as_ref(), flat.art) {
                let art = vectorcraft_geom::Rect::new(0.0, 0.0, art.0, art.1);
                let (scale_x, scale_y) = m.fitting_scales(art, (flat.width, flat.height)).unwrap_or((m.scale_x, m.scale_y));
                store_map(fields, surface, Some(&SurfaceMap { x: 0.0, y: 0.0, scale_x, scale_y, ..m.clone() }));
            }
            if Button::new(tl!("Clear")).enabled(has_map).show(ui).clicked() {
                store_map(fields, surface, None);
            }
            if Button::new(tl!("Clear All")).enabled(any_map).show(ui).clicked() {
                fields.set(MAP_ART, FieldValue::Json(json!([])));
            }
        })
        .response
    })
    .full_width()
    // The page's maps: automation sets them whole, as the rows above edit them one surface at a time.
    .companions([MAP_ART])
}

/// Map Art: the surface (first, previous, its number, next, last, and whether the viewer sees it), the symbol mapped
/// onto it, the surface laid out flat with the art's box to move, scale and rotate, the same values as numbers, Scale
/// to Fit, Clear, Clear All, Shade Artwork and Invisible Geometry. The rows edit the effect's `mapArt`, `mapShade` and
/// `mapInvisible`, which the dialog previews and applies with its other settings; the footer's Done
/// ([`map_done_button`]) goes back to the effect's first page. The number rows list only while the shown surface has a
/// map.
pub(crate) fn map_art_params(data: &Data, fields: &DialogFields, effect: &str) -> Vec<Param> {
    if !vectorcraft_effects::takes_map_art(effect) {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let Some(listed) = data.surfaces.as_ref().filter(|s| !s.is_empty()) else { return rows };
    let count = listed.len();
    let current = shown_surface(fields, count);
    let Some(surface) = listed.get(current - 1) else { return rows };
    rows.push(navigator(count, listed.iter().map(|s| s.visible).collect()));
    let maps = maps_of(fields);
    let map = maps.iter().find(|m| m.surface == current).cloned();
    let names: Vec<String> = data.doc.as_ref().map(|d| d.symbols.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    if has_room(&maps, map.is_some()) {
        let entries: Vec<Entry<String>> =
            std::iter::once(Entry::new(String::new(), tl!("None"))).chain(names.into_iter().map(|n| Entry::new(n.clone(), n))).collect();
        let shown = map.as_ref().map(|m| m.symbol.clone()).unwrap_or_default();
        // A custom row, so a pick and automation both write the map through `set_map_value`.
        let symbol = Param::custom(MAP_SYMBOL, tl!("Symbol"), move |ui, fields, _| {
            let picked = Select::new(tl!("Symbol"), Bind::given(shown.clone()), &entries).id_salt(MAP_SYMBOL).show(ui);
            if let Some(name) = picked.value().cloned() {
                // The select lists only what the map takes: a refusal cannot come from it.
                let _ = set_map_value(fields, MAP_SYMBOL, json!(name));
            }
            picked.response
        });
        rows.push(symbol.on_set(set_map_value));
    } else {
        rows.push(Param::read_only_text(MAP_SYMBOL, tl!("Symbol"), tl!("None")));
    }
    let art =
        map.as_ref().and_then(|m| data.doc.as_ref().and_then(|d| vectorcraft_effects::graphic_bounds(d, &m.symbol))).map(|r| (r.width(), r.height()));
    let flat = Flat { width: surface.width, height: surface.height, visible: surface.visible, art, doc: data.doc.clone(), revision: data.revision };
    let flat_row = flat.clone();
    rows.push(Param::custom("__mapFlat", "", move |ui, fields, _| flat_view(ui, fields, &flat_row)).full_width());
    if let Some(m) = &map {
        let unit = Some(length_unit(data.unit));
        let values = [m.x, m.y, m.scale_x, m.scale_y, m.rotation];
        let labels = ["X", "Y", tl!("Horizontal Scale"), tl!("Vertical Scale"), tl!("Rotation")];
        let bounds = [
            (-GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
            (-GRAPHIC_MAX_OFFSET, GRAPHIC_MAX_OFFSET),
            GRAPHIC_SCALE_RANGE,
            GRAPHIC_SCALE_RANGE,
            (-360.0, 360.0),
        ];
        let kinds = [unit, unit, Some(UnitKind::Percent), Some(UnitKind::Percent), Some(UnitKind::Degrees)];
        for i in 0..MAP_KEYS.len() {
            let row = Param::new(MAP_KEYS[i], labels[i], ParamKind::range(bounds[i].0, bounds[i].1, values[i], SliderScale::Linear))
                .decimals(2)
                .on_set(set_map_value);
            rows.push(match kinds[i] {
                Some(kind) => row.unit(kind),
                None => row,
            });
        }
    }
    rows.push(map_actions(flat, map.is_some(), !maps.is_empty()));
    rows.push(toggle_row(effect, MAP_SHADE, tl!("Shade Artwork")));
    rows.push(toggle_row(effect, MAP_INVISIBLE, tl!("Invisible Geometry")));
    rows
}

/// While Map Art is shown with Preview on, the surface it shows, outlined in red on the artboard.
pub(crate) fn show_outline(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) {
    let Some(surfaces) = app.three_d_dialog.as_ref().and_then(|s| s.data.read().ok().and_then(|d| d.surfaces.clone())) else { return };
    let Some(current) = with_fields(app, |dialog| {
        let fields = dialog.fields();
        (fields.text(PAGE) == Some(MAP_PAGE) && fields.bool("preview") == Some(true)).then(|| shown_surface(fields, surfaces.len()))
    })
    .flatten() else {
        return;
    };
    let Some(surface) = surfaces.get(current.saturating_sub(1)) else { return };
    let mut path = BezPath::new();
    for [a, b] in &surface.outline {
        path.move_to((a.x, a.y));
        path.line_to((b.x, b.y));
    }
    let paths = [path];
    SelectionOutline::solid(tl!("Map Art surface"), &paths).color(DataColor::new(Color32::RED)).show_at(ui, space.view.rect.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(rotation: f64) -> SurfaceMap {
        SurfaceMap { surface: 1, symbol: "S".into(), x: 10.0, y: 5.0, scale_x: 200.0, scale_y: 50.0, rotation }
    }

    #[test]
    fn a_flat_view_box_edit_moves_scales_and_turns_the_map_within_its_clamps() {
        let rect = art_rect((20.0, 10.0));
        let m = map(30.0);
        assert_eq!(map_of_box(&map_box(&m, rect), rect, &m), m, "the box and the map are one value");

        let far = map_box(&m, rect).moved(Vec2::new(GRAPHIC_MAX_OFFSET * 4.0, 0.0));
        assert_eq!(map_of_box(&far, rect, &m).x, GRAPHIC_MAX_OFFSET);

        let huge = SurfaceMap { scale_x: GRAPHIC_SCALE_RANGE.1 * 2.0, ..m.clone() };
        assert_eq!(map_of_box(&map_box(&huge, rect), rect, &huge).scale_x, GRAPHIC_SCALE_RANGE.1);

        let turned = map(190.0);
        assert_eq!(map_of_box(&map_box(&turned, rect), rect, &turned).rotation, -170.0, "a turn wraps to ±180");
    }
}
