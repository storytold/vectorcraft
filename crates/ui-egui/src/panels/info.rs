//! Info panel: cursor X/Y, selection W/H, the pointer's distance/angle from the selection, the
//! readout of the tool in use (a move, a gradient vector, a Scale, Rotate, Reflect or Shear tool
//! drag, the Zoom tool's magnification, the Paintbrush's brush) and the selection's fill and
//! stroke colour values.

use craft::keymap::CommandRef;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Picture, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::style::TypeRole;
use egui::Ui;
use serde_json::Value;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeKind, Unit};
use vectorcraft_geom::Point;
use vectorcraft_tools::Readout;

use super::{current_paints, mixed_paints, swatches};
use crate::VectorcraftApp;

/// Color values in the color's own model, for the Info panel.
pub fn color_values(c: &Color) -> Vec<(&'static str, String)> {
    match *c {
        Color::Rgb { r, g, b } => vec![("R", format!("{:.0}", r * 255.0)), ("G", format!("{:.0}", g * 255.0)), ("B", format!("{:.0}", b * 255.0))],
        Color::Cmyk { c, m, y, k } => {
            vec![
                ("C", format!("{:.0}%", c * 100.0)),
                ("M", format!("{:.0}%", m * 100.0)),
                ("Y", format!("{:.0}%", y * 100.0)),
                ("K", format!("{:.0}%", k * 100.0)),
            ]
        }
        Color::Gray { k } => vec![("K", format!("{:.0}%", k * 100.0))],
        Color::Lab { l, a, b } => vec![("L", format!("{l:.0}")), ("a", format!("{a:.0}")), ("b", format!("{b:.0}"))],
    }
}

/// The name Show Options adds for paint `p`: the swatch and the percentage of a tint, or the
/// gradient swatch a gradient is linked to.
fn paint_name(p: &Paint) -> Option<String> {
    match p {
        Paint::Solid { swatch: Some(n), tint, .. } if *tint < 1.0 => Some(format!("{n} {:.0}%", vectorcraft_color::tint_percent(*tint))),
        Paint::Gradient(g) => g.swatch.clone(),
        _ => None,
    }
}

/// One paint's block: its tile, its values, and its name (a swatch's with Show Options, else a
/// non-solid paint's kind). A paint the selected objects do not share (`mixed`) leaves it empty.
struct PaintLines {
    paint: Paint,
    mixed: bool,
    /// The monospaced `R: 230` values of a solid color.
    values: Vec<String>,
    note: Option<String>,
}

impl PaintLines {
    fn of(paint: Paint, mixed: bool, options: bool) -> Self {
        if mixed {
            return PaintLines { paint, mixed, values: Vec::new(), note: None };
        }
        let (values, note) = match &paint {
            Paint::Solid { color, .. } => {
                (color_values(color).into_iter().map(|(k, v)| format!("{k}: {v}")).collect(), paint_name(&paint).filter(|_| options))
            }
            other => (Vec::new(), Some(paint_name(other).filter(|_| options).unwrap_or_else(|| other.label()))),
        };
        PaintLines { paint, mixed, values, note }
    }
}

/// What the panel shows, ready to lay out.
pub(crate) struct InfoLines {
    left: Vec<String>,
    right: Vec<String>,
    zoom: Option<String>,
    brush: Option<String>,
    fill: PaintLines,
    stroke: PaintLines,
    /// The font, and the size and tracking, the selected type shares.
    type_block: Option<(String, String)>,
}

fn mono(ui: &mut Ui, s: &str) {
    Text::new(s.to_string()).role(TypeRole::Mono).show(ui);
}

/// One paint's row: its tile, its values and its note. A mixed paint keeps the tile's room empty.
fn paint_block(ui: &mut Ui, p: &PaintLines) {
    Row::new().show(ui, |ui, _| {
        Picture::painter("", ThumbAspect::SQUARE, |cx, rect| {
            if !p.mixed {
                swatches::paint_swatch(cx, rect, &p.paint, false, (false, false), None);
            }
        })
        .show(ui);
        for v in &p.values {
            mono(ui, v);
        }
        if let Some(note) = &p.note {
            Text::new(note.clone()).shade(Shade::Muted).show(ui);
        }
    });
}

/// What the panel reads from the document and the tool for its rows of numbers ([`Rows::of`]).
struct Shown<'a> {
    units: Unit,
    /// The rulers' zero point: X and Y count from it.
    origin: Point,
    /// The document point under the pointer.
    pointer: Point,
    /// The sides and the center of the selection's bounding box.
    size: Option<(f64, f64, Point)>,
    readout: Option<&'a Readout>,
    /// Show Options is on.
    options: bool,
}

/// The panel's rows of numbers: a left and a right column, row by row.
#[derive(Debug, Default, PartialEq)]
struct Rows {
    left: Vec<String>,
    right: Vec<String>,
}

impl Rows {
    /// X and Y are the pointer's position, and during a Rotate, Reflect or Shear tool drag the
    /// center of the selection's bounding box. W and H are the selection's size, the change in x
    /// and y while a drag moves the selection or draws a gradient vector (with the distance and the
    /// angle under them), and the percentages of a Scale tool drag. The transform tools' angles
    /// take their place during their drags.
    fn of(s: &Shown) -> Self {
        let len = |v: f64| s.units.format(v);
        let at = match s.readout {
            Some(Readout::Rotate { center, .. } | Readout::Reflect { center, .. } | Readout::Shear { center, .. }) => *center,
            _ => s.pointer,
        };
        let mut rows = Rows { left: vec![format!("X: {}", len(at.x - s.origin.x)), format!("Y: {}", len(at.y - s.origin.y))], right: vec![] };
        match s.readout {
            Some(r @ Readout::Drag { dx, dy }) => {
                rows.right = vec![format!("W: {}", len(*dx)), format!("H: {}", len(*dy))];
                let (d, a) = r.distance_angle().unwrap_or_default();
                rows.left.push(format!("D: {}", len(d)));
                rows.right.push(format!("A: {a:.1}°"));
            }
            Some(Readout::Scale { sx, sy }) => rows.right = vec![format!("W: {:.1}%", sx * 100.0), format!("H: {:.1}%", sy * 100.0)],
            Some(Readout::Rotate { angle, .. } | Readout::Reflect { angle, .. }) => rows.right = vec![format!("A: {angle:.1}°")],
            Some(Readout::Shear { axis, angle, .. }) => {
                rows.right = vec![format!("A: {axis:.1}°"), format!("{} {angle:.1}°", crate::i18n::colon(tl!("Shear")))]
            }
            Some(Readout::Brush { .. }) | None => {
                let (w, h) = s.size.map(|(w, h, _)| (len(w), len(h))).unwrap_or_default();
                rows.right = vec![format!("W: {w}"), format!("H: {h}")];
                if let (true, Some((.., c))) = (s.options, s.size) {
                    let (d, a) = Readout::drag(s.pointer - c).distance_angle().unwrap_or_default();
                    rows.left.push(format!("D: {}", len(d)));
                    rows.right.push(format!("A: {a:.1}°"));
                }
            }
        }
        rows
    }
}

/// The font and size the selected objects share when every one of them is type: the first
/// object's first style, as the Character panel shows its size. None when an object is not type or
/// two of them differ.
fn shared_type(app: &VectorcraftApp) -> Option<(String, f64, f64)> {
    let st = app.session.active()?;
    let mut shared = None;
    for id in &st.selection.objects {
        let NodeKind::Text(tx) = &st.doc.node(*id)?.kind else { return None };
        let raw = tx.first_style();
        // The size in document points, as the Character panel shows it (#1034); a collapsed
        // transform has none, so the stored size stands.
        let cs = tx.effective_char_style(&raw).unwrap_or(raw);
        let this = (format!("{} {}", cs.font_family, cs.font_style), cs.size, cs.tracking);
        match &shared {
            None => shared = Some(this),
            Some(first) if *first == this => {}
            Some(_) => return None,
        }
    }
    shared
}

/// What the panel shows for the document, the tool in use and the selection.
pub(crate) fn lines(app: &mut VectorcraftApp, ctx: &egui::Context) -> InfoLines {
    let options = !app.ui.info_hide_options;
    // W and H are the sides of the bounding box the Transform panel shows, measured as Use Preview
    // Bounds says.
    let size = app.selection_box().map(|b| (b.rect.width(), b.rect.height(), b.center()));
    let readout = app.session.tool_readout();
    let rows = Rows::of(&Shown {
        units: app.session.general_unit(),
        // X and Y count from the rulers' zero point, as the rulers and the Transform panel do.
        origin: app.ruler_origin(),
        pointer: app.hover_doc.unwrap_or_default(),
        size,
        readout: readout.as_ref(),
        options,
    });
    let zoom = (app.session.tool_id() == "zoom").then(|| {
        let zoom = app.view().map_or(1.0, |v| v.zoom);
        // PLAN: assumed `crate::state::zoom_label(zoom: f64, print: bool) -> String`, kept by hub-w1.
        format!("{} {}", crate::i18n::colon(tl!("Zoom")), crate::state::zoom_label(zoom, app.session.prefs.display_print_size))
    });
    let brush = if let Some(Readout::Brush { name }) = readout { name } else { None };
    let (f, s) = current_paints(app);
    let (fill_mixed, stroke_mixed) = mixed_paints(app, ctx);
    let type_block = shared_type(app).map(|(font, size, tracking)| {
        let line = crate::i18n::fmt(
            tl!("Size: {size}  Tracking: {tracking}"),
            &[("size", &app.session.type_unit().format(size)), ("tracking", &format!("{tracking:.0}"))],
        );
        (font, line)
    });
    InfoLines {
        left: rows.left,
        right: rows.right,
        zoom,
        brush,
        fill: PaintLines::of(f, fill_mixed, options),
        stroke: PaintLines::of(s, stroke_mixed, options),
        type_block,
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let l = lines(app, ui.ctx());
    Row::new().show(ui, |ui, _| {
        for column in [&l.left, &l.right] {
            Stack::new().show(ui, |ui, _| {
                for s in column {
                    mono(ui, s);
                }
            });
        }
    });
    for s in l.zoom.iter().chain(&l.brush) {
        mono(ui, s);
    }
    Divider::horizontal().show(ui);
    paint_block(ui, &l.fill);
    paint_block(ui, &l.stroke);
    if let Some((font, size)) = &l.type_block {
        Divider::horizontal().show(ui);
        mono(ui, font);
        mono(ui, size);
    }
}

/// The panel menu's rows.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![PanelRow::Command(CommandRef::new("ui.info.options"))]
}

/// The title of Show Options / Hide Options.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let title = if app.ui.info_hide_options { tl!("Show Options") } else { tl!("Hide Options") };
    states.set(CommandRef::new("ui.info.options"), CommandState::new().title(title));
}

/// Runs the panel menu's own rows; `None` for an id that is not one of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.info.options" => {
            app.ui.info_hide_options = !app.ui.info_hide_options;
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_appearance::{app_with_rect, run};
    use super::*;
    use serde_json::json;
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// Every line the panel shows.
    fn texts(app: &mut VectorcraftApp) -> Vec<String> {
        let l = lines(app, &crate::testing::ui_context());
        let mut out: Vec<String> = l.left.into_iter().chain(l.right).chain(l.zoom).chain(l.brush).collect();
        for p in [l.fill, l.stroke] {
            out.extend(p.values);
            out.extend(p.note);
        }
        out.extend(l.type_block.into_iter().flat_map(|(font, size)| [font, size]));
        out
    }

    fn view() -> ViewInfo {
        ViewInfo { smart_guides: false, snap_to_point: false, ..Default::default() }
    }

    fn pointer(app: &mut VectorcraftApp, kind: PointerKind, x: f64, y: f64) {
        app.session.pointer(&PointerEvent::new(kind, x, y), view()).unwrap();
    }

    fn shown(readout: Option<&Readout>, options: bool) -> Rows {
        let size = Some((100.0, 60.0, Point::new(100.0, 80.0)));
        Rows::of(&Shown { units: Unit::Points, origin: Point::new(10.0, 20.0), pointer: Point::new(130.0, 40.0), size, readout, options })
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// #1882: scaled type shows its size in document points, as the Character panel does (#1034).
    #[test]
    fn scaled_type_shows_its_scaled_size() {
        let mut app = app_with_rect();
        let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 12}))["id"].clone();
        run(&mut app, "select.set", json!({ "ids": [id] }));
        run(&mut app, "object.scale", json!({"sx": 200}));
        let size = lines(&mut app, &crate::testing::ui_context()).type_block.map(|(_, size)| size);
        assert_eq!(size.as_deref(), Some("Size: 24 pt  Tracking: 0"));
    }

    #[test]
    fn the_rows_follow_the_tool_in_use() {
        // No tool readout: the pointer from the zero point, the selection's size, and with Show
        // Options the pointer's distance and angle from the selection's center.
        let idle = shown(None, true);
        assert_eq!(idle.left, strings(&["X: 120 pt", "Y: 20 pt", "D: 50 pt"]));
        assert_eq!(idle.right, strings(&["W: 100 pt", "H: 60 pt", "A: 53.1°"]));
        assert_eq!(shown(None, false), Rows { left: strings(&["X: 120 pt", "Y: 20 pt"]), right: strings(&["W: 100 pt", "H: 60 pt"]) });
        // The Paintbrush leaves the numbers as they are.
        assert_eq!(shown(Some(&Readout::Brush { name: None }), true), idle);
        // A drag: the change in x and y, the distance and the angle, with Show Options off too.
        let drag = shown(Some(&Readout::Drag { dx: -30.0, dy: 40.0 }), false);
        assert_eq!(drag.left, strings(&["X: 120 pt", "Y: 20 pt", "D: 50 pt"]));
        assert_eq!(drag.right, strings(&["W: -30 pt", "H: 40 pt", "A: -126.9°"]));
        let scale = shown(Some(&Readout::Scale { sx: 1.5, sy: -0.25 }), true);
        assert_eq!(scale.left, strings(&["X: 120 pt", "Y: 20 pt"]));
        assert_eq!(scale.right, strings(&["W: 150.0%", "H: -25.0%"]));
        // Rotate, Reflect and Shear: the center of the selection's box and the angles.
        let center = Point::new(60.0, 70.0);
        let rotate = shown(Some(&Readout::Rotate { center, angle: 30.0 }), true);
        assert_eq!(rotate, Rows { left: strings(&["X: 50 pt", "Y: 50 pt"]), right: strings(&["A: 30.0°"]) });
        assert_eq!(shown(Some(&Readout::Reflect { center, angle: 45.0 }), true).right, strings(&["A: 45.0°"]));
        let shear = shown(Some(&Readout::Shear { center, axis: 90.0, angle: -12.5 }), true);
        assert_eq!(shear, Rows { left: strings(&["X: 50 pt", "Y: 50 pt"]), right: strings(&["A: 90.0°", "Shear: -12.5°"]) });
    }

    #[test]
    fn w_and_h_follow_use_preview_bounds() {
        let mut app = app_with_rect();
        run(&mut app, "stroke.set", json!({"weight": 10}));
        let has = |app: &mut VectorcraftApp, s: &str| texts(app).iter().any(|t| t == s);
        assert!(has(&mut app, "W: 100 pt") && has(&mut app, "H: 100 pt"), "{:?}", texts(&mut app));
        run(&mut app, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
        assert!(has(&mut app, "W: 110 pt") && has(&mut app, "H: 110 pt"), "{:?}", texts(&mut app));
    }

    #[test]
    fn a_move_shows_the_change_until_the_release() {
        let mut app = app_with_rect();
        app.session.select_tool("selection", view()).unwrap();
        pointer(&mut app, PointerKind::Down, 60.0, 60.0);
        pointer(&mut app, PointerKind::Drag, 90.0, 20.0);
        let during = texts(&mut app);
        for want in ["W: 30 pt", "H: -40 pt", "D: 50 pt", "A: 53.1°"] {
            assert!(during.iter().any(|t| t == want), "{want}: {during:?}");
        }
        pointer(&mut app, PointerKind::Up, 90.0, 20.0);
        let after = texts(&mut app);
        assert!(after.iter().any(|t| t == "W: 100 pt") && !after.iter().any(|t| t == "W: 30 pt"), "{after:?}");
    }

    #[test]
    fn several_objects_show_only_the_paint_they_share() {
        let mut app = app_with_rect();
        run(&mut app, "paint.setFill", json!({"color": "#ff0000"}));
        let other = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 40, "height": 40}))["id"].clone();
        run(&mut app, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut app, "select.all", json!({}));
        assert!(texts(&mut app).iter().any(|t| t == "R: 255"), "{:?}", texts(&mut app));
        run(&mut app, "select.set", json!({ "ids": [other] }));
        run(&mut app, "paint.setFill", json!({"color": "#00ff00"}));
        assert!(texts(&mut app).iter().any(|t| t == "G: 255"));
        run(&mut app, "select.all", json!({}));
        let all = texts(&mut app);
        assert!(!all.iter().any(|t| t == "G: 255" || t == "R: 255"), "{all:?}");
    }

    #[test]
    fn show_options_names_a_tint_and_a_gradient_swatch() {
        let tint = Paint::Solid { color: Color::rgb8(255, 128, 128), swatch: Some("Brand".into()), tint: 0.4 };
        assert_eq!(paint_name(&tint), Some("Brand 40%".into()));
        let full = Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Brand".into()), tint: 1.0 };
        assert_eq!(paint_name(&full), None);
        assert_eq!(paint_name(&Paint::None), None);
        let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
        assert_eq!(paint_name(&Paint::Gradient(Box::new(g.clone()))), None);
        g.swatch = Some("Sunset".into());
        assert_eq!(paint_name(&Paint::Gradient(Box::new(g))), Some("Sunset".into()));
    }
}
