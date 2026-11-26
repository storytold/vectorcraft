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
        ("new Window('palette { b: Button {} }')", "Window: a window of type palette is not available yet; use dialog"),
        ("new Window('dialog').add('p: TabbedPanel {}')", "add: the control type tabbedpanel is not available yet"),
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

/// A dialog built from a resource specification: 0 is the dialog, 1 the panel, 2 its group, 3 the
/// static text, 4 the edit field, 5 the button group, 6 OK and 7 Cancel.
const RESOURCE_FORM: &str = r#"
    var w = new Window("dialog { text: 'Form', ignored: 1, \
        msg: Panel { orientation: 'column', alignChildren: ['right', 'top'], text: 'Messages', \
            title: Group { st: StaticText { text: 'Title:' }, et: EditText { text: 'Sample', characters: 35 } } \
        } \
        buttons: Group { \
            ok: Button { text: 'Build', properties: {name: 'ok'} }, \
            no: Button { text: 'Cancel', properties: {name: 'cancel'} } \
        } \
    }", "not read", [0, 0, 10, 10]);
"#;

#[test]
fn a_resource_specification_builds_the_controls_it_names() {
    let (mut r, seen, _) = realm(vec![change(4, Data::String("typed".into())), click(6)]);
    let rest = r#"
        var t = w.msg.title;
        var shape = [w.text, w.bounds === undefined, w.children.length, w.msg.type, w.msg.text, w.msg.orientation, w.msg.alignChildren.join(),
            t.parent === w.msg, t.st.type, t.st.text, t.et.type, t.et.characters, w.buttons.ok.name, w.buttons.no.name, w.ignored].join('|');
        var p = w.add("more: Panel { text: 'More', e: EditText { properties: {multiline: true}, text: 'x' }, l: DropDownList { properties: {items: ['a', 'b']} } }");
        var added = [p.parent === w, w.more === undefined, p.e.properties.multiline, p.l.items.length, p.l.items[1].text].join('|');
        p.parent.remove(p);
        [shape, added, w.show(), t.et.text].join('#')
    "#;
    let source = format!("{RESOURCE_FORM} {rest}");
    assert_eq!(
        r.run(&source).unwrap(),
        JsValue::String(
            "Form|true|2|panel|Messages|column|right,top|true|statictext|Title:|edittext|35|ok|cancel|1#true|true|true|2|b#1#typed".into()
        )
    );
    let seen = seen.borrow();
    assert_eq!(nodes(&seen.descriptions[0]).len(), 8, "the dialog lists the controls the specification names");
}

#[test]
fn a_resource_slider_and_progress_bar_take_their_creation_properties_and_values() {
    let (mut r, _, _) = realm(Vec::new());
    let source = "var w = new Window(\"dialog { \
        s: Slider { properties: {name: 'size'}, minvalue: 2, maxvalue: 20, value: 7 }, \
        p: ProgressBar { properties: {name: 'done', alignment: 'fill'}, maxvalue: 50, value: 5 } }\"); \
        [w.s.name, w.s.minvalue, w.s.maxvalue, w.s.value, w.findElement('size') === w.s, w.p.name, w.p.alignment, w.p.maxvalue, w.p.value].join('|')";
    assert_eq!(r.run(source).unwrap(), JsValue::String("size|2|20|7|true|done|fill|50|5".into()));
}

#[test]
fn a_resource_list_s_items_are_the_items_the_dialog_shows() {
    let (mut r, _, _) = realm(vec![change(1, Data::Number(2.0)), click(2)]);
    let source = "var w = new Window(\"dialog { l: DropDownList { items: ['a', 'b', 'c'], selection: 1 }, ok: Button { text: 'OK' } }\"); \
        var before = [w.l.items.length, w.l.selection.text].join(); [before, w.show(), w.l.selection.text].join('|')";
    assert_eq!(r.run(source).unwrap(), JsValue::String("3,b|1|c".into()));
    let (mut r, _, _) = realm(Vec::new());
    let both =
        r.run("try { new Window(\"dialog { l: ListBox { items: ['a'], properties: {items: ['b']} } }\"); 'made' } catch (e) { e.name }").unwrap();
    assert_eq!(both, JsValue::String("TypeError".into()), "items given both ways");
    let object = r
        .run("try { new Window(\"dialog { l: ListBox { items: {}, properties: {name: 'choices'} } }\"); 'made' } catch (e) { e.name + ': ' + e.message }")
        .unwrap();
    assert_eq!(object, JsValue::String("TypeError: items: give a list's items as an array".into()), "an object is not item texts");
    let none = r
        .run("var n = new Window(\"dialog { l: ListBox { items: null, properties: {name: 'choices'} } }\"); [n.l.items.length, n.l.name].join()")
        .unwrap();
    assert_eq!(none, JsValue::String("0,choices".into()));
}

#[test]
fn an_edit_field_s_and_a_static_text_s_characters_reach_the_description() {
    let (mut r, seen, _) = realm(vec![close()]);
    r.run("var w = new Window(\"dialog { e: EditText { text: 'Sample', characters: 35 }, s: StaticText { text: 'Hi' } }\"); w.show()").unwrap();
    let first = nodes(&seen.borrow().descriptions[0]);
    assert_eq!(field(&first[1], "characters"), &Data::Number(35.0));
    assert_eq!(field(&first[2], "characters"), &Data::Undefined, "a control without characters gives none");
}

#[test]
fn a_malformed_resource_specification_throws_a_syntax_error() {
    for source in [
        "new Window('dialog { b: Button { text: } }')",
        "new Window('dialog { b: Button {} ')",
        "new Window('dialog {} trailing')",
        "new Window('dialog').add('g: Group { v: orange }')",
        "new Window('dialog').add('g: Group { o: { b: Button {} } }')",
    ] {
        let (mut r, _, _) = realm(Vec::new());
        let e = r.run(&format!("try {{ {source}; 'made' }} catch (e) {{ e.name }}")).unwrap();
        assert_eq!(e, JsValue::String("SyntaxError".into()), "{source}");
    }
    let (mut r, _, _) = realm(Vec::new());
    let unknown = r.run("try { new Window('dialog { b: Wobble {} }'); 'made' } catch (e) { e.message }").unwrap();
    assert_eq!(unknown, JsValue::String("add: there is no control type wobble".into()));
}

#[test]
fn a_resource_specification_keeps_to_the_dialog_limits() {
    let (mut r, _, _) = realm(Vec::new());
    let many = r
        .run(&format!(
            "var s = 'dialog {{'; for (var i = 0; i <= {}; i++) s += ' b' + i + ': Button {{}}'; s += ' }}'; \
             try {{ new Window(s); 'made' }} catch (e) {{ e.name }}",
            scriptui::MAX_CONTROLS
        ))
        .unwrap();
    assert_eq!(many, JsValue::String("RangeError".into()));
    let fits = r
        .run(&format!(
            "var s = 'dialog {{'; for (var i = 0; i < {}; i++) s += ' b' + i + ': Button {{}}'; s += ' }}'; new Window(s).children.length",
            scriptui::MAX_CONTROLS
        ))
        .unwrap();
    assert_eq!(fits, JsValue::Number(scriptui::MAX_CONTROLS as f64));
    let added = r
        .run(&format!(
            "var w = new Window('dialog'); for (var i = 0; i < {}; i++) w.add('button'); \
             var e = ''; try {{ w.add('g: Group {{ a: Button {{}}, b: Button {{}} }}'); }} catch (x) {{ e = x.name; }} [e, w.children.length].join()",
            scriptui::MAX_CONTROLS - 2
        ))
        .unwrap();
    assert_eq!(added, JsValue::String(format!("RangeError,{}", scriptui::MAX_CONTROLS - 1)), "the group took a place before its second button");
    let kept =
        r.run("var g = w.children[w.children.length - 1]; [g.type, g.children.length, g.a === g.children[0], g.b === undefined].join()").unwrap();
    assert_eq!(kept, JsValue::String("group,1,true,true".into()), "the first button stays in the group");
    let nested = |depth: usize| {
        format!(
            "var s = ''; for (var d = 0; d < {depth}; d++) s += 'g: Group {{ '; for (d = 0; d < {depth}; d++) s += '}} '; \
             try {{ var c = new Window('dialog {{ ' + s + '}}'); while (c.g) c = c.g; 'made ' + c.type }} catch (e) {{ e.name }}"
        )
    };
    assert_eq!(r.run(&nested(scriptui::MAX_DEPTH)).unwrap(), JsValue::String("made group".into()));
    assert_eq!(r.run(&nested(scriptui::MAX_DEPTH + 1)).unwrap(), JsValue::String("RangeError".into()));
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
    assert_eq!(r.run(source).unwrap(), JsValue::String("show: a list's items or a container's controls changed while the dialog was read".into()));
}

#[test]
fn a_control_a_later_getter_removes_is_not_in_the_description() {
    let (mut r, seen, _) = realm(vec![click(1)]);
    let source = "var w = new Window('dialog'); var ok = w.add('button', undefined, 'OK'); var b = w.add('button', undefined, 'Go'); \
        var gone = false; Object.defineProperty(b, 'text', {get: function () { if (!gone) { gone = true; w.remove(ok); } return 'Go'; }}); \
        var went = 0; b.onClick = function () { went++; w.close(5); }; ok.onClick = function () { went = -1; }; [w.show(), went].join()";
    assert_eq!(r.run(source).unwrap(), JsValue::String("5,1".into()), "the click at index 1 reaches Go, which the dialog still holds");
    let seen = seen.borrow();
    let first = nodes(&seen.descriptions[0]);
    assert_eq!(first.len(), 2, "the removed OK button is not described");
    assert_eq!(field(&first[1], "text"), &Data::String("Go".into()));
    let (mut r, _, _) = realm(Vec::new());
    let source = "var w = new Window('dialog'); var b = w.add('button', undefined, 'Go'); \
        Object.defineProperty(b, 'text', {get: function () { w.add('statictext', undefined, 'more'); return 'Go'; }}); \
        try { w.show(); } catch (e) { e.message }";
    assert_eq!(r.run(source).unwrap(), JsValue::String("show: a list's items or a container's controls changed while the dialog was read".into()));
}

#[test]
fn a_list_s_initial_items_are_read_through_their_getters() {
    let (mut r, seen, _) = realm(vec![close()]);
    let source = "var a = ['x']; Object.defineProperty(a, 0, {get: function () { return 'A'; }}); \
        var w = new Window('dialog'); w.add('listbox', undefined, a); w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(0.0));
    let seen = seen.borrow();
    assert_eq!(field(&nodes(&seen.descriptions[0])[1], "items"), &Data::Array(vec![Data::String("A".into())]));
}

#[test]
fn an_object_made_to_look_like_a_container_charges_nothing_to_the_dialog() {
    let (mut r, _, _) = realm(Vec::new());
    let source = format!(
        "var w = new Window('dialog'); var fake = {{__vcKind: 'group', __vcWindow: w, __vcDepth: 0, __vcKids: []}}; \
         fake.add = w.add; var e = ''; try {{ fake.add('button'); }} catch (x) {{ e = x.name; }} \
         var l = w.add('listbox'); var fl = {{__vcKind: 'listbox', __vcWindow: w, __vcItems: []}}; fl.add = l.add; \
         var f = ''; try {{ fl.add('item', 'A'); }} catch (x) {{ f = x.name; }} \
         for (var n = 1; n < {}; n++) w.add('statictext'); [e, f, w.children.length].join()",
        scriptui::MAX_CONTROLS
    );
    assert_eq!(r.run(&source).unwrap(), JsValue::String(format!("TypeError,TypeError,{}", scriptui::MAX_CONTROLS)));
}

#[test]
fn find_element_skips_a_control_a_name_getter_removes() {
    let (mut r, _, _) = realm(Vec::new());
    let source = "function heavy() { var junk = []; for (var k = 0; k < 50000; k++) junk.push({}); } \
        var w = new Window('dialog'); var a = w.add('button'); var b = w.add('button', undefined, 'B', {name: 'b'}); \
        Object.defineProperty(a, 'name', {get: function () { w.remove(b); b = null; heavy(); return 'a'; }}); \
        String(w.findElement('b'))";
    assert_eq!(r.run(source).unwrap(), JsValue::String("null".into()));
}

#[test]
fn a_close_that_on_close_asks_for_is_part_of_the_close_it_refuses() {
    let (mut r, _, _) = realm(vec![close(), click(1)]);
    let source = "var w = new Window('dialog'); w.add('button', undefined, 'OK'); var asked = 0; \
        w.onClose = function () { asked++; if (asked == 1) { w.close(7); return false; } }; [w.show(), asked].join()";
    assert_eq!(r.run(source).unwrap(), JsValue::String("1,2".into()), "the refused close waits for the click on OK");
}

#[test]
fn a_hidden_or_disabled_ok_button_is_not_the_default_found_by_its_text() {
    let (mut r, seen, _) = realm(vec![close()]);
    let source = "var w = new Window('dialog'); w.add('button', undefined, 'OK').enabled = false; w.add('button', undefined, 'OK').visible = false; \
        w.add('button', undefined, 'OK'); w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(0.0));
    assert_eq!(field(&seen.borrow().descriptions[0], "defaultElement"), &Data::Number(3.0), "the OK button that can be clicked is the default");
}

#[test]
fn an_edit_field_that_hides_its_text_is_refused_when_shown() {
    let (mut r, seen, _) = realm(Vec::new());
    let source = "var w = new Window('dialog'); w.add('edittext', undefined, 'secret', {noecho: true}); try { w.show(); } catch (e) { e.message }";
    assert_eq!(r.run(source).unwrap(), JsValue::String("show: an edit field that hides its text (noecho) is not available yet".into()));
    assert!(seen.borrow().descriptions.is_empty(), "the dialog never opens");
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

#[test]
fn a_new_control_s_sizes_can_be_set_a_side_at_a_time_and_reach_the_description() {
    let (mut r, seen, _) = realm(vec![click(1)]);
    let source = "var w = new Window('dialog'); var b = w.add('button', undefined, 'OK'); \
        b.preferredSize.width = 100; b.minimumSize.height = 30; b.maximumSize.width = 90; w.size.height = 200; w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(1.0));
    let seen = seen.borrow();
    let first = nodes(&seen.descriptions[0]);
    let numbers = |v: &[f64]| Data::Array(v.iter().map(|n| Data::Number(*n)).collect());
    assert_eq!(field(&first[1], "size"), &numbers(&[100.0, -1.0]), "the side left alone stays to the layout");
    assert_eq!(field(&first[1], "min"), &numbers(&[-1.0, 30.0]));
    assert_eq!(field(&first[1], "max"), &numbers(&[90.0, -1.0]));
    assert_eq!(field(&first[0], "size"), &numbers(&[-1.0, 200.0]), "size takes the place of preferredSize");
}

#[test]
fn listeners_run_in_the_capture_target_and_bubbling_phases_until_one_stops() {
    let (mut r, _, _) = realm(vec![click(2)]);
    let source = "var w = new Window('dialog'); var g = w.add('group'); var b = g.add('button', undefined, 'Go'); var log = []; \
        w.addEventListener('click', function (e) { log.push('capture ' + e.eventPhase); }, true); \
        b.onClick = function () { log.push('onClick'); }; \
        b.addEventListener('click', function (e) { log.push(e.type + ' ' + (e.target === b) + ' ' + (this === b)); }); \
        g.addEventListener('click', function (e) { log.push('group ' + (e.currentTarget === g)); }); \
        w.addEventListener('click', function () { log.push('dialog'); w.close(3); }); \
        [w.show(), log.join()].join('|')";
    assert_eq!(r.run(source).unwrap(), JsValue::String("3|capture 1,onClick,click true true,group true,dialog".into()));
    let (mut r, _, _) = realm(vec![click(1), close(), click(1), close()]);
    let source = "var w = new Window('dialog'); var b = w.add('button', undefined, 'OK'); var log = []; \
        function count() { log.push('count'); } b.addEventListener('click', count); b.addEventListener('click', count); \
        b.addEventListener('click', function (e) { e.stopPropagation(); }); \
        w.addEventListener('click', function () { log.push('dialog'); }); \
        w.addEventListener('show', function (e) { log.push('show ' + e.bubbles); }); \
        var first = w.show(); b.removeEventListener('click', count); var second = w.show(); [first, second, log.join()].join('|')";
    assert_eq!(
        r.run(source).unwrap(),
        JsValue::String("0|0|show false,count,show false".into()),
        "a button with a click listener does not close the dialog by itself, and the same listener is added once"
    );
    for (source, want) in [
        ("w.addEventListener('click', function () { w.close(3); })", "3"),
        ("w.addEventListener('click', function () { w.close(3); }, true)", "3"),
        ("w.addEventListener('click', function () {})", "1"),
    ] {
        let (mut r, _, _) = realm(vec![click(1)]);
        let source = format!("var w = new Window('dialog'); w.add('button', undefined, 'OK'); {source}; String(w.show())");
        assert_eq!(r.run(&source).unwrap(), JsValue::String(want.into()), "a dialog listener's close(result) decides what show() gives: {source}");
    }
    for (source, want) in [
        ("new Window('dialog').addEventListener('mouseover', function () {})", "Error addEventListener: the event mouseover is not available yet"),
        ("new Window('dialog').addEventListener('wobble', function () {})", "Error addEventListener: there is no event wobble"),
        ("new Window('dialog').addEventListener('click', 'code')", "TypeError addEventListener: give the handler as a function"),
    ] {
        let (mut r, _, _) = realm(Vec::new());
        let e = r.run(&format!("try {{ {source}; 'added' }} catch (e) {{ e.name + ' ' + e.message }}")).unwrap();
        assert_eq!(e, JsValue::String(want.into()), "{source}");
    }
}

#[test]
fn an_event_on_objects_made_to_look_like_controls_ends_with_a_type_error() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var b = new Window('dialog').add('button', undefined, 'Go'); var ran = false; \
             var f = {__vcKind: 'button'}; var g = {__vcKind: 'group'}; f.__vcParent = g; g.__vcParent = f; f.onClick = function () { ran = true; }; \
             try { b.notify.call(f, 'onClick'); 'ran' } catch (e) { e.name + ' ' + ran }",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("TypeError false".into()), "a parent cycle ends at once, not when the run's fuel is gone");
}

#[test]
fn lists_find_and_remove_items_and_take_their_items_in_the_creation_properties() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var d = w.add('dropdownlist', undefined, undefined, {items: ['a', 'b', 'c']}); \
             var x = w.add('listbox', undefined, {items: ['x'], name: 'xs'}); \
             d.selection = 2; var b = d.find('b'); d.remove('b'); d.remove(0); d.remove('zzz'); d.remove(7); \
             [d.items.length, d.items[0].text, d.selection.text, b.index, d.find('q'), x.items[0].text, x.name].join()",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("1,c,c,-1,,x,xs".into()));
    let both = r.run("try { new Window('dialog').add('listbox', undefined, ['a'], {items: ['b']}); 'made' } catch (e) { e.name }").unwrap();
    assert_eq!(both, JsValue::String("TypeError".into()));
    let freed = r
        .run(
            "var w = new Window('dialog'); var l = w.add('listbox'); for (var n = 0; n < 1024; n++) l.add('item', 'i'); \
             l.remove(0); l.add('item', 'again'); var g = w.add('group'); g.add('button', undefined, 'A'); g.add('button', undefined, 'B'); \
             g.remove('A'); g.remove(g.children[0]); w.remove(l); var m = w.add('listbox'); \
             for (var k = 0; k < 1024; k++) m.add('item', 'j'); for (var i = 0; i < 510; i++) w.add('button'); \
             [g.children.length, w.children.length, m.items.length].join()",
        )
        .unwrap();
    assert_eq!(freed, JsValue::String("0,512,1024".into()), "a removed item or control frees its place");
}

#[test]
fn a_group_takes_its_creation_properties_third() {
    let (mut r, seen, _) = realm(vec![close()]);
    let v = r
        .run(
            "var w = new Window('dialog'); var g = w.add('group', undefined, {orientation: 'column', alignChildren: 'left', name: 'g'}); \
             var p = w.add('panel', undefined, 'Title', {margins: 4}); w.show(); [g.orientation, g.alignChildren, g.name, g.text, p.text, p.margins].join()",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("column,left,g,,Title,4".into()));
    let seen = seen.borrow();
    assert_eq!(field(&nodes(&seen.descriptions[0])[1], "orientation"), &Data::String("column".into()));
}

#[test]
fn a_radio_button_turned_on_turns_off_the_others_of_its_set() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var r1 = w.add('radiobutton', undefined, 'A'); var r2 = w.add('radiobutton', undefined, 'B'); \
             var r3 = w.add('radiobutton', undefined, 'C'); w.add('statictext', undefined, 'Other'); var r4 = w.add('radiobutton', undefined, 'D'); \
             var c = w.add('checkbox', undefined, 'E'); var clicks = 0; r3.onClick = function () { clicks++; }; \
             r1.value = true; r2.value = true; var set = [r1.value, r2.value, r3.value]; r4.value = true; \
             r3.notify('onClick'); r3.notify(); c.notify('onClick'); \
             set.concat([r1.value, r2.value, r3.value, r4.value, c.value, clicks]).join()",
        )
        .unwrap();
    assert_eq!(
        v,
        JsValue::String("false,true,false,false,false,true,true,true,2".into()),
        "a radio button after another kind of control starts a new set"
    );
}

#[test]
fn a_click_on_a_radio_button_turns_off_only_the_radio_buttons_of_its_set() {
    let (mut r, _, _) = realm(vec![click(4), close()]);
    let source = "var w = new Window('dialog'); var r1 = w.add('radiobutton', undefined, 'A'); w.add('statictext', undefined, 'Other'); \
        var r2 = w.add('radiobutton', undefined, 'B'); var r3 = w.add('radiobutton', undefined, 'C'); r1.value = true; r2.value = true; \
        [w.show(), r1.value, r2.value, r3.value].join()";
    assert_eq!(r.run(source).unwrap(), JsValue::String("0,true,false,true".into()));
}

#[test]
fn a_group_takes_its_creation_properties_fourth_after_an_empty_third_argument() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run("var g = new Window('dialog').add('group', undefined, undefined, {name: 'g', orientation: 'column'}); [g.name, g.orientation].join()")
        .unwrap();
    assert_eq!(v, JsValue::String("g,column".into()));
}

#[test]
fn a_capturing_listener_that_stops_a_click_keeps_the_default_button_open() {
    let (mut r, _, _) = realm(vec![click(1), close()]);
    let source = "var w = new Window('dialog'); w.add('button', undefined, 'OK'); var log = []; \
        w.addEventListener('click', function (e) { log.push('stopped'); e.stopPropagation(); }, true); [w.show(), log.join()].join('|')";
    assert_eq!(r.run(source).unwrap(), JsValue::String("0|stopped".into()));
    let (mut r, _, _) = realm(vec![click(1)]);
    let source = "var w = new Window('dialog'); w.add('button', undefined, 'OK'); var seen = 0; \
        w.addEventListener('click', function () { seen++; }); [w.show(), seen].join()";
    assert_eq!(
        r.run(source).unwrap(),
        JsValue::String("1,1".into()),
        "a listener on the dialog that does not capture leaves the default button to close it"
    );
}

#[test]
fn listeners_are_limited_show_is_the_dialog_s_and_stop_propagation_reaches_only_running_events() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var b = w.add('button'); \
             for (var n = 0; n < 256; n++) b.addEventListener('click', function () {}); \
             var more; try { b.addEventListener('click', function () {}); more = 'added'; } catch (e) { more = e.name; } \
             var shown; try { b.addEventListener('show', function () {}); shown = 'added'; } catch (e) { shown = e.message; } \
             var kept, o = Object.freeze({}); var c = w.add('button'); c.addEventListener('click', function (e) { kept = e; }); c.notify(); \
             kept.stopPropagation.call(o); kept.stopPropagation(); kept.preventDefault(); \
             [more, shown, Object.getOwnPropertyNames(o).length, kept.cancelable].join('|')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("RangeError|addEventListener: the event show runs only on a dialog|0|false".into()));
}

#[test]
fn a_removed_control_leaves_its_dialog() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); w.add('statictext'); var g = w.add('group'); var b = g.add('button', undefined, 'Go'); var heard = 0; \
             w.addEventListener('click', function () { heard++; }, true); w.addEventListener('click', function () { heard++; }); \
             w.remove(g); b.notify('onClick'); var errors = []; \
             try { g.remove(0); } catch (e) { errors.push(e.message); } try { g.add('button'); } catch (e) { errors.push(e.message); } \
             var added = 0; try { while (true) { w.add('button'); added++; } } catch (e) {} \
             [g.parent, heard, added, errors.join(';')].join('|')",
        )
        .unwrap();
    assert_eq!(
        v,
        JsValue::String("|0|511|remove: the control was removed from its dialog;add: the control was removed from its dialog".into()),
        "a removed group's controls no longer count toward the dialog's limit and cannot be counted off it twice"
    );
}

#[test]
fn a_remove_that_runs_out_of_fuel_leaves_the_dialog_as_it_was() {
    let (mut r, _, _) = realm(Vec::new());
    r.run("var w = new Window('dialog'); var g = w.add('group'); for (var n = 0; n < 300; n++) g.add('button');").unwrap();
    let mut failed = 0;
    for fuel in 1..20_000 {
        r.set_limits(Limits { fuel, ..Limits::default() });
        if r.run("w.remove(g);").is_ok() {
            break;
        }
        r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
        let state = r.run("[w.children.length, g.parent === w, g.window === w, g.children[299].window === w].join()").unwrap();
        // The run may end after remove returned, with the group already removed.
        if state == JsValue::String("0,false,false,false".into()) {
            break;
        }
        failed += 1;
        assert_eq!(state, JsValue::String("1,true,true,true".into()), "a remove that ended at fuel {fuel} changed the dialog");
    }
    assert!(failed > 300, "the runs ended inside remove's walk of the group ({failed})");
    r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
    let v = r
        .run(
            "var added = 0; try { while (true) { w.add('button'); added++; } } catch (e) {} [w.children.length, g.parent, g.window, added].join('|')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("512|||512".into()), "the removed group's controls are counted off the dialog once");
}

#[test]
fn a_text_getter_that_changes_a_list_while_remove_looks_it_up_makes_remove_throw() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['a', 'b', 'c']); \
             Object.defineProperty(l.items[0], 'text', {get: function () { l.remove(1); return 'a'; }}); \
             var m; try { l.remove('c'); m = 'removed'; } catch (e) { m = e.message; } [m, l.items.length, l.items[1].text].join('|')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("remove: the list changed while the item was looked up|2|c".into()));
}

#[test]
fn a_control_a_listener_removes_while_its_click_runs_no_longer_reaches_its_containers() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var g = w.add('group'); var b = g.add('button', undefined, 'Go'); var heard = []; \
             b.addEventListener('click', function () { g.remove(b); }); \
             g.addEventListener('click', function () { heard.push('g'); }); w.addEventListener('click', function () { heard.push('w'); }); \
             b.notify('onClick'); [heard.length, b.parent, b.window].join('|')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("0||".into()));
}

#[test]
fn a_listener_removed_by_an_earlier_one_in_the_same_event_does_not_run() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var b = w.add('button'); var ran = []; function second() { ran.push(2); } \
             b.addEventListener('click', function () { ran.push(1); b.removeEventListener('click', second); }); \
             b.addEventListener('click', second); b.notify('onClick'); ran.join(',')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("1".into()));
}

#[test]
fn a_listener_removed_and_added_again_in_the_same_event_does_not_run_in_it() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var b = w.add('button'); var ran = []; var once = true; function second() { ran.push(2); } \
             b.addEventListener('click', function () { ran.push(1); if (once) { once = false; b.removeEventListener('click', second); b.addEventListener('click', second); } }); \
             b.addEventListener('click', second); b.notify('onClick'); b.notify('onClick'); ran.join(',')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("1,1,2".into()), "the second registration runs from the next event on");
}

#[test]
fn a_close_a_getter_asks_for_while_the_dialog_is_read_again_ends_show() {
    let (mut r, seen, _) = realm(vec![click(1)]);
    let source = "var w = new Window('dialog'); var b = w.add('button', undefined, 'Go'); var armed = false; \
        b.onClick = function () { armed = true; }; \
        Object.defineProperty(b, 'text', {get: function () { if (armed) { armed = false; w.close(7); } return 'Go'; }}); w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(7.0));
    assert_eq!(seen.borrow().ends, vec![(7.0, true)]);
}

#[test]
fn a_selection_a_later_getter_changes_is_described_as_it_is() {
    let (mut r, seen, _) = realm(vec![click(2)]);
    let source = "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['A', 'B']); var b = w.add('button', undefined, 'OK'); \
        var moved = false; Object.defineProperty(b, 'text', {get: function () { if (!moved) { moved = true; l.selection = 1; } return 'OK'; }}); w.show()";
    assert_eq!(r.run(source).unwrap(), JsValue::Number(1.0));
    assert_eq!(field(&nodes(&seen.borrow().descriptions[0])[1], "selection"), &Data::Number(1.0), "the description shows the selection as it is");
    let (mut r, _, _) = realm(Vec::new());
    let source = "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['A', 'B']); var b = w.add('button', undefined, 'OK'); \
        var reads = 0; Object.defineProperty(b, 'text', {get: function () { reads++; l.selection = reads % 2; return 'OK'; }}); \
        try { w.show(); } catch (e) { e.message }";
    assert_eq!(r.run(source).unwrap(), JsValue::String("show: a list's items or a container's controls changed while the dialog was read".into()));
}

#[test]
fn a_script_cannot_put_its_own_listener_list_on_a_control() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var b = w.add('button'); var a = []; a.length = 4000000000; \
             var m; try { Object.defineProperty(b, '__vcListeners', {value: a}); m = 'defined'; } catch (e) { m = e.name; } \
             b.notify('onClick'); m",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("TypeError".into()));
}

#[test]
fn an_object_made_to_look_like_an_element_reads_a_long_list_as_empty() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var b = w.add('button'); var a = []; a.length = 4000000000; \
             var f = {__vcKind: 'button', __vcListeners: a, __vcKids: a}; b.notify.call(f, 'onClick'); \
             b.findElement.call(f, 'x') === null",
        )
        .unwrap();
    assert_eq!(v, JsValue::Bool(true));
}

#[test]
fn remove_refuses_an_object_made_to_look_like_a_container_of_a_real_control() {
    let (mut r, _, _) = realm(Vec::new());
    let v = r
        .run(
            "var w = new Window('dialog'); var g = w.add('group'); var b = g.add('button'); var l = g.add('listbox', undefined, ['a']); \
             var f = {__vcKind: 'group', __vcWindow: w, __vcParent: w, __vcKids: [b]}; \
             var fl = {__vcKind: 'listbox', __vcWindow: w, __vcParent: w, __vcItems: l.items}; var m = []; \
             try { w.remove.call(f, 0); m.push('removed'); } catch (e) { m.push(e.name); } \
             try { l.removeAll.call(fl); m.push('removed'); } catch (e) { m.push(e.name); } \
             [m.join(','), b.parent === g, b.window === w].join('|')",
        )
        .unwrap();
    assert_eq!(v, JsValue::String("TypeError,TypeError|true|true".into()));
}
