//! Object › Live Paint › Gap Options: Gap Detection, Paint Stops At (Small, Medium or Large Gaps,
//! or Custom with its size), Gap Preview Color (a layer color or a custom color) and Close Gaps
//! With Paths for the selected Live Paint groups, opened on the first group's options. With Preview
//! on, the options apply to the groups live and the gaps they find draw on the canvas in the preview
//! color; OK keeps them as one undo step (`livePaint.gapOptions`). View › Show Live Paint Gaps
//! draws the gaps of the selected groups the same way ([`selected_gaps`]).
//!
//! Fields: `detect`, `size` (`small` | `medium` | `large` | `custom`), `custom` (pt), `color`
//! (`#rrggbb`, or a preset's name such as "Red"), `closeGaps` and `preview`.

use craft::ui::contract::ShowContainer as _;
use craft::ui::dialog::{DialogId, HeldOk, StackDialog as _};
use craft::ui::layout_components::Row;
use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::GapSize;
use vectorcraft_geom::{BezPath, Point};
use vectorcraft_tools::Overlay;

use super::option_form::{self, OptionForm, OptionSpec, Rows, is_on};
use super::preview::LivePreview;
use super::tile_edge_color::{checked, has_alpha};
use crate::VectorcraftApp;

/// The dialog kind of Gap Options.
pub const KIND: &str = "livePaintGapOptions";

const CMD: &str = "livePaint.gapOptions";
const WRONG_DOCUMENT: &str = "Gap Options can only be applied in the document this dialog was opened for.";
/// The undo step the preview and OK make.
const PREVIEW_LABEL: &str = "Gap Options";

/// Paint Stops At: (value, label).
const SIZES: [(&str, &str); 4] = [("small", "Small Gaps"), ("medium", "Medium Gaps"), ("large", "Large Gaps"), ("custom", "Custom")];

/// The dialog's description for the option form.
pub(super) static SPEC: OptionSpec = OptionSpec {
    key: KIND,
    title: || tl!("Gap Options").to_owned(),
    keys: &["detect", "size", "custom", "color", "closeGaps"],
    colors: &["color"],
    preview: true,
    body,
    // The custom size takes input while Custom is chosen.
    inactive: |values, key| key == "custom" && values.get("size").and_then(Value::as_str) != Some("custom"),
    run,
    cancelled: Some(cancelled),
};

/// Gap Options' running preview while its dialog is open. It lives on the app, not in the dialog: a Cancel or
/// Escape closes the dialog where no app is at hand to roll the preview back ([`cancelled`]).
pub(crate) struct Open {
    /// The dialog the preview belongs to.
    pub id: DialogId,
    /// The document that owns the preview interaction.
    document: u64,
    /// The preview the options run as.
    pub preview: LivePreview,
}

impl Open {
    fn cancel(self, app: &mut VectorcraftApp) {
        if app.session.active().is_some_and(|active| active.uid == self.document) {
            self.preview.cancel(app);
        }
    }
}

/// Open Gap Options on the selected Live Paint groups, filled in from the first one's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let active = app.session.active().ok_or("select a Live Paint group")?;
    let document = active.uid;
    let groups: Vec<u64> = active
        .selection
        .objects
        .iter()
        .filter(|id| active.doc.node(**id).is_some_and(vectorcraft_tools::builder::is_live_paint))
        .map(|id| id.0)
        .collect();
    let Some(&first) = groups.first() else { return Err("select a Live Paint group".into()) };
    let info = app.session.execute("livePaint.info", &json!({ "group": first })).map_err(|e| e.to_string())?;
    let o = &info["gapOptions"];
    let (size, custom) = match o["size"].as_str() {
        Some(id) => (id.to_string(), o["points"].clone()),
        None => ("custom".to_string(), o["size"].clone()),
    };
    let fields =
        [("detect", o["detect"].clone()), ("size", json!(size)), ("custom", custom), ("color", o["color"].clone()), ("closeGaps", json!(false))];
    let values = Map::from_iter(fields.map(|(key, value)| (key.to_owned(), value)));
    option_form::open(app, &SPEC, values, json!({ "ids": groups, "document": document })).ok_or("no window")?;
    Ok(Value::Null)
}

fn original_document(app: &VectorcraftApp, extra: &Value) -> Option<u64> {
    let document = extra["document"].as_u64()?;
    app.session.active().filter(|active| active.uid == document).map(|_| document)
}

/// `livePaint.gapOptions` parameters from the values and the groups in `extra`.
fn params(values: &Map<String, Value>, extra: &Value) -> Value {
    let size = match values.get("size").and_then(Value::as_str) {
        Some("custom") => json!(values.get("custom").and_then(Value::as_f64).unwrap_or(GapSize::SMALL)),
        Some(id) => json!(id),
        None => Value::Null,
    };
    json!({
        "ids": extra["ids"],
        "detect": is_on(values, "detect"),
        "size": size,
        "color": values.get("color").cloned().unwrap_or(Value::Null),
        "closeGaps": is_on(values, "closeGaps"),
    })
}

fn body(ui: &mut egui::Ui, rows: &mut Rows<'_>) {
    rows.check(ui, "detect", tl!("Gap Detection"), true);
    let custom = rows.text("size") == "custom";
    let sizes = SIZES.map(|(value, label)| (value, tl!(label)));
    rows.field(ui, tl!("Paint Stops At"), true, |ui, rows, label| {
        Row::new().show(ui, |ui, _| {
            rows.select(ui, "size", label, &sizes);
            rows.length(ui, "custom", tl!("Custom"), 0.0..=GapSize::MAX, custom);
        });
    });
    rows.field(ui, tl!("Gap Preview Color"), true, |ui, rows, label| rows.color(ui, "color", label, true));
    rows.check(ui, "closeGaps", tl!("Close Gaps With Paths"), true);
}

/// OK: keep the previewed options as one undo step; with no preview running they run as one.
fn run(app: &mut VectorcraftApp, values: &Map<String, Value>, extra: &Value) -> Result<Value, String> {
    let document = original_document(app, extra).ok_or_else(|| tl!(WRONG_DOCUMENT).to_owned())?;
    let preview = app.object_dialogs.gaps.take().filter(|open| open.document == document).map_or_else(LivePreview::default, |open| open.preview);
    preview.commit(app, CMD, params(values, extra))
}

/// The dialog closed without OK: roll the preview back.
fn cancelled(app: &mut VectorcraftApp) {
    if let Some(open) = app.object_dialogs.gaps.take() {
        open.cancel(app);
    }
}

/// Runs the open Gap Options' preview as its options stand, once per frame after the shell pass: with Preview on the
/// options apply to the groups, off they are rolled back; the gaps a new preview finds are kept for [`overlays`].
/// Nothing runs while an OK waits for its answer, which keeps or refuses the preview that is running.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let shown = dialogs.with(|host| {
        let form = host.get_mut::<OptionForm>().filter(|form| form.is(KIND))?;
        let source = form.extra["document"].as_u64()?;
        if !app.session.documents().iter().any(|doc| doc.uid == source) {
            host.cancel::<OptionForm>(ctx);
            return None;
        }
        let document = original_document(app, &form.extra);
        let paused = document.is_none();
        if form.preview_paused != paused {
            form.preview_paused = paused;
            if !paused && form.held.refusal().is_some_and(|(reason, _)| reason.as_str() == tl!(WRONG_DOCUMENT)) {
                form.held.clear_refusal();
            }
            ctx.request_repaint();
        }
        let Some(document) = document else {
            form.previewed = Value::Null;
            return None;
        };
        if !form.held.state().is_none_or(|state| matches!(state, HeldOk::Refused(_))) {
            return None;
        }
        let mut params = params(&form.values, &form.extra);
        // OK refuses a color with an alpha part, so the preview leaves each group's own color.
        if form.values.get("color").and_then(Value::as_str).is_some_and(has_alpha) {
            params["color"] = Value::Null;
        }
        Some((form.id(), document, params, form.preview))
    });
    let Some((id, document, params, on)) = shown else { return };
    // A preview of another dialog is rolled back before this one starts its own.
    let mut open = match app.object_dialogs.gaps.take() {
        Some(open) if open.id == id && open.document == document => open,
        stale => {
            if let Some(stale) = stale {
                stale.cancel(app);
            }
            Open { id, document, preview: LivePreview::default() }
        }
    };
    let ran = open.preview.run(app, on, PREVIEW_LABEL, CMD, &params);
    app.object_dialogs.gaps = Some(open);
    let found = match ran {
        Some(Ok(done)) => Some(done["gaps"].clone()),
        Some(Err(e)) => {
            app.status(e);
            None
        }
        None => None,
    };
    dialogs.with(|host| {
        if let Some(form) = host.get_mut_by_id::<OptionForm>(id) {
            if !on {
                form.previewed = Value::Null;
            } else if let Some(gaps) = found {
                form.previewed = gaps;
            }
        }
    });
}

/// The gaps the open Gap Options previews, as lines in its preview color (none while Preview is
/// off or another dialog is open).
pub(crate) fn overlays(app: &VectorcraftApp) -> Vec<Overlay> {
    let Some(dialogs) = app.dialogs() else { return vec![] };
    let shown = dialogs.with(|host| {
        let form = host.get::<OptionForm>().filter(|form| form.is(KIND) && form.preview)?;
        original_document(app, &form.extra)?;
        Some((form.previewed.clone(), form.values.get("color").and_then(Value::as_str)?.to_owned()))
    });
    let Some((gaps, color)) = shown else { return vec![] };
    let [r, g, b, _] = checked(&color).unwrap_or(Color::BLACK).to_rgba8(1.0);
    gaps.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
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
    use super::super::option_form::tests::{app, cancel, fields, held, is_open, press_ok, pump_all, set};
    use super::*;
    use vectorcraft_doc::{GapOptions, NodeId};

    fn options(app: &VectorcraftApp, g: u64) -> GapOptions {
        app.session.doc().unwrap().doc.node(NodeId(g)).unwrap().gap_options()
    }

    fn faces(app: &VectorcraftApp, g: u64) -> usize {
        vectorcraft_tools::builder::faces(app.session.doc().unwrap().doc.node(NodeId(g)).unwrap()).len()
    }

    /// An open path round a square, its end 2 pt short of its start, made a Live Paint group.
    fn gapped_group(app: &mut VectorcraftApp) -> u64 {
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(x, y)| json!({"x": x, "y": y}));
        let p = app.run("path.create", json!({ "anchors": anchors })).unwrap();
        app.run("select.set", json!({"ids": [p["id"]]})).unwrap();
        app.run("livePaint.make", json!({})).unwrap()["id"].as_u64().unwrap()
    }

    fn open_with_matching_groups(app: &mut VectorcraftApp) -> u64 {
        let group = gapped_group(app);
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        assert_eq!(gapped_group(app), group, "the new document reuses the group ID");
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("select.set", json!({"ids": [group]})).unwrap();
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(app, "detect", json!(true)).unwrap();
        group
    }

    #[test]
    fn gap_confirmation_stays_with_its_document() {
        let mut app = app();
        let group = open_with_matching_groups(&mut app);
        app.run("document.activate", json!({"index": 1})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        assert!(!press_ok(&mut app), "the unrelated group cannot take the edit");
        assert_eq!(options(&app, group), GapOptions::default());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        app.run("document.activate", json!({"index": 0})).unwrap();
        pump_all(&mut app);
        assert!(held(&app).is_none(), "the stale refusal clears on the original tab");
        assert!(press_ok(&mut app), "the original group's edit can be retried");
        assert!(options(&app, group).detect);
    }

    #[test]
    fn closing_the_original_document_dismisses_gap_options() {
        let mut app = app();
        let group = open_with_matching_groups(&mut app);
        app.run("document.activate", json!({"index": 1})).unwrap();
        assert!(app.session.close_document(0));
        pump_all(&mut app);
        assert!(!is_open(&app));
        assert_eq!(options(&app, group), GapOptions::default());
    }

    #[test]
    fn gap_preview_disappears_in_another_document_and_resumes_on_return() {
        let mut app = app();
        let group = open_with_matching_groups(&mut app);
        set(&app, "preview", json!(true)).unwrap();
        pump_all(&mut app);
        assert!(options(&app, group).detect);
        assert!(!overlays(&app).is_empty());
        app.run("document.activate", json!({"index": 1})).unwrap();
        let unrelated_overlays = overlays(&app);
        pump_all(&mut app);
        let paused = |app: &VectorcraftApp| app.dialogs().unwrap().with(|host| host.get::<OptionForm>().unwrap().preview_paused);
        assert!(paused(&app));
        assert_eq!(fields(&app)["preview"], json!(true));
        assert_eq!(options(&app, group), GapOptions::default());
        assert!(!app.session.in_interaction());
        assert!(unrelated_overlays.is_empty());
        assert!(overlays(&app).is_empty());
        app.run("document.activate", json!({"index": 0})).unwrap();
        pump_all(&mut app);
        assert!(!paused(&app));
        assert!(options(&app, group).detect);
        assert!(!overlays(&app).is_empty());
        cancel(&mut app);
        assert_eq!(options(&app, group), GapOptions::default());
    }

    #[test]
    fn the_menu_item_opens_gap_options_which_preview_the_gaps_and_ok_keeps_them() {
        let mut app = app();
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(x, y)| json!({"x": x, "y": y}));
        let p = app.run("path.create", json!({ "anchors": anchors })).unwrap();
        app.run("select.set", json!({"ids": [p["id"]]})).unwrap();
        // Without a Live Paint group the dialog does not open.
        assert!(app.run("ui.livePaintGapOptions", json!({})).is_err());
        let g = app.run("livePaint.make", json!({})).unwrap()["id"].as_u64().unwrap();
        // Object › Live Paint › Gap Options… runs the command that opens the dialog.
        let tree = crate::menus::menu_tree();
        let (_, object) = tree.iter().find(|(t, _)| *t == "Object").unwrap();
        let live = object
            .iter()
            .find_map(|n| match n {
                crate::menus::Node::Submenu { label, children, .. } if label == "Live Paint" => Some(children),
                _ => None,
            })
            .unwrap();
        assert!(
            live.iter().any(|n| matches!(n, crate::menus::Node::Item(i) if i.label == "Gap Options…" && i.command.id == "ui.livePaintGapOptions")),
            "{live:?}"
        );
        crate::menus::invoke(&mut app, "ui.livePaintGapOptions", json!({}));
        assert!(is_open(&app), "Object › Live Paint › Gap Options… opens the dialog");
        let shown = fields(&app);
        assert_eq!((&shown["detect"], &shown["size"], &shown["color"]), (&json!(false), &json!("small"), &json!("#ff4f4f")));
        pump_all(&mut app);
        assert!(overlays(&app).is_empty(), "no preview yet");
        // Preview with Gap Detection and Small Gaps: the square fills as one face and its gap shows.
        set(&app, "detect", json!(true)).unwrap();
        set(&app, "size", json!("small")).unwrap();
        set(&app, "color", json!("Green")).unwrap();
        set(&app, "preview", json!(true)).unwrap();
        pump_all(&mut app);
        assert_eq!(faces(&app, g), 1, "previewed");
        let shown = overlays(&app);
        assert_eq!(shown.len(), 1);
        assert!(matches!(&shown[0], Overlay::Path { color: [0x4f, 0xff, 0x4f], .. }), "{shown:?}");
        // Cancel rolls the preview back.
        cancel(&mut app);
        assert_eq!((faces(&app, g), options(&app, g)), (0, GapOptions::default()));
        assert!(overlays(&app).is_empty());
        // OK keeps a custom size as one undo step.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(&app, "detect", json!(true)).unwrap();
        set(&app, "size", json!("custom")).unwrap();
        set(&app, "custom", json!(2.5)).unwrap();
        pump_all(&mut app);
        let depth = app.session.doc().unwrap().history.undo.len();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        assert_eq!(options(&app, g).size, GapSize::Custom(2.5));
        assert_eq!(faces(&app, g), 1);
        // The dialog opens again on those options; Close Gaps With Paths adds the closing path.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        let shown = fields(&app);
        assert_eq!((&shown["size"], shown["custom"].as_f64()), (&json!("custom"), Some(2.5)));
        set(&app, "detect", json!(false)).unwrap();
        set(&app, "closeGaps", json!(true)).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(faces(&app, g), 1, "closed by a path");
        let info = app.run("livePaint.info", json!({"group": g})).unwrap();
        assert_eq!((info["sources"].as_u64(), info["gaps"].as_array().map(Vec::len)), (Some(2), Some(0)));
        // OK keeps Medium Gaps, and the dialog opens again on Large Gaps.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(&app, "size", json!("medium")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(options(&app, g).size, GapSize::Medium);
        app.run("livePaint.gapOptions", json!({"id": g, "size": "large"})).unwrap();
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        let shown = fields(&app);
        assert_eq!((&shown["size"], shown["custom"].as_f64()), (&json!("large"), Some(GapSize::LARGE)));
    }

    /// OK on the options the group has, with Preview on or off, closes the dialog and records no
    /// undo step: the redo step stays and a saved document stays saved.
    #[test]
    fn ok_on_unchanged_options_records_no_undo_step() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let g = app.run("livePaint.make", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let state = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
        };
        let before = state(&app);
        for preview in [false, true] {
            app.run("ui.livePaintGapOptions", json!({})).unwrap();
            set(&app, "preview", json!(preview)).unwrap();
            pump_all(&mut app);
            assert!(press_ok(&mut app), "preview={preview}: OK closes the dialog");
            assert!(!is_open(&app));
            assert_eq!(state(&app), before, "preview={preview}: no undo step");
            assert_eq!(options(&app, g), GapOptions::default());
        }
        // A changed option is one undo step.
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(&app, "detect", json!(true)).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(state(&app).0, before.0 + 1);
    }

    /// The canvas draws the gaps Gap Options previews, in its preview color, and stops when Preview goes off.
    #[test]
    fn the_canvas_draws_the_previewed_gaps() {
        let mut app = app();
        gapped_group(&mut app);
        let green = egui::Color32::from_rgb(0x4f, 0xff, 0x4f);
        let drawn = |app: &mut VectorcraftApp| {
            let ctx = app.ui_ctx.clone().unwrap();
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(app, ui));
            out.textures_delta.clear();
            ctx.tessellate(out.shapes, out.pixels_per_point).into_iter().any(|p| match p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().any(|v| v.color == green),
                egui::epaint::Primitive::Callback(_) => false,
            })
        };
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(&app, "detect", json!(true)).unwrap();
        set(&app, "color", json!("Green")).unwrap();
        set(&app, "preview", json!(true)).unwrap();
        pump_all(&mut app);
        assert!(!overlays(&app).is_empty());
        assert!(drawn(&mut app), "the previewed gap draws on the canvas");
        set(&app, "preview", json!(false)).unwrap();
        pump_all(&mut app);
        assert!(!drawn(&mut app), "no preview, no gap");
    }

    /// With Preview on, a Gap Preview Color with an alpha part is not previewed: the group keeps
    /// its own color, the gaps draw in black, OK refuses the color and Cancel rolls the preview back.
    #[test]
    fn a_color_with_an_alpha_part_is_not_previewed() {
        let mut app = app();
        let g = gapped_group(&mut app);
        let own = options(&app, g).color;
        app.run("ui.livePaintGapOptions", json!({})).unwrap();
        set(&app, "preview", json!(true)).unwrap();
        set(&app, "detect", json!(true)).unwrap();
        set(&app, "color", json!("#ff000080")).unwrap();
        let depth = app.session.doc().unwrap().history.undo.len();
        pump_all(&mut app);
        assert_eq!((options(&app, g).detect, options(&app, g).color), (true, own), "the group keeps its own color");
        let shown = overlays(&app);
        assert!(matches!(&shown[..], [Overlay::Path { color: [0, 0, 0], .. }]), "{shown:?}");
        assert!(!press_ok(&mut app));
        assert!(is_open(&app), "the dialog stays open");
        assert!(matches!(held(&app), Some(HeldOk::Refused(_))), "with the reason");
        cancel(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        assert_eq!((options(&app, g), undo), (GapOptions::default(), depth), "Cancel rolls the preview back");
    }
}
