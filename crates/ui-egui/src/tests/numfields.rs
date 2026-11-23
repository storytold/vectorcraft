//! The app's own number-field rules over the framework's `NumberField`: the size codec's percentage rule, the
//! Numbers Without Units Are Points preference, and the commit of typed text before the canvas acts on a press.

use std::cell::RefCell;
use std::rc::Rc;

use craft::behavior::NumberCodec;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::runtime::env::{Env, LengthUnits, UnitKind, Units};
use craft::ui::testing::Fixture;
use craft::ui::text::NumberField;
use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;

use super::scrub::window;
use crate::{VectorcraftApp, fields}; // PLAN: assumed `fields` is the module of wg.S8

/// Points per inch of the documents the fields below measure.
const PPI: f64 = 72.0;
/// How far two lengths may differ and still count as the same.
const SAME: f64 = 1e-6;

/// A length field keeping `start` points and showing `lengths`, with the codec `codec` builds from the env's units (it
/// reads the shown unit, as a length field's codec does): focus it, replace its text with `text` and press Enter. → The
/// value the field holds afterwards, in points.
fn typed(start: f64, text: &str, lengths: LengthUnits, codec: impl Fn(&Units) -> NumberCodec + 'static) -> Option<f64> {
    let value = Rc::new(RefCell::new(Some(start)));
    let shared = value.clone();
    let mut fixture = Fixture::new(move |ui| {
        let codec = codec(&Env::of(ui).units());
        NumberField::length("Width", Bind::bound(&mut *shared.borrow_mut()), lengths).codec(codec).id_salt("width").show(ui);
    });
    fixture.settle();
    let at = fixture.shown().center();
    fixture.click_at(at).idle(2).key(Key::A, Modifiers::COMMAND).type_text(text).idle(1).key(Key::Enter, Modifiers::NONE).idle(2);

    *value.borrow()
}

/// `typed` in a field that stores points and shows `shown`, reading with the size codec.
fn typed_in_size(shown: UnitKind, text: &str) -> Option<f64> {
    let lengths = LengthUnits::new(UnitKind::Points, shown, PPI);
    typed(PPI, text, lengths, move |units| fields::size_codec(units, lengths, 2, None, false))
}

#[test]
fn a_percentage_typed_in_a_field_is_a_share_of_the_value_before_it() {
    let got = typed_in_size(UnitKind::Centimetres, "3cm*50%").expect("the field reads `3cm*50%`");
    let want = 1.5 / 2.54 * PPI;
    assert!((got - want).abs() < SAME, "{got} points, not {want}");
}

/// Preferences › Units › Numbers Without Units Are Points (#394): on (the default), a bare number typed into a picas
/// field is points and a typed unit still wins; off, it is picas. Any other unit's field reads it in its own unit.
#[test]
fn bare_numbers_in_picas_fields_are_points_when_the_preference_says_so() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let mut read = |on: bool, shown: UnitKind, text: &str| {
        app.run("prefs.set", json!({"key": "numbersWithoutUnitsArePoints", "value": on})).expect("the preference is set");
        let flag = app.session.prefs.numbers_without_units_are_points;
        assert_eq!(flag, on, "the preference reads back");
        let lengths = LengthUnits::new(UnitKind::Points, shown, PPI);
        typed(PPI, text, lengths, move |units| NumberCodec::new(fields::length_reader(units, lengths, flag), 2)).expect(text)
    };
    let (picas, mm) = (UnitKind::Picas, UnitKind::Millimetres);
    let mm_pt = |n: f64| n / 25.4 * PPI;
    let cases = [
        (true, picas, "12", 12.0, "points in a picas field"),
        (true, picas, "2p6", 30.0, "picas and points as typed"),
        (true, picas, "1in", 72.0, "a typed unit wins"),
        (true, picas, "10+5", 15.0, "arithmetic in points too"),
        (true, picas, "10x2", 20.0, "x multiplies (#8435)"),
        (true, mm, "2 x 3", mm_pt(6.0), "x multiplies in millimetres"),
        (true, mm, "12", mm_pt(12.0), "a millimetre field keeps millimetres"),
        (false, picas, "12", 12.0 * 12.0, "off: picas"),
        (false, mm, "12", mm_pt(12.0), "off: millimetres"),
    ];
    for (on, shown, text, want, why) in cases {
        let got = read(on, shown, text);
        assert!((got - want).abs() < SAME, "{why}: `{text}` gave {got}, not {want}");
    }
}

/// Text typed into the Transform panel's W field is committed in the frame of a press elsewhere, before the canvas
/// acts on the press: a click that selects another object must not get the typed value.
#[test]
fn a_press_on_the_canvas_commits_to_the_object_being_edited() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).expect("a document");
    // The big rectangle first, so the small one drawn after it is the selection; the click below lands on the big one.
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).expect("the big rectangle");
    let big = app.session.doc().map(|d| d.selection.objects.clone()).expect("a document");
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).expect("the small rectangle");
    let small = app.session.doc().map(|d| d.selection.objects.clone()).expect("a document");
    let width = |app: &VectorcraftApp, ids: &_| app.session.transform_box(ids).map(|b| b.rect.width());
    assert_eq!(width(&app, &small), Some(100.0));

    let app = Rc::new(RefCell::new(app));
    let shared = app.clone();
    let mut fixture = Fixture::new(move |ui| window(&mut shared.borrow_mut(), ui));
    fixture.settle();
    // The Transform panel's W field is named by its label, "W"; a click on it focuses the field.
    let label = fixture.by_label("W").expect("the Transform panel shows the W field");
    fixture.click(&label).idle(2).key(Key::A, Modifiers::COMMAND).type_text("42").idle(1);
    // PLAN: assumed the artboard is centred in the window, so the window's centre is on the big rectangle only.
    let canvas = fixture.ctx().content_rect().center();
    fixture.click_at(canvas).idle(3);

    let app = app.borrow();
    assert_eq!(width(&app, &small), Some(42.0), "the typed value went to the object being edited");
    assert_eq!(width(&app, &big), Some(400.0), "the object the press selected is unchanged");
    assert_eq!(app.session.doc().ok().map(|d| d.selection.objects.clone()), Some(big), "the press selected the other object");
}
