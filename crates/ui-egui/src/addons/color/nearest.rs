//! Find Nearest Swatch in the interface (Object › Plug-ins › Color Effects › Find Nearest
//! Swatch…): a dialog that picks a library (the document's swatches, or a colour library or book)
//! and Fill and Stroke, and lists each colour of the selection with its closest swatches and their
//! difference, as `addon.color.nearestSwatch` finds them; Apply runs
//! `addon.color.applyNearestSwatch`.

use craft::behavior::keyed;
use craft::theme::Srgba;
use craft::ui::choice::{Checkbox, SwatchFace};
use craft::ui::contract::{Bind, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id, serde_fields, set_serde_field};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

use crate::VectorcraftApp;
use crate::addons::select::{finished, label};
use crate::panels::swatches;

/// The command Apply runs, and the dialog's kind.
pub const COMMAND: &str = "addon.color.applyNearestSwatch";
pub const KIND: &str = "nearestSwatch";
/// The query that lists the matches, and how many of each colour's the dialog shows.
const FIND: &str = "addon.color.nearestSwatch";
const MATCHES: u64 = 3;

/// What the query read: the document and its revision, the library, Fill and Stroke. An answer
/// stands while these are what the dialog shows now.
type QueryKey = (u64, u64, String, bool, bool);

/// What the dialog's controls edit; its serde names are the field names automation sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Fields {
    /// The library to match against: [`DOCUMENT_SWATCHES`] or a library id.
    pub(crate) library: String,
    /// Match the fills.
    pub(crate) fill: bool,
    /// Match the strokes.
    pub(crate) stroke: bool,
}

/// Find Nearest Swatch: the colours of the selection with their closest swatches.
pub(crate) struct NearestDialog {
    pub(crate) fields: Fields,
    /// The libraries the Library choice offers, built when the dialog opened.
    libraries: Vec<Entry<String>>,
    /// Whether something was selected when the dialog opened (it is modal, so that stays true).
    selected: bool,
    /// The last answer of the query, with what it was asked for ([`pump`]).
    pub(crate) found: Option<(QueryKey, Result<Value, String>)>,
}

/// Opens the dialog on the document's swatches, fills and strokes.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = Fields { library: DOCUMENT_SWATCHES.into(), fill: true, stroke: true };
    let libraries = swatches::library_entries(app);
    let selected = app.session.active().is_some_and(|st| !st.selection.is_empty());
    app.dialogs().ok_or("no window")?.with(|host| host.open(NearestDialog { fields, libraries, selected, found: None }));
    Ok(json!({ "dialog": KIND }))
}

/// The command's params for the dialog's `fields`.
fn params(fields: &Fields) -> Value {
    json!({ "library": fields.library, "fill": fields.fill, "stroke": fields.stroke })
}

/// One line of the answer: a colour of the selection, or a swatch near the colour above it.
struct Line {
    color: Srgba,
    text: String,
    /// The difference to the colour above; `None` for the colour itself.
    delta_e: Option<f64>,
}

/// The lines of an answer of the query: each colour, then its matches, closest first.
fn lines(found: &Value) -> Vec<Line> {
    let color = |hex: &Value| Srgba::from_hex(hex.as_str().unwrap_or_default()).unwrap_or(Srgba::rgb(0.0, 0.0, 0.0));
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let mut lines = Vec::new();
    for c in found["colors"].as_array().into_iter().flatten() {
        lines.push(Line { color: color(&c["hex"]), text: text(&c["values"]), delta_e: None });
        for m in c["matches"].as_array().into_iter().flatten() {
            lines.push(Line { color: color(&m["hex"]), text: text(&m["name"]), delta_e: Some(m["deltaE"].as_f64().unwrap_or_default()) });
        }
    }
    lines
}

impl StackDialog for NearestDialog {
    type Result = Fields;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Fields>> {
        let NearestDialog { fields, libraries, selected, found } = self;
        let selected = *selected;
        let shown = FormDialog::new(label(tl!("Find Nearest Swatch")), Bind::bound(fields)).ok_label(label(tl!("Apply"))).ok_enabled(selected).show(
            ui,
            |ui, _, fields| {
                body(ui, fields, libraries, selected, found.as_ref());
                selected
            },
        );
        finished(shown)
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Fields> {
        self.selected.then(|| self.fields.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        serde_fields(&self.fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        set_serde_field(&mut self.fields, field, value)
    }
}

/// The dialog's rows: the library, Fill and Stroke, then what the query found (or what to do first).
fn body(ui: &mut Ui, fields: &mut Fields, libraries: &[Entry<String>], selected: bool, found: Option<&(QueryKey, Result<Value, String>)>) {
    Field::new(label(tl!("Library:"))).show(ui, |ui, _| {
        let picked = Select::new(label(tl!("Library:")), Bind::given(fields.library.clone()), libraries).show(ui).value().cloned();
        if let Some(library) = picked {
            fields.library = library;
        }
    });
    Row::new().show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut fields.fill)).label(label(tl!("Fill"))).show(ui);
        Checkbox::new(Bind::bound(&mut fields.stroke)).label(label(tl!("Stroke"))).show(ui);
    });
    if !selected {
        Text::new(label(tl!("Select objects to find the swatches nearest their colors."))).shade(Shade::Muted).show(ui);
        return;
    }
    // An answer asked for other choices is the last one's: the next pass of the pump replaces it.
    let Some((_, answer)) = found.filter(|(key, _)| key.2 == fields.library && key.3 == fields.fill && key.4 == fields.stroke) else { return };
    let found = match answer {
        Ok(found) => found,
        Err(e) => {
            Text::new(Label::from(crate::i18n::message(crate::i18n::current(), e).to_string())).shade(Shade::Muted).show(ui);
            return;
        }
    };
    let lines = lines(found);
    let keys: Vec<usize> = (0..lines.len()).collect();
    let source = keyed(&keys, |i| *i);
    List::<usize>::new(label(tl!("Find Nearest Swatch"))).select(SelectMode::None).show_items(ui, &source, &mut |&i, item| match lines.get(i) {
        Some(line) => {
            let face = SwatchFace::new(line.color);
            let row = ListRow::new(line.text.clone())
                .strong(line.delta_e.is_none())
                .thumb(ThumbSize::Small, move |cx, rect| face.paint(cx, rect))
                .state(item.row.clone());
            match line.delta_e {
                Some(de) => row.trailing(format!("ΔE {de:.1}")),
                None => row,
            }
        }
        None => ListRow::new(""),
    });
}

/// Each frame: asks the query again when what it read changed (the document, its revision, the
/// library, Fill or Stroke) and keeps the answer in the dialog; Apply gives each colour its nearest
/// swatch, after the dialog closed.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    if let Some(answer) = dialogs.with(|host| host.take_by_id::<NearestDialog>(id)) {
        if let DialogResult::Ok(fields) = answer
            && let Err(e) = app.run(COMMAND, params(&fields))
        {
            app.status(e);
        }
        return;
    }
    let Some((fields, selected, asked)) = dialogs
        .with(|host| host.get_by_id::<NearestDialog>(id).map(|d| (d.fields.clone(), d.selected, d.found.as_ref().map(|(key, _)| key.clone()))))
    else {
        return;
    };
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision, fields.library.clone(), fields.fill, fields.stroke);
    if !selected || asked.as_ref() == Some(&key) {
        return;
    }
    let mut p = params(&fields);
    p["count"] = json!(MATCHES);
    let answer = app.session.execute(FIND, &p).map_err(|e| e.to_string());
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<NearestDialog>(id) {
            d.found = Some((key, answer));
        }
    });
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{DialogHost, dialog_id};
    use serde_json::json;
    use vectorcraft_color::Paint;
    use vectorcraft_engine::Session;

    use super::{COMMAND, KIND, NearestDialog};
    use crate::{VectorcraftApp, menus};

    /// The host the dialog lives on.
    fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().expect("a window").with(f)
    }

    /// The answer the dialog holds now: the colours' texts and the swatches' names of its lines.
    fn listed(app: &VectorcraftApp) -> Option<Vec<(String, bool)>> {
        let found = host(app, |h| h.get::<NearestDialog>().and_then(|d| d.found.clone()))?.1.ok()?;
        Some(super::lines(&found).into_iter().map(|l| (l.text, l.delta_e.is_some())).collect())
    }

    #[test]
    fn find_nearest_swatch_lists_the_matches_and_applies_the_nearest() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let ctx = app.ui_ctx.clone().expect("the test context");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff8000", "global": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        app.run("paint.setFill", json!({"color": "#ff8010"})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        menus::invoke(&mut app, COMMAND, serde_json::Value::Null);
        let id = dialog_id(KIND);
        crate::dialogs::host::pump(&mut app, &ctx);
        let lines = listed(&app).expect("the answer");
        assert_eq!(lines.first().map(|l| l.0.as_str()), Some("R=255 G=128 B=16"), "the fill's colour heads its matches");
        assert!(lines.iter().any(|(text, is_match)| *is_match && text == "Brand"), "Brand is a match: {lines:?}");
        // Another library lists its own swatches.
        host(&app, |h| h.set_json_field(&ctx, id, "library", json!("web-safe-216"))).unwrap();
        crate::dialogs::host::pump(&mut app, &ctx);
        let other = listed(&app).expect("the answer for the other library");
        assert!(other.iter().any(|(_, is_match)| *is_match), "the web library has matches: {other:?}");
        assert!(other.iter().all(|(text, _)| text != "Brand"), "the document's swatch is not in a web library: {other:?}");
        host(&app, |h| h.set_json_field(&ctx, id, "library", json!("document"))).unwrap();
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(listed(&app), Some(lines), "the document's answer again");
        // Apply: one undo step, and the fill is the swatch.
        let undo = app.session.doc().unwrap().history.undo.len();
        assert!(host(&app, |h| h.confirm_by_id(&ctx, id)));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(host(&app, |h| !h.is_open::<NearestDialog>()));
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let fill = st.doc.node(st.selection.objects[0]).unwrap().appearance.fill_paint();
        assert!(matches!(fill, Paint::Solid { swatch: Some(n), .. } if n == "Brand"));
        // Nothing selected: the dialog says what to do and Apply is refused.
        app.run("select.none", json!({})).unwrap();
        menus::invoke(&mut app, COMMAND, serde_json::Value::Null);
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(listed(&app), None, "no query without a selection");
        assert!(!host(&app, |h| h.confirm_by_id(&ctx, id)), "Apply is off");
        assert!(host(&app, |h| h.is_open::<NearestDialog>()));
    }
}
