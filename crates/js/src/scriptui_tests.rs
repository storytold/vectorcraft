//! ScriptUI's `Window`: building a dialog, its description, the events a host gives `show()`, the
//! handlers they run, and the limits on what a dialog may hold.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::*;

/// What the host was given: each description, and each `end`'s result and whether it was normal.
#[derive(Default)]
struct Seen {
    descriptions: Vec<Data>,
    ends: Vec<(f64, bool)>,
}

/// The events `wait` has left to give.
type Queue = Rc<RefCell<VecDeque<Data>>>;

/// A realm whose `wait` gives the events in `events`, in order, and throws when none is left
/// (or waits, where the run can).
fn realm(events: Vec<Data>) -> (Realm, Rc<RefCell<Seen>>, Queue) {
    let mut r = Realm::new();
    r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
    let seen: Rc<RefCell<Seen>> = Rc::default();
    let queue = Rc::new(RefCell::new(VecDeque::from(events)));
    let can_wait = r.wait_flag();
    let s = seen.clone();
    let open: HostFunction = Rc::new(move |args| {
        s.borrow_mut().descriptions.push(args.into_iter().next().unwrap_or(Data::Undefined));
        Ok(Data::Undefined)
    });
    let s = seen.clone();
    let q = queue.clone();
    let wait: HostFunction = Rc::new(move |args| {
        s.borrow_mut().descriptions.push(args.into_iter().next().unwrap_or(Data::Undefined));
        match q.borrow_mut().pop_front() {
            Some(e) => Ok(e),
            None if can_wait.get() => Err(HostError::Wait),
            None => Err(HostError::Error("no event left".into())),
        }
    });
    let s = seen.clone();
    let end: HostFunction = Rc::new(move |args| {
        let n = match args.first() {
            Some(Data::Number(n)) => *n,
            _ => f64::NAN,
        };
        s.borrow_mut().ends.push((n, matches!(args.get(1), Some(Data::Bool(true)))));
        Ok(Data::Undefined)
    });
    let items = vec![("open".into(), HostItem::Function(open)), ("wait".into(), HostItem::Function(wait)), ("end".into(), HostItem::Function(end))];
    r.define_script_ui(items).unwrap();
    (r, seen, queue)
}

fn click(id: usize) -> Data {
    Data::Object(vec![("event".into(), Data::String("click".into())), ("id".into(), Data::Number(id as f64))])
}

fn change(id: usize, value: Data) -> Data {
    Data::Object(vec![("event".into(), Data::String("change".into())), ("id".into(), Data::Number(id as f64)), ("value".into(), value)])
}

fn close() -> Data {
    Data::Object(vec![("event".into(), Data::String("close".into()))])
}

fn field<'a>(d: &'a Data, name: &str) -> &'a Data {
    match d {
        Data::Object(fields) => fields.iter().find(|(k, _)| k == name).map_or(&Data::Undefined, |(_, v)| v),
        _ => &Data::Undefined,
    }
}

fn nodes(d: &Data) -> Vec<Data> {
    match field(d, "nodes") {
        Data::Array(a) => a.clone(),
        _ => Vec::new(),
    }
}

/// Every built control, in the order the description lists them: 0 is the dialog.
const FORM: &str = r#"
    var w = new Window("dialog", "Form");
    var g = w.add("group");
    var label = g.add("statictext", undefined, "Name:");
    var name = g.add("edittext", undefined, "first");
    var p = w.add("panel", undefined, "Options");
    var check = p.add("checkbox", undefined, "Bold");
    var r1 = p.add("radiobutton", undefined, "A");
    var r2 = p.add("radiobutton", undefined, "B");
    var drop = p.add("dropdownlist", undefined, ["x", "y", "z"]);
    var box = p.add("listbox", undefined, ["one", "two"]);
    var slide = p.add("slider", undefined, 5, 0, 10);
    var bar = p.add("progressbar", undefined, 3, 10);
    var ok = w.add("button", undefined, "OK");
    var cancel = w.add("button", undefined, "Cancel", {name: "cancel"});
"#;

#[test]
fn a_dialog_with_each_control_gives_back_the_values_the_person_set() {
    let events = vec![
        change(3, Data::String("second".into())),
        click(5),
        click(6),
        click(7),
        change(8, Data::Number(2.0)),
        change(9, Data::Number(1.0)),
        change(10, Data::Number(7.5)),
        click(12),
    ];
    let (mut r, seen, _) = realm(events);
    let source = format!(
        "{FORM} var result = w.show(); [result, name.text, check.value, r1.value, r2.value, drop.selection.text, box.selection.index, slide.value, bar.value].join('|')"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("1|second|true|false|true|z|1|7.5|3".into()));
    let seen = seen.borrow();
    let first = nodes(&seen.descriptions[0]);
    let kinds: Vec<String> = first
        .iter()
        .map(|n| match field(n, "kind") {
            Data::String(s) => s.clone(),
            _ => String::new(),
        })
        .collect();
    let want = [
        "dialog",
        "group",
        "statictext",
        "edittext",
        "panel",
        "checkbox",
        "radiobutton",
        "radiobutton",
        "dropdownlist",
        "listbox",
        "slider",
        "progressbar",
        "button",
        "button",
    ];
    assert_eq!(kinds, want);
    assert_eq!(field(&first[3], "parent"), &Data::Number(1.0), "the field is in the group");
    assert_eq!(field(&first[8], "items"), &Data::Array(vec![Data::String("x".into()), Data::String("y".into()), Data::String("z".into())]));
    assert_eq!(field(&first[10], "maxvalue"), &Data::Number(10.0));
    assert_eq!(field(&seen.descriptions[0], "defaultElement"), &Data::Number(12.0), "OK is the default element by its text");
    assert_eq!(field(&seen.descriptions[0], "cancelElement"), &Data::Number(13.0), "Cancel is the cancel element by its name");
    assert_eq!(seen.ends, vec![(1.0, true)]);
}

#[test]
fn the_default_and_cancel_buttons_close_with_1_and_2_and_a_close_gives_0() {
    for (event, want) in [(click(12), 1.0), (click(13), 2.0), (close(), 0.0)] {
        let (mut r, _, _) = realm(vec![event]);
        assert_eq!(r.run(&format!("{FORM} w.show()")).unwrap(), JsValue::Number(want));
    }
}

#[test]
fn a_button_with_a_handler_does_not_close_the_dialog() {
    let (mut r, _, _) = realm(vec![click(12), click(13)]);
    let source = format!("{FORM} var clicks = 0; ok.onClick = function () {{ clicks++; }}; [w.show(), clicks].join()");
    assert_eq!(r.run(&source).unwrap(), JsValue::String("2,1".into()));
}

#[test]
fn close_gives_its_result_and_on_close_can_keep_the_dialog_open() {
    let (mut r, seen, _) = realm(vec![click(5), click(5)]);
    let source = format!(
        "{FORM} var asked = 0; check.onClick = function () {{ w.close(7); }}; w.onClose = function () {{ asked++; return asked > 1; }}; [w.show(), asked].join()"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("7,2".into()));
    assert_eq!(seen.borrow().ends, vec![(7.0, true)]);
}

#[test]
fn close_in_on_show_ends_show_without_waiting() {
    let (mut r, seen, _) = realm(Vec::new());
    assert_eq!(r.run(&format!("{FORM} w.onShow = function () {{ w.close(4); }}; w.show()")).unwrap(), JsValue::Number(4.0));
    assert_eq!(seen.borrow().descriptions.len(), 1, "only open saw the dialog");
}

#[test]
fn handlers_change_other_controls_and_the_next_description_shows_it() {
    let (mut r, seen, _) = realm(vec![click(5), click(12)]);
    let source =
        format!("{FORM} check.onClick = function () {{ name.text = check.value ? 'on' : 'off'; slide.enabled = false; }}; w.show(); name.text");
    assert_eq!(r.run(&source).unwrap(), JsValue::String("on".into()));
    let seen = seen.borrow();
    let after = nodes(&seen.descriptions[2]);
    assert_eq!(field(&after[3], "text"), &Data::String("on".into()));
    assert_eq!(field(&after[10], "enabled"), &Data::Bool(false));
}

#[test]
fn on_change_runs_after_on_changing() {
    let (mut r, _, _) = realm(vec![change(3, Data::String("x".into())), click(12)]);
    let source = format!(
        "{FORM} var log = []; name.onChanging = function () {{ log.push('changing ' + name.text); }}; name.onChange = function () {{ log.push('change'); }}; w.show(); log.join()"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("changing x,change".into()));
}

#[test]
fn a_handler_that_loops_ends_the_run_at_its_fuel() {
    let (mut r, seen, _) = realm(vec![click(5)]);
    r.set_limits(Limits { fuel: 200_000, ..Limits::default() });
    let e = r.run(&format!("{FORM} check.onClick = function () {{ for (;;) {{}} }}; w.show()")).unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
    assert_eq!(seen.borrow().ends, vec![(0.0, false)], "show ended without its result");
}

#[test]
fn a_dialog_cannot_be_shown_inside_its_own_or_another_show() {
    let (mut r, _, _) = realm(vec![click(5), click(12)]);
    let source = format!(
        "{FORM} var errors = []; check.onClick = function () {{ try {{ w.show(); }} catch (e) {{ errors.push(e.message); }} try {{ new Window('dialog').show(); }} catch (e) {{ errors.push(e.message); }} }}; w.show(); errors.join('|')"
    );
    assert_eq!(
        r.run(&source).unwrap(),
        JsValue::String("show: this dialog is already showing|show: a dialog cannot open while another dialog is showing".into())
    );
}

#[test]
fn a_dialog_can_be_shown_again_after_it_closed() {
    let (mut r, _, _) = realm(vec![click(12), click(13)]);
    assert_eq!(r.run(&format!("{FORM} [w.show(), w.show()].join()")).unwrap(), JsValue::String("1,2".into()));
}

#[test]
fn types_that_are_not_available_yet_throw_errors_that_say_so() {
    for (source, want) in [
        ("new Window('window')", "Window: a window of type window is not available yet; use dialog or palette"),
        ("new Window('palette')", "Window: this host shows no palettes"),
        ("new Window('dialog { b: Button {} }')", "Window: resource specifications are not available yet; build the dialog with add()"),
        ("new Window('dialog').add('tabbedpanel')", "add: the control type tabbedpanel is not available yet"),
        ("new Window('dialog').add('wobble')", "add: there is no control type wobble"),
        ("new Window('dialog').add('button').add('button')", "add: a button holds no controls"),
        (
            "new Window('dialog').add('listbox', undefined, [], {multiselect: true})",
            "add: a list box that selects several items is not available yet",
        ),
    ] {
        let (mut r, _, _) = realm(Vec::new());
        let e = r.run(&format!("try {{ {source}; 'made' }} catch (e) {{ e.message }}")).unwrap();
        assert_eq!(e, JsValue::String(want.into()), "{source}");
    }
}

#[test]
fn too_many_controls_too_deep_a_nesting_and_too_long_a_text_are_refused() {
    let (mut r, _, _) = realm(Vec::new());
    let many =
        r.run("var w = new Window('dialog'); try { for (var i = 0; i < 100000; i++) w.add('button'); } catch (e) { [e.name, i].join() }").unwrap();
    assert_eq!(many, JsValue::String(format!("RangeError,{}", scriptui::MAX_CONTROLS)));
    let deep =
        r.run("var c = new Window('dialog'); try { for (var d = 0; d < 1000; d++) c = c.add('group'); } catch (e) { [e.name, d].join() }").unwrap();
    assert_eq!(deep, JsValue::String(format!("RangeError,{}", scriptui::MAX_DEPTH)));
    let items = r.run("var l = new Window('dialog').add('dropdownlist'); try { for (var n = 0; n < 5000; n++) l.add('item', n); } catch (e) { [e.name, n].join() }").unwrap();
    assert_eq!(items, JsValue::String(format!("RangeError,{}", scriptui::MAX_ITEMS)));
    let long = r
        .run("var w = new Window('dialog'); w.add('statictext', undefined, new Array(20000).join('x')); try { w.show(); } catch (e) { e.message }")
        .unwrap();
    assert_eq!(long, JsValue::String("show: a control's text is longer than 16 KiB".into()));
}

#[test]
fn an_add_that_a_getter_or_a_conversion_reenters_keeps_to_the_limits() {
    let (mut r, _, _) = realm(Vec::new());
    let controls = r
        .run(
            "var w = new Window('dialog'); for (var i = 0; i < 511; i++) w.add('button'); \
             var props = {}; Object.defineProperty(props, 'name', {get: function () { w.add('button'); return 'n'; }}); \
             var e = ''; try { w.add('button', undefined, 'x', props); } catch (x) { e = x.name; } [e, w.children.length].join()",
        )
        .unwrap();
    assert_eq!(controls, JsValue::String(format!("RangeError,{}", scriptui::MAX_CONTROLS)), "the getter took the last place");
    let items = r
        .run(
            "var w = new Window('dialog'); var l = w.add('listbox'); for (var n = 0; n < 1023; n++) l.add('item', 'i'); \
             var t = {toString: function () { l.add('item', 'inner'); return 'outer'; }}; \
             var e = ''; try { l.add('item', t); } catch (x) { e = x.name; } [e, l.items.length].join()",
        )
        .unwrap();
    assert_eq!(items, JsValue::String(format!("RangeError,{}", scriptui::MAX_ITEMS)), "the conversion took the last place");
}

#[test]
fn a_new_list_s_items_count_only_once_it_is_added_and_too_many_are_refused_unread() {
    let (mut r, _, _) = realm(Vec::new());
    let failed = r
        .run(
            "var w = new Window('dialog'); var l = w.add('listbox'); for (var n = 0; n < 1022; n++) l.add('item', 'i'); \
             var bad = {toString: function () { throw new Error('no text'); }}; \
             var e = ''; try { w.add('dropdownlist', undefined, ['a', bad]); } catch (x) { e = x.message; } \
             l.add('item', 'y'); l.add('item', 'z'); [e, l.items.length, w.children.length].join()",
        )
        .unwrap();
    assert_eq!(failed, JsValue::String(format!("no text,{},1", scriptui::MAX_ITEMS)), "the list that was not added took no places");
    let long = r
        .run(
            "var w = new Window('dialog'); var read = false; var a = new Array(5000); a[0] = {toString: function () { read = true; return 'x'; }}; \
             var e = ''; try { w.add('listbox', undefined, a); } catch (x) { e = x.name; } [e, read, w.children.length].join()",
        )
        .unwrap();
    assert_eq!(long, JsValue::String("RangeError,false,0".into()), "an array longer than the limit is refused before its items are read");
}

#[test]
fn a_collection_while_a_bounds_or_margins_value_is_read_keeps_that_value() {
    let (mut r, seen, _) = realm(vec![close()]);
    let source = "function heavy(n) { return {valueOf: function () { var junk = []; for (var k = 0; k < 50000; k++) junk.push({}); return n; }}; } \
        var w = new Window('dialog'); var b = w.add('button', undefined, 'Go'); \
        Object.defineProperty(b, 'bounds', {get: function () { return {left: heavy(1), top: 2, right: 30, bottom: 40}; }}); \
        Object.defineProperty(w, 'margins', {get: function () { return [heavy(3), 4, 5, 6]; }}); \
        w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(0.0));
    let seen = seen.borrow();
    let first = nodes(&seen.descriptions[0]);
    let numbers = |v: &[f64]| Data::Array(v.iter().map(|n| Data::Number(*n)).collect());
    assert_eq!(field(&first[1], "bounds"), &numbers(&[1.0, 2.0, 30.0, 40.0]));
    assert_eq!(field(&first[0], "margins"), &numbers(&[3.0, 4.0, 5.0, 6.0]));
}

#[test]
fn a_list_a_later_getter_changes_is_described_as_the_person_picks_from_it() {
    let (mut r, seen, _) = realm(vec![change(1, Data::Number(0.0)), click(2)]);
    let source = "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['A']); var b = w.add('button', undefined, 'OK'); \
        var swapped = false; Object.defineProperty(b, 'text', {get: function () { if (!swapped) { swapped = true; l.removeAll(); l.add('item', 'B'); } return 'OK'; }}); \
        var picked = ''; l.onChange = function () { picked = l.selection.text; }; w.show(); picked";
    assert_eq!(r.run(source).unwrap(), JsValue::String("B".into()));
    let seen = seen.borrow();
    assert_eq!(
        field(&nodes(&seen.descriptions[0])[1], "items"),
        &Data::Array(vec![Data::String("B".into())]),
        "the description shows the list as it is"
    );
    let (mut r, _, _) = realm(Vec::new());
    let source = "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['A']); var b = w.add('button', undefined, 'OK'); \
        Object.defineProperty(b, 'text', {get: function () { l.removeAll(); l.add('item', 'B'); return 'OK'; }}); \
        try { w.show(); } catch (e) { e.message }";
    assert_eq!(r.run(source).unwrap(), JsValue::String("show: a list's items changed while the dialog was read".into()));
}

#[test]
fn a_bounds_value_that_is_not_finite_is_refused_when_shown() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r.run("var w = new Window('dialog'); w.add('button', [0, 0, Infinity, 20]); try { w.show(); } catch (e) { e.name }").unwrap();
    assert_eq!(v, JsValue::String("RangeError".into()));
}

#[test]
fn a_script_cannot_change_a_dialog_s_controls_behind_its_back() {
    let (mut r, _, _) = realm(vec![click(1)]);
    let source = "var w = new Window('dialog'); var b = w.add('button', undefined, 'OK'); \
        w.children.push(w); w.__vcKids = []; try { w.__vcKids.push(w); } catch (e) {} b.__vcKind = 'dialog'; w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(1.0));
}

#[test]
fn a_show_that_waits_resumes_with_the_host_s_answer() {
    let (mut r, seen, queue) = realm(Vec::new());
    r.start(&format!("{FORM} check.onClick = function () {{ name.text = 'clicked'; }}; var n = w.show(); n + ':' + name.text")).unwrap();
    assert_eq!(r.advance(1_000_000).unwrap(), Progress::Paused);
    assert!(r.waits(), "show waits at open's first wait");
    r.answer(click(5)).unwrap();
    assert_eq!(r.advance(1_000_000).unwrap(), Progress::Paused);
    assert!(r.waits(), "show waits again after the handler");
    queue.borrow_mut().push_back(click(12));
    r.answer(click(5)).unwrap();
    assert_eq!(r.advance(1_000_000).unwrap(), Progress::Done(JsValue::String("1:clicked".into())));
    assert_eq!(seen.borrow().ends, vec![(1.0, true)]);
}

/// What a palette host was asked: each call's name and its palette id (0 for none).
type Calls = Rc<RefCell<Vec<(String, f64)>>>;

/// A realm with palette host functions: `paletteShow` gives ids from 1, and `paletteEvent` gives
/// the events in `events`, in order, then `null`.
fn palette_realm(events: Vec<Data>) -> (Realm, Calls) {
    palette_realm_with(events, Vec::new(), Vec::new())
}

/// [`palette_realm`], whose `paletteTake` also gives the events in `taken`, in order, then `null`,
/// whose `palettePending` says whether any of them is left, and whose `paletteDraw` gives the
/// answers in `draws`, in order, then `false`.
fn palette_realm_with(events: Vec<Data>, taken: Vec<Data>, draws: Vec<bool>) -> (Realm, Calls) {
    let mut r = Realm::new();
    r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
    let calls: Calls = Rc::default();
    let events = Rc::new(RefCell::new(VecDeque::from(events)));
    let id_of = |args: &[Data], at: usize| match args.get(at) {
        Some(Data::Number(n)) => *n,
        _ => 0.0,
    };
    let record = |name: &'static str, at: usize| -> HostFunction {
        let c = calls.clone();
        Rc::new(move |args| {
            c.borrow_mut().push((name.to_string(), id_of(&args, at)));
            Ok(Data::Undefined)
        })
    };
    let c = calls.clone();
    let show: HostFunction = Rc::new(move |args| {
        let given = id_of(&args, 1);
        let id = if given > 0.0 { given } else { 1.0 + c.borrow().iter().filter(|(n, _)| n == "paletteShow").count() as f64 };
        c.borrow_mut().push(("paletteShow".into(), id));
        Ok(Data::Number(id))
    });
    let event: HostFunction = Rc::new(move |_| Ok(events.borrow_mut().pop_front().unwrap_or(Data::Null)));
    let taken = Rc::new(RefCell::new(VecDeque::from(taken)));
    let c = calls.clone();
    let left = taken.clone();
    let take: HostFunction = Rc::new(move |_| {
        c.borrow_mut().push(("paletteTake".into(), 0.0));
        Ok(left.borrow_mut().pop_front().unwrap_or(Data::Null))
    });
    let c = calls.clone();
    let pending: HostFunction = Rc::new(move |_| {
        c.borrow_mut().push(("palettePending".into(), 0.0));
        Ok(Data::Bool(!taken.borrow().is_empty()))
    });
    let draws = RefCell::new(VecDeque::from(draws));
    let c = calls.clone();
    let draw: HostFunction = Rc::new(move |_| {
        c.borrow_mut().push(("paletteDraw".into(), 0.0));
        Ok(Data::Bool(draws.borrow_mut().pop_front().unwrap_or(false)))
    });
    let none: HostFunction = Rc::new(|_| Err(HostError::Error("no dialog here".into())));
    let items = vec![
        ("open".into(), HostItem::Function(none.clone())),
        ("wait".into(), HostItem::Function(none.clone())),
        ("end".into(), HostItem::Function(none)),
        ("palette".into(), HostItem::Function(record("palette", 0))),
        ("paletteShow".into(), HostItem::Function(show)),
        ("paletteUpdate".into(), HostItem::Function(record("paletteUpdate", 1))),
        ("paletteHide".into(), HostItem::Function(record("paletteHide", 0))),
        ("paletteClose".into(), HostItem::Function(record("paletteClose", 0))),
        ("paletteEvent".into(), HostItem::Function(event)),
        ("paletteTake".into(), HostItem::Function(take)),
        ("palettePending".into(), HostItem::Function(pending)),
        ("paletteDraw".into(), HostItem::Function(draw)),
    ];
    r.define_script_ui(items).unwrap();
    (r, calls)
}

fn palette_event(palette: f64, event: Data) -> Data {
    let Data::Object(mut fields) = event else { return Data::Null };
    fields.push(("palette".into(), Data::Number(palette)));
    Data::Object(fields)
}

fn names(calls: &Calls) -> Vec<String> {
    calls.borrow().iter().map(|(n, _)| n.clone()).collect()
}

#[test]
fn a_palette_s_show_returns_at_once_and_its_events_run_its_handlers() {
    let (mut r, calls) = palette_realm(vec![palette_event(1.0, click(2)), palette_event(1.0, change(3, Data::String("typed".into())))]);
    let made = r
        .run(
            "var p = new Window('palette', 'Tools'); var label = p.add('statictext', undefined, 'idle'); \
             var b = p.add('button', undefined, 'Go'); var t = p.add('edittext', undefined, ''); var log = []; \
             b.onClick = function () { log.push('click'); label.text = 'clicked'; }; \
             t.onChange = function () { log.push('change ' + t.text); }; String(p.show())",
        )
        .unwrap();
    assert_eq!(made, JsValue::String("undefined".into()), "show() gives undefined without waiting");
    assert_eq!(names(&calls), ["palette", "paletteShow", "paletteUpdate"]);
    r.run("Window.__vcPaletteEvent(); Window.__vcPaletteEvent();").unwrap();
    assert_eq!(r.run("log.join('|') + '|' + label.text").unwrap(), JsValue::String("click|change typed|clicked".into()));
    let updates = |calls: &Calls| calls.borrow().iter().filter(|(n, id)| n == "paletteUpdate" && *id == 1.0).count();
    assert_eq!(updates(&calls), 1, "the handlers give the host nothing");
    r.refresh_script_ui().unwrap();
    assert_eq!(updates(&calls), 2, "the refresh after the run gives the host the new controls");
    assert_eq!(r.run("Window.__vcPaletteEvent(); 'nothing'").unwrap(), JsValue::String("nothing".into()), "no event left: nothing runs");
}

#[test]
fn closing_a_palette_runs_on_close_which_can_keep_it_open() {
    let (mut r, calls) = palette_realm(vec![palette_event(1.0, close()), palette_event(1.0, close()), palette_event(1.0, click(1))]);
    r.run(
        "var p = new Window('palette'); var b = p.add('button', undefined, 'Go'); var clicks = 0; b.onClick = function () { clicks++; }; \
         var asked = 0; p.onClose = function () { asked++; p.close(); return asked > 1; }; p.show();",
    )
    .unwrap();
    r.run("Window.__vcPaletteEvent();").unwrap();
    assert!(!names(&calls).contains(&"paletteClose".to_string()), "the first onClose kept it open");
    r.run("Window.__vcPaletteEvent(); Window.__vcPaletteEvent();").unwrap();
    assert_eq!(r.run("[asked, clicks].join()").unwrap(), JsValue::String("2,0".into()), "a close inside onClose does not run it again");
    assert_eq!(calls.borrow().iter().filter(|(n, _)| n == "paletteClose").count(), 1);
    assert!(calls.borrow().contains(&("paletteClose".into(), 1.0)));
}

fn count(calls: &Calls, name: &str) -> usize {
    calls.borrow().iter().filter(|(n, _)| n == name).count()
}

#[test]
fn update_in_the_script_s_own_code_gives_the_controls_only_when_the_host_draws_the_palette() {
    let (mut r, calls) = palette_realm_with(Vec::new(), Vec::new(), vec![true, false, false, true]);
    r.start(
        "var p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 4); p.show(); \
         for (var k = 1; k <= 4; k++) { bar.value = k; p.update(); }",
    )
    .unwrap();
    while r.advance(1_000_000).unwrap() == Progress::Paused {}
    assert_eq!(count(&calls, "paletteDraw"), 4, "each update() asks whether the palette is drawn");
    assert_eq!(count(&calls, "paletteUpdate"), 1 + 2, "show(), then only the update() calls the host draws");
}

#[test]
fn update_in_a_handler_neither_asks_to_draw_the_palette_nor_gives_its_controls() {
    let (mut r, calls) = palette_realm_with(vec![palette_event(1.0, click(2))], Vec::new(), vec![true; 3]);
    r.run(
        "var p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 3); var b = p.add('button', undefined, 'Go'); \
         b.onClick = function () { for (var k = 1; k <= 3; k++) { bar.value = k; p.update(); } }; p.show();",
    )
    .unwrap();
    let before = count(&calls, "paletteUpdate");
    r.start("Window.__vcPaletteEvent();").unwrap();
    while r.advance(1_000_000).unwrap() == Progress::Paused {}
    assert_eq!(count(&calls, "paletteDraw"), 0, "a handler cannot pause, so its update() is not drawn");
    assert_eq!(count(&calls, "paletteUpdate"), before, "the handler's update() calls give the host no controls");
    r.refresh_script_ui().unwrap();
    assert_eq!(count(&calls, "paletteUpdate") - before, 1, "the controls go to the host once the run ends");
}

#[test]
fn update_hands_a_waiting_click_to_the_palette_so_a_cancel_button_stops_a_loop() {
    let (mut r, calls) = palette_realm_with(Vec::new(), vec![palette_event(1.0, click(2))], Vec::new());
    let done = r
        .run(
            "var p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 100); \
             var cancel = p.add('button', undefined, 'Cancel'); var stopped = false; \
             cancel.onClick = function () { stopped = true; }; p.show(); \
             var k = 0; while (!stopped && k < 100) { k++; bar.value = k; p.update(); } k + ':' + stopped",
        )
        .unwrap();
    assert_eq!(done, JsValue::String("1:true".into()), "the first update() runs the click's onClick");
    assert_eq!(count(&calls, "paletteTake"), 1, "update() takes events while one waits");
    assert!(count(&calls, "palettePending") >= 2, "update() asks again after each event");
    let order = names(&calls);
    let take = order.iter().position(|n| n == "paletteTake");
    let last_update = order.iter().rposition(|n| n == "paletteUpdate");
    assert!(take.zip(last_update).is_some_and(|(t, u)| u < t), "the host gets the controls before the event is taken");
}

#[test]
fn an_update_a_getter_calls_while_a_click_waits_leaves_the_click_to_the_update_taking_it() {
    let (mut r, calls) = palette_realm_with(Vec::new(), vec![palette_event(1.0, click(1))], Vec::new());
    let done = r
        .run(
            "var p = new Window('palette'); var b = p.add('button', undefined, 'Go'); var clicks = 0; var reads = 0; var armed = false; \
             b.onClick = function () { clicks++; }; \
             Object.defineProperty(p, 'text', {get: function () { if (armed) { reads++; p.update(); } return 'P'; }}); \
             p.show(); armed = true; p.update(); clicks + ':' + (reads > 0)",
        )
        .unwrap();
    assert_eq!(done, JsValue::String("1:true".into()), "the getter's update() runs, and the click's onClick runs once");
    assert_eq!(count(&calls, "paletteTake"), 1);
}

#[test]
fn update_throws_for_a_window_that_is_not_showing() {
    let (mut r, _, _) = realm(vec![click(12)]);
    let source = format!(
        "{FORM} var before = 'none'; try {{ w.update(); }} catch (e) {{ before = e.message; }} var during = 'none'; \
         ok.onClick = function () {{ w.update(); during = 'ran'; w.close(1); }}; [w.show(), before, during].join('|')"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("1|update: the window is not showing|ran".into()));
    let (mut r, _) = palette_realm(Vec::new());
    let hidden = r
        .run(
            "var p = new Window('palette'); var log = []; try { p.update(); } catch (e) { log.push('before show'); } \
             p.show(); p.update(); p.hide(); try { p.update(); } catch (e) { log.push('hidden'); } log.join()",
        )
        .unwrap();
    assert_eq!(hidden, JsValue::String("before show,hidden".into()));
}

#[test]
fn update_on_a_showing_dialog_its_group_or_its_panel_runs_in_a_handler() {
    let (mut r, _, _) = realm(vec![click(12)]);
    let source = format!(
        "{FORM} var done = 'none'; ok.onClick = function () {{ bar.value = 7; w.update(); g.update(); p.update(); done = 'ran'; w.close(1); }}; \
         [w.show(), done].join('|')"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("1|ran".into()));
}

#[test]
fn update_follows_whether_the_host_shows_a_palette_not_its_visible_property() {
    let (mut r, calls) = palette_realm_with(Vec::new(), Vec::new(), vec![true; 4]);
    r.run("var p = new Window('palette'); var g = p.add('group'); var b = g.add('button', undefined, 'Go'); p.show();").unwrap();
    let before = count(&calls, "paletteUpdate");
    r.start(
        "var log = []; p.visible = false; p.update(); g.update(); log.push('shown'); \
         p.hide(); p.visible = true; try { p.update(); } catch (e) { log.push('hidden'); } \
         try { b.update(); } catch (e) { log.push(e.name); } log.join()",
    )
    .unwrap();
    let log = loop {
        if let Progress::Done(v) = r.advance(1_000_000).unwrap() {
            break v;
        }
    };
    assert_eq!(log, JsValue::String("shown,hidden,TypeError".into()));
    assert_eq!(count(&calls, "paletteUpdate") - before, 2, "the palette and its group each give the host the controls while it shows");
}

#[test]
fn a_palette_list_change_after_its_items_changed_throws() {
    let seen = |index: f64| {
        let Data::Object(mut fields) = palette_event(1.0, change(1, Data::Number(index))) else { return Data::Null };
        fields.push(("items".into(), Data::Array(vec![Data::String("a".into()), Data::String("b".into())])));
        // The two items the list was made with.
        fields.push(("revision".into(), Data::Number(2.0)));
        Data::Object(fields)
    };
    let (mut r, _) = palette_realm(vec![seen(1.0), seen(1.0), seen(1.0), seen(1.0)]);
    r.run("var p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); var got = []; l.onChange = function () { got.push(String(l.selection)); }; p.show();")
        .unwrap();
    r.run("Window.__vcPaletteEvent(); l.removeAll(); l.add('item', 'c'); l.add('item', 'd');").unwrap();
    let message = "a palette's list changed before the person's choice was taken";
    let e = r.run("try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String(message.into()), "the same number of items, other texts");
    r.run("l.removeAll(); l.add('item', 'a');").unwrap();
    let e = r.run("try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String(message.into()), "fewer items");
    r.run("l.removeAll(); l.add('item', 'a'); l.add('item', 'b');").unwrap();
    let e = r.run("try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String(message.into()), "new items with the same texts");
    assert_eq!(r.run("got.join()").unwrap(), JsValue::String("b".into()));
}

/// A palette list change as the host gives it: the item index, the items the person saw (`a` and
/// `b`) and the revision of a list made with those two items.
fn seen_list_change(index: f64) -> Data {
    let Data::Object(mut fields) = palette_event(1.0, change(1, Data::Number(index))) else { return Data::Null };
    fields.push(("items".into(), Data::Array(vec![Data::String("a".into()), Data::String("b".into())])));
    fields.push(("revision".into(), Data::Number(2.0)));
    Data::Object(fields)
}

#[test]
fn a_palette_list_change_after_an_item_s_text_changed_throws() {
    let (mut r, _) = palette_realm(vec![seen_list_change(1.0)]);
    r.run("var p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); var got = []; l.onChange = function () { got.push(String(l.selection)); }; p.show();")
        .unwrap();
    r.run("l.items[1].text = 'z';").unwrap();
    let e = r.run("try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String("a palette's list changed before the person's choice was taken".into()), "the same items, one with another text");
    assert_eq!(r.run("got.join()").unwrap(), JsValue::String("".into()));
}

#[test]
fn a_palette_list_change_throws_when_an_item_s_text_getter_replaces_the_items() {
    let (mut r, _) = palette_realm(vec![seen_list_change(1.0)]);
    // The event reads the first item's text twice: for the palette's description, then to compare
    // it with the items the person saw. The second read replaces the items with ones of the same
    // texts.
    r.run(
        "var p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); var got = []; \
         l.onChange = function () { got.push(String(l.selection)); }; var reads = -1; \
         Object.defineProperty(l.items[0], 'text', {get: function () { if (reads >= 0 && ++reads == 2) { l.removeAll(); l.add('item', 'a'); l.add('item', 'b'); } return 'a'; }}); \
         p.show();",
    )
    .unwrap();
    let e = r.run("reads = 0; try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String("a palette's list changed before the person's choice was taken".into()));
    assert_eq!(r.run("[reads, got.join()].join()").unwrap(), JsValue::String("2,".into()));
}

#[test]
fn a_palette_event_finds_its_control_by_its_place() {
    let placed = |id: usize, place: &[f64]| {
        let Data::Object(mut fields) = palette_event(1.0, click(id)) else { return Data::Null };
        fields.push(("place".into(), Data::Array(place.iter().map(|&k| Data::Number(k)).collect())));
        Data::Object(fields)
    };
    // The host's index 1 is the group, where a control a getter added would have moved the button;
    // the place is the button's, the second control of the palette.
    let (mut r, _) = palette_realm(vec![placed(1, &[1.0]), placed(2, &[2.0])]);
    r.run("var p = new Window('palette'); var g = p.add('group'); var b = p.add('button', undefined, 'Go'); var clicks = 0; b.onClick = function () { clicks++; }; p.show();")
        .unwrap();
    r.run("Window.__vcPaletteEvent();").unwrap();
    assert_eq!(r.run("clicks").unwrap(), JsValue::Number(1.0), "the click reaches the control at its place");
    let e = r.run("try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String("a palette event names a control the palette no longer holds".into()), "no control at the place");
    assert_eq!(r.run("clicks").unwrap(), JsValue::Number(1.0));
}

#[test]
fn a_palette_event_does_not_run_a_handler_once_a_getter_closed_the_palette() {
    let (mut r, calls) = palette_realm(vec![palette_event(1.0, click(2))]);
    r.run(
        "var p = new Window('palette'); var label = p.add('statictext', undefined, 'idle'); var b = p.add('button', undefined, 'Go'); \
         var clicks = 0; b.onClick = function () { clicks++; }; var armed = false; \
         Object.defineProperty(label, 'text', {get: function () { if (armed) { armed = false; p.close(); } return 'idle'; }}); p.show();",
    )
    .unwrap();
    // The event reads the label's text for the palette's description, which closes the palette.
    r.run("armed = true; Window.__vcPaletteEvent();").unwrap();
    assert_eq!(r.run("clicks").unwrap(), JsValue::Number(0.0), "the click is not given to a closed palette");
    assert!(calls.borrow().contains(&("paletteClose".into(), 1.0)));
}

#[test]
fn a_palette_list_change_does_not_run_on_change_once_an_item_s_text_getter_closed_the_palette() {
    let (mut r, _) = palette_realm(vec![seen_list_change(1.0)]);
    // The event reads the first item's text twice: for the palette's description, then to compare
    // it with the items the person saw. The second read closes the palette.
    r.run(
        "var p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); var got = []; \
         l.onChange = function () { got.push(String(l.selection)); }; var reads = -1; \
         Object.defineProperty(l.items[0], 'text', {get: function () { if (reads >= 0 && ++reads == 2) { p.close(); } return 'a'; }}); \
         p.show();",
    )
    .unwrap();
    let e = r.run("reads = 0; try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String("none".into()));
    assert_eq!(r.run("[reads, got.join(), String(l.selection)].join()").unwrap(), JsValue::String("2,,null".into()));
}

#[test]
fn a_palette_event_does_not_run_a_handler_whose_getter_closed_the_palette() {
    let (mut r, calls) = palette_realm(vec![palette_event(1.0, click(1))]);
    r.run(
        "var p = new Window('palette'); var b = p.add('button', undefined, 'Go'); var clicks = 0; var reads = 0; \
         Object.defineProperty(b, 'onClick', {get: function () { reads++; p.close(); return function () { clicks++; }; }}); p.show();",
    )
    .unwrap();
    r.run("Window.__vcPaletteEvent();").unwrap();
    assert_eq!(r.run("[reads, clicks].join()").unwrap(), JsValue::String("1,0".into()), "the handler read from a closed palette does not run");
    assert!(calls.borrow().contains(&("paletteClose".into(), 1.0)));
}

#[test]
fn a_palette_list_change_is_dropped_once_an_item_s_text_getter_closed_the_palette_and_on_close_changed_the_list() {
    let (mut r, _) = palette_realm(vec![seen_list_change(1.0)]);
    // The second read of the first item's text closes the palette, and its `onClose` adds an item.
    r.run(
        "var p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); var got = []; \
         l.onChange = function () { got.push(String(l.selection)); }; var reads = -1; \
         p.onClose = function () { l.add('item', 'c'); }; \
         Object.defineProperty(l.items[0], 'text', {get: function () { if (reads >= 0 && ++reads == 2) { p.close(); } return 'a'; }}); \
         p.show();",
    )
    .unwrap();
    let e = r.run("reads = 0; try { Window.__vcPaletteEvent(); 'none'; } catch (e) { e.message; }").unwrap();
    assert_eq!(e, JsValue::String("none".into()), "the event for a closed palette is dropped without an error");
    assert_eq!(r.run("[reads >= 2, l.items.length, got.join(), String(l.selection)].join()").unwrap(), JsValue::String("true,3,,null".into()));
}

#[test]
fn a_palette_s_visible_follows_hide_and_show() {
    let (mut r, _) = palette_realm(Vec::new());
    let v = r
        .run("var p = new Window('palette'); p.add('button', undefined, 'Go'); p.show(); var a = p.visible; p.hide(); var b = p.visible; p.show(); [a, b, p.visible].join()")
        .unwrap();
    assert_eq!(v, JsValue::String("true,false,true".into()));
}

#[test]
fn a_close_inside_a_dialog_s_on_close_does_nothing() {
    let (mut r, seen, _) = realm(vec![close(), click(13)]);
    let source = format!(
        "{FORM} var asked = 0; w.onClose = function () {{ asked++; if (asked == 1) {{ w.close(5); return false; }} return true; }}; [w.show(), asked].join()"
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String("2,2".into()), "the dialog waited for the click on Cancel");
    assert_eq!(seen.borrow().ends, vec![(2.0, true)]);
}

#[test]
fn bounds_whose_ends_add_up_past_the_largest_number_stay_finite() {
    let (mut r, seen, _) = realm(vec![close()]);
    r.run("var w = new Window('dialog'); var b = w.add('button', undefined, 'Go'); b.bounds = {x: 1e308, y: 0, width: 1e308, height: 20}; w.show()")
        .unwrap();
    let described = nodes(&seen.borrow().descriptions[0]);
    let want = Data::Array(vec![Data::Number(1e308), Data::Number(0.0), Data::Number(f64::MAX), Data::Number(20.0)]);
    assert_eq!(field(&described[1], "bounds"), &want);
}

#[test]
fn hide_keeps_the_palette_and_show_shows_it_again_with_the_same_id() {
    let (mut r, calls) = palette_realm(Vec::new());
    r.run("var p = new Window('palette'); p.add('button', undefined, 'Go'); p.show(); p.hide(); p.show();").unwrap();
    let shows: Vec<f64> = calls.borrow().iter().filter(|(n, _)| n == "paletteShow" || n == "paletteHide").map(|(_, id)| *id).collect();
    assert_eq!(shows, [1.0, 1.0, 1.0]);
}

#[test]
fn findelement_items_and_selection_follow_the_list() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var g = w.add('group'); var d = g.add('dropdownlist', undefined, [], {name: 'pick'}); \
             d.add('item', 'a'); var b = d.add('item', 'b'); d.selection = 1; var s1 = d.selection.text; b.selected = false; \
             var s2 = d.selection; d.selection = 0; d.removeAll(); [w.findElement('pick') === d, s1, s2, d.selection, d.items.length, String(b)].join()",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("true,b,,,0,b".into()));
}
