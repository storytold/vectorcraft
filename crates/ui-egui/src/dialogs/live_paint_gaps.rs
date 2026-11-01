//! Object › Live Paint › Gap Options: Gap Detection, Paint Stops At (Small, Medium or Large Gaps,
//! or Custom with its size), Gap Preview Color (a layer color or a custom color) and Close Gaps
//! With Paths for the selected Live Paint groups, opened on the first group's options. With Preview
//! on, the options apply to the groups live and the gaps they find draw on the canvas in the preview
//! color; OK keeps them as one undo step (`livePaint.gapOptions`). View › Show Live Paint Gaps
//! draws the gaps of the selected groups the same way ([`selected_gaps`]).
//!
//! Fields: `detect`, `size` (`small` | `medium` | `large` | `custom`), `custom` (pt), `color`
//! (`#rrggbb`, or a preset's name such as "Red"), `closeGaps`, `preview`, `__ids` (the groups) and
//! `__gaps` (the gaps the last preview found, as `livePaint.gapOptions` returns them).

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{GapSize, LAYER_COLORS};
use vectorcraft_geom::{BezPath, Point};
use vectorcraft_tools::Overlay;

use super::tile_edge_color::resolve;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Gap Options.
pub const KIND: &str = "livePaintGapOptions";

const CMD: &str = "livePaint.gapOptions";

/// Width of the label column.
const LABEL_W: f32 = 120.0;

/// Paint Stops At: (value, label).
const SIZES: [(&str, &str); 4] = [("small", "Small Gaps"), ("medium", "Medium Gaps"), ("large", "Large Gaps"), ("custom", "Custom")];

/// The private field holding the gaps of the last preview.
const GAPS: &str = "__gaps";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Gap Options").into(), body, confirm, preview: true, min_width: 340.0, ..DialogSpec::FORM };

/// Open Gap Options on the selected Live Paint groups, filled in from the first one's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let groups: Vec<u64> = app
        .session
        .doc()
        .map(|d| {
            let doc = &d.doc;
            d.selection.objects.iter().filter(|id| doc.node(**id).is_some_and(vectorcraft_tools::builder::is_live_paint)).map(|id| id.0).collect()
        })
        .unwrap_or_default();
    let Some(&first) = groups.first() else { return Err("select a Live Paint group".into()) };
    let info = app.session.execute("livePaint.info", &json!({ "group": first })).map_err(|e| e.to_string())?;
    let o = &info["gapOptions"];
    let (size, custom) = match o["size"].as_str() {
        Some(id) => (id.to_string(), o["points"].clone()),
        None => ("custom".to_string(), o["size"].clone()),
    };
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({
            "detect": o["detect"],
            "size": size,
            "custom": custom,
            "color": o["color"],
            "closeGaps": false,
            "preview": false,
            "__ids": groups,
        }),
    ));
    Ok(Value::Null)
}

/// `livePaint.gapOptions` parameters from the fields.
fn params(d: &Dialog) -> Value {
    let size = match d.str("size").as_str() {
        "custom" => json!(d.f64("custom", GapSize::SMALL)),
        id => json!(id),
    };
    json!({
        "ids": d.fields.get("__ids").cloned().unwrap_or_default(),
        "detect": d.bool("detect"),
        "size": size,
        "color": d.str("color"),
        "closeGaps": d.bool("closeGaps"),
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::check(ui, d, "detect", tl!("Gap Detection"));
    ui.add_space(6.0);
    let size = d.str("size");
    let shown = SIZES.iter().find(|(v, _)| *v == size).map_or(SIZES[0].1, |(_, l)| l);
    let labels = SIZES.map(|(_, l)| l);
    widgets::label_row(ui, tl!("Paint Stops At:"), LABEL_W, |ui| {
        if let Some((value, _)) = widgets::dropdown(ui, "gap-size", shown, &labels, 130.0).and_then(|i| SIZES.get(i)) {
            d.fields.insert("size".into(), json!(value));
        }
        ui.add_space(6.0);
        // The custom size field is enabled while Custom is chosen.
        ui.add_enabled_ui(size == "custom", |ui| form::length(ui, d, "custom", app.session.general_unit(), 72.0));
    });
    let color = resolve(&d.str("color")).unwrap_or(Color::BLACK);
    let [r, g, b, _] = color.to_rgba8(1.0);
    let preset = LAYER_COLORS.iter().position(|(_, c)| *c == [r, g, b]);
    let names: Vec<&str> = LAYER_COLORS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    widgets::label_row(ui, tl!("Gap Preview Color:"), LABEL_W, |ui| {
        if let Some((_, [r, g, b])) =
            widgets::dropdown(ui, "gap-color", names[preset.unwrap_or(LAYER_COLORS.len())], &names, 130.0).and_then(|i| LAYER_COLORS.get(i))
        {
            d.fields.insert("color".into(), json!(Color::rgb8(*r, *g, *b).to_hex()));
        }
        // The color button sets any other color; the dropdown then shows Custom.
        let mut rgb = [r, g, b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            d.fields.insert("color".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
        }
    });
    ui.add_space(6.0);
    form::check(ui, d, "closeGaps", tl!("Close Gaps With Paths"));
    let p = params(d);
    if let Some(r) = form::preview_result(app, ui, d, "Gap Options", CMD, p) {
        d.fields.insert(GAPS.into(), r["gaps"].clone());
    }
    if !d.bool("preview") {
        d.fields.remove(GAPS);
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}

/// The gaps the open Gap Options previews, as lines in its preview color (none while Preview is
/// off or another dialog is open).
pub(crate) fn overlays(app: &VectorcraftApp) -> Vec<Overlay> {
    let Some(d) = app.ui.dialog.as_ref().filter(|d| d.kind == KIND && d.bool("preview")) else { return vec![] };
    let [r, g, b, _] = resolve(&d.str("color")).unwrap_or(Color::BLACK).to_rgba8(1.0);
    let gaps = d.fields.get(GAPS).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    gaps.iter()
        .filter_map(|gap| {
            let v: Vec<f64> = gap.as_array()?.iter().filter_map(Value::as_f64).collect();
            let [x0, y0, x1, y1] = v.as_slice() else { return None };
            Some(gap_line(Point::new(*x0, *y0), Point::new(*x1, *y1), [r, g, b]))
        })
        .collect()
}

/// View › Show Live Paint Gaps: the gaps of the selected Live Paint groups, as lines in each
/// group's Gap Preview Color, drawn as the preview draws them.
pub(crate) fn selected_gaps(app: &VectorcraftApp) -> Vec<Overlay> {
    let Some(st) = app.session.active() else { return vec![] };
    let groups = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).filter(|n| vectorcraft_tools::builder::is_live_paint(n));
    groups
        .flat_map(|n| {
            let color = n.gap_options().color;
            vectorcraft_engine::cmd::live_paint_gaps(n).into_iter().map(move |[a, b]| gap_line(a, b, color))
        })
        .collect()
}

/// A gap from `a` to `b` as a line in `color`, 2 screen points wide.
fn gap_line(a: Point, b: Point, color: [u8; 3]) -> Overlay {
    let mut path = BezPath::new();
    path.move_to(a);
    path.line_to(b);
    Overlay::Path { path, color, width: 2.0, dashed: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{GapOptions, NodeId};
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn options(app: &VectorcraftApp, g: u64) -> GapOptions {
        app.session.doc().unwrap().doc.node(NodeId(g)).unwrap().gap_options()
    }

    fn faces(app: &VectorcraftApp, g: u64) -> usize {
        vectorcraft_tools::builder::faces(app.session.doc().unwrap().doc.node(NodeId(g)).unwrap()).len()
    }

    #[test]
    fn the_menu_item_opens_gap_options_which_preview_the_gaps_and_ok_keeps_them() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        // An open path round a square, its end 2 pt short of its start.
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(x, y)| json!({"x": x, "y": y}));
        let p = app.run("path.create", json!({ "anchors": anchors })).unwrap();
        app.run("select.set", json!({"ids": [p["id"]]})).unwrap();
        // Without a Live Paint group the dialog does not open.
        assert!(app.run("ui.livePaintGapOptions", json!({})).is_err());
        let g = app.run("livePaint.make", json!({})).unwrap()["id"].as_u64().unwrap();
        // Object › Live Paint › Gap Options… runs the command that opens the dialog.
        let tree = crate::menus::menu_tree();
        let (_, object) = tree.iter().find(|(t, _)| *t == "Object").unwrap();
        let live = object.iter().find_map(|i| if let crate::menus::Item::Sub("Live Paint", items) = i { Some(items) } else { None }).unwrap();
        assert!(live.iter().any(|i| matches!(i, crate::menus::Item::Cmd("Gap Options…", "ui.livePaintGapOptions", _))), "{live:?}");
        crate::menus::invoke(&mut app, "ui.livePaintGapOptions", json!({}));
        let d = app.ui.dialog.as_ref().expect("Object › Live Paint › Gap Options… opens the dialog");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.bool("detect"), d.str("size"), d.str("color")), (false, "small".to_string(), "#ff4f4f".to_string()));
        frame(&mut app);
        assert!(overlays(&app).is_empty(), "no preview yet");
        // Preview with Gap Detection and Small Gaps: the square fills as one face and its gap shows.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("detect".into(), json!(true));
        d.fields.insert("size".into(), json!("small"));
        d.fields.insert("color".into(), json!("Green"));
        d.fields.insert("preview".into(), json!(true));
        frame(&mut app);
        assert_eq!(faces(&app, g), 1, "previewed");
        let shown = overlays(&app);
        assert_eq!(shown.len(), 1);
        assert!(matches!(&shown[0], Overlay::Path { color: [0x4f, 0xff, 0x4f], .. }), "{shown:?}");
        // Cancel rolls the preview back.
        crate::dialogs::cancel(&mut app);
        assert_eq!((faces(&app, g), options(&app, g)), (0, GapOptions::default()));
        assert!(overlays(&app).is_empty());
        // OK keeps a custom size as one undo step.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("detect".into(), json!(true));
        d.fields.insert("size".into(), json!("custom"));
        d.fields.insert("custom".into(), json!(2.5));
        frame(&mut app);
        let depth = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        assert_eq!(options(&app, g).size, GapSize::Custom(2.5));
        assert_eq!(faces(&app, g), 1);
        // The dialog opens again on those options; Close Gaps With Paths adds the closing path.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.str("size"), d.f64("custom", 0.0)), ("custom".to_string(), 2.5));
        d.fields.insert("detect".into(), json!(false));
        d.fields.insert("closeGaps".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(faces(&app, g), 1, "closed by a path");
        let info = app.run("livePaint.info", json!({"group": g})).unwrap();
        assert_eq!((info["sources"].as_u64(), info["gaps"].as_array().map(Vec::len)), (Some(2), Some(0)));
        // OK keeps Medium Gaps, and the dialog opens again on Large Gaps.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!("medium"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(options(&app, g).size, GapSize::Medium);
        app.run("livePaint.gapOptions", json!({"id": g, "size": "large"})).unwrap();
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("size"), d.f64("custom", 0.0)), ("large".to_string(), GapSize::LARGE));
    }
}
