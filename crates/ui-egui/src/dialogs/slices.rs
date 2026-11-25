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
use craft::ui::dialog::DialogResult;
use serde_json::{Map, Value, json};

use crate::VectorcraftApp;
// PLAN: assumed `crate::canvas::general_lengths(app: &VectorcraftApp) -> LengthUnits` (cv.B5b)
use crate::canvas::general_lengths;

/// The dialog kind of Slice Options.
pub const OPTIONS: &str = "sliceOptions";
/// The dialog kind of Divide Slices.
pub const DIVIDE: &str = "divideSlices";

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
    let v = app.session.execute("object.slice.options", &json!({})).map_err(|e| e.to_string())?;
    let map = v.as_object().cloned().unwrap_or_default();
    let mut kinds = vec![SliceKind::Image, SliceKind::NoImage];
    if map.get("htmlText").and_then(Value::as_bool).unwrap_or(false) {
        kinds.push(SliceKind::HtmlText);
    }
    let dialog = SliceOptionsDialog::new(options_of(&map)).kinds(&kinds);
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// Open Divide Slices: two rows and two columns, the sizes halving the first selected slice.
pub fn open_divide(app: &mut VectorcraftApp) -> Result<Value, String> {
    let lengths = general_lengths(app);
    let st = app.session.active().ok_or("no document open")?;
    let first = st.selection.slices.iter().chain(&st.selection.objects).find_map(|id| st.doc.slice_bounds(*id));
    let (w, h) = first.map_or((50.0, 50.0), |r| (r.width() / 2.0, r.height() / 2.0));
    let dialog = DivideSlicesDialog::new(DivideSlices::new(Some(DivideBy::Count(2)), Some(DivideBy::Count(2)))).lengths(lengths).start_sizes(h, w);
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// Run the answer of whichever slice dialog closed with OK. The framework dialogs hold no answer, so a
/// refusal goes to the status line and the dialog is already closed.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(host) = app.dialogs() else { return };
    let (options, divide) = host.with(|h| (h.take::<SliceOptionsDialog>(), h.take::<DivideSlicesDialog>()));
    let answers = [
        options.and_then(|r| if let DialogResult::Ok(o) = r { Some(("object.slice.options", options_json(&o))) } else { None }),
        divide.and_then(|r| if let DialogResult::Ok(d) = r { Some(("object.slice.divide", divide_json(&d))) } else { None }),
    ];
    for (cmd, params) in answers.into_iter().flatten() {
        if let Err(e) = app.run(cmd, params) {
            app.status(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::{DialogHost, DialogId};
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
