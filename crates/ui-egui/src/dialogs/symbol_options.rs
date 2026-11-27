//! Symbol Options: name, Export Type (Graphic or Movie Clip), Symbol Type (Dynamic or Static) and
//! Registration for a new symbol
//! from the selection (`symbol.new`), or for the current symbol in the Symbols panel
//! (`symbol.options`).
//!
//! Fields: `name`, `nineSlice` (bool) with `sliceLeft` / `sliceTop` / `sliceRight` / `sliceBottom`
//! (percent of the symbol's size, -1600..1600: below 0 or above 100 for a guide outside the art),
//! `exportType` (`movieClip` | `graphic`), `type` (`dynamic` |
//! `static`; a new symbol is dynamic), `registration` (0..8, the 9-point grid;
//! 4 = centre), `alignToPixelGrid` (bool, off for a new symbol). Whether the dialog edits an existing
//! symbol, and whether a new symbol leaves the selection as ordinary art, are held by its OK.
//! Help:
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html>.

use craft::ui::contract::{Bind, Outcome as _, Show as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::{Label, UnitKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The dialog kind of Symbol Options.
pub const KIND: &str = "symbolOptions";

/// The 9-slice guide fields, in `[left, top, right, bottom]` order.
const SLICE_KEYS: [&str; 4] = ["sliceLeft", "sliceTop", "sliceRight", "sliceBottom"];

/// A guide field's value when the dialog has none: a quarter of the symbol's size, in percent.
const DEFAULT_SLICE_PERCENT: f64 = 25.0;

/// The range of a guide field, in percent: a guide may lie outside the art, up to
/// [`vectorcraft_doc::MAX_GUIDE_FRACTION`] times its size from its edge.
const SLICE_PERCENT_RANGE: std::ops::RangeInclusive<f64> = -vectorcraft_doc::MAX_GUIDE_FRACTION * 100.0..=vectorcraft_doc::MAX_GUIDE_FRACTION * 100.0;

/// Sets the 9-slice fields for `guides` (percent of the symbol's size, unrounded; a quarter each when
/// off) and returns the guides as fractions, `[left, top, right, bottom]`, for [`slice_params`].
fn slice_fields(fields: &mut DialogFields, guides: Option<vectorcraft_doc::SymbolNineSlice>) -> [f64; 4] {
    let g = guides.unwrap_or_default();
    fields.set("nineSlice", FieldValue::Bool(guides.is_some()));
    let opened = [g.left, g.top, g.right, g.bottom];
    for (key, v) in SLICE_KEYS.iter().zip(opened) {
        fields.set(*key, FieldValue::Number(v * 100.0));
    }
    opened
}

/// The guides the dialog's fields describe, as the `nineSlice` / `nineSliceGuides` params. A field
/// still showing the guide it opened with (`opened`) gives that guide exactly.
fn slice_params(fields: &DialogFields, opened: [f64; 4]) -> Value {
    let [l, t, r, b] = std::array::from_fn(|i| {
        let percent = fields.number(SLICE_KEYS[i]).unwrap_or(DEFAULT_SLICE_PERCENT);
        if opened[i] * 100.0 == percent { opened[i] } else { percent / 100.0 }
    });
    if fields.bool("nineSlice").unwrap_or(false) {
        json!({ "nineSlice": true, "nineSliceGuides": [l, t, r, b] })
    } else {
        json!({ "nineSlice": false })
    }
}

/// The dialog's rows. `name` is what the name field shows when it has no value.
fn rows(name: &str) -> Vec<Param> {
    // The reference app's default Export Type is Movie Clip.
    let export = vec![("movieClip".to_string(), Label::from(tl!("Movie Clip"))), ("graphic".to_string(), Label::from(tl!("Graphic")))];
    // A dynamic symbol's instances may change the appearance of parts of its art and stay linked.
    let kind = vec![("dynamic".to_string(), Label::from(tl!("Dynamic Symbol"))), ("static".to_string(), Label::from(tl!("Static Symbol")))];
    let mut rows = vec![
        Param::new("name", tl!("Name:"), ParamKind::text(name)),
        Param::new("exportType", tl!("Export Type:"), ParamKind::choice(export, Some("movieClip".to_string()))),
        Param::new("type", tl!("Symbol Type:"), ParamKind::radio(kind, Some("dynamic".to_string()))),
        Param::custom("registration", tl!("Registration:"), |ui, fields, _| {
            let at = fields
                .int("registration")
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| Anchor9::ALL.get(i).copied())
                .unwrap_or(Anchor9::Centre);
            let shown = ReferencePoint::new(tl!("Registration:"), Bind::given(Some(at))).look(ReferencePointLook::Squares).show(ui);
            if let Some(i) = shown.value().and_then(Option::as_ref).and_then(|picked| Anchor9::ALL.iter().position(|a| a == picked)) {
                fields.set("registration", FieldValue::Int(i as i64));
            }
            shown.response().clone()
        })
        .opens_with(FieldValue::Int(4)),
        Param::new("nineSlice", tl!("Enable Guides for 9-Slice Scaling"), ParamKind::bool(false)),
    ];
    for (key, label) in SLICE_KEYS.into_iter().zip([tl!("Left:"), tl!("Top:"), tl!("Right:"), tl!("Bottom:")]) {
        rows.push(
            Param::number(key, label, DEFAULT_SLICE_PERCENT, 1.0, 3)
                .unit(UnitKind::Percent)
                .bounds(Some(*SLICE_PERCENT_RANGE.start()), Some(*SLICE_PERCENT_RANGE.end()))
                .visible_when("nineSlice", true),
        );
    }
    rows.push(Param::new("alignToPixelGrid", tl!("Align to Pixel Grid"), ParamKind::bool(false)));
    rows
}

/// Opens Symbol Options on `fields`. OK makes a new symbol, or, with `edit` (the symbol being
/// edited), applies the fields to it; `keep_art` leaves the selection as ordinary art for a new one;
/// `opened` are the guides the dialog opened with.
fn open_dialog(app: &mut VectorcraftApp, fields: DialogFields, edit: Option<String>, keep_art: bool, opened: [f64; 4]) -> Result<Value, String> {
    let document = app.session.active().ok_or("no document open")?.uid;
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let name = fields.text("name").unwrap_or_default().to_string();
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |_| ParamDialog::new(KIND, tl!("Symbol Options"), rows(&name)));
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| -> Result<(), String> {
        if !app.session.active().is_some_and(|active| active.uid == document) {
            return Err(tl!("This symbol belongs to the document that opened the dialog.").to_owned());
        }
        let f = &result.fields;
        let name = f.text("name").unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return Err("name is empty".into());
        }
        let from = edit.as_deref().unwrap_or(&name);
        let mut params = json!({
            "name": from,
            "exportType": f.text("exportType").unwrap_or_default(),
            "type": f.text("type").unwrap_or_default(),
            "registration": f.int("registration").unwrap_or(4),
            "alignToPixelGrid": f.bool("alignToPixelGrid").unwrap_or(false),
        });
        merge(&mut params, slice_params(f, opened));
        if edit.is_some() {
            if from != name {
                params["newName"] = json!(name);
            }
            app.run("symbol.options", params)
        } else {
            if keep_art {
                params["keepArt"] = json!(true);
            }
            app.run("symbol.new", params)
        }
        .map(|_| ())
    };
    crate::dialogs::host::open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// Open Symbol Options to name a new symbol made from the selection. With `keep_art`, OK leaves
/// the selection as ordinary art instead of replacing it with an instance.
pub fn open(app: &mut VectorcraftApp, keep_art: bool) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("nothing selected".into());
    }
    let mut fields = DialogFields::new();
    fields.set("name", FieldValue::Text(st.doc.unique_symbol_name("New Symbol")));
    fields.set("exportType", FieldValue::Text("movieClip".into()));
    fields.set("type", FieldValue::Text("dynamic".into()));
    fields.set("registration", FieldValue::Int(4));
    fields.set("alignToPixelGrid", FieldValue::Bool(false));
    let opened = slice_fields(&mut fields, None);
    open_dialog(app, fields, None, keep_art, opened)
}

/// Open Symbol Options on an existing symbol (`name`, else the Symbols panel's current).
pub fn open_edit(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let name = match name.filter(|n| !n.is_empty()) {
        Some(n) if st.doc.symbols.iter().any(|s| s.name == n) => n.to_string(),
        Some(n) => return Err(format!("no symbol named `{n}`")),
        None => st
            .doc
            .unknown
            .get("currentSymbol")
            .and_then(|v| v.as_str())
            .filter(|n| st.doc.symbols.iter().any(|s| s.name == *n))
            .map(str::to_string)
            .or_else(|| st.doc.symbols.first().map(|s| s.name.clone()))
            .ok_or_else(|| "the document has no symbols".to_string())?,
    };
    let export = st.doc.symbol_export_type(&name).id();
    let registration = st.doc.symbol_registration(&name).0;
    let kind = if st.doc.symbols.iter().any(|s| s.name == name && s.dynamic) { "dynamic" } else { "static" };
    let mut fields = DialogFields::new();
    fields.set("name", FieldValue::Text(name.clone()));
    fields.set("exportType", FieldValue::Text(export.to_string()));
    fields.set("type", FieldValue::Text(kind.into()));
    fields.set("registration", FieldValue::Int(registration as i64));
    fields.set("alignToPixelGrid", FieldValue::Bool(st.doc.symbol_pixel_aligned(&name)));
    let opened = slice_fields(&mut fields, st.doc.symbol_nine_slice(&name));
    open_dialog(app, fields, Some(name), false, opened)
}

/// Copy the keys of the object `extra` into the object `into`.
fn merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        a.extend(b);
    }
}

#[cfg(test)]
mod document_tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::super::tests_warp::{accept, set, windowed};
    use super::*;

    #[test]
    fn editing_symbol_options_does_not_rename_another_drawings_symbol() {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        for side in [30, 50] {
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": side, "height": side})).unwrap();
            app.run("symbol.new", json!({"name": "Badge"})).unwrap();
        }
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        let other = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();

        activate(&mut app, 0);
        open_edit(&mut app, Some("Badge")).unwrap();
        let id = dialog_id(KIND);
        set(&app, id, "name", json!("Source badge"));
        activate(&mut app, 1);
        accept(&mut app, id);
        assert_eq!(app.session.active().unwrap().doc, other);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        activate(&mut app, 0);
        accept(&mut app, id);
        assert!(app.session.active().unwrap().doc.symbols.iter().any(|symbol| symbol.name == "Source badge"));
    }
}
