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
        ("new Window('palette')", "Window: a window of type palette is not available yet; use dialog"),
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
