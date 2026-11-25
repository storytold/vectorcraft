//! Shape size dialogs (a click with a shape tool): Rectangle, Rounded Rectangle, Ellipse, Polygon,
//! Star, Line Segment, Arc and Spiral tool options, and the two grids (size, divider counts and skews, switches).
//! The Line Segment, Arc and grid dialogs are also their tools' Tool Options (a double-click on the
//! tool, [`open_options`]): OK then sets the options the tool's next drag and click use, and draws
//! nothing. Arc Segment Tool Options also shows a picture of the arc its fields describe.

// PLAN: `mod tool_options;` in dialogs.rs and the `shapes::open` arms of `open_tool_dialog` in dialogs/tools.rs
// belong to object-dialogs-w5.
use craft::behavior::Mixed;
use craft::ui::contract::{Bind, Show as _, SideLabel};
use craft::ui::dialog::dialog_id;
use craft::ui::display::{Picture, PictureSize};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::style::DataColor;
use craft::ui::text::{LinkMode, LinkedFields, LinkedLayout};
use craft::ui::value_drag::SliderScale;
use serde_json::{Map, Value, json};
use vectorcraft_geom::{Point, Rect};

use super::form;
use super::host::open_held;
use super::tool_options::{length_of, reference_index, reference_param, set_options};
use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;

/// The tools whose Tool Options are this dialog.
pub(crate) const OPTION_TOOLS: [&str; 4] = ["rectangularGrid", "polarGrid", "arc", "lineSegment"];

/// The click dialogs whose Width and Height share a Constrain Width and Height Proportions link
/// (<https://krankykids.com/cheatsheets/illustrator_cs6/shape_tool_rectangle.html>,
/// <https://www.krankykids.com/cheatsheets/illustrator_cs6/shape_tool_ellipse.html>): the same
/// link as the Transform panel's, and the next of these dialogs opens with it as this one left it.
const SIZE_KINDS: [&str; 3] = ["rectangle", "roundedRectangle", "ellipse"];

/// Where a shape tool was clicked: the dialog's shape is drawn there, and with `alt` held a box shape
/// is centred on it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Click {
    /// The click, in document points.
    pub at: Point,
    /// Whether Alt was held (draw from the centre).
    pub alt: bool,
}

/// The title of the `kind` dialog; `tool_options` for the dialog of the tool's Tool Options.
fn title(kind: &str, tool_options: bool) -> &'static str {
    match kind {
        "rectangle" => tl!("Rectangle"),
        "roundedRectangle" => tl!("Rounded Rectangle"),
        "ellipse" => tl!("Ellipse"),
        "polygon" => tl!("Polygon"),
        "star" => tl!("Star"),
        "spiral" => tl!("Spiral"),
        "rectangularGrid" if tool_options => tl!("Rectangular Grid Tool Options"),
        "rectangularGrid" => tl!("Rectangular Grid"),
        "polarGrid" if tool_options => tl!("Polar Grid Tool Options"),
        "polarGrid" => tl!("Polar Grid"),
        "arc" => tl!("Arc Segment Tool Options"),
        _ => tl!("Line Segment Tool Options"),
    }
}

/// Open `tool`'s Tool Options (one of [`OPTION_TOOLS`]), as a double-click on the tool does
/// (<https://helpx.adobe.com/illustrator/using/tool-techniques/rectangular-grid-tool.html>,
/// <https://helpx.adobe.com/illustrator/using/tool-techniques/arc-tool.html>): the click dialog's
/// fields at the tool's options, with no click to draw at.
pub(crate) fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !OPTION_TOOLS.contains(&tool) {
        return Err(format!("the {tool} tool has no options"));
    }
    open(app, tool, None);
    Ok(json!({ "dialog": tool }))
}

/// Opens the `kind` dialog on the tool's options: a click dialog that draws at `click`, or with no click the
/// tool's Tool Options, which draw nothing. Without a window it reports that on the status line.
pub(crate) fn open(app: &mut VectorcraftApp, kind: &str, click: Option<Click>) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status(format!("no window to show the {kind} dialog"));
        return;
    };
    let lengths = general_lengths(app);
    let mut values = start_values(app, kind);
    if SIZE_KINDS.contains(&kind) {
        values.insert("constrain".into(), json!(app.object_dialogs.shape_link));
    }
    // The click is among the fields (no row shows it), where OK reads it.
    if let Some(click) = click {
        values.insert("x".into(), json!(click.at.x));
        values.insert("y".into(), json!(click.at.y));
    }
    let ink = (kind == "arc" && click.is_none()).then(|| ArcInk::of(&app.session));
    let params = shape_params(kind, &values, lengths, ink);
    let fields = form::fields_of(&params, &values);
    let (key, title) = (kind.to_owned(), title(kind, click.is_none()));
    let dialog = HostedParamDialog::new(&ctx, kind, fields, move |fields: &mut DialogFields| {
        ParamDialog::new(key.clone(), title, params.clone()).values(Bind::bound(fields))
    });
    let kind_owned = kind.to_owned();
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| confirm(app, &kind_owned, click, &result.fields);
    if open_held(app, dialog, ok).is_none() {
        app.status(format!("no window to show the {kind} dialog"));
    }
}

/// The values the `kind` dialog opens with: the tool's options (and General › Corner Radius for the Rounded
/// Rectangle), as numbers in points. A key with no value is left out, so its row opens on its default.
fn start_values(app: &VectorcraftApp, kind: &str) -> Map<String, Value> {
    let o = app.session.tool_options_of(kind);
    let kept = |key: &str, default: Value| o.get(key).cloned().unwrap_or(default);
    let size = |key: &str, default: f64| json!(o.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(default));
    let flag = |key: &str, default: bool| o.get(key).and_then(Value::as_bool).unwrap_or(default);
    let values = match kind {
        "rectangle" => json!({"width": 100.0, "height": 100.0}),
        "ellipse" => json!({"width": 100.0, "height": 100.0, "anchors": kept("anchors", Value::Null)}),
        // The radius starts at General › Corner Radius, which OK sets to the radius typed.
        "roundedRectangle" => json!({"width": 100.0, "height": 100.0, "radius": app.session.prefs.corner_radius}),
        "polygon" => json!({"radius": 50.0, "sides": kept("sides", Value::Null)}),
        "star" => json!({"radius1": 50.0, "radius2": 25.0, "points": kept("points", Value::Null)}),
        "lineSegment" => json!({"length": size("length", 100.0), "angle": size("angle", 0.0), "fillLine": flag("fillLine", false)}),
        "spiral" => json!({
            "radius": 50.0,
            "decay": kept("decay", json!(80.0)),
            "segments": kept("segments", json!(10)),
            "clockwise": flag("clockwise", true),
        }),
        "arc" => json!({
            "lengthX": size("lengthX", 100.0),
            "lengthY": size("lengthY", 100.0),
            "reference": kept("reference", json!(0)),
            "arcType": if flag("closed", false) { "closed" } else { "open" },
            "baseAlong": kept("baseAlong", json!("x")),
            "slope": kept("slope", json!(50)),
            "fillArc": flag("fillArc", false),
        }),
        // The grids: the tool's stored options (Default Size, Reference Point, divider counts and skews, switches).
        "rectangularGrid" | "polarGrid" => {
            let mut grid = o.as_object().cloned().unwrap_or_default();
            grid.insert("width".into(), size("width", 100.0));
            grid.insert("height".into(), size("height", 100.0));
            grid.entry("reference").or_insert(json!(0));
            Value::Object(grid)
        }
        _ => Value::Null,
    };
    let mut values = object(values).unwrap_or_default();
    values.retain(|_, value| !value.is_null());
    values
}

/// The rows of the `kind` dialog over its start `values`; `ink` is the arc picture's, for the Arc's Tool Options.
fn shape_params(kind: &str, values: &Map<String, Value>, lengths: LengthUnits, ink: Option<ArcInk>) -> Vec<Param> {
    match kind {
        "rectangle" | "roundedRectangle" | "ellipse" => {
            let mut params = vec![size_param(lengths, kind == "ellipse")];
            match kind {
                "roundedRectangle" => params.push(Param::number("radius", tl!("Radius:"), 0.0, 1.0, 3).length(lengths)),
                "ellipse" => params.push(Param::new("anchors", tl!("Anchors:"), ParamKind::int(None, None, 4))),
                _ => {}
            }
            params
        }
        "arc" => arc_params(lengths, ink),
        _ => {
            let mut params = Vec::new();
            if matches!(kind, "rectangularGrid" | "polarGrid") {
                params.push(reference_param("reference", ""));
            }
            params.extend(form::grid_params(kind, values, lengths));
            let switches = match kind {
                "spiral" => vec![("clockwise", tl!("Clockwise"), true)],
                "lineSegment" => vec![("fillLine", tl!("Fill Line"), false)],
                "rectangularGrid" => vec![("frame", tl!("Use Outside Rectangle As Frame"), true), ("fill", tl!("Fill Grid"), false)],
                "polarGrid" => vec![("compound", tl!("Create Compound Path From Ellipses"), false), ("fill", tl!("Fill Grid"), false)],
                _ => Vec::new(),
            };
            params.extend(switches.into_iter().map(|(key, label, on)| Param::new(key, label, ParamKind::bool(on))));
            params
        }
    }
}

/// The Arc's rows; with `ink` (Tool Options) the picture of the arc they describe comes last.
fn arc_params(lengths: LengthUnits, ink: Option<ArcInk>) -> Vec<Param> {
    let contextual = |context: &str, text: &'static str| Label::from(crate::i18n::tr_ctx(crate::i18n::current(), context, text));
    let mut params = vec![
        reference_param("reference", ""),
        Param::number("lengthX", tl!("Length X-Axis:"), 100.0, 1.0, 3).length(lengths),
        Param::number("lengthY", tl!("Length Y-Axis:"), 100.0, 1.0, 3).length(lengths),
        Param::new(
            "arcType",
            tl!("Type:"),
            ParamKind::choice(
                vec![("open".to_owned(), contextual("shape", "Open")), ("closed".to_owned(), contextual("shape", "Closed"))],
                Some("open".to_owned()),
            ),
        ),
        Param::new(
            "baseAlong",
            tl!("Base Along:"),
            ParamKind::choice(
                vec![("x".to_owned(), contextual("axis", "X Axis")), ("y".to_owned(), contextual("axis", "Y Axis"))],
                Some("x".to_owned()),
            ),
        ),
        Param::new("slope", tl!("Slope:"), ParamKind::range(-100.0, 100.0, 50.0, SliderScale::Linear))
            .marks_labelled([(-100.0, tl!("Concave")), (100.0, tl!("Convex"))]),
        Param::new("fillArc", tl!("Fill Arc"), ParamKind::bool(false)),
    ];
    if let Some(ink) = ink {
        params.push(Param::custom("preview", tl!("Preview"), move |ui, fields, _| arc_picture(ui, fields, ink)).full_width());
    }
    params
}

/// The Width and Height row of the box shapes with its link; `copy` (the Ellipse) makes the Height caption a link
/// that copies Width into Height.
fn size_param(lengths: LengthUnits, copy: bool) -> Param {
    Param::custom("size", "", move |ui, fields, _| size_row(ui, fields, lengths, copy)).companions(["width", "height", "constrain"])
}

/// Width and Height (in points) with the link beside the two rows. While the link is on, typing one side sets
/// the other, so the shape keeps the proportions it had when the link was turned on.
fn size_row(ui: &mut egui::Ui, fields: &mut DialogFields, lengths: LengthUnits, copy: bool) -> egui::Response {
    let mut parts = ["width", "height"].map(|key| Mixed::Same(Some(length_of(fields, key, 100.0))));
    let mut link = fields.bool("constrain").unwrap_or(false);
    let mut row = LinkedFields::new(tl!("Size"), Bind::bound(&mut parts), Bind::bound(&mut link))
        .mode(LinkMode::Ratio)
        .layout(LinkedLayout::Column)
        .lengths(lengths)
        .captions([tl!("Width:"), tl!("Height:")]);
    if copy {
        // PLAN: assumed `LinkedFields::caption_with(i: usize, SideLabel) -> Self` and `LinkedOutput::caption_link: Option<usize>`
        // (widgets-w2, row requested in fix/requests/sheets-widgets-1.md).
        row = row.caption_with(1, SideLabel::new().link().tooltip(tl!("Copy Width to Height")));
    }
    let out = row.show(ui);
    for (key, part) in ["width", "height"].into_iter().zip(parts) {
        if let Some(side) = part.same().copied().flatten().filter(|side| fields.number(key) != Some(*side)) {
            fields.set(key, side);
        }
    }
    if fields.bool("constrain") != Some(link) {
        fields.set("constrain", link);
    }
    // Height takes the Width the dialog holds (help page "Draw ellipses",
    // <https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=140>);
    // a Width that does not read leaves Height as it is.
    if out.caption_link == Some(1) {
        let width = length_of(fields, "width", f64::NAN);
        if width.is_finite() {
            fields.set("height", width);
        }
    }
    out.edit.response.clone()
}

/// The Line Segment dialog's length and angle (degrees), a negative length turned into the same
/// line drawn the other way: -80 pt at 0° is 80 pt at 180°. The line a click draws and the options
/// OK keeps both use it, so the next line goes the way this one went.
fn line_length_angle(fields: &DialogFields) -> (f64, f64) {
    let (length, angle) = (length_of(fields, "length", 100.0), fields.number("angle").unwrap_or(0.0));
    if length < 0.0 { (-length, (angle + 180.0).rem_euclid(360.0)) } else { (length, angle) }
}

/// The Arc's Base Along: "x" or "y" ("x" when the dialog holds none).
fn base_along(fields: &DialogFields) -> &str {
    fields.text("baseAlong").unwrap_or("x")
}

/// The Arc tool's options from its dialog's fields.
fn arc_options(fields: &DialogFields) -> Value {
    json!({
        "closed": fields.text("arcType").is_some_and(|t| t.eq_ignore_ascii_case("closed")),
        "lengthX": length_of(fields, "lengthX", 100.0).abs(),
        "lengthY": length_of(fields, "lengthY", 100.0).abs(),
        "reference": reference_index(fields, "reference"),
        "baseAlong": base_along(fields),
        "slope": fields.number("slope").unwrap_or(50.0),
        "fillArc": fields.bool("fillArc").unwrap_or(false),
    })
}

/// The grids' divider counts and skews: (rows / concentric, columns / radial).
fn grid_keys(kind: &str) -> ([&'static str; 2], [&'static str; 2]) {
    if kind == "polarGrid" { (["concentric", "radial"], ["concentricSkew", "radialSkew"]) } else { (["rows", "columns"], ["rowSkew", "columnSkew"]) }
}

/// A grid's options from its dialog: the switches as they are, the typed fields as numbers, its Default Size
/// and Reference Point.
fn grid_options(kind: &str, fields: &DialogFields) -> Map<String, Value> {
    let (counts, skews) = grid_keys(kind);
    let mut options = Map::new();
    for (count, skew) in counts.into_iter().zip(skews) {
        options.insert(count.into(), json!((fields.number(count).unwrap_or(5.0) as u64).min(999)));
        options.insert(skew.into(), json!(fields.number(skew).unwrap_or(0.0)));
    }
    for key in [if kind == "polarGrid" { "compound" } else { "frame" }, "fill"] {
        options.insert(key.into(), json!(fields.bool(key).unwrap_or(false)));
    }
    options.insert("width".into(), json!(length_of(fields, "width", 100.0)));
    options.insert("height".into(), json!(length_of(fields, "height", 100.0)));
    options.insert("reference".into(), json!(reference_index(fields, "reference")));
    options
}

/// The margin around the arc in Arc Segment Tool Options' picture.
const ARC_PREVIEW_MARGIN: f32 = 12.0;

/// The arc the Arc tool draws with the options OK sets from `fields` ([`arc_options`]; a slope past -100
/// or 100, which OK refuses, is drawn at that end, and a Base Along other than Y along X), as `shape.arc` draws it
/// with its reference point at the origin and General › Constrain Angle `constrain` (degrees)
/// turning it about that point. `None` when a length or the slope is not finite.
fn arc_preview_path(fields: &DialogFields, constrain: f64) -> Option<vectorcraft_geom::path::PathData> {
    let o = arc_options(fields);
    let num = |key: &str| o[key].as_f64().filter(|v| v.is_finite());
    let (lx, ly, slope) = (num("lengthX")?, num("lengthY")?, num("slope")?.clamp(-100.0, 100.0));
    let base_x = !o["baseAlong"].as_str().is_some_and(|b| b.eq_ignore_ascii_case("y"));
    let closed = o["closed"].as_bool().unwrap_or(false);
    let path = vectorcraft_geom::shapes::arc_segment(Point::ZERO, reference_index(fields, "reference"), lx, ly, base_x, slope, closed);
    Some(match vectorcraft_geom::constrain_rotation(constrain) {
        Some(rotation) => path.transformed(vectorcraft_geom::rotate_deg(rotation)),
        None => path,
    })
}

/// The arc in the dialog's picture, in screen points: its outline's points, whether it is closed,
/// and where its reference point sits.
struct ArcPreview {
    outline: Vec<egui::Pos2>,
    closed: bool,
    reference: egui::Pos2,
}

/// The arc of [`arc_preview_path`] fitted into `rect` with [`ARC_PREVIEW_MARGIN`] around it,
/// keeping its proportions. `None` when the fields describe no arc or the fitted points are not
/// finite.
fn arc_preview_shape(fields: &DialogFields, constrain: f64, rect: egui::Rect) -> Option<ArcPreview> {
    let path = arc_preview_path(fields, constrain)?;
    let b = path.bounds()?.union_pt(Point::ZERO);
    let (room_w, room_h) = (f64::from(rect.width() - 2.0 * ARC_PREVIEW_MARGIN), f64::from(rect.height() - 2.0 * ARC_PREVIEW_MARGIN));
    // A side far thinner than the other (a length of 0) does not limit the scale.
    let extent = b.width().max(b.height());
    let fit = |side: f64, room: f64| if side > extent * 1e-6 { room / side } else { f64::INFINITY };
    let scale = fit(b.width(), room_w).min(fit(b.height(), room_h));
    if !(scale.is_finite() && scale > 0.0 && room_w > 0.0 && room_h > 0.0) {
        return None;
    }
    let c = rect.center();
    let to_screen = vectorcraft_geom::Affine::translate((f64::from(c.x), f64::from(c.y)))
        * vectorcraft_geom::Affine::scale(scale)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    let pos = |p: Point| egui::pos2(p.x as f32, p.y as f32);
    let mut outline = Vec::new();
    vectorcraft_geom::flatten(path.transformed(to_screen).to_bezpath(), 0.25, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) => outline.push(pos(p)),
        _ => {}
    });
    // A closed arc's last segment ends where it starts; the outline keeps its corner as the last point.
    let closed = path.is_closed();
    if closed && outline.len() > 2 && outline.first() == outline.last() {
        outline.pop();
    }
    let reference = pos(to_screen * Point::ZERO);
    let finite = |p: &egui::Pos2| p.x.is_finite() && p.y.is_finite();
    (outline.len() >= 2 && outline.iter().all(finite) && finite(&reference)).then_some(ArcPreview { outline, closed, reference })
}

/// A paint as one color for the arc picture: a solid color as it is, a gradient or pattern gray.
fn arc_preview_color(paint: &vectorcraft_color::Paint) -> Option<egui::Color32> {
    match paint {
        vectorcraft_color::Paint::None => None,
        vectorcraft_color::Paint::Solid { color, tint, .. } => Some(crate::panels::c32(&color.tinted(*tint))),
        _ => Some(egui::Color32::from_gray(160)),
    }
}

/// What the arc picture draws in, read when the dialog opens: the current fill and stroke (black when there is
/// no stroke, as `shape.arc` draws it) and General › Constrain Angle, which turns the arc.
#[derive(Clone, Copy)]
struct ArcInk {
    fill: Option<egui::Color32>,
    stroke: egui::Color32,
    constrain: f64,
}

impl ArcInk {
    fn of(session: &vectorcraft_engine::Session) -> Self {
        ArcInk {
            fill: arc_preview_color(&session.paint.fill),
            stroke: arc_preview_color(&session.paint.stroke).unwrap_or(egui::Color32::BLACK),
            constrain: session.prefs.constrain_angle,
        }
    }
}

/// The arc Arc Segment Tool Options sets, drawn below its fields on white and redrawn on every pass from the
/// fields as they are then: in the current stroke, filled with the current fill when Fill Arc is on, turned by
/// General › Constrain Angle, with a mark on its reference point.
fn arc_picture(ui: &mut egui::Ui, fields: &DialogFields, ink: ArcInk) -> egui::Response {
    let filled = fields.bool("fillArc").unwrap_or(false);
    Picture::painter(tl!("Preview"), ThumbAspect::new(4, 3), |cx, rect| {
        cx.with_draw(|draw| {
            let data = DataColor::new;
            draw.data_fill(&PaintShape::Rect(rect, egui::CornerRadius::ZERO), data(egui::Color32::WHITE));
            let Some(arc) = arc_preview_shape(fields, ink.constrain, rect) else { return };
            // A closed arc is seen whole from its corner (its last point), and an open arc closed by its chord.
            if let Some(fill) = ink.fill.filter(|_| filled) {
                draw.data_fill(&PaintShape::Polygon(arc.outline.clone()), data(fill));
            }
            let line = if arc.closed { PaintShape::Polygon(arc.outline) } else { PaintShape::Path(arc.outline) };
            draw.stroke(&line, StrokeWidth::Medium, data(ink.stroke));
            let mark = PaintShape::Circle(arc.reference, 3.0);
            draw.data_fill(&mark, data(egui::Color32::WHITE));
            draw.stroke(&mark, StrokeWidth::Medium, data(ink.stroke));
        });
    })
    .extent(PictureSize::Sample(ThumbSize::Large))
    .show(ui)
}

/// The Ellipse dialog's Anchors: the nearest whole number from 2 to 1000 (4, a live ellipse, when
/// left empty or not a number).
fn ellipse_anchors(fields: &DialogFields) -> u64 {
    let n = fields.number("anchors").unwrap_or(4.0);
    if n.is_finite() { n.round().clamp(2.0, 1000.0) as u64 } else { 4 }
}

/// The top-left corner of a rectangle or ellipse of `width` × `height`: the click itself, or with Alt held (draw
/// from centre) the click minus half the size.
fn box_origin(click: Click, width: f64, height: f64) -> (f64, f64) {
    if click.alt { (click.at.x - width / 2.0, click.at.y - height / 2.0) } else { (click.at.x, click.at.y) }
}

/// The top-left corner of a grid of `width` × `height` whose reference point `reference` sits on the click.
fn placed(click: Click, reference: usize, width: f64, height: f64) -> Point {
    let at = vectorcraft_geom::reference_point(Rect::new(0.0, 0.0, width, height), reference);
    Point::new(click.at.x - at.x, click.at.y - at.y)
}

/// A JSON object's members; `None` for any other value.
fn object(value: Value) -> Option<Map<String, Value>> {
    match value {
        Value::Object(members) => Some(members),
        _ => None,
    }
}

/// The options OK keeps for the tool of the `kind` dialog, from its fields; `None` for a kind that keeps none.
fn option_values(kind: &str, fields: &DialogFields) -> Option<Map<String, Value>> {
    let count = |key: &str, default: f64, min: f64| fields.number(key).unwrap_or(default).clamp(min, 1000.0) as u64;
    let options = match kind {
        "polygon" => json!({"sides": count("sides", 6.0, 3.0)}),
        "star" => json!({"points": count("points", 5.0, 3.0)}),
        "ellipse" => json!({"anchors": ellipse_anchors(fields)}),
        "lineSegment" => {
            let (length, angle) = line_length_angle(fields);
            json!({"length": length, "angle": angle, "fillLine": fields.bool("fillLine").unwrap_or(false)})
        }
        "arc" => arc_options(fields),
        "rectangularGrid" | "polarGrid" => Value::Object(grid_options(kind, fields)),
        _ => return None,
    };
    object(options)
}

/// OK of the `kind` dialog: Tool Options set the tool's options and draw nothing; a click dialog draws at the
/// click, then keeps the options and General › Corner Radius. Every refusal comes before the draw, so a refused OK
/// draws nothing and the dialog stays open for a second OK.
fn confirm(app: &mut VectorcraftApp, kind: &str, click: Option<Click>, fields: &DialogFields) -> Result<(), String> {
    let options = option_values(kind, fields);
    let Some(click) = click else {
        return match options {
            Some(options) if OPTION_TOOLS.contains(&kind) => set_options(app, kind, options),
            _ => Err(format!("the {kind} tool has no options")),
        };
    };
    let corner_radius = if kind == "roundedRectangle" { Some(rounded_radius(app, fields)?) } else { None };
    let at = |key, clicked| fields.number(key).unwrap_or(clicked);
    let click = Click { at: Point::new(at("x", click.at.x), at("y", click.at.y)), ..click };
    let (x, y) = (click.at.x, click.at.y);
    let (width, height) = (length_of(fields, "width", 100.0), length_of(fields, "height", 100.0));
    let (bx, by) = box_origin(click, width, height);
    let (id, params) = match kind {
        "ellipse" => ("shape.ellipse", json!({"x": bx, "y": by, "width": width, "height": height, "anchors": ellipse_anchors(fields)})),
        "polygon" => (
            "shape.polygon",
            json!({"cx": x, "cy": y, "radius": length_of(fields, "radius", 50.0), "sides": fields.number("sides").unwrap_or(6.0) as u64}),
        ),
        "star" => (
            "shape.star",
            json!({
                "cx": x,
                "cy": y,
                "radius1": length_of(fields, "radius1", 50.0),
                "radius2": length_of(fields, "radius2", 25.0),
                "points": fields.number("points").unwrap_or(5.0) as u64,
            }),
        ),
        "spiral" => (
            "shape.spiral",
            json!({
                "cx": x,
                "cy": y,
                "radius": length_of(fields, "radius", 50.0).max(0.01),
                "decay": fields.number("decay").unwrap_or(80.0).clamp(5.0, 99.99),
                "segments": (fields.number("segments").unwrap_or(10.0) as u64).clamp(2, 1000),
                "clockwise": fields.bool("clockwise").unwrap_or(false),
            }),
        ),
        "rectangularGrid" | "polarGrid" => {
            let top_left = placed(click, reference_index(fields, "reference"), width, height);
            let mut grid = grid_options(kind, fields);
            grid.insert("x".into(), json!(top_left.x));
            grid.insert("y".into(), json!(top_left.y));
            (if kind == "polarGrid" { "shape.polarGrid" } else { "shape.rectangularGrid" }, Value::Object(grid))
        }
        "lineSegment" => {
            let (l, a) = line_length_angle(fields);
            let a = a.to_radians();
            let mut p = json!({"x1": x, "y1": y, "x2": x + l * a.cos(), "y2": y - l * a.sin()});
            if fields.bool("fillLine").unwrap_or(false) {
                p["fill"] = json!(true);
            }
            ("shape.line", p)
        }
        "arc" => (
            "shape.arc",
            json!({
                "x": x,
                "y": y,
                "lengthX": length_of(fields, "lengthX", 100.0),
                "lengthY": length_of(fields, "lengthY", 100.0),
                "reference": reference_index(fields, "reference"),
                "baseAlong": base_along(fields),
                "slope": fields.number("slope").unwrap_or(50.0),
                "closed": fields.text("arcType").is_some_and(|t| t.eq_ignore_ascii_case("closed")),
                "fillArc": fields.bool("fillArc").unwrap_or(false),
            }),
        ),
        "rectangle" | "roundedRectangle" => (
            "shape.rectangle",
            json!({
                "x": bx,
                "y": by,
                "width": width,
                "height": height,
                "radius": corner_radius.unwrap_or_else(|| length_of(fields, "radius", 0.0)),
            }),
        ),
        _ => return Err(format!("the {kind} tool has no size dialog")),
    };
    let options = options.or_else(|| {
        (kind == "spiral")
            .then(|| object(json!({"decay": params["decay"], "segments": params["segments"], "clockwise": params["clockwise"]})))
            .flatten()
    });
    // Options the tool would refuse leave the dialog open, before anything is drawn.
    if let Some(options) = &options {
        vectorcraft_tools::option_check::check_values(vectorcraft_tools::create(kind).as_ref(), options)?;
    }
    // On the active plane while the perspective grid shows, sized in plane units from the click; else along
    // General › Constrain Angle, turned about the click (a grid's reference point, the other shapes' anchor).
    let (id, params) = vectorcraft_engine::perspective_click(&app.session, id, &params, click.at)
        .unwrap_or_else(|| (id.to_owned(), vectorcraft_engine::constrain_click(&app.session, id, params, click.at)));
    app.run(&id, params)?;
    if let Some(options) = options {
        set_options(app, kind, options)?;
    }
    // The radius of the rounded rectangle drawn is General › Corner Radius from then on.
    if let Some(r) = corner_radius.filter(|r| *r != app.session.prefs.corner_radius) {
        app.run("prefs.set", json!({"key": "cornerRadius", "value": r}))?;
    }
    Ok(())
}

/// The Rounded Rectangle dialog's radius in points, checked as General › Corner Radius checks it
/// (an error leaves the dialog open and draws nothing). Without the field, the preference.
fn rounded_radius(app: &VectorcraftApp, fields: &DialogFields) -> Result<f64, String> {
    let r = if fields.contains("radius") { length_of(fields, "radius", f64::NAN) } else { app.session.prefs.corner_radius };
    vectorcraft_engine::cmd::prefscmds::validate("cornerRadius", &json!(r))?;
    Ok(r)
}

/// Keeps the link of the next size dialog as the open one leaves it: copies the open Rectangle, Rounded
/// Rectangle or Ellipse dialog's `constrain` into `ObjectDialogs::shape_link` each frame, so the link carries
/// over also when the dialog is cancelled.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let link =
        dialogs.with(|host| SIZE_KINDS.iter().find_map(|kind| host.get_by_id::<HostedParamDialog>(dialog_id(*kind))?.fields().bool("constrain")));
    if let Some(link) = link {
        app.object_dialogs.shape_link = link;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::testing::Fixture;
    use vectorcraft_engine::Session;

    /// An app whose window's dialog host shows in a fixture, over a new 300 × 300 document.
    fn shown() -> (VectorcraftApp, Fixture) {
        // PLAN: assumed `dialogs::host::window_dialogs(ctx) -> WindowDialogs` (wg.S1, hub-w2).
        let fixture = Fixture::new(|ui| crate::dialogs::host::window_dialogs(ui.ctx()).with(|host| host.run(ui)));
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui_ctx = Some(fixture.ctx().clone());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        (app, fixture)
    }

    fn is_open(app: &VectorcraftApp, kind: &str) -> bool {
        app.dialogs().expect("a window").with(|host| host.get_by_id::<HostedParamDialog>(dialog_id(kind)).is_some())
    }

    /// The values the open `kind` dialog holds.
    fn open_fields(app: &VectorcraftApp, kind: &str) -> DialogFields {
        app.dialogs()
            .expect("a window")
            .with(|host| host.get_by_id::<HostedParamDialog>(dialog_id(kind)).map(|dialog| dialog.fields().clone()))
            .expect("the dialog is open")
    }

    /// Sets `key` of the open `kind` dialog as automation does.
    fn set(app: &VectorcraftApp, fx: &Fixture, kind: &str, key: &str, value: Value) -> Result<(), String> {
        let ctx = fx.ctx().clone();
        app.dialogs().expect("a window").with(|host| host.set_json_field(&ctx, dialog_id(kind), key, value))
    }

    /// Two frames of the window, each followed by the module pumps: the held OK is filed in the first and answered
    /// in the second.
    fn settle(app: &mut VectorcraftApp, fx: &mut Fixture) {
        let ctx = fx.ctx().clone();
        for _ in 0..2 {
            fx.frame(vec![]);
            crate::dialogs::host::pump(app, &ctx);
        }
    }

    fn press_ok(app: &mut VectorcraftApp, fx: &mut Fixture, kind: &str) {
        let ctx = fx.ctx().clone();
        // Whether OK filed its answer shows in the asserts of the trial that pressed it.
        let _ = app.dialogs().expect("a window").with(|host| host.confirm_by_id(&ctx, dialog_id(kind)));
        settle(app, fx);
    }

    fn cancel(app: &mut VectorcraftApp, fx: &mut Fixture, kind: &str) {
        let ctx = fx.ctx().clone();
        let _ = app.dialogs().expect("a window").with(|host| host.cancel_by_id(&ctx, dialog_id(kind)));
        settle(app, fx);
    }

    fn drawn(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().doc.layers[0].children().unwrap().len()
    }

    #[test]
    fn spiral_click_dialog_uses_and_updates_tool_options() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "spiral"})).unwrap();
        app.run("tool.setOption", json!({"tool": "spiral", "values": {"decay": 72.0, "segments": 14, "clockwise": false}})).unwrap();

        super::super::open_tool_dialog(&mut app, "spiral", json!({"x": 40.0, "y": 60.0}));
        let opened = open_fields(&app, "spiral");
        assert_eq!(
            (opened.number("radius"), opened.number("decay"), opened.number("segments"), opened.bool("clockwise")),
            (Some(50.0), Some(72.0), Some(14.0), Some(false))
        );
        for (key, value) in [("radius", json!(35.0)), ("decay", json!(65.0)), ("segments", json!(12)), ("clockwise", json!(true))] {
            set(&app, &fx, "spiral", key, value).unwrap();
        }
        press_ok(&mut app, &mut fx, "spiral");

        assert!(!is_open(&app, "spiral"));
        assert_eq!(drawn(&app), 1);
        assert_eq!(app.session.tool_options_of("spiral"), json!({"decay": 65.0, "segments": 12, "clockwise": true}));

        // A decay typed past 99.99 % draws the spiral at 99.99 %, the most a spiral draws, and
        // keeps that decay.
        super::super::open_tool_dialog(&mut app, "spiral", json!({"x": 40.0, "y": 60.0}));
        set(&app, &fx, "spiral", "decay", json!(120.0)).unwrap();
        press_ok(&mut app, &mut fx, "spiral");
        assert!(!is_open(&app, "spiral"));
        assert_eq!(drawn(&app), 2);
        assert_eq!(app.session.tool_options_of("spiral")["decay"], json!(99.99));
    }

    /// With General › Constrain Angle at 90°, the Spiral dialog's spiral starts straight up from the
    /// click, and the tool's options keep no turn.
    #[test]
    fn spiral_click_dialog_starts_along_the_constrain_angle() {
        let (mut app, mut fx) = shown();
        app.run("prefs.set", json!({"key": "constrainAngle", "value": 90})).unwrap();
        app.run("tool.select", json!({"tool": "spiral"})).unwrap();
        super::super::open_tool_dialog(&mut app, "spiral", json!({"x": 40.0, "y": 60.0}));
        set(&app, &fx, "spiral", "radius", json!(35.0)).unwrap();
        press_ok(&mut app, &mut fx, "spiral");

        let st = app.session.doc().unwrap();
        let id = *st.selection.objects.first().unwrap();
        let path = st.doc.node(id).unwrap().path_data().unwrap();
        let outer = path.subpaths[0].anchors.last().unwrap().p;
        assert_eq!(outer, Point::new(40.0, 25.0));
        assert!(app.session.tool_options_of("spiral").get("rotation").is_none());
    }

    /// The arc dialog's fields as the Arc's Tool Options hold them, with `fields` over them.
    fn arc_dialog(fields: Value) -> DialogFields {
        let mut d = DialogFields::new();
        for (key, value) in [("lengthX", 100.0), ("lengthY", 60.0), ("slope", 50.0)] {
            d.set(key, value);
        }
        d.set("reference", 0_i64);
        d.set("baseAlong", "x");
        d.set("arcType", "open");
        for (k, v) in fields.as_object().into_iter().flatten() {
            d.set(k.clone(), serde_json::from_value::<craft::ui::patterns::form_dialogs::FieldValue>(v.clone()).unwrap());
        }
        d
    }

    fn anchor_points(p: &vectorcraft_geom::path::PathData) -> Vec<(f64, f64, f64, f64, f64, f64)> {
        p.anchors().map(|(_, _, a)| (a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y)).collect()
    }

    /// The preview is the arc `shape.arc` draws from the same values at a click on the origin, for
    /// each Type, Base Along, reference point and slope, and turned by General › Constrain Angle.
    #[test]
    fn the_arc_preview_is_the_arc_shape_arc_draws() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        for constrain in [0.0, 30.0] {
            app.run("prefs.set", json!({"key": "constrainAngle", "value": constrain})).unwrap();
            for (closed, base, reference, slope) in
                [(false, "x", 0, 50.0), (true, "x", 4, 100.0), (false, "y", 8, -100.0), (true, "y", 2, 0.0), (false, "x", 6, -40.0)]
            {
                let d =
                    arc_dialog(json!({"arcType": if closed { "closed" } else { "open" }, "baseAlong": base, "reference": reference, "slope": slope}));
                let preview = arc_preview_path(&d, app.session.prefs.constrain_angle).expect("an arc");
                let params = json!({"x": 0.0, "y": 0.0, "lengthX": 100.0, "lengthY": 60.0, "reference": reference, "baseAlong": base, "slope": slope, "closed": closed});
                let params = vectorcraft_engine::constrain_click(&app.session, "shape.arc", params, Point::ZERO);
                app.run("shape.arc", params).unwrap();
                let doc = &app.session.doc().unwrap().doc;
                let drawn = doc.layers[0].children().unwrap().last().unwrap().path_data().unwrap().clone();
                let (a, b) = (anchor_points(&preview), anchor_points(&drawn));
                assert_eq!(a.len(), b.len(), "{constrain} {closed} {base} {reference} {slope}");
                for (p, q) in a.iter().zip(&b) {
                    let close = [(p.0, q.0), (p.1, q.1), (p.2, q.2), (p.3, q.3), (p.4, q.4), (p.5, q.5)].iter().all(|(u, v)| (u - v).abs() < 1e-6);
                    assert!(close, "{constrain} {closed} {base} {reference} {slope}: {a:?} {b:?}");
                }
                assert_eq!(preview.is_closed(), closed);
            }
        }
    }

    /// The picture follows Slope, Type, Base Along and the reference point, keeps the arc inside
    /// the frame, and stays finite for a length of 0 or a huge one.
    #[test]
    fn the_arc_preview_follows_the_fields_and_fits_its_frame() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(160.0, 120.0));
        let shape = |fields: Value| arc_preview_shape(&arc_dialog(fields), 0.0, rect).expect("an arc");
        let inside = |s: &ArcPreview| {
            let room = rect.shrink(ARC_PREVIEW_MARGIN - 0.01);
            s.outline.iter().chain([&s.reference]).all(|p| room.contains(*p))
        };
        let base = shape(json!({}));
        assert!(inside(&base), "{:?}", base.outline);
        // Slope 0 is a straight line: two points.
        assert_eq!(shape(json!({"slope": 0.0})).outline.len(), 2);
        for changed in [json!({"slope": -50.0}), json!({"arcType": "closed"}), json!({"baseAlong": "y"}), json!({"lengthY": "30 pt"})] {
            let s = shape(changed.clone());
            assert!(inside(&s), "{changed}");
            assert_ne!(s.outline, base.outline, "{changed}");
        }
        assert!(shape(json!({"arcType": "closed"})).closed && !base.closed);
        // The reference mark sits on each of the nine points of the arc's box.
        let marks: Vec<egui::Pos2> = (0..9).map(|i| shape(json!({"reference": i})).reference).collect();
        assert!((marks[4] - rect.center()).length() < 1e-3, "{marks:?}");
        for row in 0..3 {
            assert!(marks[row * 3].x < marks[row * 3 + 1].x && marks[row * 3 + 1].x < marks[row * 3 + 2].x, "{marks:?}");
            assert!(marks[row].y < marks[row + 3].y && marks[row + 3].y < marks[row + 6].y, "{marks:?}");
        }
        for lengths in [json!({"lengthX": 0.0}), json!({"lengthY": 0.0}), json!({"lengthX": 1e300, "lengthY": 1e-300}), json!({"lengthX": -80.0})] {
            assert!(inside(&shape(lengths.clone())), "{lengths}");
        }
    }

    /// A slope outside -100 to 100 or a Base Along other than X or Y is refused by its row and leaves the
    /// fields as they were; for the values the rows keep, the picture shows the arc of the options OK sets: the
    /// same picture as a dialog holding those options.
    #[test]
    fn the_arc_preview_shows_the_options_ok_sets() {
        let (mut app, mut fx) = shown();
        open_options(&mut app, "arc").unwrap();
        for (key, bad) in [("slope", json!(140.0)), ("slope", json!(-250.0)), ("baseAlong", json!("z"))] {
            assert!(set(&app, &fx, "arc", key, bad.clone()).is_err(), "{key} {bad}");
        }
        let kept = open_fields(&app, "arc");
        assert_eq!((kept.number("slope"), kept.text("baseAlong")), (Some(50.0), Some("x")));

        set(&app, &fx, "arc", "slope", json!(-40.0)).unwrap();
        set(&app, &fx, "arc", "reference", json!(6)).unwrap();
        let shown_arc = arc_preview_path(&open_fields(&app, "arc"), 0.0).expect("an arc");
        press_ok(&mut app, &mut fx, "arc");

        let kept = app.session.tool_options_of("arc");
        let fields = arc_dialog(json!({
            "lengthX": kept["lengthX"],
            "lengthY": kept["lengthY"],
            "slope": kept["slope"],
            "baseAlong": kept["baseAlong"],
            "reference": kept["reference"],
            "arcType": if kept["closed"].as_bool() == Some(true) { "closed" } else { "open" },
        }));
        let want = arc_preview_path(&fields, 0.0).expect("an arc");
        assert_eq!(anchor_points(&shown_arc), anchor_points(&want), "kept {kept}");
    }

    /// Double-clicking the Arc tool shows the picture in Arc Segment Tool Options; a click on the
    /// canvas does not. Changing the options and pressing OK draws nothing and adds no undo step.
    #[test]
    fn arc_tool_options_show_a_preview_and_ok_draws_nothing() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "arc"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        open_options(&mut app, "arc").unwrap();
        settle(&mut app, &mut fx);
        assert!(fx.by_label("Preview").is_some(), "Tool Options show the picture");
        set(&app, &fx, "arc", "slope", json!(-30.0)).unwrap();
        press_ok(&mut app, &mut fx, "arc");
        assert!(!is_open(&app, "arc"));
        assert_eq!(drawn(&app), 0);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        assert_eq!(app.session.tool_options_of("arc")["slope"], json!(-30.0));

        let (mut app, mut fx) = shown();
        open(&mut app, "arc", Some(Click { at: Point::new(40.0, 60.0), alt: false }));
        settle(&mut app, &mut fx);
        assert!(fx.by_label("Preview").is_none(), "a click dialog has no picture");
    }

    /// The Rounded Rectangle click dialog opens on General › Corner Radius, and OK makes the radius
    /// typed that preference: the next dialog and the tool's next drag use it.
    #[test]
    fn rounded_rectangle_click_dialog_opens_on_and_sets_the_corner_radius() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "roundedRectangle"})).unwrap();
        app.run("prefs.set", json!({"key": "cornerRadius", "value": 20})).unwrap();

        super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
        assert_eq!(open_fields(&app, "roundedRectangle").number("radius"), Some(20.0));
        set(&app, &fx, "roundedRectangle", "radius", json!(36.0)).unwrap();
        press_ok(&mut app, &mut fx, "roundedRectangle");

        assert!(!is_open(&app, "roundedRectangle"));
        let n = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].clone();
        assert_eq!(crate::panels::corner_radius(&app, &n), Some(36.0));
        assert_eq!(app.session.prefs.corner_radius, 36.0);
        assert_eq!(app.session.tool_options_of("roundedRectangle")["cornerRadius"], json!(36.0));
        super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
        assert_eq!(open_fields(&app, "roundedRectangle").number("radius"), Some(36.0));
    }

    /// A radius General › Corner Radius would refuse is answered with an error: the dialog stays open, nothing
    /// is drawn and the preference is kept.
    #[test]
    fn rounded_rectangle_click_dialog_refuses_a_radius_out_of_range() {
        let (mut app, mut fx) = shown();
        let before = app.session.prefs.corner_radius;
        for bad in [json!(-1.0), json!(1297.0)] {
            super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
            set(&app, &fx, "roundedRectangle", "radius", bad.clone()).unwrap();
            press_ok(&mut app, &mut fx, "roundedRectangle");
            assert!(is_open(&app, "roundedRectangle"), "{bad}");
            assert_eq!(drawn(&app), 0, "{bad}");
            assert_eq!(app.session.prefs.corner_radius, before, "{bad}");
            cancel(&mut app, &mut fx, "roundedRectangle");
        }
    }

    /// The link of a size dialog carries to the next Rectangle, Rounded Rectangle or Ellipse, on and off, also
    /// when the dialog it was turned in is cancelled.
    #[test]
    fn the_size_link_carries_to_the_next_size_dialog() {
        let (mut app, mut fx) = shown();
        let click = Some(Click { at: Point::new(10.0, 10.0), alt: false });
        open(&mut app, "rectangle", click);
        assert_eq!(open_fields(&app, "rectangle").bool("constrain"), Some(false));
        set(&app, &fx, "rectangle", "constrain", json!(true)).unwrap();
        settle(&mut app, &mut fx);
        cancel(&mut app, &mut fx, "rectangle");

        open(&mut app, "ellipse", click);
        assert_eq!(open_fields(&app, "ellipse").bool("constrain"), Some(true));
        set(&app, &fx, "ellipse", "constrain", json!(false)).unwrap();
        settle(&mut app, &mut fx);
        cancel(&mut app, &mut fx, "ellipse");

        open(&mut app, "roundedRectangle", click);
        assert_eq!(open_fields(&app, "roundedRectangle").bool("constrain"), Some(false));
    }

    /// With the link on, typing a width over a 40 × 20 rectangle's and leaving the field draws it 80 × 40.
    #[test]
    fn typing_a_width_with_the_link_on_draws_the_shape_in_proportion() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        open(&mut app, "rectangle", Some(Click { at: Point::new(10.0, 10.0), alt: false }));
        for (key, value) in [("width", json!(40.0)), ("height", json!(20.0)), ("constrain", json!(true))] {
            set(&app, &fx, "rectangle", key, value).unwrap();
        }
        settle(&mut app, &mut fx);
        let width = fx.by_label("Width:").expect("the Width field");
        fx.click(&width);
        fx.key(egui::Key::A, egui::Modifiers::COMMAND);
        fx.type_text("80 pt");
        fx.key(egui::Key::Tab, egui::Modifiers::NONE);
        settle(&mut app, &mut fx);
        press_ok(&mut app, &mut fx, "rectangle");

        assert!(!is_open(&app, "rectangle"));
        let doc = &app.session.doc().unwrap().doc;
        let b = doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
        assert!((b.width() - 80.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "{b:?}");
    }

    /// The Ellipse dialog opens with Anchors 4, which draws a live ellipse; OK with 6 draws a plain
    /// path of 6 anchors, and the next Ellipse dialog opens on 6.
    #[test]
    fn the_ellipse_dialog_draws_and_keeps_its_anchor_count() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
        let last = |app: &VectorcraftApp| {
            let doc = &app.session.doc().unwrap().doc;
            doc.layers[0].children().unwrap().last().cloned().expect("an ellipse")
        };

        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
        assert_eq!(open_fields(&app, "ellipse").number("anchors"), Some(4.0));
        press_ok(&mut app, &mut fx, "ellipse");
        let n = last(&app);
        assert!(matches!(n.live_shape(), Some(vectorcraft_doc::LiveShape::Ellipse { .. })), "{:?}", n.live_shape());

        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 150.0, "y": 150.0}));
        set(&app, &fx, "ellipse", "anchors", json!(6)).unwrap();
        press_ok(&mut app, &mut fx, "ellipse");
        let n = last(&app);
        assert!(n.live_shape().is_none());
        assert_eq!(n.path_data().unwrap().anchor_count(), 6);
        let b = n.geometric_bounds().unwrap();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6, "{b:?}");

        // Kept as the Ellipse tool's `anchors` option for the next click, after another tool too.
        assert_eq!(app.session.tool_options_of("ellipse")["anchors"], json!(6));
        app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 150.0}));
        assert_eq!(open_fields(&app, "ellipse").number("anchors"), Some(6.0));
    }

    /// Pressing the Ellipse dialog's Height caption copies Width into Height, as one undo step once OK draws the
    /// circle; Cancel draws nothing.
    #[test]
    fn clicking_height_in_the_ellipse_dialog_copies_width_into_height() {
        let (mut app, mut fx) = shown();
        let click = Some(Click { at: Point::new(10.0, 10.0), alt: false });
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let before = undo(&app);

        open(&mut app, "ellipse", click);
        cancel(&mut app, &mut fx, "ellipse");
        assert_eq!((drawn(&app), undo(&app)), (0, before));

        open(&mut app, "ellipse", click);
        set(&app, &fx, "ellipse", "width", json!(64.0)).unwrap();
        set(&app, &fx, "ellipse", "height", json!(30.0)).unwrap();
        settle(&mut app, &mut fx);
        // The caption, a link, not the Height field its name also labels.
        let height = fx
            .by_role(craft::ui::contract::Role::LINK)
            .into_iter()
            .find(|t| fx.tree().name_of(t.node) == Some("Height:"))
            .expect("the Height caption");
        fx.click(&height);
        settle(&mut app, &mut fx);
        assert_eq!(open_fields(&app, "ellipse").number("height"), Some(64.0));
        press_ok(&mut app, &mut fx, "ellipse");

        assert_eq!(undo(&app), before + 1);
        let doc = &app.session.doc().unwrap().doc;
        let b = doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
        assert!((b.width() - 64.0).abs() < 1e-6 && (b.height() - 64.0).abs() < 1e-6, "{b:?}");
    }

    /// A negative Line Segment length draws the same line the other way, and the options OK keeps are the line
    /// the other way too, in a click dialog and in Tool Options.
    #[test]
    fn a_negative_line_length_draws_and_keeps_the_same_direction() {
        let (mut app, mut fx) = shown();
        app.run("tool.select", json!({"tool": "lineSegment"})).unwrap();
        let line_bounds = |app: &VectorcraftApp, i: usize| {
            let doc = &app.session.doc().unwrap().doc;
            let n = &doc.layers[0].children().unwrap()[i];
            n.path_data().and_then(|p| p.bounds()).expect("line bounds")
        };

        super::super::open_tool_dialog(&mut app, "lineSegment", json!({"x": 100.0, "y": 60.0}));
        set(&app, &fx, "lineSegment", "length", json!(-80.0)).unwrap();
        set(&app, &fx, "lineSegment", "angle", json!(0.0)).unwrap();
        press_ok(&mut app, &mut fx, "lineSegment");
        let first = line_bounds(&app, 0);
        assert!((first.x0 - 20.0).abs() < 1e-6 && (first.x1 - 100.0).abs() < 1e-6, "{first:?}");
        let kept = app.session.tool_options_of("lineSegment");
        assert_eq!((kept["length"].as_f64(), kept["angle"].as_f64()), (Some(80.0), Some(180.0)));

        // The next click opens at what was kept and draws the same way.
        super::super::open_tool_dialog(&mut app, "lineSegment", json!({"x": 100.0, "y": 120.0}));
        press_ok(&mut app, &mut fx, "lineSegment");
        let second = line_bounds(&app, 1);
        assert!((second.x0 - 20.0).abs() < 1e-6 && (second.x1 - 100.0).abs() < 1e-6, "{second:?}");

        // Tool Options keep the same turned line and draw nothing.
        open_options(&mut app, "lineSegment").unwrap();
        set(&app, &fx, "lineSegment", "length", json!(-50.0)).unwrap();
        set(&app, &fx, "lineSegment", "angle", json!(90.0)).unwrap();
        press_ok(&mut app, &mut fx, "lineSegment");
        let kept = app.session.tool_options_of("lineSegment");
        assert_eq!((kept["length"].as_f64(), kept["angle"].as_f64()), (Some(50.0), Some(270.0)));
        assert_eq!(drawn(&app), 2);
    }
}
