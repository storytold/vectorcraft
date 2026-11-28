//! Object → Slice → Slice Options… and Divide Slices….
//!
//! Slice Options (`sliceOptions`, also a double click with the Slice Selection tool) edits the
//! selected slices' options: fields `kind` (`image`, `noImage`, `htmlText`), `name`, `url`,
//! `target`, `message`, `alt`, `cellText`, `hAlign`, `vAlign` and `background` (`null`,
//! `"matte"` or `"#rrggbb"`). OK runs `object.slice.options`.
//!
//! Divide Slices (`divideSlices`): `rows` and `columns` (a count or, with `rowsBy` or `columnsBy`
//! `"size"`, a length in points; `null` while the axis is off). OK runs `object.slice.divide`.

use craft::draw::ui::slice::{
    DivideBy, DivideSlices, DivideSlicesDialog, SliceBackground, SliceCellAlign, SliceCellVAlign, SliceKind, SliceOptions, SliceOptionsDialog,
};
use craft::theme::Srgba;
use serde_json::{Map, Value, json};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
// PLAN: assumed `crate::canvas::general_lengths(app: &VectorcraftApp) -> LengthUnits` (cv.B5b)
use crate::canvas::general_lengths;

/// The dialog kind of Slice Options.
pub const OPTIONS: &str = "sliceOptions";
/// The dialog kind of Divide Slices.
pub const DIVIDE: &str = "divideSlices";

#[derive(Default)]
pub(crate) struct SliceDialogs {
    options: Option<Target>,
    divide: Option<Target>,
}

/// The document and effective slice selection used when opening either dialog.
#[derive(PartialEq)]
struct Target {
    document: u64,
    slices: Vec<NodeId>,
}

impl Target {
    fn new(document: u64, mut slices: Vec<NodeId>) -> Self {
        slices.sort_unstable();
        Self { document, slices }
    }

    fn current(app: &VectorcraftApp) -> Option<Self> {
        let document = app.session.active()?.uid;
        Some(Self::new(document, app.session.selected_slice_ids()))
    }
}

/// The command's key of each slice type, alignment and vertical alignment.
const KINDS: [(&str, SliceKind); 3] = [("image", SliceKind::Image), ("noImage", SliceKind::NoImage), ("htmlText", SliceKind::HtmlText)];
const ALIGNS: [(&str, SliceCellAlign); 4] =
    [("default", SliceCellAlign::Default), ("left", SliceCellAlign::Left), ("center", SliceCellAlign::Center), ("right", SliceCellAlign::Right)];
const VALIGNS: [(&str, SliceCellVAlign); 5] = [
    ("default", SliceCellVAlign::Default),
    ("top", SliceCellVAlign::Top),
    ("middle", SliceCellVAlign::Middle),
    ("baseline", SliceCellVAlign::Baseline),
    ("bottom", SliceCellVAlign::Bottom),
];

/// The command's key of `value`.
fn key_of<T: Copy + PartialEq>(table: &[(&'static str, T)], value: T) -> &'static str {
    table.iter().find(|(_, v)| *v == value).map_or("", |(k, _)| *k)
}

/// The value of the command's `key`; the default for a key the table lacks.
fn value_of<T: Copy + Default>(table: &[(&str, T)], key: &str) -> T {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or_default()
}

/// The dialog's options from the answer of `object.slice.options`.
fn options_of(v: &Map<String, Value>) -> SliceOptions {
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut o = SliceOptions::default();
    o.kind = value_of(&KINDS, &text("kind"));
    o.name = text("name");
    o.url = text("url");
    o.target = text("target");
    o.message = text("message");
    o.alt = text("alt");
    o.cell_text = text("text");
    o.h_align = value_of(&ALIGNS, &text("hAlign"));
    o.v_align = value_of(&VALIGNS, &text("vAlign"));
    o.background = match text("background").as_str() {
        "" => SliceBackground::None,
        "matte" => SliceBackground::Matte,
        hex => Srgba::from_hex(hex).map_or(SliceBackground::None, SliceBackground::Color),
    };
    o
}

/// The parameters of `object.slice.options` for the dialog's options.
fn options_json(o: &SliceOptions) -> Value {
    let background = match o.background {
        SliceBackground::Matte => "matte".to_string(),
        SliceBackground::Color(c) => c.to_hex(),
        _ => String::new(),
    };
    json!({
        "kind": key_of(&KINDS, o.kind), "name": o.name, "url": o.url, "target": o.target, "message": o.message, "alt": o.alt,
        "text": o.cell_text, "hAlign": key_of(&ALIGNS, o.h_align), "vAlign": key_of(&VALIGNS, o.v_align), "background": background,
    })
}

/// The parameters of `object.slice.divide`: per axis that is on, its count or its size.
fn divide_json(d: &DivideSlices) -> Value {
    let mut p = Map::new();
    for (by, count, size) in [(d.rows, "rows", "rowHeight"), (d.columns, "columns", "columnWidth")] {
        match by {
            Some(DivideBy::Count(n)) => p.insert(count.into(), json!(n)),
            Some(DivideBy::Size(s)) => p.insert(size.into(), json!(s)),
            _ => None,
        };
    }
    Value::Object(p)
}

/// Open Slice Options on the selected slices' options (those of the first one).
pub fn open_options(app: &mut VectorcraftApp) -> Result<Value, String> {
    let source = Target::current(app).ok_or("no document open")?;
    let v = app.session.execute("object.slice.options", &json!({})).map_err(|e| e.to_string())?;
    let map = v.as_object().cloned().unwrap_or_default();
    let mut kinds = vec![SliceKind::Image, SliceKind::NoImage];
    if map.get("htmlText").and_then(Value::as_bool).unwrap_or(false) {
        kinds.push(SliceKind::HtmlText);
    }
    let dialog = SliceOptionsDialog::new(options_of(&map)).kinds(&kinds);
    app.dialogs().ok_or("no window")?.with(|host| {
        host.open(dialog);
        // Replacing a still-open dialog leaves its Cancel behind.
        let _ = host.take::<SliceOptionsDialog>();
    });
    app.slice_dialogs.options = Some(source);
    Ok(Value::Null)
}

/// Open Divide Slices: two rows and two columns, the sizes halving the first selected slice.
pub fn open_divide(app: &mut VectorcraftApp) -> Result<Value, String> {
    let lengths = general_lengths(app);
    let st = app.session.active().ok_or("no document open")?;
    let selected = app.session.selected_slice_ids();
    let first = selected.iter().find_map(|id| st.doc.slice_bounds(*id));
    let source = Target::new(st.uid, selected);
    let (w, h) = first.map_or((50.0, 50.0), |r| (r.width() / 2.0, r.height() / 2.0));
    let dialog = DivideSlicesDialog::new(DivideSlices::new(Some(DivideBy::Count(2)), Some(DivideBy::Count(2)))).lengths(lengths).start_sizes(h, w);
    app.dialogs().ok_or("no window")?.with(|host| {
        host.open(dialog);
        let _ = host.take::<DivideSlicesDialog>();
    });
    app.slice_dialogs.divide = Some(source);
    Ok(Value::Null)
}

/// Run the answer of whichever slice dialog closed with OK. The framework dialogs hold no answer, so a
/// refusal goes to the status line and the dialog is already closed.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(host) = app.dialogs() else { return };
    let (options, divide) = host.with(|h| (h.take::<SliceOptionsDialog>(), h.take::<DivideSlicesDialog>()));
    let answers = [
        options.and_then(|r| {
            let source = app.slice_dialogs.options.take();
            r.ok().map(|o| (source, "object.slice.options", options_json(&o)))
        }),
        divide.and_then(|r| {
            let source = app.slice_dialogs.divide.take();
            r.ok().map(|d| (source, "object.slice.divide", divide_json(&d)))
        }),
    ];
    for (source, cmd, params) in answers.into_iter().flatten() {
        if !source.is_some_and(|source| Some(source) == Target::current(app)) {
            app.status(tl!("The slice selection changed while the dialog was open."));
        } else if let Err(e) = app.run(cmd, params) {
            app.status(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{DialogHost, DialogId};
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.session.paint.stroke = vectorcraft_color::Paint::None;
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("object.slice.make", json!({"ids": [id]})).unwrap();
        app
    }

    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("a window").with(f)
    }

    /// The one open dialog: its id and its fields.
    fn the_dialog(app: &VectorcraftApp) -> (DialogId, Value) {
        let d = host(app, |h| h.open_dialogs().into_iter().next()).expect("a dialog is open");
        (d.id, d.fields.unwrap_or(Value::Null))
    }

    fn set(app: &VectorcraftApp, field: &str, value: Value) {
        let (ctx, (id, _)) = (app.ui_ctx.clone().unwrap(), the_dialog(app));
        host(app, |h| h.set_json_field(&ctx, id, field, value)).unwrap();
    }

    /// OK on the open dialog, then the frame's pump runs its answer; whether OK was enabled.
    fn ok(app: &mut VectorcraftApp) -> bool {
        let (ctx, (id, _)) = (app.ui_ctx.clone().unwrap(), the_dialog(app));
        let enabled = host(app, |h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
        enabled
    }

    fn add_slice(app: &mut VectorcraftApp, x: f64) -> NodeId {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("object.slice.make", json!({"ids": [id]})).unwrap();
        NodeId(id)
    }

    fn select_slice(app: &mut VectorcraftApp, id: NodeId) {
        app.run("object.slice.select", json!({"slices": [id.0]})).unwrap();
    }

    fn check_changed_selection(dialog: &str, divide: bool) {
        let mut app = app();
        let source = app.session.active().unwrap().doc.slice_ids()[0];
        let destination = add_slice(&mut app, 80.0);
        select_slice(&mut app, source);
        app.run(dialog, json!({})).unwrap();
        if !divide {
            set(&app, "name", json!("Original"));
        }
        select_slice(&mut app, destination);
        let st = app.session.active().unwrap();
        let undo = st.history.undo.len();
        let document = st.doc.clone();

        assert!(ok(&mut app));
        let st = app.session.active().unwrap();
        assert_eq!(*st.doc, *document, "the changed selection is untouched");
        assert_eq!(st.history.undo.len(), undo);
        assert_eq!(app.ui.status, "The slice selection changed while the dialog was open.");
    }

    #[test]
    fn stale_slice_dialog_options_leaves_the_new_selection_untouched() {
        check_changed_selection("object.slice.options", false);
    }

    #[test]
    fn stale_slice_dialog_divide_leaves_the_new_selection_untouched() {
        check_changed_selection("object.slice.divide", true);
    }

    #[test]
    fn stale_slice_dialog_refuses_a_different_document_with_matching_selected_ids() {
        let mut app = app();
        let from = app.session.active().unwrap().doc.slice_ids()[0];
        app.run("object.slice.options", json!({})).unwrap();
        set(&app, "name", json!("Source"));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let to = add_slice(&mut app, 0.0);
        select_slice(&mut app, to);
        assert_eq!(from, to, "selection IDs are local to each document");
        let st = app.session.active().unwrap();
        let undo = st.history.undo.len();

        assert!(ok(&mut app));
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.slice_options(to).unwrap().name, "");
        assert_eq!(st.history.undo.len(), undo);
    }

    #[test]
    fn replacing_slice_options_keeps_the_new_dialogs_target() {
        let mut app = app();
        app.run("object.slice.options", json!({})).unwrap();
        let other = add_slice(&mut app, 80.0);
        select_slice(&mut app, other);
        app.run("object.slice.options", json!({})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        pump(&mut app, &ctx);
        set(&app, "name", json!("Replacement"));

        assert!(ok(&mut app));
        assert_eq!(app.session.active().unwrap().doc.slice_options(other).unwrap().name, "Replacement");
    }

    #[test]
    fn confirming_slice_options_still_works_when_the_same_slices_are_selected_in_a_different_order() {
        let mut app = app();
        let first = app.session.active().unwrap().doc.slice_ids()[0];
        let second = add_slice(&mut app, 80.0);
        app.run("object.slice.select", json!({"slices": [first.0, second.0]})).unwrap();
        app.run("object.slice.options", json!({})).unwrap();
        set(&app, "alt", json!("Two slices"));
        app.run("object.slice.select", json!({"slices": [second.0, first.0]})).unwrap();

        assert!(ok(&mut app));
        let document = &app.session.active().unwrap().doc;
        assert_eq!(document.slice_options(first).unwrap().alt, "Two slices");
        assert_eq!(document.slice_options(second).unwrap().alt, "Two slices");
    }

    #[test]
    fn slice_options_opens_on_the_selection_and_ok_sets_them() {
        let mut app = app();
        // The menu item (no params) opens the dialog through the app.
        app.run("object.slice.options", json!({})).unwrap();
        assert!(host(&app, |h| h.is_open::<SliceOptionsDialog>()));
        assert_eq!(the_dialog(&app).1["kind"], "image");
        set(&app, "kind", json!("noImage"));
        set(&app, "cellText", json!("Hello <i>cell</i>"));
        set(&app, "background", json!("#00ff00"));
        assert!(ok(&mut app));
        let l = app.run("slice.list", json!({})).unwrap();
        let own = l["slices"].as_array().unwrap().iter().find(|s| s["source"] == "object").unwrap().clone();
        assert_eq!(own["options"]["kind"], "noImage");
        assert_eq!(own["options"]["text"], "Hello <i>cell</i>");
        assert_eq!(own["options"]["background"], "#00ff00");
        // One undo step.
        app.run("edit.undo", json!({})).unwrap();
        let l = app.run("slice.list", json!({})).unwrap();
        assert!(l["slices"].as_array().unwrap().iter().any(|s| s["options"]["kind"] == "image" && s["source"] == "object"));
    }

    #[test]
    fn divide_slices_cuts_by_count_or_size() {
        let mut app = app();
        app.run("object.slice.divide", json!({})).unwrap();
        assert!(host(&app, |h| h.is_open::<DivideSlicesDialog>()));
        set(&app, "columnsBy", json!("size"));
        assert_eq!(the_dialog(&app).1["columns"].as_f64(), Some(30.0), "half of the 60 pt slice");
        set(&app, "columnsBy", json!("count"));
        set(&app, "columns", json!(3));
        set(&app, "rowsBy", json!("size"));
        set(&app, "rows", json!(15));
        assert!(ok(&mut app));
        // 40 pt in 15 pt rows: 3 rows; 3 columns.
        let slices = app.session.active().unwrap().doc.slices.len();
        assert_eq!(slices, 9);
        app.run("object.slice.divide", json!({})).unwrap();
        set(&app, "rows", Value::Null);
        set(&app, "columns", Value::Null);
        assert!(!ok(&mut app), "no axis is on, so OK is off");
        assert_eq!(app.session.active().unwrap().doc.slices.len(), slices);
    }
}
