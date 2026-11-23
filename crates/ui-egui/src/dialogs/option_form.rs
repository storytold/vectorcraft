//! The one dialog behind Live Paint Bucket Options, Live Paint Selection Tool Options, Shape Builder Tool
//! Options, Tile Edge Color and Gap Options: a form over JSON values on the window's dialog host. Each dialog is
//! an [`OptionSpec`]: its keys, its rows, what OK runs. OK files the values as a held answer; [`pump`] checks the
//! colours, runs the spec and answers it, so a refusal keeps the dialog open with its reason.

use std::ops::RangeInclusive;

use craft::theme::variant::Tone;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormDialog, HostRequests};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label, LengthUnits, UnitKind};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::ColorRequest;
use craft::ui::text::NumberField;
use egui::{Context, Ui};
use serde_json::{Map, Value, json};

use super::tile_edge_color::checked;
pub(super) use super::tool_options::title as tool_title;
use super::{layer_color, live_paint_bucket, live_paint_gaps, live_paint_selection, shape_builder_options, tile_edge_color};
use crate::VectorcraftApp;


/// What OK does with a dialog's checked values and its extra value.
type Run = fn(&mut VectorcraftApp, &Map<String, Value>, &Value) -> Result<Value, String>;

/// What one of the dialogs the form serves is.
pub(super) struct OptionSpec {
    /// The dialog kind: the key automation and the host know the dialog by.
    pub key: &'static str,
    /// The title.
    pub title: fn() -> String,
    /// The values the dialog edits; a field automation sets is one of them.
    pub keys: &'static [&'static str],
    /// The keys whose value is a colour: OK refuses a value that is not one.
    pub colors: &'static [&'static str],
    /// Whether the footer has the Preview check box (the `preview` field).
    pub preview: bool,
    /// The rows.
    pub body: fn(&mut Ui, &mut Rows<'_>),
    /// Whether the row of `key` takes no input with these values.
    pub inactive: fn(&Map<String, Value>, &str) -> bool,
    /// What OK does with the checked values and [`OptionForm::extra`].
    pub run: Run,
    /// What closing the dialog without OK releases (a running preview).
    pub cancelled: Option<fn(&mut VectorcraftApp)>,
}

/// Every dialog the form serves, to find the spec of a finished dialog by its kind.
static SPECS: [&OptionSpec; 5] =
    [&live_paint_bucket::SPEC, &live_paint_selection::SPEC, &shape_builder_options::SPEC, &tile_edge_color::SPEC, &live_paint_gaps::SPEC];

/// The rows of a dialog over its values: each row reads a key and writes it when the user changes it.
pub(super) struct Rows<'a> {
    values: &'a mut Map<String, Value>,
    lengths: LengthUnits,
    /// The Select Custom Color… request this pass filed.
    request: Option<ColorRequest>,
}

/// `text` as a label; the caller has translated it.
fn word(text: &str) -> Label {
    Label::from(text.to_owned())
}

/// Whether `values[key]` is `true`.
pub(super) fn is_on(values: &Map<String, Value>, key: &str) -> bool {
    values.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A select of `labels` showing the entry at `current`; the entry picked this pass.
pub(super) fn pick(ui: &mut Ui, label: &str, current: usize, labels: &[&str]) -> Option<usize> {
    let entries: Vec<Entry<usize>> = labels.iter().enumerate().map(|(at, text)| Entry::new(at, word(text))).collect();
    Select::new(word(label), Bind::given(current), &entries).show(ui).value().copied()
}

impl Rows<'_> {
    /// `values[key]` as text.
    pub(super) fn text(&self, key: &str) -> &str {
        self.values.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    /// Whether `values[key]` is `true`.
    pub(super) fn flag(&self, key: &str) -> bool {
        is_on(self.values, key)
    }

    /// `values[key]` as a number, `default` when it holds none.
    pub(super) fn float(&self, key: &str, default: f64) -> f64 {
        self.values.get(key).and_then(Value::as_f64).unwrap_or(default)
    }

    /// Sets `values[key]`.
    pub(super) fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_owned(), value);
    }

    /// A row named `label` over what `body` shows; `body` gets the label for its control. A disabled row takes no input.
    pub(super) fn field(&mut self, ui: &mut Ui, label: &str, enabled: bool, mut body: impl FnMut(&mut Ui, &mut Self, &str)) {
        Field::new(word(label)).enabled(enabled).show(ui, |ui, _| body(ui, self, label));
    }

    /// A check box over `values[key]`.
    pub(super) fn check(&mut self, ui: &mut Ui, key: &str, label: &str, enabled: bool) {
        let shown = Checkbox::new(Bind::given(self.flag(key))).label(word(label)).enabled(enabled).show(ui);
        if let Some(on) = shown.value().copied() {
            self.set(key, json!(on));
        }
    }

    /// A select over `values[key]`, one of the `(value, label)` options.
    pub(super) fn select(&mut self, ui: &mut Ui, key: &str, label: &str, options: &[(&str, &str)]) {
        let labels: Vec<&str> = options.iter().map(|(_, text)| *text).collect();
        let current = options.iter().position(|(value, _)| *value == self.text(key)).unwrap_or(0);
        if let Some((value, _)) = pick(ui, label, current, &labels).and_then(|at| options.get(at)) {
            self.set(key, json!(value));
        }
    }

    /// Radio buttons over `values[key]`, one of the `(value, label)` options, side by side.
    pub(super) fn radios(&mut self, ui: &mut Ui, key: &str, label: &str, options: &[(&str, &str)]) {
        let mut chosen = self.text(key).to_owned();
        RadioGroup::new(word(label), Bind::bound(&mut chosen)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (value, text) in options {
                    Radio::new((*value).to_owned()).label(word(text)).show(ui);
                }
            });
        });
        if chosen != self.text(key) {
            self.set(key, json!(chosen));
        }
    }

    /// A number field over `values[key]`, counting `unit`, kept within `range`.
    pub(super) fn number(&mut self, ui: &mut Ui, key: &str, label: &str, range: RangeInclusive<f64>, unit: UnitKind, enabled: bool) {
        let current = self.float(key, *range.start());
        let symbol = Env::of(ui).units().symbol(unit);
        let shown = NumberField::new(word(label), Bind::given(Some(current))).range(range).unit_kind(unit).unit(symbol).enabled(enabled).show(ui);
        if let Some(Some(number)) = shown.value().copied() {
            self.set(key, json!(number));
        }
    }

    /// A length field over `values[key]`, kept in points and shown in the General unit, within `range` (points).
    pub(super) fn length(&mut self, ui: &mut Ui, key: &str, label: &str, range: RangeInclusive<f64>, enabled: bool) {
        let current = self.values.get(key).and_then(Value::as_f64);
        let shown = NumberField::length(word(label), Bind::given(current), self.lengths).range(range).enabled(enabled).show(ui);
        if let Some(Some(length)) = shown.value().copied() {
            self.set(key, json!(length));
        }
    }

    /// The layer-colour row over `values[key]`; its Select Custom Color… request goes to the host.
    pub(super) fn color(&mut self, ui: &mut Ui, key: &str, label: &str, enabled: bool) {
        let request = layer_color::row(ui, self.values, key, word(label), enabled);
        self.request = request.or(self.request.take());
    }
}

/// An open dialog of the form.
pub(crate) struct OptionForm {
    spec: &'static OptionSpec,
    id: DialogId,
    /// The values the spec's keys hold.
    pub(super) values: Map<String, Value>,
    /// What the spec's `run` needs beside the values: the groups Gap Options applies to.
    pub(super) extra: Value,
    /// Lengths kept in points and shown in the General unit, read when the dialog opened.
    lengths: LengthUnits,
    /// The Preview check box.
    pub(super) preview: bool,
    /// Whether the dialog's container is open.
    open: bool,
    /// The OK the dialog filed for [`pump`] to answer.
    pub(super) held: HeldAnswer<Map<String, Value>>,
    requests: HostRequests,
    /// The gaps Gap Options' last preview found, as `livePaint.gapOptions` returns them; `null` while none.
    pub(super) previewed: Value,
}

impl OptionForm {
    /// Whether the dialog is the one of kind `key`.
    pub(super) fn is(&self, key: &str) -> bool {
        self.spec.key == key
    }
}

/// Opens the dialog `spec` describes on `values`; `None` without a window.
pub(super) fn open(app: &mut VectorcraftApp, spec: &'static OptionSpec, values: Map<String, Value>, extra: Value) -> Option<DialogId> {
    let dialogs = app.dialogs()?;
    let lengths = crate::canvas::rulers::general_lengths(app);
    let form = OptionForm {
        spec,
        id: dialog_id(spec.key),
        values,
        extra,
        lengths,
        preview: false,
        open: true,
        held: HeldAnswer::new(),
        requests: HostRequests::new(),
        previewed: Value::Null,
    };
    Some(dialogs.with(|host| host.open(form)))
}

/// Opens the dialog `spec` describes on the options `tool` keeps for the spec's keys.
pub(super) fn open_tool(app: &mut VectorcraftApp, spec: &'static OptionSpec, tool: &str) -> Result<Value, String> {
    let options = app.session.tool_options_of(tool);
    let values = spec.keys.iter().filter_map(|key| Some((key.to_string(), options.get(*key)?.clone()))).collect();
    open(app, spec, values, Value::Null).ok_or("no window")?;
    Ok(json!({ "dialog": spec.key }))
}

/// Sets `tool`'s options to `values`; the tool refuses a value it cannot take and sets none.
pub(super) fn set_tool_options(app: &mut VectorcraftApp, tool: &str, values: &Map<String, Value>) -> Result<Value, String> {
    app.run("tool.setOption", json!({ "tool": tool, "values": values }))
}

/// Whether `new` is a value a field holding `current` takes: a switch, a number or a text, of the kind it has.
fn same_kind(current: Option<&Value>, new: &Value) -> bool {
    match current {
        Some(current) => {
            matches!((current, new), (Value::Bool(_), Value::Bool(_)) | (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)))
        }
        None => matches!(new, Value::Bool(_) | Value::Number(_) | Value::String(_)),
    }
}

impl StackDialog for OptionForm {
    type Result = Map<String, Value>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(self.spec.key)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let measuring = craft::ui::runtime::measure::active(ui.ctx());
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = self.held.is_accepted() && !measuring;
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let OptionForm { spec, values, lengths, preview, open, held, requests, .. } = self;
        let (spec, lengths) = (*spec, *lengths);
        let title = (spec.title)();
        let refusal = held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone()));
        let mut form = FormDialog::new(word(&title), Bind::bound(&mut *values))
            .opened(Bind::bound(open))
            .accept_stays_open(true)
            .ok_enabled(!held.is_waiting())
            .answers(answers);
        if spec.preview {
            form = form.preview(Some(Bind::bound(preview)));
        }
        let shown = form.show(ui, |ui, form, values| {
            let answers = form.take_answers();
            for key in spec.colors {
                layer_color::apply_answers(values, key, &answers);
            }
            let mut rows = Rows { values: &mut *values, lengths, request: None };
            FieldGroup::new(word(&title)).show(ui, |ui, _| (spec.body)(ui, &mut rows));
            if let Some(request) = rows.request.take() {
                form.requests().request_color(request);
            }
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == *values) {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            if finish {
                form.request_close_as(CloseReason::Activated);
            }
            true
        });
        requests.extend(shown.requests);
        if held.refusal().is_some_and(|(_, refused)| *refused != *values) {
            held.clear_refusal();
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if shown.picked == Some(0) && !held.is_waiting() {
            held.file(values.clone());
        }
        if finish && shown.closed == Some(CloseReason::Activated) {
            return Some(held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as the button: files the values for [`pump`] to answer, and answers them once the app accepted them.
    fn confirm(&mut self, _: &Context) -> Option<Self::Result> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if !self.held.is_waiting() {
            self.held.file(self.values.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// The values by key, and `preview` where the dialog has the check box.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.values.clone();
        if self.spec.preview {
            fields.insert("preview".to_owned(), json!(self.preview));
        }
        Some(Value::Object(fields))
    }

    /// Sets a field as its row does: a key the dialog has, whose row takes input now, to a value of the kind the
    /// field holds. A colour is checked at OK, as the dialog's OK does.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if field == "preview" && self.spec.preview {
            self.preview = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            return Ok(());
        }
        if !self.spec.keys.contains(&field) {
            return Err(FieldRefusal::Unknown.message(field));
        }
        if (self.spec.inactive)(&self.values, field) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        if !same_kind(self.values.get(field), &value) {
            return Err(FieldRefusal::Rejects(&value).message(field));
        }
        self.values.insert(field.to_owned(), value);
        Ok(())
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    fn answer_color(&mut self, key: &str, color: craft::theme::Srgba) {
        self.requests.answer_color(key, color);
    }

    fn cancelled(&mut self, _: &Context) {
        self.requests.cancel();
    }
}

/// Answers the OK the open dialog filed: its colours are checked, then its spec runs; a refusal keeps the dialog open
/// with the reason. Releases what a dialog closed without OK held. Once per frame, after the shell pass.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let filed = dialogs.with(|host| {
        let form = host.get_mut::<OptionForm>()?;
        Some((form.spec, form.extra.clone(), form.held.take()?))
    });
    if let Some((spec, extra, answer)) = filed {
        let verdict = accept(app, spec, &answer, &extra);
        dialogs.with(|host| {
            if let Some(form) = host.get_mut::<OptionForm>() {
                form.held.answer(verdict.map_err(Label::from));
            }
        });
        ctx.request_repaint();
    }
    if let Some((DialogResult::Cancel, Some(key))) = dialogs.with(|host| host.take_keyed::<OptionForm>())
        && let Some(cancelled) = SPECS.iter().find(|spec| spec.key == key).and_then(|spec| spec.cancelled)
    {
        cancelled(app);
    }
}

/// `answer` accepted or refused: every colour is a colour, and the spec runs.
fn accept(app: &mut VectorcraftApp, spec: &OptionSpec, answer: &Map<String, Value>, extra: &Value) -> Result<(), String> {
    for key in spec.colors {
        if let Some(color) = answer.get(*key).and_then(Value::as_str) {
            checked(color)?;
        }
    }
    (spec.run)(app, answer, extra).map(|_| ())
}

#[cfg(test)]
pub(super) mod tests {
    use vectorcraft_engine::Session;

    use super::*;

    /// An app with a window context for the dialog host and a blank document.
    pub(in crate::dialogs) fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    /// The pumps of the dialogs of the form, in the order a frame runs them.
    pub(in crate::dialogs) fn pump_all(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        pump(app, &ctx);
        live_paint_gaps::pump(app, &ctx);
    }

    /// Whether the form is open.
    pub(in crate::dialogs) fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|host| host.is_open::<OptionForm>())
    }

    /// The fields of the open form, as automation lists them.
    pub(in crate::dialogs) fn fields(app: &VectorcraftApp) -> Value {
        app.dialogs().unwrap().with(|host| host.get::<OptionForm>().expect("a dialog of the form is open").json_fields().unwrap())
    }

    /// Sets `field` of the open form as automation does.
    pub(in crate::dialogs) fn set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|host| {
            let id = host.get::<OptionForm>().expect("a dialog of the form is open").id();
            host.set_json_field(&ctx, id, field, value)
        })
    }

    /// Where the held OK of the open form stands.
    pub(in crate::dialogs) fn held(app: &VectorcraftApp) -> Option<HeldOk> {
        app.dialogs().unwrap().with(|host| host.get::<OptionForm>().and_then(|form| form.held_state()))
    }

    /// Presses OK: the form files its values, the pumps answer them, and an accepted OK closes the dialog. Whether the
    /// dialog closed; a refused OK leaves it open with the reason.
    pub(in crate::dialogs) fn press_ok(app: &mut VectorcraftApp) -> bool {
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        host.with(|host| host.confirm::<OptionForm>(&ctx));
        pump_all(app);
        // Another OK after a refusal would file the refused values again.
        host.with(|host| host.get::<OptionForm>().and_then(|form| form.held_state()) == Some(HeldOk::Accepted) && host.confirm::<OptionForm>(&ctx))
    }

    /// Closes the open form as Cancel or Escape does, then runs the pumps, which release what the dialog held.
    pub(in crate::dialogs) fn cancel(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|host| host.cancel::<OptionForm>(&ctx));
        pump_all(app);
    }

    /// Whether `a` and `b` are the same value, numbers by their value.
    fn same(a: &Value, b: &Value) -> bool {
        match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        }
    }

    /// One dialog of the table: the tool that opens it, what it opens on, what is set and what OK leaves.
    struct Case {
        tool: &'static str,
        opens_on: Vec<(&'static str, Value)>,
        sets: Vec<(&'static str, Value)>,
        leaves: Vec<(&'static str, Value)>,
    }

    /// Double-clicking a tool opens its options on the tool's values; a field set and OK give them to the tool, which
    /// keeps them across tools, and the dialog opens on them again.
    #[test]
    fn each_options_dialog_opens_on_its_tools_options_and_ok_sets_them() {
        let cases = [
            Case {
                tool: "livePaintBucket",
                opens_on: vec![
                    ("paintFills", json!(true)),
                    ("paintStrokes", json!(false)),
                    ("cursorSwatchPreview", json!(true)),
                    ("highlight", json!(true)),
                    ("color", json!("#ff2a2a")),
                    ("width", json!(4.0)),
                ],
                sets: vec![
                    ("paintFills", json!(false)),
                    ("paintStrokes", json!(true)),
                    ("cursorSwatchPreview", json!(false)),
                    ("color", json!("Green")),
                    ("width", json!(8)),
                ],
                leaves: vec![
                    ("paintFills", json!(false)),
                    ("paintStrokes", json!(true)),
                    ("cursorSwatchPreview", json!(false)),
                    ("color", json!("#4fff4f")),
                    ("width", json!(8.0)),
                ],
            },
            Case {
                tool: "livePaintSelection",
                opens_on: vec![("selectFills", json!(true)), ("selectStrokes", json!(true)), ("color", json!("#ff2a2a")), ("width", json!(4.0))],
                sets: vec![("selectFills", json!(false)), ("color", json!("Green")), ("width", json!(8))],
                leaves: vec![("selectFills", json!(false)), ("selectStrokes", json!(true)), ("color", json!("#4fff4f")), ("width", json!(8.0))],
            },
            Case {
                tool: "shapeBuilder",
                opens_on: vec![
                    ("openFilledAsClosed", json!(true)),
                    ("pickColorFrom", json!("artwork")),
                    ("cursorSwatchPreview", json!(false)),
                    ("selection", json!("freeform")),
                    ("highlightFill", json!(true)),
                    ("highlightStroke", json!(true)),
                    ("highlightColor", json!("#ff4f4f")),
                ],
                sets: vec![
                    ("openFilledAsClosed", json!(false)),
                    ("pickColorFrom", json!("swatches")),
                    ("cursorSwatchPreview", json!(true)),
                    ("selection", json!("straightLine")),
                    ("highlightFill", json!(false)),
                    ("highlightColor", json!("Green")),
                ],
                leaves: vec![
                    ("openFilledAsClosed", json!(false)),
                    ("pickColorFrom", json!("swatches")),
                    ("cursorSwatchPreview", json!(true)),
                    ("selection", json!("straightLine")),
                    ("highlightFill", json!(false)),
                    ("highlightColor", json!("#4fff4f")),
                ],
            },
        ];
        for Case { tool, opens_on, sets, leaves } in cases {
            let mut app = app();
            crate::toolbar::open_options(&mut app, tool).unwrap();
            let shown = fields(&app);
            for (key, want) in &opens_on {
                assert!(same(&shown[*key], want), "{tool}: {key} opens on {want}, not {}", shown[*key]);
            }
            assert!(set(&app, "nope", json!(true)).is_err(), "{tool}: a key the dialog has not is refused");
            for (key, value) in sets {
                set(&app, key, value).unwrap_or_else(|e| panic!("{tool}: {e}"));
            }
            assert!(press_ok(&mut app), "{tool}: OK closes the dialog");
            assert!(!is_open(&app));
            // The tool keeps them while another tool is chosen, and the dialog opens on them again.
            app.select_tool("selection");
            let kept = app.session.tool_options_of(tool);
            for (key, want) in &leaves {
                assert!(same(&kept[*key], want), "{tool}: {key} stays {want}, not {}", kept[*key]);
            }
            crate::toolbar::open_options(&mut app, tool).unwrap();
            let shown = fields(&app);
            for (key, want) in &leaves {
                assert!(same(&shown[*key], want), "{tool}: {key} reopens on {want}, not {}", shown[*key]);
            }
        }
    }

    /// A row that takes no input refuses its field; a colour with an alpha part is refused at OK: the dialog stays
    /// open with the reason and the tool's options stay as they were.
    #[test]
    fn a_disabled_row_refuses_its_field_and_a_bad_colour_keeps_the_dialog_open() {
        let mut app = app();
        crate::toolbar::open_options(&mut app, "livePaintBucket").unwrap();
        set(&app, "highlight", json!(false)).unwrap();
        assert!(set(&app, "color", json!("Green")).is_err(), "Color takes no input while Highlight is off");
        set(&app, "highlight", json!(true)).unwrap();
        set(&app, "color", json!("#ff000080")).unwrap();
        assert!(!press_ok(&mut app));
        assert!(is_open(&app));
        assert!(matches!(held(&app), Some(HeldOk::Refused(_))));
        assert_eq!(app.session.tool_options_of("livePaintBucket")["color"], json!("#ff2a2a"));
        set(&app, "color", json!("#ff0000")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.tool_options_of("livePaintBucket")["color"], json!("#ff0000"));
    }
}
