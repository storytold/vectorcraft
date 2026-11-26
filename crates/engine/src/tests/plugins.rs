//! WebAssembly plug-ins (#121): object filters through `plugin.run` (one undo step,
//! transactional), live-effect plug-ins in appearance stacks, and the `plugin.*` management
//! commands.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_plugins::wat as tpl;

use crate::*;

/// The example Desaturate plug-in (built from `crates/plugins/example`).
const DESATURATE: &[u8] = include_bytes!("../../../plugins/tests/fixtures/desaturate.wasm");
const DESATURATE_ID: &str = "org.vectorcraft.example.desaturate";

/// Held by the tests that set the Additional Plug-ins Folder preference or run `plugin.reload`:
/// the folder last loaded and its report are kept for the whole process, so another test loading
/// its own folder between two steps would change what the next step loads or lists.
static PREFS_FOLDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// Installs `wasm`, in place of any module installed with its id (`replace: true`).
fn install(s: &mut Session, wasm: &[u8]) -> Value {
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "replace": true}))
}

fn install_wat(s: &mut Session, src: &str) -> Value {
    install(s, &wat::parse_str(src).unwrap())
}

/// A session started to take `grant` and `allowHeavy` from requests, as most tests here install.
fn session() -> Session {
    let mut s = strict_session();
    s.allow_plugin_grants_from_requests();
    s
}

/// A session as MCP and the CLI have by default: `grant` and `allowHeavy` need an approval.
fn strict_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s
}

/// A 10 × 10 rectangle at `x` filled with `fill` and no stroke.
fn rect(s: &mut Session, x: f64, fill: &str) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"ids": [id.0], "color": fill}));
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn triangle() -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(15.0, 20.0)], true))
}

#[test]
fn object_filter_recolours_the_selection_as_one_undo_step() {
    let mut s = session();
    let info = install(&mut s, DESATURATE);
    assert_eq!((info["id"].as_str(), info["kind"].as_str()), (Some(DESATURATE_ID), Some("filter")));
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#0000ff");
    run(&mut s, "select.all", json!({}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "params": {"amount": 100}}));
    assert_eq!(r["ids"], json!([a.0, b.0]));
    for id in [a, b] {
        let [r, g, b] = node(&s, id).appearance.fill_paint().color().unwrap().to_rgb_uncalibrated();
        assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6, "grey: {r} {g} {b}");
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Desaturate");
    assert_eq!(st.selection.objects, [a, b]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    // Plug-in parameters may also be top-level keys; wrong types are refused before it runs.
    run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "amount": 0}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(matches!(s.execute("plugin.run", &json!({"id": DESATURATE_ID, "amount": "all"})), Err(EngineError::BadParams { .. })));
}

/// `plugin.run` refuses an `ids` list with an entry that is not an object id, with an error naming
/// the key, the entry and its value, and changes nothing. With `ids` left out or null the filter
/// runs on the selection, as it does on the list of the selection.
#[test]
fn object_filter_ids_entries_must_be_ids_and_ids_left_out_take_the_selection() {
    // A session with the plug-in, a red and a blue rectangle, and the red one selected. The
    // document has no creation date, so documents made in different seconds compare equal.
    let fresh = || {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 400, "height": 200, "created": null}));
        install(&mut s, DESATURATE);
        let a = rect(&mut s, 10.0, "#ff0000");
        let b = rect(&mut s, 40.0, "#0000ff");
        run(&mut s, "select.set", json!({"ids": [a.0]}));
        (s, a, b)
    };
    let (_, a, b) = fresh();
    for (bad, entry) in [(json!(["abc"]), r#"entry 0 is "abc""#), (json!([a.0, true]), "entry 1 is true"), (json!([{}]), "entry 0 is {}")] {
        let (mut s, ..) = fresh();
        let st = s.doc().unwrap();
        let before = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
        let r = s.execute("plugin.run", &json!({"id": DESATURATE_ID, "amount": 100, "ids": bad}));
        let want = format!("`ids` {entry}, not a non-negative integer object id");
        assert!(matches!(&r, Err(EngineError::BadParams { cmd, msg }) if cmd == "plugin.run" && *msg == want), "{bad}: {r:?}");
        let st = s.doc().unwrap();
        assert!((st.doc.clone(), st.selection.clone(), st.history.undo.len()) == before, "{bad}: something changed");
    }
    let gray = |s: &Session, id: NodeId| {
        let [r, g, b] = node(s, id).appearance.fill_paint().color().unwrap().to_rgb_uncalibrated();
        (r - g).abs() < 1e-6 && (g - b).abs() < 1e-6
    };
    let mut docs = vec![];
    for ids in [None, Some(Value::Null), Some(json!([a.0]))] {
        let (mut s, ..) = fresh();
        let mut p = json!({"id": DESATURATE_ID, "amount": 100});
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        let r = run(&mut s, "plugin.run", p);
        assert_eq!(r["ids"], json!([a.0]), "{ids:?}");
        assert!(gray(&s, a) && !gray(&s, b), "{ids:?}: only the selected rectangle turns gray");
        docs.push(s.doc().unwrap().doc.clone());
    }
    assert!(docs.windows(2).all(|w| w[0] == w[1]), "left out, null and the selection's list give the same document");
}

#[test]
fn object_filters_add_replace_and_generate_objects() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#00ff00");
    run(&mut s, "select.all", json!({}));
    // Keeps A, drops B and adds a triangle above A.
    let out =
        json!({"objects": [{"id": a.0}, {"path": triangle(), "fills": [{"type": "solid", "color": {"model": "gray", "k": 1}}], "name": "Tri"}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.replace", "filter", "{}"), &out.to_string()));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.replace"}));
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids[0], a);
    let st = s.doc().unwrap();
    assert!(st.doc.node(b).is_none());
    let layer: Vec<NodeId> = st.doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [a, ids[1]]);
    assert_eq!(st.doc.node(ids[1]).unwrap().name.as_deref(), Some("Tri"));
    assert_eq!(st.selection.objects, ids);
    // A generator with nothing selected adds to the current layer.
    let generated = json!({"objects": [{"path": triangle()}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.generate", "filter", "{}"), &generated.to_string()));
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.generate"}));
    let made = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().last().unwrap().id, made);
}

/// A generator can make guides and lock what it adds, in its one undo step; those objects are left
/// out of the selection that follows, as Make Guides and Object › Lock leave them.
#[test]
fn filters_make_guides_and_locked_objects_left_unselected() {
    let mut s = session();
    let out = json!({"objects": [
        {"path": triangle(), "name": "Guide", "guide": true},
        {"path": triangle(), "name": "Locked", "locked": true},
        {"path": triangle(), "name": "Plain"}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.guides", "filter", "{}"), &out.to_string()));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.guides"}));
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert!(matches!(node(&s, ids[0]).kind, vectorcraft_doc::NodeKind::Path { guide: true, .. }));
    assert!(node(&s, ids[1]).locked);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [ids[2]]);
    assert_eq!(st.history.undo.len(), undo + 1);
}

/// A compound path handed back with its path as it was keeps its members, hidden and locked ones
/// included. A changed path replaces its members, and is refused while a member is hidden, locked,
/// a guide or not a path, as is turning it into a plain path.
#[test]
fn a_filter_keeps_or_replaces_a_compound_path_s_members_whole() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeKind};
    use vectorcraft_geom::FillRule;
    let mut s = session();
    let compound_of = |s: &mut Session, members: Vec<Node>| {
        let mut id = NodeId(0);
        s.edit("Set up", |d, _| {
            let n = Node::new(d.alloc_id(), NodeKind::Compound { children: members.into_iter().map(Arc::new).collect(), rule: FillRule::NonZero });
            let layer = d.layers[0].id;
            id = d.insert(Some(layer), usize::MAX, n).unwrap();
            Ok(())
        })
        .unwrap();
        id
    };
    let members = |s: &Session, id: NodeId| match &node(s, id).kind {
        NodeKind::Compound { children, .. } => children.iter().map(|c| (c.id, c.visible, c.locked, c.path_data().cloned())).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let new_ids = |s: &mut Session| s.edit("Set up", |d, _| Ok([d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id()])).unwrap();
    let [a, b, c, d] = new_ids(&mut s);
    let [g, h, i, j] = new_ids(&mut s);
    let hidden = |id: NodeId, x: f64| {
        let mut n = Node::path(id, square_at(x), Appearance::default());
        n.visible = false;
        n
    };
    let locked = |id: NodeId, x: f64| {
        let mut n = Node::path(id, square_at(x), Appearance::default());
        n.locked = true;
        n
    };
    // A member with an empty subpath, which plug-in output can't hold, still counts as handed back.
    let empty = Node::path(g, PathData::single(SubPath::polyline(&[], false)), Appearance::default());
    let kept = compound_of(&mut s, vec![hidden(a, 40.0), locked(b, 80.0), empty]);
    let plain =
        compound_of(&mut s, vec![Node::path(c, square_at(120.0), Appearance::default()), Node::path(d, square_at(160.0), Appearance::default())]);
    let was = members(&s, kept);
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.compound.echo", "filter", "{}"), tpl::ECHO, "", 1));
    run(&mut s, "plugin.run", json!({"id": "test.engine.compound.echo", "ids": [kept.0]}));
    assert_eq!(members(&s, kept), was, "handed back as it was, the members stay as they were");
    // New geometry for a compound with a hidden or a locked member, or the same geometry as a plain
    // path, is refused, and the document is left alone.
    let mut moved = square_at(200.0);
    moved.subpaths.extend(square_at(240.0).subpaths);
    let reshape = |id: &str, target: NodeId, extra: Value| {
        let mut o = json!({"id": target.0, "path": moved.clone()});
        o.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        tpl::returning(&tpl::manifest(id, "filter", "{}"), &json!({ "objects": [o] }).to_string())
    };
    let hidden_only = compound_of(&mut s, vec![hidden(h, 360.0)]);
    let locked_only = compound_of(&mut s, vec![Node::path(i, square_at(360.0), Appearance::default()), locked(j, 380.0)]);
    // A member that is itself a compound path, shown and unlocked but holding a hidden member: its
    // geometry isn't in the input, so a new path or a plain path would drop it.
    let [k, l, m, _] = new_ids(&mut s);
    let inner = Node::new(k, NodeKind::Compound { children: vec![Arc::new(hidden(l, 400.0))], rule: FillRule::NonZero });
    let nested = compound_of(&mut s, vec![Node::path(m, square_at(440.0), Appearance::default()), inner]);
    let was_nested = node(&s, nested).clone();
    run(&mut s, "plugin.run", json!({"id": "test.engine.compound.echo", "ids": [nested.0]}));
    assert_eq!(node(&s, nested).kind, was_nested.kind, "handed back as it was, a nested compound member stays");
    let flagged = "a hidden, locked or guide member";
    for (target, why) in [(hidden_only, flagged), (locked_only, flagged), (nested, "a member that isn't a path")] {
        install_wat(&mut s, &reshape("test.engine.compound.reshape", target, json!({})));
        let flatten = tpl::returning(
            &tpl::manifest("test.engine.compound.flatten", "filter", "{}"),
            &json!({"objects": [{"id": target.0, "type": "path"}]}).to_string(),
        );
        install_wat(&mut s, &flatten);
        let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
        for id in ["test.engine.compound.reshape", "test.engine.compound.flatten"] {
            let e = s.execute("plugin.run", &json!({"id": id, "ids": [target.0]})).unwrap_err().to_string();
            assert!(e.contains(&format!("compound path {} has {why}", target.0)), "{id}: {e}");
            assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len()), before, "{id}");
        }
    }
    // Without such members, new geometry replaces the members: one per subpath, in the old members' places.
    install_wat(&mut s, &reshape("test.engine.compound.reshape", plain, json!({})));
    run(&mut s, "plugin.run", json!({"id": "test.engine.compound.reshape", "ids": [plain.0]}));
    let now = members(&s, plain);
    assert_eq!(now.iter().map(|m| m.3.clone()).collect::<Vec<_>>(), [Some(square_at(200.0)), Some(square_at(240.0))]);
    assert!(now.iter().all(|m| m.1 && !m.2), "shown and unlocked members: {now:?}");
    assert_eq!(now.iter().map(|m| m.0).collect::<Vec<_>>(), [c, d], "the members keep their places' ids");
    // A guide member is refused the same way, unless the output sets `guide` for the whole object.
    let guide_of = |s: &mut Session| {
        let [e, f, ..] = new_ids(s);
        let mut guide_member = Node::path(e, square_at(280.0), Appearance::default());
        if let NodeKind::Path { guide, .. } = &mut guide_member.kind {
            *guide = true;
        }
        compound_of(s, vec![guide_member, Node::path(f, square_at(320.0), Appearance::default())])
    };
    let guides = |s: &Session, id: NodeId| match &node(s, id).kind {
        NodeKind::Compound { children, .. } => children.iter().map(|c| matches!(c.kind, NodeKind::Path { guide: true, .. })).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let guide = guide_of(&mut s);
    install_wat(&mut s, &reshape("test.engine.compound.reshape", guide, json!({})));
    let err = s.execute("plugin.run", &json!({"id": "test.engine.compound.reshape", "ids": [guide.0]})).unwrap_err().to_string();
    assert!(err.contains("guide member"), "{err}");
    let flatten = tpl::returning(
        &tpl::manifest("test.engine.compound.flatten.unflagged", "filter", "{}"),
        &json!({"objects": [{"id": guide.0, "type": "path"}]}).to_string(),
    );
    install_wat(&mut s, &flatten);
    let err = s.execute("plugin.run", &json!({"id": "test.engine.compound.flatten.unflagged", "ids": [guide.0]})).unwrap_err().to_string();
    assert!(err.contains(&format!("compound path {} has {flagged}", guide.0)), "{err}");
    assert_eq!(guides(&s, guide), [true, false], "a refused flatten leaves the guide member as it was");
    for (id, on) in [("test.engine.compound.unguide", false), ("test.engine.compound.reguide", true)] {
        let target = guide_of(&mut s);
        install_wat(&mut s, &reshape(id, target, json!({ "guide": on })));
        run(&mut s, "plugin.run", json!({"id": id, "ids": [target.0]}));
        assert_eq!(members(&s, target).iter().map(|m| m.3.clone()).collect::<Vec<_>>(), [Some(square_at(200.0)), Some(square_at(240.0))], "{id}");
        assert_eq!(guides(&s, target), [on, on], "{id}: every new member takes the output's `guide`");
    }
    // Turning it into a plain path is allowed the same way when the output sets `guide`.
    let target = guide_of(&mut s);
    let flatten = tpl::returning(
        &tpl::manifest("test.engine.compound.flatten.guide", "filter", "{}"),
        &json!({"objects": [{"id": target.0, "type": "path", "guide": true}]}).to_string(),
    );
    install_wat(&mut s, &flatten);
    run(&mut s, "plugin.run", json!({"id": "test.engine.compound.flatten.guide", "ids": [target.0]}));
    assert!(matches!(node(&s, target).kind, NodeKind::Path { guide: true, .. }), "{:?}", node(&s, target).kind);
}

#[test]
fn failing_plugins_leave_the_document_and_history_alone() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let man = |id: &str| tpl::manifest(id, "filter", "{}");
    install_wat(&mut s, &tpl::module(&man("test.engine.trap"), "unreachable", "", 1));
    install_wat(&mut s, &tpl::returning(&man("test.engine.far"), r#"{"objects":[{"path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#));
    install_wat(&mut s, &tpl::returning(&man("test.engine.declines"), r#"{"error":"Select two paths."}"#));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for id in ["test.engine.trap", "test.engine.far", "test.engine.declines", "test.engine.missing"] {
        let e = s.execute("plugin.run", &json!({"id": id})).unwrap_err();
        if id == "test.engine.declines" {
            assert!(e.to_string().contains("Select two paths."), "{e}");
        }
        assert_eq!(s.doc().unwrap().doc, before, "{id}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{id}");
    }
}

/// A filter's output may report a `notice`, which `plugin.run` returns once the output is applied,
/// escaped and capped by the host, with the plug-in's name as `pluginName`. It needs no permission: a filter granted none reports one too.
/// A `notice` that is not a string refuses the output, and the document and history stay as they
/// were.
#[test]
fn a_filter_s_notice_is_returned_once_its_output_is_applied() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000").0;
    let quiet = "test.engine.notice.none";
    install(&mut s, &asking(quiet, "[]", &json!({"objects": [], "notice": "Nothing to tidy"})));
    let r = run(&mut s, "plugin.run", json!({"id": quiet}));
    assert_eq!((&r["plugin"], &r["notice"]), (&json!(quiet), &json!("Nothing to tidy")));
    assert_eq!(r["pluginName"], json!(format!("Test {quiet}")));
    let reading = |s: &mut Session, id: &str, output: &Value| {
        let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, output));
        run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"], "replace": true}))
    };
    let noisy = "test.engine.notice.escaped";
    let long = format!("Done\u{202E}\n{}", "x".repeat(500));
    reading(&mut s, noisy, &json!({"objects": [{"id": id}], "notice": long}));
    let shown = run(&mut s, "plugin.run", json!({"id": noisy, "ids": [id]}))["notice"].as_str().unwrap_or_default().to_string();
    assert!(shown.starts_with("Done\\u{202E}\\u{000A}xx") && shown.ends_with('…'), "{shown}");
    assert_eq!(shown.chars().count(), vectorcraft_plugins::objects::MAX_NOTICE_CHARS);
    let plain = "test.engine.notice.plain";
    reading(&mut s, plain, &json!({"objects": [{"id": id}]}));
    let r = run(&mut s, "plugin.run", json!({"id": plain, "ids": [id]}));
    assert!(r.get("notice").is_none() && r.get("pluginName").is_none(), "{r}");
    let bad = "test.engine.notice.bad";
    install(&mut s, &asking(bad, "[]", &json!({"objects": [], "notice": 3})));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let e = s.execute("plugin.run", &json!({"id": bad})).unwrap_err().to_string();
    assert!(e.contains("`notice` must be a string with visible text"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

/// A plug-in command's result carries its run's `notice` beside the command's id, and so does the
/// step of a batch that runs it.
#[test]
fn a_plugin_command_returns_its_run_s_notice() {
    let mut s = session();
    let id = "test.engine.notice.command";
    let commands = r#"[{"id":"count","label":"Count"}]"#;
    install_wat(&mut s, &tpl::returning(&with_commands(id, "filter", commands), r#"{"objects":[],"notice":"Counted 0 objects"}"#));
    let count = crate::cmd::plugin::command_id(id, "count");
    let r = run(&mut s, &count, json!({}));
    assert_eq!((&r["command"], &r["notice"]), (&json!("count"), &json!("Counted 0 objects")));
    let batch = run(&mut s, "command.batch", json!({"commands": [{"command": count, "params": {}}]}));
    assert!(batch.to_string().contains("Counted 0 objects"), "{batch}");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

#[test]
fn live_effect_plugins_reshape_editable_art_and_expand() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    // An effect that draws a triangle, and one that hands the geometry back.
    let tri = json!({"objects": [{"path": triangle()}]});
    install_wat(
        &mut s,
        &tpl::returning(&tpl::manifest("test.engine.tri", "effect", r#"{"size":{"type":"number","min":0,"max":10,"default":3}}"#), &tri.to_string()),
    );
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.echo", "effect", "{}"), tpl::ECHO, "", 1));
    let catalog = run(&mut s, "effect.list", json!({}))["catalog"].clone();
    let entry = catalog.as_array().unwrap().iter().find(|e| e["id"] == "plugin.test.engine.tri").expect("listed");
    assert_eq!((entry["label"].as_str(), entry["menu"].clone()), (Some("Test test.engine.tri…"), json!(["Effect", "Plug-ins"])));
    assert_eq!(entry["defaults"], json!({"size": 3.0}));
    // A filter can't be applied as an effect, nor an effect run as a filter.
    install(&mut s, DESATURATE);
    assert!(s.execute("effect.apply", &json!({"ids": [a.0], "effect": format!("plugin.{DESATURATE_ID}")})).is_err());
    assert!(s.execute("plugin.run", &json!({"id": "test.engine.tri"})).is_err());

    let effected = |s: &Session| {
        let n = node(s, a);
        let path = n.path_data().unwrap().clone();
        let b = path.bounds().unwrap();
        vectorcraft_render::effects::apply_geometry(&n.appearance.effects, &path, b)
    };
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.echo"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap(), "identity");
    // Re-evaluated when the object changes (non-destructive).
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.move", json!({"dx": 50, "dy": 0}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    // A parameter of another type than the manifest gives it is refused.
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let wrong = json!({"ids": [a.0], "effect": "plugin.test.engine.tri", "params": {"size": "4 pt"}});
    let e = s.execute("effect.apply", &wrong).unwrap_err().to_string();
    assert!(e.contains("`size` must be a number"), "{e}");
    assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len()), before);
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.tri", "params": {"size": 4}}));
    assert_eq!(effected(&s), triangle());
    let e = s.execute("effect.setParams", &json!({"ids": [a.0], "index": 1, "params": {"size": true}})).unwrap_err().to_string();
    assert!(e.contains("`size` must be a number"), "{e}");
    assert_eq!(node(&s, a).appearance.effects[1].params, json!({"size": 4}));
    // Uninstalled: the geometry is drawn as it is.
    run(&mut s, "plugin.remove", json!({"id": "test.engine.tri"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.tri", "effect", "{}"), &tri.to_string()));
    // Expand Appearance bakes it.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut baked = None;
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            baked = Some(p.clone());
        }
    });
    assert_eq!(baked.unwrap().bounds(), triangle().bounds());
}

/// A live effect is given its object's fills and strokes, read only, and which of them it sits on;
/// a change of paint runs it again.
#[test]
fn live_effects_are_given_their_object_s_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [id.0], "color": "#000000"}));
    run(&mut s, "stroke.set", json!({"ids": [id.0], "weight": 5}));
    // The red fill as the host writes paints: a JSON value, its keys sorted.
    let red = serde_json::to_value(&node(&s, id).appearance.fill().unwrap().paint).unwrap().to_string();
    let tri = json!({"objects": [{"path": triangle()}]}).to_string();
    // Each draws the triangle while it is given what it looks for, else hands the geometry back.
    let effect = |id: &str, given, needle: &str| tpl::when_given(&tpl::manifest(id, "effect", "{}"), given, needle, &tri);
    install_wat(&mut s, &effect("test.engine.wide", tpl::Given::Input, r#""width":5.0"#));
    install_wat(&mut s, &effect("test.engine.red", tpl::Given::Input, &format!(r#""fills":[{red}]"#)));
    install_wat(&mut s, &effect("test.engine.second", tpl::Given::Params, r#""item":{"stroke":1}"#));
    let effected = |s: &Session, item: Option<usize>| {
        let n = node(s, id);
        let path = n.path_data().unwrap().clone();
        let ctx = vectorcraft_render::effects::GeomContext::of(&n);
        let (effects, ctx) = match item {
            Some(i) => match &n.appearance.items[i] {
                vectorcraft_doc::AppearanceItem::Stroke(st) => (st.effects.clone(), ctx.item(&n.appearance.items[i])),
                other => panic!("not a stroke: {other:?}"),
            },
            None => (n.appearance.effects.clone(), ctx),
        };
        vectorcraft_render::effects::apply_geometry_with(&effects, &path, path.bounds().unwrap(), &ctx)
    };
    let own = node(&s, id).path_data().unwrap().clone();
    let remove = |s: &mut Session| run(s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": null}));
    for effect in ["plugin.test.engine.wide", "plugin.test.engine.red"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": null}));
        assert_eq!(effected(&s, None), triangle(), "{effect}: given the 5 pt stroke and the red fill");
        remove(&mut s);
    }
    // Run again once the stroke is thinner.
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.wide", "item": null}));
    run(&mut s, "stroke.set", json!({"ids": [id.0], "weight": 3, "item": 1}));
    assert_eq!(effected(&s, None), own, "a 3 pt stroke");
    remove(&mut s);
    // On the second of two strokes, and not on the first.
    run(&mut s, "appearance.addStroke", json!({"ids": [id.0]}));
    let top = node(&s, id).appearance.items.len() - 1;
    for item in [1, top] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.second", "item": item}));
    }
    assert_eq!(effected(&s, Some(1)), own, "the first stroke");
    assert_eq!(effected(&s, Some(top)), triangle(), "the second stroke");
}

/// Expand gives a plug-in effect on a fill the context drawing gives it (`_context.item`), so a
/// gradient fill is expanded into the shape the canvas fills.
#[test]
fn expanding_a_fill_gives_its_plug_in_effect_the_fill_it_sits_on() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear"}}));
    let tri = json!({"objects": [{"path": triangle()}]}).to_string();
    let on_fill = tpl::when_given(&tpl::manifest("test.engine.onfill", "effect", "{}"), tpl::Given::Params, r#""item":{"fill":0}"#, &tri);
    install_wat(&mut s, &on_fill);
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.onfill", "item": fill}));
    // Drawn, the gradient fills the triangle: a point of it outside the rectangle is painted.
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let drawn = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 40, 40, vectorcraft_geom::Affine::IDENTITY, &opts);
    assert_ne!(drawn.pixel(15, 1), [255, 255, 255, 255], "drawn as a triangle");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.expand", json!({"steps": 2}));
    // The gradient's strips sit in a clip group whose clipping path is the shape filled.
    let group = s.doc().unwrap().selection.objects[0];
    let clip = node(&s, group).children().unwrap()[0].clone();
    assert_eq!(clip.path_data().unwrap().bounds(), triangle().bounds(), "the triangle the canvas fills");
}

/// A plug-in effect on a fill, before the fill's own 3D Revolve, is given the fill it sits on
/// (`_context.item`), so the fill revolves the shape the plug-in returns for it.
#[test]
fn a_fills_revolve_gives_the_plug_in_effect_before_it_the_fill_it_sits_on() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let tri = json!({"objects": [{"path": triangle()}]}).to_string();
    let on_fill = tpl::when_given(&tpl::manifest("test.engine.revolvefill", "effect", "{}"), tpl::Given::Params, r#""item":{"fill":0}"#, &tri);
    install_wat(&mut s, &on_fill);
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.revolvefill", "item": fill}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "threeD.revolve", "item": fill}));
    let n = node(&s, id);
    // The same object with the triangle as its path and only the Revolve on its fill.
    let mut expected = n.clone();
    if let vectorcraft_doc::NodeKind::Path { path, .. } = &mut expected.kind {
        *path = triangle();
    }
    expected.appearance.items[fill].effects_mut().retain(|e| e.id == "threeD.revolve");
    let art = vectorcraft_render::effects::revolve_art(&n).unwrap();
    assert_eq!(art, vectorcraft_render::effects::revolve_art(&expected).unwrap(), "the fill revolves the triangle");
}

/// The manifest of a test live effect `id` that declares `paintOutput` (at `api` 2), or that
/// doesn't (`paints` off).
fn painting(id: &str, paints: bool) -> String {
    let declared = if paints { r#","api":2,"paintOutput":true"# } else { "" };
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect"{declared},"params":{{}}}}"#)
}

/// A 20 × 20 square at `x`, 40.
fn square_at(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 40.0), Point::new(x + 20.0, 40.0), Point::new(x + 20.0, 60.0), Point::new(x, 60.0)], true))
}

fn solid(hex: &str) -> Value {
    serde_json::to_value(Paint::solid(Color::from_hex(hex).unwrap())).unwrap()
}

/// The 200 × 200 top left of the document, drawn on white.
fn drawn(s: &Session) -> vectorcraft_render::Rendered {
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A live effect that declares `paintOutput` draws each piece it returns by itself, with the paints
/// it gives (a piece without `fills` keeps the object's), in place of the object; the same output
/// from an effect that doesn't declare it is one path painted by the object's fill. Drawing and
/// export leave the document alone, and Expand Appearance writes the pieces as ordinary paths in
/// one undo step.
#[test]
fn a_painted_live_effect_draws_its_pieces_with_their_own_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}, {"path": square_at(80.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.paints", true), &out));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.geometry", false), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.geometry"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (RED, RED, WHITE), "without paintOutput the paint is ignored");
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.paints"}));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (GREEN, RED, WHITE), "each piece with its own paint");
    // An upstream paint edit reaches the piece that keeps the object's fill; Undo brings it back.
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "color": "#0000ff"}));
    assert_eq!((drawn(&s).pixel(50, 50), drawn(&s).pixel(90, 50)), (GREEN, BLUE));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(drawn(&s).pixel(90, 50), RED);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(drawn(&s).pixel(90, 50), BLUE);
    run(&mut s, "edit.undo", json!({}));
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains("#00ff00") && svg.contains("#ff0000"), "{svg}");
    assert_eq!(s.doc().unwrap().doc, before, "drawing and export don't change the document");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Expanded: two ordinary paths, green and red, and no live effect, as one undo step.
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let mut paths = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            assert!(n.appearance.effects.is_empty(), "no live effect left");
            paths.push((p.bounds(), n.appearance.fill().map(|f| f.paint.clone())));
        }
    });
    let fill = |hex: &str| Some(Paint::solid(Color::from_hex(hex).unwrap()));
    assert_eq!(paths, [(square_at(40.0).bounds(), fill("#00ff00")), (square_at(80.0).bounds(), fill("#ff0000"))]);
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, id).appearance.effects[0].id, "plugin.test.engine.paints");
}

/// On a fill, a painted effect returns only fills: they replace that fill on each piece. A result
/// that breaks a rule (strokes from an effect on a fill, a swatch the document doesn't have) is
/// refused whole: the object is drawn as if the effect weren't there, and `plugin.info` names the
/// plug-in and the field.
#[test]
fn a_painted_effect_on_a_fill_and_refused_results() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let fills = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}]}).to_string();
    let strokes = json!({"objects": [{"path": square_at(40.0), "strokes": [{"paint": solid("#00ff00"), "width": 2}]}]}).to_string();
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Not a swatch");
    let unknown = json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.fillpaint", true), &fills));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.strokepaint", true), &strokes));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.swatchpaint", true), &unknown));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.fillpaint", "item": fill}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the fill's piece, green");
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.fillpaint"}))["lastError"], Value::Null);
    // A second painted effect on the same fill paints what it returns too.
    let blue = json!({"objects": [{"path": square_at(80.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.bluepaint", true), &blue));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.bluepaint", "item": fill}));
    let img = drawn(&s);
    assert_eq!((img.pixel(90, 50), img.pixel(50, 50), img.pixel(15, 15)), (BLUE, WHITE, WHITE), "the second effect's piece, blue");
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 1, "item": fill}));
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": fill}));
    for (effect, item, field) in
        [("test.engine.strokepaint", json!(fill), "objects[0].strokes"), ("test.engine.swatchpaint", Value::Null, "objects[0].fills[0].swatch")]
    {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": format!("plugin.{effect}"), "item": item}));
        let img = drawn(&s);
        assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (WHITE, RED), "{effect}: drawn as it is");
        let e = run(&mut s, "plugin.info", json!({"id": effect}))["lastError"].as_str().unwrap_or_default().to_string();
        assert!(e.starts_with(&format!("Test {effect}: ")) && e.contains(field), "{effect}: {e}");
        let n = node(&s, id);
        let kept = if item.is_null() { n.appearance.effects.clone() } else { n.appearance.items[fill].effects().to_vec() };
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [format!("plugin.{effect}")], "the effect record stays");
        run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": item}));
    }
}

/// A fill's Drop Shadow before a painted effect that returns two fills, and a 200% Transform effect
/// after it: the shadow of each returned fill follows the Transform effect. The square the effect
/// returns spans 30 to 70 pt once scaled, so both half-opaque shadows, 20 pt to the right, cover
/// 85 pt as they cover 75 pt.
#[test]
fn a_shadow_on_each_fill_a_painted_effect_returns_follows_the_transform_after_it() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00"), solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.shadowedfills", true), &out));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for (effect, params) in [
        ("stylize.dropShadow", json!({"x": 10, "y": 0, "blur": 0, "opacity": 50})),
        ("plugin.test.engine.shadowedfills", json!({})),
        ("distort.transform", json!({"scaleH": 200, "scaleV": 200})),
    ] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "params": params, "item": fill}));
    }
    let img = drawn(&s);
    let (near, far) = (img.pixel(75, 50), img.pixel(85, 50));
    assert_ne!(near, WHITE, "the shadows reach past the square");
    assert_eq!(far, near, "both shadows at 20 pt");
}

/// A Transform effect with Scale Strokes & Effects on after a painted effect on a fill leaves the
/// Drop Shadow after it as it is on every paint the piece is drawn with.
#[test]
fn a_transform_after_a_painted_fill_effect_leaves_the_later_shadow_on_every_paint() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let two = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00"), solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twopaints", true), &two));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.twopaints", "item": fill}));
    let mut n = node(&s, id);
    let later = [
        vectorcraft_doc::Effect { id: "distort.transform".into(), params: json!({"scaleH": 200, "scaleV": 200, "strokes": true}), visible: true },
        vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: json!({"x": 10.0, "y": 0.0, "blur": 0.0}), visible: true },
    ];
    n.appearance.items[fill].effects_mut().extend(later);
    let art = vectorcraft_render::effects::reshape(Some(&s.doc().unwrap().doc), &n, None).unwrap();
    let mut offsets = vec![];
    art.walk(&mut |m| {
        for item in &m.appearance.items {
            for r in vectorcraft_render::effects::raster_effects(item.effects()) {
                if let vectorcraft_render::effects::RasterFx::DropShadow { dx, dy, .. } = r {
                    offsets.push((dx, dy));
                }
            }
        }
    });
    assert_eq!(offsets, [(10.0, 0.0), (10.0, 0.0)], "one shadow on each of the piece's two fills");
}

/// A result refused for a swatch the document doesn't have stays cached: once the document has the
/// swatch, the next check of that result clears the error.
#[test]
fn a_refused_swatch_s_error_clears_once_the_document_has_the_swatch() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let mut later = solid("#00ff00");
    later["swatch"] = json!("Later");
    let output = json!({"objects": [{"path": square_at(40.0), "fills": [later]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterswatch", true), &output));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.laterswatch"}));
    drawn(&s);
    assert!(run(&mut s, "plugin.info", json!({"id": "test.engine.laterswatch"}))["lastError"].is_string());
    run(&mut s, "swatch.new", json!({"name": "Later", "color": "#00ff00", "global": true}));
    drawn(&s);
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.laterswatch"}))["lastError"], Value::Null);
}

/// An effect that paints nothing has no references to check, so replacing a refused painted effect
/// with one clears the refusal's error once it runs.
#[test]
fn a_replacement_that_paints_nothing_clears_a_refused_swatch_s_error() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Absent");
    let output = json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.replaced", true), &output));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.replaced"}));
    drawn(&s);
    assert!(run(&mut s, "plugin.info", json!({"id": "test.engine.replaced"}))["lastError"].is_string());
    install_wat(&mut s, &tpl::returning(&painting("test.engine.replaced", false), &output));
    drawn(&s);
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.replaced"}))["lastError"], Value::Null);
}

/// A painted effect on a group may link its pieces to the document's swatches.
#[test]
fn a_painted_group_effect_links_to_the_document_s_swatches() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00", "global": true}));
    run(&mut s, "select.all", json!({}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.grouppaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "plugin.test.engine.grouppaint"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the group's piece, green");
}

/// A painted effect on a group leaves the group's art as it is when a path in it has a linked
/// opacity mask, which can't take the pieces as the path does: the mask still covers the path.
#[test]
fn a_painted_group_effect_leaves_art_with_a_linked_mask_as_it_is() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let m = rect(&mut s, 10.0, "#ffffff");
    run(&mut s, "select.set", json!({"ids": [a.0, m.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({}));
    rect(&mut s, 100.0, "#0000ff");
    run(&mut s, "select.all", json!({}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.maskedpaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "plugin.test.engine.maskedpaint"}));
    let art = vectorcraft_render::effects::evaluate_container(Some(&s.doc().unwrap().doc), &node(&s, g)).unwrap();
    let mut masked = vec![];
    art.walk(&mut |c| masked.extend(c.extras().mask.as_ref().map(|k| (c.geometric_bounds(), k.art.geometric_bounds()))));
    assert!(!masked.is_empty(), "the masked path");
    for (path, mask) in masked {
        assert_eq!(path, mask, "the mask covers the path");
    }
}

/// The output of a painted effect that returns one green square linked to swatch `Leaf`.
fn leaf_square() -> String {
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}]}).to_string()
}

/// A Pathfinder operation and baked appearance take a painted effect's pieces, swatch links
/// included, as the canvas draws them: a group with a painted effect inside a group with a live
/// Pathfinder effect draws the green piece, and baking a path's appearance gives the piece linked
/// to the swatch.
#[test]
fn pathfinder_and_baked_appearance_take_painted_pieces_with_swatch_links() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.leafpaint", true), &leaf_square()));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let inner = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [inner.0], "effect": "plugin.test.engine.leafpaint"}));
    let outer = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [outer.0], "effect": "pathfinder.add"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the Pathfinder operation takes the green piece");

    let id = rect(&mut s, 120.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.leafpaint"}));
    let st = s.doc().unwrap();
    let baked = vectorcraft_render::effects::bake_appearance(Some(&st.doc), st.doc.node(id).unwrap()).unwrap();
    let mut fills = vec![];
    baked.walk(&mut |n| {
        if let Some(p) = n.path_data() {
            fills.push((p.bounds(), n.appearance.fill().map(|f| f.paint.clone())));
        }
    });
    let mut leaf = Paint::solid(Color::from_hex("#00ff00").unwrap());
    if let Paint::Solid { swatch, .. } = &mut leaf {
        *swatch = Some("Leaf".into());
    }
    assert_eq!(fills, [(square_at(40.0).bounds(), Some(leaf))]);
}

/// A swatch-linked piece on screen is drawn when the path it comes from is off screen: the
/// renderer's bounds of the path take in the piece.
#[test]
fn a_swatch_linked_piece_is_not_culled_with_its_off_screen_path() {
    let mut s = session();
    let id = rect(&mut s, 500.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.culled", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.culled"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// The renderer measures a painted piece with its own strokes: a piece off screen whose wide stroke
/// reaches the view is drawn, and the bounds raster effects work over take in that stroke, on a
/// path with no stroke of its own and with a blur on the stroke the effect is on.
#[test]
fn a_piece_s_own_stroke_counts_in_the_bounds_of_what_is_drawn() {
    let mut s = session();
    let wide = json!({"objects": [{"path": square_at(-120.0), "strokes": [{"paint": solid("#00ff00"), "width": 300}]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.widestroke", true), &wide));
    let id = rect(&mut s, -300.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.widestroke"}));
    assert_eq!(drawn(&s).pixel(30, 50), GREEN, "only the piece's stroke is on screen");
    let bounds = |s: &Session, id: NodeId| vectorcraft_render::drawn_bounds(&s.doc().unwrap().doc, &node(s, id)).unwrap();
    assert!(bounds(&s, id).x1 >= 50.0, "{:?}", bounds(&s, id));

    let blurred = rect(&mut s, -300.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [blurred.0], "color": "#0000ff", "width": 1}));
    let stroke = node(&s, blurred).appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "plugin.test.engine.widestroke", "item": stroke}));
    run(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "blur.gaussian", "params": {"radius": 10}, "item": stroke}));
    assert!(bounds(&s, blurred).x1 > 55.0, "the blur spreads the piece's stroke: {:?}", bounds(&s, blurred));
}

/// A retained renderer's kept bounds follow the document's swatches: a swatch-linked piece on
/// screen appears when its swatch is added, though the path it comes from is off screen.
#[test]
fn a_retained_renderer_shows_a_piece_once_its_swatch_exists() {
    let mut s = session();
    let id = rect(&mut s, 500.0, "#ff0000");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.lateleaf", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.lateleaf"}));
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = vectorcraft_render::Renderer::new();
    let at_piece = |r: &mut vectorcraft_render::Renderer, s: &Session| {
        r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50)
    };
    assert_eq!(at_piece(&mut r, &s), WHITE, "the piece is refused");
    run(&mut s, "select.none", json!({}));
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    assert_eq!(at_piece(&mut r, &s), GREEN, "the piece is drawn with its swatch");
}

/// Object › Expand of a gradient fill works piece by piece, for a painted effect on the object and
/// on the fill: a piece that keeps the gradient has it expanded, and a piece with a paint of its own,
/// here linked to the document's swatch, keeps that paint on its own path.
#[test]
fn expanding_a_gradient_fill_keeps_each_piece_s_own_paint() {
    for on_fill in [false, true] {
        let mut s = session();
        let id = rect(&mut s, 10.0, "#ff0000");
        run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear"}}));
        run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
        let mut leaf = solid("#00ff00");
        leaf["swatch"] = json!("Leaf");
        let out = json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}, {"path": square_at(100.0)}]}).to_string();
        install_wat(&mut s, &tpl::returning(&painting("test.engine.expandref", true), &out));
        let item = if on_fill { json!(node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap()) } else { Value::Null };
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.expandref", "item": item}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.expand", json!({"object": false, "fill": true, "stroke": false}));
        let img = drawn(&s);
        assert_eq!((img.pixel(15, 15), img.pixel(50, 50)), (WHITE, GREEN), "on_fill {on_fill}: the input is gone and the piece keeps its paint");
        assert!(![WHITE, GREEN].contains(&img.pixel(110, 50)), "on_fill {on_fill}: the other piece shows the gradient");
        let mut own = Paint::solid(Color::from_hex("#00ff00").unwrap());
        if let Paint::Solid { swatch, .. } = &mut own {
            *swatch = Some("Leaf".into());
        }
        let mut kept = vec![];
        let mut plugins = 0;
        s.doc().unwrap().doc.walk(|n| {
            plugins += n
                .appearance
                .effects
                .iter()
                .chain(n.appearance.items.iter().flat_map(|i| i.effects()))
                .filter(|e| e.id.starts_with("plugin."))
                .count();
            if n.path_data().is_some_and(|p| p.bounds() == square_at(40.0).bounds()) {
                kept.push(n.appearance.fill().map(|f| f.paint.clone()));
            }
        });
        assert_eq!(kept, [Some(own)], "on_fill {on_fill}");
        assert_eq!(plugins, 0, "on_fill {on_fill}: no live effect is left");
    }
}

/// `object.expand.info` measures each piece a painted effect returns: a piece whose repeating
/// gradient needs more runs across it than are written (1,000) reports no gradient to expand, as
/// Expand leaves it a gradient, and the same gradient in 10 runs reports one.
#[test]
fn expand_info_measures_the_gradient_on_each_painted_piece() {
    for (end, expands) in [(120.0, true), (100.2, false)] {
        let mut s = session();
        let id = rect(&mut s, 10.0, "#ff0000");
        let gradient = json!({"kind": "linear", "spread": "repeat", "repeats": 10, "start": [100, 50], "end": [end, 50]});
        run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": gradient}));
        let out = json!({"objects": [{"path": square_at(100.0)}]}).to_string();
        install_wat(&mut s, &tpl::returning(&painting("test.engine.expandruns", true), &out));
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.expandruns"}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        let info = s.execute("object.expand.info", &json!({})).unwrap();
        assert_eq!((info["gradient"].clone(), info["fill"].clone()), (json!(expands), json!(expands)), "{end}");
        let only_fill = s.execute("object.expand", &json!({"object": false, "fill": true, "stroke": false}));
        assert_eq!(only_fill.is_ok(), expands, "{end}: {only_fill:?}");
    }
}

/// Object › Expand of a pattern fill works piece by piece: a piece that keeps the pattern has its
/// tiles expanded, and a piece with a paint of its own keeps that paint.
#[test]
fn expanding_a_pattern_fill_keeps_each_piece_s_own_paint() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let tile = rect(&mut s, 300.0, "#00ff00");
    run(&mut s, "object.pattern.make", json!({"ids": [tile.0], "name": "Tiles", "edit": false}));
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "swatch": "Tiles"}));
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#0000ff")]}, {"path": square_at(100.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.expandtiles", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.expandtiles"}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.expand", json!({"object": false, "fill": true, "stroke": false}));
    let img = drawn(&s);
    assert_eq!((img.pixel(15, 15), img.pixel(50, 50)), (WHITE, BLUE), "the input is gone and the piece keeps its paint");
    assert!(near(img.pixel(110, 50), GREEN), "the other piece shows the tiles");
    let mut patterns = 0;
    s.doc().unwrap().doc.walk(|n| patterns += n.appearance.items.iter().filter(|i| matches!(i.paint(), Paint::Pattern { .. })).count());
    assert_eq!(patterns, 0, "the tiles are objects");
}

/// A path's bounds and the paints of its fills and strokes, each stroke with its weight.
type PaintedPath = (Option<vectorcraft_geom::Rect>, Vec<(Paint, Option<f64>)>);

/// The paths of `id`'s reshaped art that paint, in paint order, each with the fills and strokes of
/// it that paint.
fn painted_paths(s: &Session, id: NodeId) -> Vec<PaintedPath> {
    let st = s.doc().unwrap();
    let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
    let mut paths = vec![];
    art.walk(&mut |n| {
        if let Some(p) = n.path_data() {
            let items: Vec<_> = n
                .appearance
                .items
                .iter()
                .map(|i| match i {
                    vectorcraft_doc::AppearanceItem::Fill(f) => (f.paint.clone(), None),
                    vectorcraft_doc::AppearanceItem::Stroke(st) => (st.paint.clone(), Some(st.width)),
                })
                .filter(|(paint, _)| !paint.is_none())
                .collect();
            if !items.is_empty() {
                paths.push((p.bounds(), items));
            }
        }
    });
    paths
}

fn paint(hex: &str) -> Paint {
    Paint::solid(Color::from_hex(hex).unwrap())
}

/// Two painted effects on one fill, the first returning a piece with two fills: the second runs on
/// each of them, told which fill it is on. On the second fill it returns a blue square at 80 pt; on
/// the first it returns a result that is refused, which leaves only that fill, red, on the square at
/// 40 pt.
#[test]
fn a_later_painted_effect_runs_on_every_fill_a_piece_returns() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let two = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let blue = json!({"objects": [{"path": square_at(80.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twofills", true), &two));
    install_wat(&mut s, &tpl::when_given(&painting("test.engine.onsecond", true), tpl::Given::Params, r#""item":{"fill":1}"#, &blue));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.twofills", "item": fill}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.onsecond", "item": fill}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50)), (RED, BLUE), "the first fill as it was, the second moved and blue");
    let (a, b) = (square_at(40.0).bounds(), square_at(80.0).bounds());
    assert_eq!(painted_paths(&s, id), [(a, vec![(paint("#ff0000"), None)]), (b, vec![(paint("#0000ff"), None)])]);
    // The same result for both fills: each is replaced.
    let one = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.onsecond", true), &one));
    assert_eq!(painted_paths(&s, id), [(a, vec![(paint("#0000ff"), None)]), (a, vec![(paint("#0000ff"), None)])]);
}

/// On a stroke, a later painted effect runs on each stroke the first returns, with its own weight
/// and place among them, and keeps their order.
#[test]
fn a_later_painted_effect_runs_on_every_stroke_a_piece_returns() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [id.0], "color": "#000000"}));
    let two = json!({"objects": [{"path": square_at(40.0), "strokes": [{"paint": solid("#ff0000"), "width": 2}, {"paint": solid("#00ff00"), "width": 6}]}]})
        .to_string();
    let blue = json!({"objects": [{"path": square_at(80.0), "strokes": [{"paint": solid("#0000ff"), "width": 4}]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twostrokes", true), &two));
    install_wat(&mut s, &tpl::when_given(&painting("test.engine.onsecondstroke", true), tpl::Given::Params, r#""item":{"stroke":1}"#, &blue));
    let stroke = node(&s, id).appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.twostrokes", "plugin.test.engine.onsecondstroke"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": stroke}));
    }
    let stroked: Vec<_> = painted_paths(&s, id).into_iter().filter(|(_, items)| items.iter().any(|(_, w)| w.is_some())).collect();
    let (a, b) = (square_at(40.0).bounds(), square_at(80.0).bounds());
    assert_eq!(stroked, [(a, vec![(paint("#ff0000"), Some(2.0))]), (b, vec![(paint("#0000ff"), Some(4.0))])]);
}

/// A third painted effect runs on every fill the second returned, for each fill the first returned.
#[test]
fn a_third_painted_effect_visits_every_fill_from_the_second() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let first = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let second = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#0000ff"), solid("#00ffff")]}]}).to_string();
    let third = json!({"objects": [{"path": square_at(80.0), "fills": [solid("#ffff00")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.third.a", true), &first));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.third.b", true), &second));
    install_wat(&mut s, &tpl::when_given(&painting("test.engine.third.c", true), tpl::Given::Params, r#""item":{"fill":1}"#, &third));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["a", "b", "c"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": format!("plugin.test.engine.third.{effect}"), "item": fill}));
    }
    let (a, b) = (square_at(40.0).bounds(), square_at(80.0).bounds());
    let pair = [(a, vec![(paint("#0000ff"), None)]), (b, vec![(paint("#ffff00"), None)])];
    assert_eq!(painted_paths(&s, id), [pair.clone(), pair].concat());
}

/// A later live effect that changes geometry only runs on each fill a painted effect returned, told
/// which fill it is on: on the second it moves the square to 80 pt, and the first stays at 40 pt.
#[test]
fn a_later_geometry_effect_uses_each_generated_fill_s_context() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let two = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let moved = json!({"objects": [{"path": square_at(80.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.geomfills", true), &two));
    install_wat(&mut s, &tpl::when_given(&painting("test.engine.geomsecond", false), tpl::Given::Params, r#""item":{"fill":1}"#, &moved));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.geomfills", "plugin.test.engine.geomsecond"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": fill}));
    }
    let (a, b) = (square_at(40.0).bounds(), square_at(80.0).bounds());
    assert_eq!(painted_paths(&s, id), [(a, vec![(paint("#ff0000"), None)]), (b, vec![(paint("#00ff00"), None)])]);
}

/// A later Outline Stroke effect runs on each stroke a painted effect returned, with that stroke's
/// weight: the 2 pt stroke's outline is 22 pt wide and the 6 pt stroke's 26 pt, in their order.
#[test]
fn a_later_outline_stroke_uses_each_generated_stroke_s_weight() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [id.0], "color": "#000000"}));
    let two = json!({"objects": [{"path": square_at(40.0), "strokes": [{"paint": solid("#ff0000"), "width": 2}, {"paint": solid("#00ff00"), "width": 6}]}]})
        .to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.outlinestrokes", true), &two));
    let stroke = node(&s, id).appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.outlinestrokes", "item": stroke}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "path.outlineStroke", "params": {}, "item": stroke}));
    let st = s.doc().unwrap();
    let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
    // Each outline is a compound path (the ring around the square): its width and its strokes.
    let mut outlines = vec![];
    art.walk(&mut |n| {
        if let vectorcraft_doc::NodeKind::Compound { children, .. } = &n.kind {
            let bounds = children.iter().filter_map(|c| c.path_data().and_then(PathData::bounds)).reduce(|a, b| a.union(b));
            let strokes: Vec<_> = n
                .appearance
                .items
                .iter()
                .filter_map(|i| match i {
                    vectorcraft_doc::AppearanceItem::Stroke(st) => Some((st.paint.clone(), st.width)),
                    vectorcraft_doc::AppearanceItem::Fill(_) => None,
                })
                .collect();
            outlines.push((bounds.map(|b| (b.width() * 1000.0).round() / 1000.0), strokes));
        }
    });
    assert_eq!(
        outlines,
        [(Some(22.0), vec![(paint("#ff0000"), 2.0)]), (Some(26.0), vec![(paint("#00ff00"), 6.0)])],
        "each stroke is outlined at its own weight"
    );
}

/// A later painted effect's runs on the fills a piece returned share the effects' budget: each run
/// that fits gives all its pieces, and once the budget is spent the remaining fills stay together
/// on the piece's path, with none of their run's pieces.
#[test]
fn painted_fills_share_the_budget_without_partial_results() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let fills = vec![solid("#ff0000"); vectorcraft_plugins::objects::MAX_ITEMS];
    let first = json!({"objects": [{"path": square_at(40.0), "fills": fills}]}).to_string();
    let tiny = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.1, 0.0), Point::new(0.0, 0.1)], true));
    let many = json!({"objects": vec![json!({"path": tiny}); 400]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.budget.fills", true), &first));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.budget.many", true), &many));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.budget.fills", "plugin.test.engine.budget.many"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": fill}));
    }
    let paths = painted_paths(&s, id);
    let tinies = paths.iter().filter(|(b, _)| *b == tiny.bounds()).count();
    let kept: Vec<_> = paths.iter().filter(|(b, _)| *b == square_at(40.0).bounds()).collect();
    assert_eq!(kept.len(), 1, "the fills left out stay together");
    let ran = vectorcraft_plugins::objects::MAX_ITEMS - kept[0].1.len();
    assert!(ran > 1 && !kept[0].1.is_empty(), "{ran} runs gave their pieces");
    assert_eq!(tinies, ran * 400, "every run that fit gave all its pieces");
    assert!(paths.len() <= vectorcraft_plugins::objects::MAX_OBJECTS, "{} paths", paths.len());
}

/// A later painted effect runs on each of 256 fills a painted effect returned on an 8,000-anchor
/// path, although 256 copies of that path would pass the effects' budget, when what it returns
/// fits.
#[test]
fn a_later_painted_effect_runs_on_many_fills_of_a_large_piece_when_its_results_fit() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let zigzag: Vec<Point> = (0..8_000).map(|i| Point::new(40.0 + f64::from(i) * 20.0 / 8_000.0, if i % 2 == 0 { 40.0 } else { 60.0 })).collect();
    let large = PathData::single(SubPath::polyline(&zigzag, true));
    let fills = vec![solid("#ff0000"); vectorcraft_plugins::objects::MAX_ITEMS];
    let first = json!({"objects": [{"path": large, "fills": fills}]}).to_string();
    let tiny = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.1, 0.0), Point::new(0.0, 0.1)], true));
    let small = json!({"objects": [{"path": tiny}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.large.fills", true), &first));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.large.small", true), &small));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.large.fills", "plugin.test.engine.large.small"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": fill}));
    }
    let paths = painted_paths(&s, id);
    let tinies = paths.iter().filter(|(b, _)| *b == tiny.bounds()).count();
    assert_eq!(tinies, vectorcraft_plugins::objects::MAX_ITEMS, "every fill's run");
    assert!(paths.iter().all(|(b, _)| *b != large.bounds()), "no fill left on the large path");
}

/// An open path of `curves` tall curves, each drawn with about 5,200 lines.
fn tall_curves(curves: u32) -> PathData {
    let h = 3_000_000.0;
    let anchors = (0..=curves)
        .map(|i| {
            let x = 40.0 + f64::from(i);
            vectorcraft_geom::Anchor::with_handles(Point::new(x, 0.0), Point::new(x, -h), Point::new(x, h))
        })
        .collect();
    PathData::single(SubPath::new(anchors, false))
}

/// A later painted effect's run on the first of two fills a painted effect returned on a path that
/// takes most of the effects' budget gives its result when it and the second fill's result fit
/// together, although the result and another copy of the path would not.
#[test]
fn a_later_painted_run_counts_the_results_after_it_before_keeping_room_for_its_input() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let (large, result) = (tall_curves(1_000), tall_curves(600));
    let tiny = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.1, 0.0), Point::new(0.0, 0.1)], true));
    let two = json!({"objects": [{"path": large, "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let on_first = json!({"objects": [{"path": result}]}).to_string();
    let on_second = json!({"objects": [{"path": tiny}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.tall.fills", true), &two));
    let second = painting("test.engine.tall.each", true);
    install_wat(&mut s, &tpl::when_given_else(&second, tpl::Given::Params, r#""item":{"fill":0}"#, &on_first, &on_second));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.tall.fills", "plugin.test.engine.tall.each"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": fill}));
    }
    let want = [(result.bounds(), vec![(paint("#ff0000"), None)]), (tiny.bounds(), vec![(paint("#00ff00"), None)])];
    assert_eq!(painted_paths(&s, id), want, "both runs give their results");
}

/// When the results of a later painted effect's runs on the two fills of a large path do not fit
/// together, and the first fill's result and another copy of the path do not fit either, the first
/// fill stays on the path and the second fill's result, which fits beside the path, is drawn.
#[test]
fn a_later_painted_run_that_did_not_fit_beside_an_earlier_result_fits_once_that_is_left_out() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let (large, first, second) = (tall_curves(577), tall_curves(1_058), tall_curves(770));
    let two = json!({"objects": [{"path": large, "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let on_first = json!({"objects": [{"path": first}]}).to_string();
    let on_second = json!({"objects": [{"path": second}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.refit.fills", true), &two));
    let each = painting("test.engine.refit.each", true);
    install_wat(&mut s, &tpl::when_given_else(&each, tpl::Given::Params, r#""item":{"fill":0}"#, &on_first, &on_second));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    for effect in ["plugin.test.engine.refit.fills", "plugin.test.engine.refit.each"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": fill}));
    }
    let want = [(large.bounds(), vec![(paint("#ff0000"), None)]), (second.bounds(), vec![(paint("#00ff00"), None)])];
    assert_eq!(painted_paths(&s, id), want, "the second fill's result is drawn");
}

/// A Transform effect before a painted effect on a fill moves the fill's placed gradient: a piece
/// that keeps the fill, and the fill drawn on the moved shape after a refused result, both have the
/// gradient at its moved position.
#[test]
fn a_painted_effect_on_a_fill_keeps_a_placed_gradient_with_its_moved_shape() {
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Not a swatch");
    for (name, out, want) in [
        ("keeps", json!({"objects": [{"path": square_at(40.0)}]}).to_string(), square_at(40.0).bounds()),
        ("refused", json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string(), square_at(110.0).bounds()),
    ] {
        let mut s = session();
        let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 40, "width": 20, "height": 20}))["id"].as_u64().unwrap());
        run(&mut s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
        let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
        run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear", "stops": stops}}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "paint.setGradientGeom", json!({"start": [14, 50], "end": [26, 50], "stroke": false}));
        let plugin = format!("test.engine.carry{name}");
        install_wat(&mut s, &tpl::returning(&painting(&plugin, true), &out));
        let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "distort.transform", "params": {"moveH": 100}, "item": fill}));
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": format!("plugin.{plugin}"), "item": fill}));
        let st = s.doc().unwrap();
        let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
        let mut seen = vec![];
        art.walk(&mut |n| {
            if let (Some(p), Some(Paint::Gradient(g))) = (n.path_data(), n.appearance.items.iter().find(|i| i.is_fill()).map(|i| i.paint())) {
                seen.push((p.bounds(), g.geom.map(|g| (g.start.x, g.end.x))));
            }
        });
        assert_eq!(seen, [(want, Some((114.0, 126.0)))], "{name}");
    }
}

/// The geometry of a painted result that names a swatch counts only when the document has that
/// swatch: without a document, and with one that lacks it, the effect keeps its input.
#[test]
fn painted_geometry_is_used_only_when_its_references_are_in_the_document() {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_with};
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.geomref", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.geomref"}));
    let n = node(&s, id);
    let path = n.path_data().unwrap().clone();
    let input = path.bounds().unwrap();
    let geometry = |doc: Option<&vectorcraft_doc::Document>| {
        apply_geometry_with(&n.appearance.effects, &path, input, &GeomContext { doc, ..GeomContext::of(&n) }).bounds()
    };
    assert_eq!(geometry(None), Some(input), "no document");
    assert_eq!(geometry(Some(&s.doc().unwrap().doc)), Some(input), "no such swatch");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    assert_eq!(geometry(Some(&s.doc().unwrap().doc)), square_at(40.0).bounds(), "the piece's geometry");
}

/// Type with a painted effect, in a group with a live Pathfinder effect: the operation takes the
/// piece linked to the document's swatch.
#[test]
fn pathfinder_takes_the_swatch_linked_pieces_of_type() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 20}))["id"].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.typepaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [t.0], "effect": "plugin.test.engine.typepaint"}));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.divide"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// Type whose fill has a painted effect, in a group with a geometry effect inside a live Pathfinder
/// effect: the operation takes the piece linked to the document's swatch.
#[test]
fn pathfinder_takes_swatch_linked_pieces_of_type_in_a_reshaped_group() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 20}))["id"].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.memberpaint", true), &leaf_square()));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "plugin.test.engine.memberpaint", "item": 0}));
    let inner = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [inner.0], "effect": "path.offsetPath", "params": {"offset": 0}}));
    let outer = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [outer.0], "effect": "pathfinder.divide"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// A painted effect on an image's own fill that links its piece to a swatch: hit tests and bounds
/// use the piece the canvas draws.
#[test]
fn an_image_s_painted_fill_is_hit_and_measured_as_drawn() {
    let mut s = session();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let r = run(&mut s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [65, 65]}));
    let im = NodeId(r["ids"][0].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    let out = json!({"objects": [{"path": square_at(120.0), "fills": [leaf]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.imagepaint", true), &out));
    run(&mut s, "select.set", json!({"ids": [im.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "convertToShape.rectangle", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "plugin.test.engine.imagepaint", "item": 0}));
    assert_eq!(drawn(&s).pixel(130, 50), GREEN, "the canvas draws the piece");
    let doc = &s.doc().unwrap().doc;
    assert!(vectorcraft_doc::hit::hit_test(doc, Point::new(130.0, 50.0), Default::default()).is_some(), "a click on the piece hits the image");
    let boxes = doc.symbol_boxes();
    assert!(doc.node(im).unwrap().reach_bounds_with(&boxes).unwrap().x1 >= 140.0, "the bounds take in the piece");
}

/// A path with a painted effect takes part in a Pathfinder operation as its pieces, each with its
/// own paint and the path's opacity: Divide on a group holding it makes one face per piece,
/// painted as the piece is.
#[test]
fn a_path_with_a_painted_effect_takes_part_in_pathfinder_as_its_pieces() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}, {"path": square_at(80.0), "fills": [solid("#0000ff")]}]})
        .to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twopaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.twopaint"}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.divide"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (GREEN, BLUE, WHITE), "one face per piece");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "transparency.set", json!({"item": null, "opacity": 50}));
    let st = s.doc().unwrap();
    let hook = |c: &vectorcraft_doc::Node| vectorcraft_render::effects::outline_object(Some(&st.doc), c);
    let faces = vectorcraft_render::effects::pathfinder_children(Some(&st.doc), st.doc.node(g).unwrap(), Some(&hook)).unwrap();
    let faces: Vec<_> = faces.iter().map(|f| (f.appearance.fill().map(|f| f.paint.clone()), f.opacity)).collect();
    let fill = |hex: &str| Some(Paint::solid(Color::from_hex(hex).unwrap()));
    assert_eq!(faces, [(fill("#0000ff"), 0.5), (fill("#00ff00"), 0.5)]);
}

/// Art a renderer keeps for an object with a painted effect is made again when a swatch its pieces
/// name leaves the document (the object is then drawn as if the effect weren't there) and when
/// Undo brings the swatch back.
#[test]
fn kept_art_follows_the_swatches_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.keptpaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.keptpaint"}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| {
        let img = r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
        (img.pixel(50, 50), img.pixel(15, 15))
    };
    assert_eq!(draw(&s), (GREEN, WHITE), "the piece, green");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    assert_eq!(draw(&s), (WHITE, RED), "without the swatch, the object as it is");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(draw(&s), (GREEN, WHITE), "the swatch back, the piece again");
}

/// A piece linked to a global swatch shows the swatch's edited color, in art a renderer keeps and
/// in a kept raster of a blur over it, and its earlier color after Undo.
#[test]
fn a_piece_linked_to_a_global_swatch_takes_its_edited_color() {
    for blur in [false, true] {
        let mut s = session();
        let id = rect(&mut s, 10.0, "#ff0000");
        install_wat(&mut s, &tpl::returning(&painting("test.engine.globalleaf", true), &leaf_square()));
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.globalleaf"}));
        if blur {
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
        }
        run(&mut s, "select.none", json!({}));
        run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00", "global": true}));
        let mut r = vectorcraft_render::Renderer::new();
        let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut at_piece = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50);
        assert!(near(at_piece(&s), GREEN), "the piece, in the swatch's color (blur {blur})");
        run(&mut s, "swatch.edit", json!({"name": "Leaf", "color": "#0000ff"}));
        let px = at_piece(&s);
        assert!(near(px, BLUE), "the swatch's new color (blur {blur}): {px:?}");
        run(&mut s, "edit.undo", json!({}));
        assert!(near(at_piece(&s), GREEN), "the swatch's color after Undo (blur {blur})");
    }
}

/// Art a renderer keeps for an object whose plug-in starts to paint its pieces when it is replaced
/// follows the swatches those pieces name from then on.
#[test]
fn kept_art_follows_the_swatches_once_a_replaced_plugin_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterpaint", false), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.laterpaint"}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| {
        let img = r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
        (img.pixel(50, 50), img.pixel(15, 15))
    };
    assert_eq!(draw(&s), (RED, WHITE), "the square in the object's paint");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterpaint", true), &leaf_square()));
    assert_eq!(draw(&s), (GREEN, WHITE), "the piece, green");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    assert_eq!(draw(&s), (WHITE, RED), "without the swatch, the object as it is");
}

/// Whether `px` is within 40 levels of `want` on every channel. A blur leaves the middle of a
/// shape close to its color.
fn near(px: [u8; 4], want: [u8; 4]) -> bool {
    px.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 40)
}

/// A kept raster of a blur over a painted effect is made again when a swatch its piece names
/// leaves the document (the object is then drawn as if the effect weren't there) and when Undo
/// brings the swatch back.
#[test]
fn a_kept_raster_follows_the_swatches_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.blurpaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.blurpaint"}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), GREEN) && near(img.pixel(15, 15), WHITE), "the piece, green and blurred");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), WHITE) && near(img.pixel(15, 15), RED), "without the swatch, the object as it is: {:?}", img.pixel(50, 50));
    run(&mut s, "edit.undo", json!({}));
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), GREEN) && near(img.pixel(15, 15), WHITE), "the swatch back, the piece again");
}

/// A kept raster of a blur over a painted effect is made again when the tile of a pattern its
/// piece names changes and the pattern keeps its name.
#[test]
fn a_kept_raster_follows_the_tiles_of_patterns_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let green = rect(&mut s, 300.0, "#00ff00");
    let blue = rect(&mut s, 320.0, "#0000ff");
    let tile = |r: f64, art: vectorcraft_doc::Node| vectorcraft_doc::pattern::PatternDef {
        tile: vectorcraft_geom::Rect::new(r, 10.0, r + 10.0, 20.0),
        ..vectorcraft_doc::pattern::PatternDef::new("Tiles", vec![std::sync::Arc::new(art)])
    };
    let (first, second) = (tile(300.0, node(&s, green)), tile(320.0, node(&s, blue)));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).patterns.push(first);
    let paint = serde_json::to_value(Paint::Pattern { pattern: "Tiles".into(), xf: vectorcraft_geom::Affine::IDENTITY }).unwrap();
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [paint]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.tilepaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.tilepaint"}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50);
    assert!(near(draw(&s), GREEN), "the piece in the green tile");
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).patterns[0] = second;
    let px = draw(&s);
    assert!(near(px, BLUE), "the piece in the new tile: {px:?}");
}

/// Every painted piece counts against the effects' budget, however small its path: two painted
/// effects in a stack that each return 350 tiny triangles make no more than 100,000 objects, not
/// 350 × 350 of them.
#[test]
fn painted_pieces_keep_to_the_effects_budget() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let tiny = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.1, 0.0), Point::new(0.0, 0.1)], true));
    let out = json!({"objects": vec![json!({"path": tiny}); 350]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.many", true), &out));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.many"}));
    }
    let st = s.doc().unwrap();
    let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
    let mut paths = 0;
    art.walk(&mut |n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. })));
    assert!((350..=100_000).contains(&paths), "{paths} paths");
}

/// A clock for a document's deferrals that reads `step` milliseconds later each time it is read;
/// the test changes the step as it goes.
fn stepping_clock(step: &std::sync::Arc<std::sync::atomic::AtomicU64>) -> vectorcraft_plugins::deferred::Clock {
    let (now, step) = (std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)), step.clone());
    std::sync::Arc::new(move || Some(now.fetch_add(step.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed)))
}

/// While a document is drawn on screen, the plug-in effects of its first view share a time budget.
/// A stage left out draws its input art, is listed by `effect.deferredStatus`, and stays left out
/// on later drawings; Effect › Refresh Deferred Effects runs it again with a fresh budget, and an
/// export draws every stage. Another document has a budget of its own.
#[test]
fn the_first_view_defers_plug_in_effects_past_its_budget() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 30.0, "#ff0000");
    for (id, x) in [("test.engine.deferred.a", 100.0), ("test.engine.deferred.b", 140.0)] {
        let out = json!({"objects": [{"path": square_at(x)}]}).to_string();
        install_wat(&mut s, &tpl::returning(&tpl::manifest(id, "effect", "{}"), &out));
    }
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.deferred.a"}));
    run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": "plugin.test.engine.deferred.b"}));
    // Each reading is 3 s later: the first run's result comes back after the budget, and the
    // second run doesn't start.
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    let img = fx.view(|| drawn(&s));
    fx.first_view_done();
    assert_eq!((img.pixel(15, 15), img.pixel(35, 15), img.pixel(110, 50), img.pixel(150, 50)), (RED, RED, WHITE, WHITE), "the input art");
    assert_eq!(fx.take_report(), Some(2));
    let status = run(&mut s, "effect.deferredStatus", json!({}));
    let listed = |id: NodeId, plugin: &str| json!({"objectId": id.0, "effectIndex": 0, "pluginId": plugin, "reason": "budget"});
    assert_eq!(status, json!({"budgetMs": 2000, "effects": [listed(a, "test.engine.deferred.a"), listed(b, "test.engine.deferred.b")]}));
    assert_eq!(fx.view(|| drawn(&s)).pixel(110, 50), WHITE, "a later drawing leaves it deferred");

    // Refreshed with a fresh budget, in which the clock stands still.
    assert!(s.execute("effect.refreshDeferred", &json!({"ids": ["a"]})).is_err());
    assert!(s.execute("effect.refreshDeferred", &json!({"ids": [999_999]})).is_err(), "not an object of the document");
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    let refreshed = run(&mut s, "effect.refreshDeferred", json!({"ids": [a.0]}));
    assert_eq!(refreshed, json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    let img = fx.view(|| drawn(&s));
    assert_eq!((img.pixel(110, 50), img.pixel(150, 50)), (RED, WHITE));
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([listed(b, "test.engine.deferred.b")]));

    // An export draws every stage; the view then uses the result.
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains(r#"<rect x="140" y="40" width="20" height="20""#), "the square at 140: {svg}");
    assert_eq!(fx.view(|| drawn(&s)).pixel(150, 50), RED);
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([]));
    assert!(s.execute("effect.refreshDeferred", &json!({})).is_err(), "nothing is deferred");

    // Another document: its first view has a budget of its own.
    let mut other = session();
    let c = rect(&mut other, 50.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(170.0)}]}).to_string();
    install_wat(&mut other, &tpl::returning(&tpl::manifest("test.engine.deferred.c", "effect", "{}"), &out));
    run(&mut other, "effect.apply", json!({"ids": [c.0], "effect": "plugin.test.engine.deferred.c"}));
    let still = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let fx_c = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&still));
    other.doc_mut().unwrap().plugin_effects = fx_c.clone();
    assert_eq!(fx_c.view(|| drawn(&other)).pixel(180, 50), RED);
    fx_c.first_view_done();
    assert_eq!(fx_c.take_report(), Some(0));
}

/// A later painted effect deferred on the fills a piece returned is listed once, at its place among
/// the object's fill's effects, and a refresh of the object runs it on each of them.
#[test]
fn deferred_runs_on_the_fills_a_piece_returned_keep_the_fill_s_stage() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let two = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let blue = json!({"objects": [{"path": square_at(80.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.deferred.two", true), &two));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.deferred.later", true), &blue));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.deferred.two", "item": fill}));
    // The first effect's result is kept from a drawing outside a view's budget.
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.deferred.later", "item": fill}));
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    let img = fx.view(|| drawn(&s));
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50)), (GREEN, WHITE), "the fills on the first effect's piece");
    let places = run(&mut s, "effect.deferredStatus", json!({}))["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["objectId"].as_u64().unwrap(), e["item"].as_u64(), e["effectIndex"].as_u64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(places, [(id.0, Some(fill as u64), 1)], "the fill's stage, listed once");
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({"ids": [id.0]})), json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    let img = fx.view(|| drawn(&s));
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50)), (WHITE, BLUE), "both fills replaced");
}

/// Deferred stages are listed where their effects are: after an effect that changes no geometry,
/// and on a fill after a Transform effect that reaches its appearance, each of two instances of
/// one plug-in at its own place.
#[test]
fn deferred_stages_are_listed_at_their_effects_places() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 60.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    // An effect that paints its pieces: the object is drawn from its reshaped art.
    install_wat(&mut s, &tpl::returning(&painting("test.engine.deferred.at", true), &out));
    const FX: &str = "plugin.test.engine.deferred.at";
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "stylize.dropShadow"}));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": FX}));
    }
    run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": "distort.transform", "params": {"strokes": true}, "item": 0}));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": FX, "item": 0}));
    }
    // No run has time to give a result.
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    let places = run(&mut s, "effect.deferredStatus", json!({}))["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["objectId"].as_u64().unwrap(), e["item"].as_u64(), e["effectIndex"].as_u64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(places, [(a.0, None, 1), (a.0, None, 2), (b.0, Some(0), 1), (b.0, Some(0), 2)]);
}

/// A refresh that does not draw a stage (its object, or a group holding it, is hidden) leaves it
/// deferred, and a stage whose effect has been removed or hidden, or whose object has been removed,
/// is no longer listed and does not enable Effect › Refresh Deferred Effects.
#[test]
fn refresh_keeps_stages_it_did_not_draw_and_drops_removed_ones() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 30.0, "#ff0000");
    let c = rect(&mut s, 50.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.deferred.kept", "effect", "{}"), &out));
    run(&mut s, "effect.apply", json!({"ids": [a.0, b.0, c.0], "effect": "plugin.test.engine.deferred.kept"}));
    run(&mut s, "select.set", json!({"ids": [c.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    fx.first_view_done();
    let listed = |s: &mut Session| {
        run(s, "effect.deferredStatus", json!({}))["effects"].as_array().unwrap().iter().map(|e| e["objectId"].as_u64().unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(listed(&mut s), [a.0, b.0, c.0]);

    s.execute("object.hide", &json!({"ids": [a.0, g.0]})).unwrap();
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    let refreshed = run(&mut s, "effect.refreshDeferred", json!({"ids": [a.0, c.0]}));
    assert_eq!(refreshed, json!({"refreshed": 0, "stillDeferred": 2, "failed": 0}), "a hidden object is not drawn");
    assert_eq!(listed(&mut s), [a.0, b.0, c.0]);

    run(&mut s, "effect.remove", json!({"ids": [a.0], "index": 0}));
    assert_eq!(listed(&mut s), [b.0, c.0], "its effect was removed");
    run(&mut s, "effect.setParams", json!({"ids": [b.0], "index": 0, "visible": false}));
    assert_eq!(listed(&mut s), [c.0], "its effect was hidden");
    s.execute("edit.clear", &json!({"ids": [b.0, g.0]})).unwrap();
    assert_eq!(listed(&mut s), Vec::<u64>::new(), "its object was deleted");
    assert!(s.execute("effect.refreshDeferred", &json!({})).is_err(), "nothing is deferred");
}

/// A deferred stage in a symbol's art is listed by the object in that art, and a refresh runs it
/// where the symbol is placed.
#[test]
fn deferred_stages_in_symbol_art_are_listed_and_refreshed() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.deferred.symbol", "effect", "{}"), &out));
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.deferred.symbol"}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let instance = run(&mut s, "symbol.new", json!({"name": "Deferred"}))["id"].as_u64().unwrap();
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    fx.first_view_done();
    assert_eq!(fx.take_report(), Some(1));
    let effects = run(&mut s, "effect.deferredStatus", json!({}))["effects"].clone();
    let object = effects[0]["objectId"].as_u64().unwrap();
    assert_eq!(effects.as_array().map(Vec::len), Some(1), "{effects}");
    assert!(object != instance && s.doc().unwrap().doc.node(NodeId(object)).is_none(), "an object of the symbol's art: {effects}");

    step.store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({})), json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([]));
}

/// `effect.refreshDeferred` takes the id of an object in the art of a symbol, pattern, brush or
/// opacity mask, as `effect.deferredStatus` lists it: one with a deferred stage is refreshed, and
/// one without (or whose stage is gone) adds nothing to the counts. The id of art that is no
/// longer in the document is an error.
#[test]
fn refresh_takes_objects_in_definition_art() {
    let mut s = session();
    const FX: &str = "plugin.test.engine.deferred.nested";
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.deferred.nested", "effect", "{}"), &out));
    // A symbol whose art holds an object with the effect and one without.
    let staged = rect(&mut s, 10.0, "#ff0000");
    let plain = rect(&mut s, 30.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [staged.0], "effect": FX}));
    run(&mut s, "select.set", json!({"ids": [staged.0, plain.0]}));
    run(&mut s, "symbol.new", json!({"name": "Nested"}));
    // A pattern, an art brush (its source deleted) and an opacity mask, none with a stage.
    let tile = rect(&mut s, 50.0, "#00ff00");
    run(&mut s, "select.set", json!({"ids": [tile.0]}));
    run(&mut s, "object.pattern.make", json!({"name": "Tile", "width": 20, "height": 20}));
    run(&mut s, "object.pattern.done", json!({}));
    let bar = rect(&mut s, 70.0, "#0000ff");
    run(&mut s, "select.set", json!({"ids": [bar.0]}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Bar"}));
    run(&mut s, "edit.clear", json!({"ids": [bar.0]}));
    let masked = rect(&mut s, 90.0, "#ff0000");
    let mask = rect(&mut s, 90.0, "#ffffff");
    run(&mut s, "select.set", json!({"ids": [masked.0, mask.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({}));
    // A layer object with a stage that stays deferred, and one without.
    let kept = rect(&mut s, 130.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [kept.0], "effect": FX}));
    let bare = rect(&mut s, 150.0, "#ff0000");

    // The ids of the objects of `art` that are not in the layers.
    let nested = |s: &Session, art: &vectorcraft_doc::Node| {
        let d = &s.doc().unwrap().doc;
        let mut ids = Vec::new();
        vectorcraft_doc::swatches::walk_with_masks(art, &mut |n| {
            if d.node(n.id).is_none() {
                ids.push(n.id.0);
            }
        });
        ids
    };
    let d = s.doc().unwrap().doc.clone();
    let in_symbol = nested(&s, &d.symbols[0].art);
    let in_pattern = d.patterns.iter().flat_map(|p| p.art.iter().flat_map(|a| nested(&s, a))).collect::<Vec<_>>();
    let brushes = vectorcraft_brush::library(&d);
    let in_brush = brushes.iter().flat_map(|b| b.art().into_iter().flat_map(|a| nested(&s, a)).collect::<Vec<_>>()).collect::<Vec<_>>();
    let in_mask = d.node(masked).and_then(|n| n.extras().mask.as_ref()).map(|m| nested(&s, &m.art)).unwrap_or_default();
    for (what, ids) in [("symbol", &in_symbol), ("pattern", &in_pattern), ("brush", &in_brush), ("opacity mask", &in_mask)] {
        assert!(!ids.is_empty(), "the {what} art has an object outside the layers");
    }

    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    fx.first_view_done();
    let listed = |s: &mut Session| {
        run(s, "effect.deferredStatus", json!({}))["effects"].as_array().unwrap().iter().map(|e| e["objectId"].as_u64().unwrap()).collect::<Vec<_>>()
    };
    let deferred = listed(&mut s);
    let [nested_staged, last] = deferred[..] else { panic!("two stages: {deferred:?}") };
    assert!(in_symbol.contains(&nested_staged) && last == kept.0, "{deferred:?} {in_symbol:?}");
    let Some(&nested_plain) = in_symbol.iter().find(|i| **i != nested_staged) else { panic!("{in_symbol:?}") };

    // Objects without a stage, each on its own and together: nothing to refresh.
    let none = json!({"refreshed": 0, "stillDeferred": 0, "failed": 0});
    let mut plain_ids = vec![nested_plain, in_pattern[0], in_brush[0], in_mask[0], bare.0];
    for id in &plain_ids {
        assert_eq!(run(&mut s, "effect.refreshDeferred", json!({"id": id})), none, "object {id}");
    }
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({"ids": plain_ids})), none);
    assert_eq!(listed(&mut s), [nested_staged, kept.0]);

    // The staged object of the symbol's art, by its id, with the others.
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    plain_ids.push(nested_staged);
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({"ids": plain_ids})), json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    assert_eq!(listed(&mut s), [kept.0]);
    // Its stage is gone; the object is still there.
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({"id": nested_staged})), none);

    // Art no longer in the document, and an id of no object.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).patterns.clear();
    assert!(s.execute("effect.refreshDeferred", &json!({"id": in_pattern[0]})).is_err(), "the pattern is gone");
    assert!(s.execute("effect.refreshDeferred", &json!({"ids": [nested_plain, 999_999]})).is_err(), "not an object of the document");
    assert_eq!(listed(&mut s), [kept.0]);
}

/// `CRAFT_PLUGIN_PAINTS_WASM=<folder>` saves a test live effect to try in the app
/// (`stroke-width-test-effect.wasm`): it draws a triangle over the area 100,80–300,230 while its
/// object's stroke is 5 pt wide, and hands the geometry back otherwise.
#[test]
fn stroke_width_test_effect() {
    let Some(dir) = std::env::var_os("CRAFT_PLUGIN_PAINTS_WASM") else { return };
    let triangle = PathData::single(SubPath::polyline(&[Point::new(100.0, 230.0), Point::new(300.0, 230.0), Point::new(200.0, 80.0)], true));
    let manifest = r#"{"id":"org.example.strokewidth","name":"Stroke Width (test plug-in)","version":"1","kind":"effect",
        "description":"Draws a triangle while the object's stroke is 5 pt wide.","params":{}}"#;
    let src = tpl::when_given(manifest, tpl::Given::Input, r#""width":5.0"#, &json!({"objects": [{"path": triangle}]}).to_string());
    std::fs::write(std::path::Path::new(&dir).join("stroke-width-test-effect.wasm"), wat::parse_str(src).unwrap()).unwrap();
}

/// A filter is given its objects' effect records and their strokes' join, cap and miter limit, and
/// may return them. The records are admitted as effect.apply would admit them; one that isn't
/// refuses the whole output, and nothing changes.
#[test]
fn filters_set_effects_and_stroke_styles() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    install_wat(
        &mut s,
        &tpl::module(&tpl::manifest("test.engine.fx", "effect", r#"{"n":{"type":"int","min":0,"max":3,"default":1}}"#), tpl::ECHO, "", 1),
    );
    let black = json!({"type": "solid", "color": {"model": "gray", "k": 1.0}});
    let out = |fx: Value| json!({"objects": [{"id": id.0, "strokes": [{"paint": black, "width": 4, "join": "round", "cap": "square", "miterLimit": 4}], "effects": fx}]});
    let filter = |s: &mut Session, name: &str, fx: Value| {
        install_wat(s, &tpl::returning(&tpl::manifest(name, "filter", "{}"), &out(fx).to_string()));
        s.execute("plugin.run", &json!({"id": name, "ids": [id.0]}))
    };
    let fx = json!([{"id": "distort.roughen", "params": {"size": 3}}, {"id": "plugin.test.engine.fx", "params": {"n": 2}, "visible": false}]);
    filter(&mut s, "test.engine.setfx", fx).unwrap();
    let n = node(&s, id);
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.width, st.join, st.cap, st.miter_limit), (4.0, vectorcraft_doc::LineJoin::Round, vectorcraft_doc::LineCap::Square, 4.0));
    let effects = &n.appearance.effects;
    assert_eq!(effects.iter().map(|e| (e.id.as_str(), e.visible)).collect::<Vec<_>>(), [("distort.roughen", true), ("plugin.test.engine.fx", false)]);
    assert_eq!(effects[0].params, vectorcraft_render::effects::merged_params("distort.roughen", &json!({"size": 3})), "filled in");
    assert_eq!(effects[1].params, json!({"n": 2}));
    // Handed back unchanged (an echo), nothing changes, not even the records' parameters.
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.same", "filter", "{}"), tpl::ECHO, "", 1));
    let before = s.doc().unwrap().doc.clone();
    run(&mut s, "plugin.run", json!({"id": "test.engine.same", "ids": [id.0]}));
    assert_eq!(s.doc().unwrap().doc, before);
    // Refused: what effect.apply refuses, a live effect that isn't installed, parameters its
    // manifest refuses, a texture to read from a file, two 3D effects.
    for (name, fx, why) in [
        ("test.engine.unknown", json!([{"id": "distort.nothing"}]), "no such effect"),
        ("test.engine.key", json!([{"id": "distort.roughen", "params": {"bogus": 1}}]), "bogus"),
        ("test.engine.gone", json!([{"id": "plugin.test.engine.gone"}]), "no such effect"),
        ("test.engine.badn", json!([{"id": "plugin.test.engine.fx", "params": {"n": "two"}}]), "`n`"),
        ("test.engine.file", json!([{"id": "distort.glass", "params": {"textureImage": {"path": "/etc/hosts"}}}]), "textureImage"),
        ("test.engine.file2", json!([{"id": "texture.texturizer", "params": {"textureImage": {"path": "/etc/hosts"}}}]), "textureImage"),
        ("test.engine.file3", json!([{"id": "artistic.roughPastels", "params": {"textureImage": {"path": "/etc/hosts"}}}]), "textureImage"),
        ("test.engine.two3d", json!([{"id": "threeD.extrude"}, {"id": "threeD.extrude"}]), "one 3D effect"),
    ] {
        let e = filter(&mut s, name, fx).unwrap_err().to_string();
        assert!(e.contains("objects[0].effects") && e.contains(why), "{name}: {e}");
        assert_eq!(s.doc().unwrap().doc, before, "{name}");
    }
}

/// `CRAFT_PLUGIN_FRAME_WASM=<folder>` saves a test filter to try in the app
/// (`rough-frame-test-filter.wasm`): it adds a frame around 60,40–340,260 with a 6 pt stroke with
/// round joins and caps, roughened by the Roughen effect.
#[test]
fn rough_frame_test_filter() {
    let Some(dir) = std::env::var_os("CRAFT_PLUGIN_FRAME_WASM") else { return };
    let frame = PathData::single(SubPath::polyline(
        &[Point::new(60.0, 40.0), Point::new(340.0, 40.0), Point::new(340.0, 260.0), Point::new(60.0, 260.0)],
        true,
    ));
    let ink = json!({"type": "solid", "color": {"model": "rgb", "r": 0.15, "g": 0.2, "b": 0.45}});
    let out = json!({"objects": [{"name": "Frame", "path": frame, "fills": [],
        "strokes": [{"paint": ink, "width": 6, "join": "round", "cap": "round"}],
        "effects": [{"id": "distort.roughen", "params": {"size": 2, "detail": 8, "points": "smooth"}}]}]});
    let manifest = r#"{"id":"org.example.frame","name":"Rough Frame (test plug-in)","version":"1","kind":"filter",
        "description":"Adds a frame whose stroke is roughened by the Roughen effect.","params":{}}"#;
    let src = tpl::returning(manifest, &out.to_string());
    std::fs::write(std::path::Path::new(&dir).join("rough-frame-test-filter.wasm"), wat::parse_str(src).unwrap()).unwrap();
}

/// A filter keeps data with the document: it writes it with `documentData` in the same undo step as
/// its objects (`null` removes it) and reads what it last wrote in `_context.documentData`, which
/// no other filter is given. It is the document's foreign data, so the native file keeps it with
/// no section of its own. More than the limit is refused.
#[test]
fn filters_keep_their_data_with_the_document() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let pid = "test.engine.data";
    let returning = |out: Value| tpl::returning(&tpl::manifest(pid, "filter", "{}"), &out.to_string());
    let keep = |data: Value| json!({"objects": [{"id": id.0}], "documentData": data});
    let data = |s: &Session| s.doc().unwrap().doc.plugin_data(pid).cloned();
    let filter = |s: &mut Session, pid: &str| s.execute("plugin.run", &json!({"id": pid, "ids": [id.0]}));
    install_wat(&mut s, &returning(keep(json!({"pen": 2}))));
    filter(&mut s, pid).unwrap();
    assert_eq!(data(&s), Some(json!({"pen": 2})));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(data(&s), None, "one undo step with the objects");
    run(&mut s, "edit.redo", json!({}));
    // Read back: these name the object only when they are given what they look for.
    let named = |name: &str| json!({"objects": [{"id": id.0, "name": name}]}).to_string();
    let when_given =
        |pid: &str, needle: &str, name: &str| tpl::when_given(&tpl::manifest(pid, "filter", "{}"), tpl::Given::Params, needle, &named(name));
    install_wat(&mut s, &when_given(pid, r#""documentData":{"pen":2}"#, "Pen 2"));
    filter(&mut s, pid).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("Pen 2"));
    // Another filter is given nothing: what a filter keeps is its own.
    let other = "test.engine.other";
    install_wat(&mut s, &when_given(other, r#""documentData":null"#, "Nothing kept"));
    filter(&mut s, other).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("Nothing kept"));
    // Saved with the document, among its foreign data.
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(saved.plugin_data(pid), Some(&json!({"pen": 2})));
    assert_eq!(saved.unknown.get(vectorcraft_doc::plugin_data::PLUGIN_DATA), Some(&json!({pid: {"pen": 2}})));
    // Too much is refused, and the document stays as it was; null removes the data.
    let before = s.doc().unwrap().doc.clone();
    let big = "x".repeat(vectorcraft_plugins::objects::MAX_DOCUMENT_DATA);
    install_wat(&mut s, &returning(keep(json!(big))));
    let e = filter(&mut s, pid).unwrap_err().to_string();
    assert!(e.contains("documentData"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    install_wat(&mut s, &returning(keep(Value::Null)));
    filter(&mut s, pid).unwrap();
    assert_eq!(data(&s), None);
}

/// A filter run whose output changes nothing (an echo) records no undo step, leaves a saved
/// document unmodified and the document as it was, and still selects its objects. One that changes
/// one of the objects records exactly one step, and running it again records none.
#[test]
fn a_filter_run_that_changes_nothing_records_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#0000ff");
    let echo = "test.engine.noop.echo";
    install_wat(&mut s, &tpl::module(&tpl::manifest(echo, "filter", "{}"), tpl::ECHO, "", 1));
    run(&mut s, "select.none", json!({}));
    s.doc_mut().unwrap().mark_saved();
    let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let r = run(&mut s, "plugin.run", json!({"id": echo, "ids": [a.0, b.0]}));
    assert_eq!(r["ids"], json!([a.0, b.0]));
    let st = s.doc().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &doc), "the document stays as it was");
    assert_eq!((st.history.undo.len(), st.is_dirty()), (undo, false), "no undo step; still saved");
    assert_eq!(st.selection.objects, [a, b], "its objects are selected");
    // Renaming one object of the two is one undo step.
    let renamer = "test.engine.noop.renamer";
    let renamed = json!({"objects": [{"id": a.0, "name": "Renamed"}, {"id": b.0}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest(renamer, "filter", "{}"), &renamed.to_string()));
    run(&mut s, "plugin.run", json!({"id": renamer, "ids": [a.0, b.0]}));
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.is_dirty()), (undo + 1, true));
    assert_eq!(node(&s, a).name.as_deref(), Some("Renamed"));
    // Run again, it renames nothing: no second step.
    run(&mut s, "plugin.run", json!({"id": renamer, "ids": [a.0, b.0]}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    for id in [echo, renamer] {
        run(&mut s, "plugin.remove", json!({"id": id}));
    }
}

/// A filter that writes the data it already keeps with the document records no undo step; writing
/// other data, or removing it, records one, and removing data it does not keep records none.
#[test]
fn a_filter_that_keeps_the_same_data_records_no_undo_step() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let pid = "test.engine.noop.data";
    let keep = |s: &mut Session, data: Value| {
        let out = json!({"objects": [{"id": id.0}], "documentData": data});
        install_wat(s, &tpl::returning(&tpl::manifest(pid, "filter", "{}"), &out.to_string()));
        run(s, "plugin.run", json!({"id": pid, "ids": [id.0]}));
        s.doc().unwrap().history.undo.len()
    };
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(keep(&mut s, json!({"pen": 2})), undo + 1);
    assert_eq!(keep(&mut s, json!({"pen": 2})), undo + 1, "the same data: no step");
    assert_eq!(keep(&mut s, json!({"pen": 3})), undo + 2);
    assert_eq!(keep(&mut s, Value::Null), undo + 3, "removed");
    assert_eq!(keep(&mut s, Value::Null), undo + 3, "nothing kept to remove: no step");
    assert_eq!(s.doc().unwrap().doc.plugin_data(pid), None);
    run(&mut s, "plugin.remove", json!({"id": pid}));
}

/// A plug-in command whose run changes nothing records no undo step, and so does a filter or a
/// command that may call commands and hands back what it is given.
#[test]
fn a_plugin_command_that_changes_nothing_records_no_undo_step() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let undo = s.doc().unwrap().history.undo.len();
    let id = "test.engine.noop.cmds";
    install_wat(&mut s, &grower(id));
    run(&mut s, &format!("plugin.{id}.tidy"), json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "tidy hands back its input");
    run(&mut s, "plugin.remove", json!({"id": id}));
    let calling = "test.engine.noop.calling";
    install_echo_calling(&mut s, calling, r#"[{"id":"same","label":"Same"}]"#);
    run(&mut s, &format!("plugin.{calling}.same"), json!({}));
    run(&mut s, "plugin.run", json!({"id": calling}));
    assert!(!s.in_interaction());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "an echo that may call commands");
    run(&mut s, "plugin.remove", json!({"id": calling}));
}

#[test]
fn plugin_management_commands() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    install(&mut s, DESATURATE);
    let list = run(&mut s, "plugin.list", json!({}));
    assert!(list["plugins"].as_array().unwrap().iter().any(|p| p["id"] == DESATURATE_ID));
    let info = run(&mut s, "plugin.info", json!({"id": DESATURATE_ID}));
    assert_eq!(info["defaults"], json!({"amount": 100.0}));
    assert_eq!(info["params"], "{amount: 0..100 (100)}");
    assert_eq!(info["manifest"]["kind"], "filter");
    // Refused installs.
    for bad in [
        json!({}),
        json!({"dataBase64": "!!"}),
        json!({"dataBase64": "AGFzbQ=="}),
        json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE), "replace": false}),
    ] {
        assert!(s.execute("plugin.install", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("plugin.info", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.remove", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.reload", &json!({})).is_err(), "no folder set");

    // The Additional Plug-ins Folder preference installs what's in it.
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = tpl::returning(&tpl::manifest("test.engine.folder", "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(dir.join("folder.wasm"), wat::parse_str(src).unwrap()).unwrap();
    std::fs::write(dir.join("broken.wasm"), b"\0asm?").unwrap();
    run(&mut s, "prefs.set", json!({"key": "pluginsFolder", "value": dir.display().to_string()}));
    assert!(vectorcraft_plugins::registry::get("test.engine.folder").is_some());
    run(&mut s, "plugin.remove", json!({"id": "test.engine.folder"}));
    let r = run(&mut s, "plugin.reload", json!({}));
    assert_eq!(r["loaded"], json!(["test.engine.folder"]));
    assert_eq!(r["failed"].as_array().unwrap().len(), 1);
    assert_eq!(run(&mut s, "plugin.list", json!({}))["folder"]["loaded"], json!(["test.engine.folder"]));
    std::fs::remove_dir_all(&dir).ok();
}

/// Clearing the Additional Plug-ins Folder forgets the folder: setting the same folder again
/// installs what is in it, as setting any other folder does.
#[test]
fn plugins_folder_set_again_after_clearing_installs_its_plugins() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::tests_links::Folder::new("plugin-folder-cleared");
    let id = "test.engine.foldercleared";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(dir.0.join("cleared.wasm"), wat::parse_str(src).unwrap()).unwrap();
    let mut s = session();
    let folder = json!({"key": "pluginsFolder", "value": dir.0.display().to_string()});
    run(&mut s, "prefs.set", folder.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed from the folder");
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "prefs.set", json!({"key": "pluginsFolder", "value": ""}));
    run(&mut s, "prefs.set", folder);
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again when the folder is set again");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Under automation read roots, `plugin.list` names the plug-in folder last loaded, its failed
/// files and a plug-in's source file only inside them: a folder outside is `null`, and a file
/// outside is named by its file name.
#[test]
fn plugin_list_leaves_out_paths_outside_the_read_root() {
    use crate::file_access::{AutomationRoots, confine};
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let base = crate::tests_links::Folder::new("plugin-list-read-root");
    let (inside, outside) = (base.0.join("in"), base.0.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let id = "test.engine.listroots";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(outside.join("hidden.wasm"), wat::parse_str(src).unwrap()).unwrap();
    std::fs::write(outside.join("broken.wasm"), b"\0asm?").unwrap();
    let folder = outside.to_string_lossy().to_string();
    let mut s = session();
    let r = run(&mut s, "plugin.reload", json!({"path": folder}));
    assert_eq!(r["loaded"], json!([id]), "{r}");
    let broken = outside.join("broken.wasm").to_string_lossy().to_string();
    let source = outside.join("hidden.wasm").to_string_lossy().to_string();
    let entry = |list: &Value| list["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();

    // The person sees the paths.
    let person = run(&mut s, "plugin.list", json!({}));
    assert_eq!((&person["folder"]["folder"], &person["folder"]["failed"][0]["file"]), (&json!(folder), &json!(broken)), "{person}");
    assert_eq!(entry(&person)["source"], json!(source));

    // The client sees their names.
    let roots = AutomationRoots::new(&[&inside], &[]).unwrap();
    let listed = confine(roots.as_ref(), || s.execute("plugin.list", &json!({}))).unwrap();
    let report = &listed["folder"];
    assert_eq!((&report["folder"], &report["loaded"], &report["failed"][0]["file"]), (&Value::Null, &json!([id]), &json!("broken.wasm")), "{listed}");
    assert_eq!(entry(&listed)["source"], "hidden.wasm");
    let info = confine(roots.as_ref(), || s.execute("plugin.info", &json!({"id": id}))).unwrap();
    assert_eq!(info["source"], "hidden.wasm");
    let outside_text = json!(folder).to_string();
    for reply in [&listed, &info] {
        assert!(!reply.to_string().contains(outside_text.trim_matches('"')), "no path outside the root: {reply}");
    }

    // A read root holding the folder shows it whole.
    let wider = AutomationRoots::new(&[&outside], &[]).unwrap();
    let listed = confine(wider.as_ref(), || s.execute("plugin.list", &json!({}))).unwrap();
    assert_eq!(listed["folder"]["folder"], json!(folder), "{listed}");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A command a script runs loads a plug-in folder's files through the script's read gate also
/// without automation roots: a link in a folder the script may read, to a module outside it, is
/// refused whether or not its name is valid text.
#[cfg(unix)]
#[test]
fn a_script_loads_no_plugin_from_outside_its_folder_through_a_name_that_is_not_text() {
    use crate::file_access::{ScriptGate, ScriptScope, with_script_gate};
    use std::os::unix::ffi::OsStrExt;
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let base = crate::tests_links::Folder::new("plugin-script-name-not-text");
    let (inside, outside) = (base.0.join("in"), base.0.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let (text_id, bytes_id) = ("test.engine.script.textname", "test.engine.script.bytesname");
    for (id, file, link) in [(text_id, "text.wasm", &b"a.wasm"[..]), (bytes_id, "bytes.wasm", &b"b\xff.wasm"[..])] {
        let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
        std::fs::write(outside.join(file), wat::parse_str(src).unwrap()).unwrap();
        std::os::unix::fs::symlink(outside.join(file), inside.join(std::ffi::OsStr::from_bytes(link))).unwrap();
    }
    let gate = ScriptGate {
        read: ScriptScope::of_folder(&inside),
        write: ScriptScope::default(),
        documents: vec![],
        linked: Default::default(),
        also: None,
    };
    let mut s = session();
    let folder = inside.to_string_lossy().to_string();
    let r = with_script_gate(gate, || s.execute("plugin.reload", &json!({"path": folder})));
    let installed = [text_id, bytes_id].map(vectorcraft_plugins::registry::remove);
    assert_eq!(installed, [false, false], "a script loaded a module from outside its folder: {r:?}");
    let r = r.unwrap();
    assert_eq!(r["loaded"], json!([]), "{r}");
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(2), "{r}");
}

/// Under automation read roots, a plug-in turned off for the next launch is listed with a file
/// outside them named by its file name, and `plugin.setEnabled` turns it on by that name.
#[test]
fn disabled_plugins_name_files_outside_the_read_root_by_their_names() {
    if !in_own_process("disabled_plugins_name_files_outside_the_read_root_by_their_names") {
        return;
    }
    use crate::file_access::{AutomationRoots, confine};
    let base = crate::tests_links::Folder::new("plugin-disabled-read-root");
    let (inside, outside) = (base.0.join("in"), base.0.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let id = "test.engine.disabledroots";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    let file = outside.join("off.wasm");
    std::fs::write(&file, wat::parse_str(src).unwrap()).unwrap();
    let file = file.to_string_lossy().to_string();
    let mut s = session();
    run(&mut s, "plugin.install", json!({"path": file}));
    let roots = AutomationRoots::new(&[&inside], &[]).unwrap();
    // The client turns it off by the source plugin.list shows.
    let listed = confine(roots.as_ref(), || s.execute("plugin.list", &json!({}))).unwrap();
    let source = listed["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap()["source"].clone();
    assert_eq!(source, "off.wasm", "{listed}");
    confine(roots.as_ref(), || s.execute("plugin.setEnabled", &json!({"file": source, "enabled": false}))).unwrap();
    assert_eq!(run(&mut s, "plugin.safeMode", json!({}))["disabled"][0]["file"], json!(file), "the person sees the path");

    let outside_text = json!(outside.to_string_lossy()).to_string();
    for cmd in ["plugin.list", "plugin.safeMode"] {
        let r = confine(roots.as_ref(), || s.execute(cmd, &json!({}))).unwrap();
        assert_eq!(r["disabled"][0]["file"], "off.wasm", "{cmd}: {r}");
        assert!(!r.to_string().contains(outside_text.trim_matches('"')), "{cmd}: no path outside the root: {r}");
    }
    let r = confine(roots.as_ref(), || s.execute("plugin.setEnabled", &json!({"file": "off.wasm", "enabled": true}))).unwrap();
    assert_eq!((&r["changed"], &r["disabled"]), (&json!(true), &json!([])), "{r}");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// The files and folders in the plug-in store `dir`, without the file windows lock while they
/// change it.
fn store_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name() != crate::store::DISABLED_LOCK)).count()
}

/// #2541: a plug-in installed in a session with a plug-in store is installed again in the next
/// session that attaches the store, and Remove Plug-in takes it out of the store.
#[test]
fn installed_plugins_are_kept_in_the_store() {
    let folder = crate::tests_links::Folder::new("plugin-store");
    let id = "test.engine.stored";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_wat(&mut s, &src);
    assert_eq!(store_files(&folder.0), 1, "one file in the store");
    vectorcraft_plugins::registry::remove(id);

    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store");
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(store_files(&folder.0), 0, "removed from the store");

    // A file that isn't a plug-in is skipped; the store still attaches.
    std::fs::write(folder.0.join("broken.wasm"), b"\0asm?").unwrap();
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none());
}

/// Held by the tests that set [`crate::store::BEFORE_STORED_LOAD`].
static STORED_HOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A stored module's load waits on its worker thread until the test lets it go
/// ([`crate::store::BEFORE_STORED_LOAD`]). The hook is cleared when this is dropped.
struct HeldLoad {
    _lock: std::sync::MutexGuard<'static, ()>,
    entered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    go: std::sync::Arc<std::sync::atomic::AtomicBool>,
    gave_up: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl HeldLoad {
    /// Holds the load of the file at `path`. A load the test never lets go gives up after a
    /// million yields (so a test of code that waits for it ends) and says so in `gave_up`.
    fn new(path: std::path::PathBuf) -> HeldLoad {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let lock = STORED_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let (entered, go, gave_up) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (e, g, q) = (entered.clone(), go.clone(), gave_up.clone());
        *crate::store::BEFORE_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(move |p: &std::path::Path| {
            if p != path {
                return;
            }
            e.store(true, Ordering::SeqCst);
            if !(0..1_000_000).any(|_| {
                std::thread::yield_now();
                g.load(Ordering::SeqCst)
            }) {
                q.store(true, Ordering::SeqCst);
            }
        }));
        HeldLoad { _lock: lock, entered, go, gave_up }
    }

    fn entered(&self) -> bool {
        self.entered.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn release(&self) {
        self.go.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn gave_up(&self) -> bool {
        self.gave_up.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for HeldLoad {
    fn drop(&mut self) {
        self.release();
        *crate::store::BEFORE_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// A store holding the module of a filter `id` in `<id>.wasm`.
fn store_with(name: &str, id: &str) -> (crate::tests_links::Folder, std::path::PathBuf) {
    let folder = crate::tests_links::Folder::new(name);
    let path = folder.0.join(format!("{id}.wasm"));
    std::fs::create_dir_all(&folder.0).unwrap();
    std::fs::write(&path, wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    (folder, path)
}

/// With loads in the background, attaching a plug-in store returns while a stored module is
/// still loading: the plug-in installs once its load is done, at the next plug-in command.
#[test]
fn a_store_attached_with_background_loads_returns_before_its_plugins_load() {
    let id = "test.engine.background";
    let (folder, path) = store_with("plugin-background", id);
    let held = HeldLoad::new(path);
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    let returned_while_loading = !held.gave_up();
    while !held.entered() {
        std::thread::yield_now();
    }
    assert!(returned_while_loading, "attaching the store waited for the module to load");
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "nothing installs before its load is done");
    assert!(s.poll_plugin_loads(), "still loading");
    held.release();
    let listed = run(&mut s, "plugin.list", json!({}));
    assert!(listed.to_string().contains(id), "plugin.list waits for the load and lists it: {listed}");
    assert!(!s.poll_plugin_loads());
    assert!(vectorcraft_plugins::registry::remove(id));
}

/// Stored files that can't be opened take nothing from the bytes the stored plug-ins may read
/// together: a module after 512 MiB of them still installs.
#[cfg(unix)]
#[test]
fn unreadable_stored_files_leave_the_store_budget_to_the_others() {
    use std::os::unix::fs::PermissionsExt;
    let id = "test.engine.budget.after";
    let (folder, _) = store_with("plugin-budget-unreadable", id);
    let per_file = vectorcraft_plugins::Limits::default().max_module_bytes as u64;
    let locked: Vec<_> = (0..crate::store::MAX_PLUGIN_STORE.div_ceil(per_file))
        .map(|n| {
            // Sparse files sorted before the module, each as long as a module may be.
            let path = folder.0.join(format!("a{n:03}.wasm"));
            std::fs::File::create(&path).unwrap().set_len(per_file).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            path
        })
        .collect();
    // A process that may read any file (root) opens them all: nothing to check.
    if std::fs::File::open(&locked[0]).is_ok() {
        return;
    }
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let installed = vectorcraft_plugins::registry::get(id).is_some();
    for path in &locked {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(installed, "the files that couldn't be opened used up the store's bytes");
    assert!(vectorcraft_plugins::registry::remove(id));
}

/// A stored file whose read fails on the worker thread gives back what it took of the bytes the
/// stored plug-ins may read together, so a module after files that use them all up still installs.
#[test]
fn stored_files_whose_reads_fail_leave_the_store_budget_to_the_others() {
    struct Unhook {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for Unhook {
        fn drop(&mut self) {
            *crate::store::BEFORE_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    let id = "test.engine.budget.failed";
    let (folder, module) = store_with("plugin-budget-failed", id);
    let len = std::fs::metadata(&module).unwrap().len();
    // Files sorted before the module, as long as it, that use up the bytes this test's store may read.
    for n in 0..4 {
        std::fs::write(folder.0.join(format!("a{n:03}.wasm")), vec![0; len as usize]).unwrap();
    }
    crate::store::PLUGIN_STORE_BUDGET.with(|b| b.set(4 * len));
    let _unhook = Unhook { _lock: STORED_HOOK.lock().unwrap_or_else(|e| e.into_inner()) };
    // Each of those files goes away as its load starts, so its read fails.
    let dir = folder.0.clone();
    *crate::store::BEFORE_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(move |p: &std::path::Path| {
        if p.parent() == Some(dir.as_path()) && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('a')) {
            let _ = std::fs::remove_file(p);
        }
    }));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let installed = vectorcraft_plugins::registry::get(id).is_some();
    crate::store::PLUGIN_STORE_BUDGET.with(|b| b.set(crate::store::MAX_PLUGIN_STORE));
    assert!(installed, "the files whose reads failed used up the store's bytes");
    assert!(vectorcraft_plugins::registry::remove(id));
}

/// A stored file replaced after its module loaded, by one of the same length with the same modified
/// time, is skipped: its content no longer holds the module. That holds for a file modified moments
/// ago and for one modified an hour ago.
#[test]
fn a_stored_file_replaced_with_the_same_stamp_while_it_loaded_is_skipped() {
    struct Unhook {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for Unhook {
        fn drop(&mut self) {
            *crate::store::AFTER_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    let _unhook = Unhook { _lock: STORED_HOOK.lock().unwrap_or_else(|e| e.into_inner()) };
    let hour = std::time::Duration::from_secs(3600);
    // A modified time ahead of the clock counts as moments ago however long the test takes.
    for (n, modified) in [std::time::SystemTime::now() + hour, std::time::SystemTime::now() - hour].into_iter().enumerate() {
        let id = format!("test.engine.background.replaced{n}");
        let (folder, path) = store_with(&format!("plugin-background-replaced{n}"), &id);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
        let replaced = path.clone();
        // Set once the file holds other bytes with the old stamp, so the case reaches the check.
        let rewritten = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = rewritten.clone();
        *crate::store::AFTER_STORED_LOAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(move |p: &std::path::Path| {
            if p == replaced {
                let mut bytes = std::fs::read(p).unwrap();
                bytes.reverse();
                std::fs::write(p, &bytes).unwrap();
                std::fs::File::options().write(true).open(p).unwrap().set_modified(modified).unwrap();
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }));
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        let installed = vectorcraft_plugins::registry::remove(&id);
        assert!(rewritten.load(std::sync::atomic::Ordering::SeqCst), "the file was not replaced while it loaded ({n})");
        assert!(!installed, "the module installed from a file that no longer holds it ({n})");
    }
}

/// While a session loads the plug-ins folder in the background, another session's `plugin.list`
/// reports the folder as loading; once the load installs, it reports what loaded.
#[test]
fn another_session_lists_a_folder_still_loading_as_loading() {
    // Other tests' `prefs.set` replaces the folder last loaded, which the process keeps.
    if !in_own_process("another_session_lists_a_folder_still_loading_as_loading") {
        return;
    }
    let id = "test.engine.listedloading";
    let (folder, _) = store_with("plugin-listed-loading", id);
    let mut background = session();
    background.load_plugins_in_background(true);
    run(&mut background, "prefs.set", json!({"values": {"pluginsFolder": folder.0.display().to_string()}}));
    let mut other = session();
    let listed = run(&mut other, "plugin.list", json!({}));
    assert_eq!(listed["folder"]["loading"], true, "{listed}");
    background.finish_plugin_loads();
    let listed = run(&mut other, "plugin.list", json!({}));
    assert!(vectorcraft_plugins::registry::remove(id));
    assert_eq!(listed["folder"]["loaded"], json!([id]), "{listed}");
    assert!(listed["folder"].get("loading").is_none(), "{listed}");
}

/// A `plugin.reload` doesn't replace the record of a folder load that started after it: once that
/// load installs, `plugin.list` reports its folder.
#[test]
fn a_reload_keeps_the_record_of_a_folder_load_started_after_it() {
    // Other tests' `prefs.set` replaces the folder last loaded, which the process keeps.
    if !in_own_process("a_reload_keeps_the_record_of_a_folder_load_started_after_it") {
        return;
    }
    let (reloaded, later) = ("test.engine.reloadorder.reloaded", "test.engine.reloadorder.later");
    let (reload_folder, _) = store_with("plugin-reload-order-reloaded", reloaded);
    let (later_folder, _) = store_with("plugin-reload-order-later", later);
    let other = std::rc::Rc::new(std::cell::RefCell::new(None));
    let started = other.clone();
    let path = later_folder.0.display().to_string();
    // Another session sets the plug-ins folder while the reload loads.
    crate::cmd::plugin::BEFORE_RELOAD_INSTALLS.set(Some(Box::new(move || {
        let mut background = session();
        background.load_plugins_in_background(true);
        run(&mut background, "prefs.set", json!({"values": {"pluginsFolder": path}}));
        *started.borrow_mut() = Some(background);
    })));
    let mut s = session();
    run(&mut s, "plugin.reload", json!({"path": reload_folder.0.display().to_string()}));
    let mut background = other.borrow_mut().take().expect("the other session's load started while the reload loaded");
    background.finish_plugin_loads();
    let listed = run(&mut s, "plugin.list", json!({}));
    assert!(vectorcraft_plugins::registry::remove(reloaded) && vectorcraft_plugins::registry::remove(later));
    assert_eq!(listed["folder"]["loaded"], json!([later]), "{listed}");
}

/// The plug-ins folder already loaded isn't loaded again when the preferences are applied, unless
/// a file in it found no loader thread to load it: then it is.
#[test]
fn a_folder_in_which_a_file_found_no_thread_is_loaded_again() {
    if !in_own_process("a_folder_in_which_a_file_found_no_thread_is_loaded_again") {
        return;
    }
    let id = "test.engine.nothread";
    let (folder, _) = store_with("plugin-no-thread", id);
    let mut s = session();
    run(&mut s, "prefs.set", json!({"values": {"pluginsFolder": folder.0.display().to_string()}}));
    assert!(vectorcraft_plugins::registry::remove(id), "installed from the folder");
    crate::cmd::plugin::sync_prefs(&mut s);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "a folder already loaded was loaded again");
    crate::cmd::plugin::folder_found_no_thread();
    crate::cmd::plugin::sync_prefs(&mut s);
    assert!(vectorcraft_plugins::registry::remove(id), "the folder wasn't loaded again");
}

/// A folder listed while the stored plug-ins still load, in the same session, replaces a stored
/// plug-in with a module of the same id once it loads, as it does when the loads run one after the
/// other: what an earlier load of the session installed doesn't count as a change made elsewhere.
#[test]
fn a_folder_queued_behind_the_stored_plugins_replaces_one_with_its_id() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let id = "test.engine.queued.same";
    let (store, stored) = store_with("plugin-queued-store", id);
    let (folder, from_folder) = store_with("plugin-queued-folder", id);
    let held = HeldLoad::new(stored);
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(store.0.clone());
    while !held.entered() {
        std::thread::yield_now();
    }
    s.prefs.plugins_folder = folder.0.display().to_string();
    crate::cmd::plugin::sync_prefs(&mut s);
    held.release();
    s.finish_plugin_loads();
    let source = vectorcraft_plugins::registry::get(id).and_then(|p| p.source().map(str::to_string));
    assert!(vectorcraft_plugins::registry::remove(id));
    assert_eq!(source.as_deref(), Some(from_folder.display().to_string().as_str()), "the folder's module replaces the stored one");
}

/// A query that only reads runs while the stored plug-ins load in the background, and doesn't
/// wait for them; a plug-in command does.
#[test]
fn a_query_runs_while_the_plugins_load() {
    let id = "test.engine.background.query";
    let (folder, path) = store_with("plugin-background-query", id);
    let held = HeldLoad::new(path);
    let mut s = session();
    run(&mut s, "file.new", json!({}));
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    while !held.entered() {
        std::thread::yield_now();
    }
    run(&mut s, "document.info", json!({}));
    let waited = held.gave_up() || !s.plugins_loading();
    held.release();
    assert!(!waited, "document.info waited for the plug-ins to load");
    assert!(crate::waits_for_plugin_loads("plugin.list") && !crate::waits_for_plugin_loads("document.info"));
    s.finish_plugin_loads();
    assert!(vectorcraft_plugins::registry::remove(id));
}

/// The stored plug-ins load without holding the lock that `plugin.install` and `plugin.remove`
/// take, so another session's plug-in changes don't wait for them.
#[test]
fn stored_plugins_load_without_holding_the_plugin_changes_lock() {
    let id = "test.engine.unlocked";
    let (folder, path) = store_with("plugin-unlocked", id);
    let held = HeldLoad::new(path);
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    while !held.entered() && !held.gave_up() {
        std::thread::yield_now();
    }
    // Other tests take the lock for a moment; the load holding it would hold it throughout (a
    // tenth of the yields the held load waits before it gives up).
    let free = (0..100_000).any(|_| {
        std::thread::yield_now();
        crate::cmd::plugin::plugin_changes_free()
    });
    // A load that gave up waiting no longer holds anything, so a free lock then shows nothing.
    let gave_up = held.gave_up();
    held.release();
    s.finish_plugin_loads();
    assert!(!gave_up, "the held load gave up before the lock was looked at");
    assert!(free, "the lock was held while the module loaded");
    assert!(vectorcraft_plugins::registry::remove(id));
}

/// Loads canceled as the app quits still end for what they installed: a stored module listing no
/// permissions that installed before a later one finished loading has its id's grant forgotten,
/// in the process and in `grants.json`, and the module still loading doesn't install.
#[test]
fn canceled_loads_forget_the_grants_of_what_they_installed() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let (first, later) = ("test.engine.cancel.a", "test.engine.cancel.b");
    let (folder, _) = store_with("plugin-cancel", first);
    let held_path = folder.0.join(format!("{later}.wasm"));
    std::fs::write(&held_path, wat::parse_str(tpl::returning(&tpl::manifest(later, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    let empty = json!({"objects": []});
    let approved = asking(first, r#"["doc.read"]"#, &empty);
    let grants = folder.0.join("grants.json");
    let entry = json!([{"id": first, "hash": sha256_hex(&approved), "permissions": ["doc.read"]}]);
    std::fs::write(&grants, serde_json::to_vec(&entry).unwrap()).unwrap();
    let held = HeldLoad::new(held_path);
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    while vectorcraft_plugins::registry::get(first).is_none() && !held.gave_up() {
        s.poll_plugin_loads();
        std::thread::yield_now();
    }
    assert!(vectorcraft_plugins::registry::get(first).is_some(), "the first module installed while the second loads");
    s.cancel_plugin_loads();
    // The file goes once it holds no grant.
    let listed: Vec<Value> = std::fs::read(&grants).map(|b| serde_json::from_slice(&b).unwrap()).unwrap_or_default();
    assert!(listed.iter().all(|e| e["id"] != first), "{listed:?}");
    assert!(vectorcraft_plugins::registry::granted(first).is_none());
    assert!(vectorcraft_plugins::registry::get(later).is_none(), "the module still loading doesn't install");
    assert!(!s.plugins_loading());
    drop(held);
    assert!(vectorcraft_plugins::registry::remove(first));
}

/// The grants for a stored module that lists no permissions stop counting when it installs, in the
/// same turn, while a later module of the load is still loading.
#[test]
fn a_stored_module_without_permissions_ends_its_grant_when_it_installs() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let (first, later) = ("test.engine.retire.a", "test.engine.retire.b");
    let (folder, _) = store_with("plugin-retire", first);
    let held_path = folder.0.join(format!("{later}.wasm"));
    std::fs::write(&held_path, wat::parse_str(tpl::returning(&tpl::manifest(later, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    let empty = json!({"objects": []});
    let approved = asking(first, r#"["doc.read"]"#, &empty);
    let grants = folder.0.join("grants.json");
    let entry = json!([{"id": first, "hash": sha256_hex(&approved), "permissions": ["doc.read"]}]);
    std::fs::write(&grants, serde_json::to_vec(&entry).unwrap()).unwrap();
    let held = HeldLoad::new(held_path);
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    while vectorcraft_plugins::registry::get(first).is_none() && !held.gave_up() {
        s.poll_plugin_loads();
        std::thread::yield_now();
    }
    assert!(vectorcraft_plugins::registry::get(first).is_some(), "the first module installed while the second loads");
    assert!(s.plugins_loading(), "the second module is still loading");
    let listed: Vec<Value> = std::fs::read(&grants).map(|b| serde_json::from_slice(&b).unwrap()).unwrap_or_default();
    assert!(listed.iter().all(|e| e["id"] != first), "{listed:?}");
    assert!(vectorcraft_plugins::registry::granted(first).is_none());
    drop(held);
    s.finish_plugin_loads();
    assert!(vectorcraft_plugins::registry::remove(first));
    assert!(vectorcraft_plugins::registry::remove(later));
}

/// A grant the load removed for a stored module without permissions doesn't go back for a later
/// file with the same id when another window or process granted the id since: that grant stays, and
/// the later module, which it doesn't cover, is skipped.
#[test]
fn a_grant_given_elsewhere_since_the_load_removed_one_is_kept() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let id = "test.engine.retire.kept";
    let folder = crate::tests_links::Folder::new("plugin-retire-kept");
    std::fs::create_dir_all(&folder.0).unwrap();
    let empty = json!({"objects": []});
    let (first, later) = (folder.0.join("a.wasm"), folder.0.join("b.wasm"));
    std::fs::write(&first, wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    std::fs::write(&later, &approved).unwrap();
    let grants = folder.0.join("grants.json");
    let entry =
        |hash: String| crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": hash, "permissions": ["doc.read"]}]).to_string());
    std::fs::write(&grants, entry(sha256_hex(&approved))).unwrap();
    let held = HeldLoad::new(later.clone());
    let mut s = session();
    s.load_plugins_in_background(true);
    s.attach_plugin_store(folder.0.clone());
    while vectorcraft_plugins::registry::get(id).is_none() && !held.gave_up() {
        s.poll_plugin_loads();
        std::thread::yield_now();
    }
    // Another process grants the id for another module while the later file loads.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [{"path": triangle()}]}));
    std::fs::write(&grants, entry(sha256_hex(&other))).unwrap();
    drop(held);
    s.finish_plugin_loads();
    let source = vectorcraft_plugins::registry::get(id).and_then(|p| p.source().map(str::to_string));
    assert!(vectorcraft_plugins::registry::remove(id));
    let listed = crate::store::grant_entries(&std::fs::read(&grants).unwrap());
    assert_eq!(source.as_deref(), Some(first.display().to_string().as_str()), "the later module installed");
    assert_eq!(listed.iter().map(|e| e["hash"].clone()).collect::<Vec<_>>(), [json!(sha256_hex(&other))], "{listed:?}");
}

/// A grant the load removed for a stored module without permissions that can't be written back for
/// a later file with the same id leaves that file's module uninstalled: the store doesn't grant it.
#[test]
fn a_grant_that_cannot_be_put_back_leaves_the_later_module_uninstalled() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let id = "test.engine.retire.unwritten";
    let folder = crate::tests_links::Folder::new("plugin-retire-unwritten");
    std::fs::create_dir_all(&folder.0).unwrap();
    let empty = json!({"objects": []});
    let (first, later) = (folder.0.join("a.wasm"), folder.0.join("b.wasm"));
    std::fs::write(&first, wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap()).unwrap();
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    std::fs::write(&later, &approved).unwrap();
    let grants = folder.0.join("grants.json");
    std::fs::write(
        &grants,
        crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]}]).to_string()),
    )
    .unwrap();
    // The first write removes the grant; the second, which puts it back, finds a folder where the
    // file goes and fails.
    let blocked = grants.clone();
    crate::store::AFTER_GRANTS_READ.set(Some(Box::new(move || {
        crate::store::AFTER_GRANTS_READ.set(Some(Box::new(move || {
            let _ = std::fs::remove_file(&blocked);
            std::fs::create_dir_all(&blocked).unwrap();
        })));
    })));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    crate::store::AFTER_GRANTS_READ.set(None);
    let source = vectorcraft_plugins::registry::get(id).and_then(|p| p.source().map(str::to_string));
    let _ = std::fs::remove_dir(&grants);
    assert!(vectorcraft_plugins::registry::remove(id));
    assert_eq!(source.as_deref(), Some(first.display().to_string().as_str()), "the later module installed without its grant in the store");
}

/// A grant the load put back for a later stored file with the same id is removed again when that
/// file's module doesn't install because the id was installed elsewhere meanwhile: the store doesn't
/// grant bytes that weren't installed.
#[test]
fn a_grant_put_back_for_a_module_that_did_not_install_is_removed_again() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let id = "test.engine.retire.superseded";
    let folder = crate::tests_links::Folder::new("plugin-retire-superseded");
    std::fs::create_dir_all(&folder.0).unwrap();
    let empty = json!({"objects": []});
    let (first, later) = (folder.0.join("a.wasm"), folder.0.join("b.wasm"));
    let plain = wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#)).unwrap();
    std::fs::write(&first, &plain).unwrap();
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    std::fs::write(&later, &approved).unwrap();
    let grants = folder.0.join("grants.json");
    std::fs::write(
        &grants,
        crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]}]).to_string()),
    )
    .unwrap();
    // The first write removes the grant; while the second puts it back, the id is installed
    // elsewhere.
    crate::store::AFTER_GRANTS_READ.set(Some(Box::new(move || {
        crate::store::AFTER_GRANTS_READ.set(Some(Box::new(move || {
            vectorcraft_plugins::registry::install_bytes(&plain).unwrap();
        })));
    })));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    crate::store::AFTER_GRANTS_READ.set(None);
    let source = vectorcraft_plugins::registry::get(id).and_then(|p| p.source().map(str::to_string));
    assert!(vectorcraft_plugins::registry::remove(id));
    // A grants file that went away holds no grant; one still there must read as a list.
    let listed: Vec<Value> = match std::fs::read(&grants) {
        Ok(b) => match serde_json::from_slice::<Value>(&b).unwrap() {
            Value::Array(entries) => entries,
            Value::Object(mut file) => match file.remove("grants") {
                Some(Value::Array(entries)) => entries,
                other => panic!("the grants file holds no list: {other:?}"),
            },
            other => panic!("the grants file holds no list: {other:?}"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => panic!("the grants file could not be read: {e}"),
    };
    assert_eq!(source, None, "the later module replaced the one installed elsewhere");
    assert!(listed.iter().all(|e| e["id"] != id), "the grant put back stayed in the store: {listed:?}");
}

/// A session that waits for its loads and sets the plug-ins folder another session is still
/// loading in the background loads the folder itself: its plug-in is installed when `prefs.set`
/// returns, and the other session's load, when it installs, counts the same module as loaded.
#[test]
fn a_folder_another_session_is_loading_is_loaded_by_a_session_that_waits() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let id = "test.engine.sharedfolder";
    let (folder, _) = store_with("plugin-shared-folder", id);
    let path = folder.0.display().to_string();
    let mut background = session();
    background.load_plugins_in_background(true);
    run(&mut background, "prefs.set", json!({"values": {"pluginsFolder": path}}));
    assert!(background.plugins_loading(), "the folder loads in the background");
    let mut waits = session();
    run(&mut waits, "prefs.set", json!({"values": {"pluginsFolder": path}}));
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "prefs.set returned before the folder's plug-in installed");
    let report = run(&mut background, "plugin.list", json!({}));
    assert!(!background.plugins_loading());
    assert!(vectorcraft_plugins::registry::remove(id), "{report}");
}

/// A heavy test filter `id` that counts `steps` down, then adds a triangle named "Plug-in".
fn heavy(id: &str, steps: u64) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","budget":"heavy"}}"#);
    let output = json!({"objects": [{"path": triangle(), "name": "Plug-in"}]});
    wat::parse_str(tpl::spinning(&manifest, steps, &output.to_string())).unwrap()
}

/// A filter that asks for a heavy budget installs only once the user agrees (`allowHeavy`);
/// `plugin.inspect` tells whoever installs it so beforehand, and installs nothing.
#[test]
fn a_heavy_filter_installs_once_the_user_agrees() {
    let mut s = session();
    let id = "test.engine.heavy";
    let data = vectorcraft_format::base64_encode(&heavy(id, 1));
    // With the file's name, as the app passes it.
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data, "name": "route.wasm"}));
    assert_eq!((inspected["id"].as_str(), inspected["budget"].as_str()), (Some(id), Some("heavy")));
    assert_eq!(inspected["source"], "route.wasm");
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "inspecting installs nothing");
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("heavy budget") && e.contains("5 minutes") && e.contains("allowHeavy"), "{e}");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    let info = run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true}));
    assert_eq!(info["budget"], "heavy");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().limits().fuel_base, vectorcraft_plugins::HEAVY_FUEL);
    let listed = run(&mut s, "plugin.list", json!({}));
    assert!(listed["plugins"].as_array().unwrap().iter().any(|p| p["id"] == id && p["budget"] == "heavy"));
    // A light plug-in's entry says nothing of budgets.
    assert!(install(&mut s, DESATURATE).get("budget").is_none());
}

/// A module that carries its manifest in a `vc_manifest` custom section is inspected and installed
/// without any of its code running: its start function traps and its `vc_manifest` export loops.
#[test]
fn a_plugin_with_a_manifest_section_installs_without_running() {
    let mut s = session();
    let id = "test.engine.section";
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","abi":1}}"#);
    let extra = r#"(func (export "vc_manifest") (result i64) (loop $l (br $l)) (i64.const 0)) (func $trap unreachable) (start $trap)"#;
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::sectioned(&manifest, tpl::ECHO, extra, 1)).unwrap());
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data}));
    assert_eq!(inspected["id"], id);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "inspecting installs nothing");
    assert_eq!(run(&mut s, "plugin.install", json!({"dataBase64": data}))["id"], id);
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    run(&mut s, "plugin.remove", json!({"id": id}));
}

#[test]
fn invalid_install_flags_are_refused() {
    let mut s = session();
    let id = "test.engine.heavy.flags";
    let data = vectorcraft_format::base64_encode(&heavy(id, 1));
    for allow in [json!("true"), json!(1)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": allow})).unwrap_err().to_string();
        assert!(e.contains("`allowHeavy` must be true or false"), "allowHeavy {allow}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with allowHeavy {allow}");
    }
    // Null, as for every command, is the same as leaving the flag out.
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": null})).unwrap_err().to_string();
    assert!(e.contains("heavy budget"), "{e}");
    run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true}));
    for replace in [json!("false"), json!(0)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": true, "replace": replace})).unwrap_err().to_string();
        assert!(e.contains("`replace` must be true or false"), "replace {replace}: {e}");
    }
    run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true, "replace": null}));
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    // A plug-in with the usual budget is refused too.
    let light = "test.engine.light.flags";
    let manifest = format!(r#"{{"id":"{light}","name":"Test {light}","version":"1","kind":"filter"}}"#);
    let output = json!({"objects": [{"path": triangle(), "name": "Plug-in"}]});
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::spinning(&manifest, 1, &output.to_string())).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": "false"})).unwrap_err().to_string();
    assert!(e.contains("`allowHeavy` must be true or false"), "{e}");
    assert!(vectorcraft_plugins::registry::get(light).is_none());
}

/// A test filter `id` (named "Test `id`") whose manifest lists `permissions` (a JSON list) and that
/// returns `output`.
fn asking(id: &str, permissions: &str, output: &Value) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"2.1","kind":"filter","permissions":{permissions}}}"#);
    wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap()
}

/// A plug-in whose manifest lists permissions installs only once they are all granted (`grant`),
/// which `plugin.inspect` tells beforehand (`needsApproval`) with the permissions and the module's
/// hash. The grant is recorded with that hash and covers only that module: replacing the plug-in
/// (`replace: true`) with one with other bytes needs a new grant. Removing it forgets the grant. A
/// plug-in without `permissions` installs as before, with the ones its kind implies.
#[test]
fn a_plugin_that_lists_permissions_installs_once_they_are_granted() {
    let mut s = session();
    let id = "test.engine.permissions.install";
    let empty = json!({"objects": []});
    let data = |perms: &str| vectorcraft_format::base64_encode(&asking(id, perms, &empty));
    let reads = data(r#"["doc.read"]"#);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": reads}));
    assert_eq!((&inspected["needsApproval"], &inspected["permissions"]), (&json!(true), &json!(["doc.read"])));
    let hash = vectorcraft_plugins::permissions::sha256_hex(&asking(id, r#"["doc.read"]"#, &empty));
    assert_eq!(inspected["hash"], hash);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "inspecting installs nothing");
    // Refused without a grant that holds every permission it lists, naming them.
    for (grant, says) in [
        (Value::Null, "asks for the permissions doc.read"),
        (json!([]), "asks for the permissions doc.read"),
        (json!(["doc.write"]), "asks for the permissions doc.read"),
        (json!(["doc.read", "net.fetch"]), "unknown permission `net.fetch`"),
        (json!("doc.read"), "`grant` must be a list"),
        (json!([1]), "`grant` must be a list"),
    ] {
        let e = s.execute("plugin.install", &json!({"dataBase64": reads, "grant": grant})).unwrap_err().to_string();
        assert!(e.contains(says), "grant {grant}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with grant {grant}");
    }
    // `hash` pins the module the user was shown.
    for wrong in [json!("0".repeat(64)), json!(1)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": reads, "grant": ["doc.read"], "hash": wrong})).unwrap_err().to_string();
        assert!(e.contains("SHA-256 `hash`"), "hash {wrong}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with hash {wrong}");
    }
    let installed = run(&mut s, "plugin.install", json!({"dataBase64": reads, "grant": ["doc.read"], "hash": hash}));
    assert_eq!(installed["permissions"], json!(["doc.read"]));
    let g = vectorcraft_plugins::registry::granted(id).unwrap();
    assert_eq!((g.hash.as_str(), g.permissions.as_slice()), (hash.as_str(), &[vectorcraft_plugins::permissions::Permission::DocRead][..]));
    // Replaced with another module that lists the same permissions: granted again, the grant now
    // records the new module's hash.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&other)}))["needsApproval"], true);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other), "grant": ["doc.read"], "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, vectorcraft_plugins::permissions::sha256_hex(&other));
    // One that asks for more is refused until it is granted all of them.
    let more = data(r#"["doc.read","doc.write"]"#);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": more}))["needsApproval"], true);
    let e = s.execute("plugin.install", &json!({"dataBase64": more, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("doc.read, doc.write") && e.contains("`grant`"), "{e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().manifest().permissions().len(), 1, "the one installed stays");
    run(&mut s, "plugin.install", json!({"dataBase64": more, "grant": ["doc.write", "doc.read", "state"], "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().permissions.len(), 2, "only what it lists is recorded");
    // Removed, it is asked again.
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsApproval"], true);
    // A plug-in without `permissions` installs as before, with its kind's.
    let plain = install(&mut s, DESATURATE);
    assert_eq!(plain["permissions"], json!(["doc.read", "doc.write", "state"]));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE)}))["needsApproval"], false);
}

/// A module with other bytes under an id that was granted is not covered, even when it lists the
/// same permissions: `plugin.inspect` says so, `plugin.install` refuses it and leaves the installed
/// module and its grant alone, and a new `grant` installs it and records its hash. The same bytes
/// install again without asking, and a module that lists no permissions never asks.
#[test]
fn a_grant_covers_only_the_module_that_was_approved() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let mut s = session();
    let id = "test.engine.permissions.bytes";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let (first_data, changed_data) = (vectorcraft_format::base64_encode(&first), vectorcraft_format::base64_encode(&changed));
    let (first_hash, changed_hash) = (sha256_hex(&first), sha256_hex(&changed));
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": changed_data}))["needsApproval"], true);
    let e = s.execute("plugin.install", &json!({"dataBase64": changed_data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("doc.read") && e.contains("`grant`"), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": changed_data, "replace": true, "hash": changed_hash})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "`hash` is not an approval: {e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), first_hash, "the installed module stays");
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, first_hash, "its grant stays");
    // The same bytes need no new grant.
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": first_data}))["needsApproval"], false);
    run(&mut s, "plugin.install", json!({"dataBase64": first_data}));
    // A new grant installs the changed module, and the old bytes are no longer covered.
    run(&mut s, "plugin.install", json!({"dataBase64": changed_data, "grant": ["doc.read"], "hash": changed_hash, "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), changed_hash);
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, changed_hash);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": first_data}))["needsApproval"], true);
    run(&mut s, "plugin.remove", json!({"id": id}));

    // A manifest without a list, or with an empty one, asks for nothing, whatever its bytes.
    let exempt = "test.engine.permissions.exempt";
    for (permissions, out) in [("null", json!({"objects": []})), ("null", json!({"objects": [], "error": null})), ("[]", json!({"objects": []}))] {
        let data = vectorcraft_format::base64_encode(&asking(exempt, permissions, &out));
        assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": data}))["needsApproval"], false, "{permissions}");
        run(&mut s, "plugin.install", json!({"dataBase64": data, "replace": true}));
    }
    run(&mut s, "plugin.remove", json!({"id": exempt}));
}

/// Installing a module that lists no permissions under a granted id forgets the grant, so the
/// formerly approved bytes need a new approval. A refused install leaves the grant, and a module
/// that lists permissions keeps replacing it as before.
#[test]
fn a_module_without_permissions_ends_the_grant_of_its_id() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let mut s = session();
    let id = "test.engine.permissions.replaced";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let granted = || vectorcraft_plugins::registry::granted(id).map(|g| g.hash);
    for permissions in ["null", "[]"] {
        let plain = vectorcraft_format::base64_encode(&asking(id, permissions, &empty));
        run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
        // Refused installs change nothing.
        let e = s.execute("plugin.install", &json!({"dataBase64": plain, "hash": "0".repeat(64), "replace": true})).unwrap_err().to_string();
        assert!(e.contains("SHA-256 `hash`"), "{e}");
        let e = s.execute("plugin.install", &json!({"dataBase64": plain, "replace": false})).unwrap_err().to_string();
        assert!(e.contains("already installed"), "{e}");
        let e = s.execute("plugin.install", &json!({"dataBase64": plain})).unwrap_err().to_string();
        assert!(e.contains("`replace: true`"), "{e}");
        assert_eq!(granted(), Some(sha256_hex(&approved)), "{permissions}: a refused install leaves the grant");

        run(&mut s, "plugin.install", json!({"dataBase64": plain, "replace": true}));
        assert_eq!(granted(), None, "{permissions}");
        assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true, "{permissions}");
        let e = s.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
        assert!(e.contains("`grant`"), "{permissions}: {e}");
        assert!(!vectorcraft_plugins::registry::get(id).unwrap().manifest().asks_permissions(), "{permissions}: the module installed stays");
    }
    // With no module installed under the id, the grant ends the same way.
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    vectorcraft_plugins::registry::remove(id);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "null", &empty))}));
    assert_eq!(granted(), None);
    // A module that lists permissions records its own grant.
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    assert_eq!(granted(), Some(sha256_hex(&approved)));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// In a plug-in store, a module that lists no permissions removes only its own id's entry from
/// `grants.json`, and the formerly approved bytes need a new approval after a restart. A store
/// that holds such a module and a grant for its id has that grant forgotten when it is attached.
#[test]
fn a_module_without_permissions_ends_the_grant_kept_in_the_store() {
    use vectorcraft_plugins::permissions::{Grant, GrantToken, Permission, sha256_hex};
    let folder = crate::tests_links::Folder::new("plugin-store-stale-grant");
    let id = "test.engine.permissions.storedstale";
    let other = "test.engine.permissions.storedother";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let other_wasm = asking(other, r#"["doc.read"]"#, &empty);
    let grants = folder.0.join("grants.json");
    let ids = || -> Vec<String> {
        crate::store::grant_entries(&std::fs::read(&grants).unwrap()).iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
    };
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other_wasm), "grant": ["doc.read"]}));
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    assert_eq!(ids().len(), 2);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(ids(), vec![other.to_string()], "only this id's entry goes");
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert!(vectorcraft_plugins::registry::granted(other).is_some(), "the other plug-in's grant stays");

    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut next, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);

    // A grant the store already holds for an id whose stored module lists no permissions.
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let other_entry = json!({"id": other, "hash": sha256_hex(&other_wasm), "permissions": ["doc.read"]});
    std::fs::write(&grants, crate::store::signed_grants_file(&folder.0, &json!([entry, other_entry]).to_string())).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the module that lists nothing still loads");
    assert_eq!(ids(), vec![other.to_string()]);
    assert!(vectorcraft_plugins::registry::get(other).is_some());

    // So does an entry this version can't read, and a grant held only by the process.
    let future = json!({"id": id, "hash": "00", "permissions": ["doc.read", "ui.panel"]});
    std::fs::write(&grants, crate::store::signed_grants_file(&folder.0, &json!([future, other_entry]).to_string())).unwrap();
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::grant(GrantToken::after_engine_approval(Grant {
        id: id.into(),
        hash: sha256_hex(&approved),
        permissions: vec![Permission::DocRead],
    }));
    let mut fourth = session();
    fourth.attach_plugin_store(folder.0.clone());
    assert_eq!(ids(), vec![other.to_string()]);
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    run(&mut fourth, "plugin.remove", json!({"id": id}));
    run(&mut fourth, "plugin.remove", json!({"id": other}));
}

/// A module listing no permissions that can't be written to the plug-in store leaves the stored
/// module in place, so `grants.json` keeps the grant that module needs at the next launch.
#[test]
fn a_failed_store_write_of_a_module_without_permissions_keeps_the_stored_grant() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-write-plain");
    let id = "test.engine.permissions.storefail";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let before = std::fs::read(&grants).unwrap();
    let file = s.stored_plugin_file(id).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(std::fs::read(&grants).unwrap(), before, "the grant stays with the module left in the store");
    assert!(vectorcraft_plugins::registry::granted(id).is_none(), "the process runs the module that lists nothing");

    std::fs::remove_dir(&file).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module listing no permissions whose id's grant can't be removed from `grants.json` doesn't stay
/// in the store, so the grant and the module it covers stay together. It does stay when the module
/// before it can't be read back, and when the grants file is too large or doesn't hold a list and so
/// approves nothing.
#[test]
fn a_grant_that_cannot_be_removed_keeps_the_stored_module() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-forget");
    let id = "test.engine.permissions.forgetfail";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let replace = json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true});
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    // An entry this version can't use is written back as it was read: `1e9` as `1000000000.0`, which
    // makes the rewritten file larger than the 1 MiB that is read.
    let entry = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();

    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert!(vectorcraft_plugins::registry::granted(id).is_none(), "the process runs the module that lists nothing");

    let oversized = vectorcraft_plugins::Limits::default().max_module_bytes as u64 + 1;
    std::fs::File::options().write(true).open(&file).unwrap().set_len(oversized).unwrap();
    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a module that can't be read back can't be put back");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    let inspect = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    assert_eq!(run(&mut s, "plugin.inspect", inspect)["needsApproval"], true, "the grant left behind stops counting");

    std::fs::write(&file, &approved).unwrap();
    std::fs::write(&grants, b"not a list").unwrap();
    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a grants file that isn't a list approves nothing");

    std::fs::write(&file, &approved).unwrap();
    std::fs::write(&grants, vec![b' '; (1 << 20) + 1]).unwrap();
    run(&mut s, "plugin.install", replace);
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a grants file over its size limit approves nothing");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant that can't be removed stops counting for the session when the module it covers can't be
/// written back either: the store holds the new module, and the old bytes would otherwise install
/// without approval.
#[test]
fn a_grant_left_behind_when_its_module_cannot_go_back_stops_counting() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back");
    let id = "test.engine.permissions.failedputback";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    crate::store::PUT_BACK_FAILS.set(false);
    assert_eq!(std::fs::read(&file).unwrap(), plain, "the store keeps the new module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true, "the grant left behind stops counting");
    let refused = s.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// An entry of `grants.json` that this version can't use is kept for a version that can, so when it
/// can't be removed, neither `plugin.remove` nor a replacing module gets past it.
#[test]
fn an_unusable_entry_that_cannot_be_removed_stops_plugin_remove() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-remove-other");
    let id = "test.engine.permissions.removeother";
    let empty = json!({"objects": []});
    let first = asking(id, "null", &empty);
    let second = asking(id, "[]", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first)}));
    let file = s.stored_plugin_file(id).unwrap();
    let entry = json!({"id": id, "hash": "0".repeat(64), "permissions": ["no.such.permission"]});
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();

    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert_eq!(std::fs::read(&file).unwrap(), first, "the store keeps the module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&second), "replace": true}));
    assert_eq!(std::fs::read(&file).unwrap(), first, "the store gets its module back");

    std::fs::write(&grants, serde_json::to_vec(&json!([entry])).unwrap()).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the removal takes the entry out of the file");
}

/// A grant that can't be removed when no module is stored for its id has no module to go with, so
/// it stops counting for the session.
#[test]
fn a_grant_that_cannot_be_removed_and_has_no_stored_module_stops_counting() {
    let folder = crate::tests_links::Folder::new("plugin-store-grant-no-module");
    let id = "test.engine.permissions.forgetnomodule";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let entry = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();
    std::fs::remove_file(&file).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert!(!std::path::Path::new(&file).exists(), "the store keeps no module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant that a load of the plug-in store should forget and can't stays out of the approvals in
/// the process: both when `grants.json` can't be rewritten and when it can't be read at the load.
/// A grant recorded later counts again, and another store's grant for the id counts.
#[test]
fn a_grant_the_load_cannot_remove_stops_counting_in_the_process() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-load-forget");
    let elsewhere = crate::tests_links::Folder::new("plugin-store-load-forget-other");
    let id = "test.engine.permissions.loadforget";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let needs_approval = json!({"dataBase64": approved_data});
    let grants = folder.0.join("grants.json");
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let list = json!([entry]).to_string();
    let recorded = |dir: &std::path::Path| crate::store::signed_grants_file(dir, &list);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain)}));

    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), full, "the load can't rewrite the file");
    assert_eq!(run(&mut next, "plugin.inspect", needs_approval.clone())["needsApproval"], true);
    let refused = next.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");

    std::fs::write(elsewhere.0.join("grants.json"), recorded(&elsewhere.0)).unwrap();
    next.attach_plugin_store(elsewhere.0.clone());
    assert_eq!(run(&mut next, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "that store's grant counts");

    // A load that can rewrite the file forgets the grant, and the store has nothing ignored again.
    std::fs::write(&grants, recorded(&folder.0)).unwrap();
    next.attach_plugin_store(folder.0.clone());
    assert!(!grants.exists(), "the load removes the grant");

    // A directory at the file's path gives a read error on every platform.
    std::fs::create_dir(&grants).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    std::fs::remove_dir(&grants).unwrap();
    std::fs::write(&grants, recorded(&folder.0)).unwrap();
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the file reads again, and the grant doesn't count");

    // Attaching the same store again, with no stored module to try the removal for, keeps it out.
    let file = third.stored_plugin_file(id).unwrap();
    std::fs::remove_file(&file).unwrap();
    third.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the same store's grant stays out");

    // A later load in the session doesn't count it either, for the bytes it covers.
    std::fs::write(&file, &approved).unwrap();
    vectorcraft_plugins::registry::remove(id);
    third.load_stored_plugins();
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "the formerly approved module isn't loaded");

    run(&mut third, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval)["needsApproval"], false, "the user granted it again");
    run(&mut third, "plugin.remove", json!({"id": id}));
}

/// A grant one session stops counting stops counting for every session in the process attached to
/// the same folder, by any path to it: one attached before, one attached after with no stored module
/// to try the removal for, and one that comes back from another folder. A successful write of the
/// grant in any of them makes it count in all of them again.
#[test]
fn an_ignored_stored_grant_is_shared_by_sessions_on_the_same_folder() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-shared-ignore");
    let elsewhere = crate::tests_links::Folder::new("plugin-store-shared-ignore-other");
    let id = "test.engine.permissions.sharedignore";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let needs_approval = json!({"dataBase64": approved_data});
    let grants = folder.0.join("grants.json");
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let list = json!([entry]).to_string();
    let recorded = |dir: &std::path::Path| crate::store::signed_grants_file(dir, &list);
    std::fs::write(&grants, recorded(&folder.0)).unwrap();
    let mut first = session();
    first.attach_plugin_store(folder.0.clone());
    let inside = folder.0.join("inside");
    std::fs::create_dir(&inside).unwrap();
    let mut other = session();
    other.attach_plugin_store(inside.join(".."));
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();
    run(&mut first, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain)}));
    let file = first.stored_plugin_file(id).unwrap();
    assert!(!std::path::Path::new(&file).exists(), "the store keeps no module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);

    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the other session doesn't count it");
    let refused = other.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");
    std::fs::write(&file, &approved).unwrap();
    vectorcraft_plugins::registry::remove(id);
    other.load_stored_plugins();
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "the formerly approved module isn't loaded");
    std::fs::remove_file(&file).unwrap();

    let mut later = session();
    later.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut later, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "a session attached later doesn't count it");

    std::fs::write(elsewhere.0.join("grants.json"), recorded(&elsewhere.0)).unwrap();
    other.attach_plugin_store(elsewhere.0.clone());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "that store's grant counts");
    other.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "back on the folder, it doesn't");

    std::fs::write(&grants, recorded(&folder.0)).unwrap();
    assert!(later.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the file can be written");
    for s in [&mut first, &mut other, &mut later] {
        assert_eq!(run(s, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "it counts again");
    }
    std::fs::remove_file(&grants).unwrap();
}

/// A plug-in folder that doesn't exist yet gets the key it gets once it is made, and every path to a
/// folder gets the same key.
#[test]
fn a_plugin_folder_has_one_key_for_every_path_to_it() {
    use crate::store::plugin_store_key;
    let folder = crate::tests_links::Folder::new("plugin-store-key");
    let dir = folder.0.join("Plug-ins");
    let before = plugin_store_key(&dir);
    assert_eq!(plugin_store_key(&dir.join("sub").join("..")), before);
    assert_eq!(plugin_store_key(&folder.0.join(".").join("Plug-ins")), before);
    std::fs::create_dir(&dir).unwrap();
    assert_eq!(plugin_store_key(&dir), before, "the key stays when the folder is made");
    assert_eq!(plugin_store_key(&dir.join(".")), before);
    #[cfg(windows)]
    {
        let plain = std::path::absolute(&dir).unwrap();
        let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", plain.display()));
        assert_eq!(plugin_store_key(&verbatim), before, "a verbatim path names the same folder");
    }
}

/// Sessions attached to a plug-in folder before it exists, by paths that only name the same folder
/// once it is made, share the grants the process doesn't count for it. A grant that one session
/// writes while another fails to remove it keeps counting.
#[test]
fn sessions_attached_before_the_folder_exists_share_its_ignored_grants() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-late-folder");
    let id = "test.engine.permissions.latefolder";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let needs_approval = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    std::fs::create_dir_all(folder.0.join("a").join("b")).unwrap();
    let dir = folder.0.join("a").join("Plug-ins");
    // On disk `link/..` is `a`, so `link/../Plug-ins` is the folder; by name it is a `Plug-ins` beside `a`.
    // On Windows the folder is named in other letter case.
    #[cfg(unix)]
    let alias = {
        std::os::unix::fs::symlink(folder.0.join("a").join("b"), folder.0.join("link")).unwrap();
        folder.0.join("link").join("..").join("Plug-ins")
    };
    #[cfg(windows)]
    let alias = folder.0.join("A").join("PLUG-INS");
    #[cfg(not(any(unix, windows)))]
    let alias = dir.clone();
    let mut first = session();
    first.attach_plugin_store(dir.clone());
    let mut other = session();
    other.attach_plugin_store(alias);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("grants.json"), crate::store::signed_grants_file(&dir, &json!([entry]).to_string())).unwrap();
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    first.ignore_stored_grant(id, first.grant_writes());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the other session doesn't count it");
    assert!(other.store_grant(id, Some(serde_json::from_value(entry.clone()).unwrap())), "the file can be written");
    assert_eq!(run(&mut first, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "it counts again");

    // A write that fails in one session while another writes the grant for the id leaves the grant
    // counting.
    let before = first.grant_writes();
    assert!(other.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the other session writes the grant");
    first.ignore_stored_grant(id, before);
    assert_eq!(run(&mut first, "plugin.inspect", needs_approval)["needsApproval"], false, "the written grant counts");
    std::fs::remove_file(dir.join("grants.json")).unwrap();
}

/// Grant writes that change nothing, each for another id, don't grow what the process keeps past its
/// limit.
#[test]
fn the_process_keeps_a_bounded_number_of_grant_writes() {
    let folder = crate::tests_links::Folder::new("plugin-store-writes-kept");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    for n in 0..300 {
        assert!(s.store_grant(&format!("test.engine.permissions.unknown{n}"), None), "nothing to remove is a success");
    }
    assert!(Session::grant_writes_kept() <= 256, "{}", Session::grant_writes_kept());
}

/// A session attached by a link to a plug-in folder matches the grants the process doesn't count
/// with the folder the link leads to now, not the one it led to when the session was attached.
#[cfg(unix)]
#[test]
fn a_linked_plugin_folder_uses_the_ignored_grants_of_where_the_link_leads() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-relinked");
    let id = "test.engine.permissions.relinked";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let needs_approval = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let (old, new, link) = (folder.0.join("old"), folder.0.join("new"), folder.0.join("Plug-ins"));
    for dir in [&old, &new] {
        std::fs::create_dir(dir).unwrap();
        std::fs::write(dir.join("grants.json"), crate::store::signed_grants_file(dir, &json!([entry]).to_string())).unwrap();
    }
    std::os::unix::fs::symlink(&old, &link).unwrap();
    let mut linked = session();
    linked.attach_plugin_store(link.clone());
    assert_eq!(run(&mut linked, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&new, &link).unwrap();
    let mut direct = session();
    direct.attach_plugin_store(new.clone());
    direct.ignore_stored_grant(id, direct.grant_writes());
    assert_eq!(run(&mut linked, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the link leads to the ignored grant");
    assert!(linked.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the file can be written");
    assert_eq!(run(&mut direct, "plugin.inspect", needs_approval)["needsApproval"], false, "it counts again");
}

/// A grants file that can't be opened or read may name the old module once it reads again, so
/// neither a module listing no permissions nor `plugin.remove` gets past it. A directory at the
/// file's path gives a read error on every platform.
#[test]
fn a_grants_file_that_cannot_be_read_keeps_the_grant_and_the_module_it_covers() {
    let folder = crate::tests_links::Folder::new("plugin-store-unread-grants");
    let id = "test.engine.permissions.unreadgrants";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir(&grants).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"], "replace": true}));
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store keeps the module");

    std::fs::remove_dir(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the file reads again and the grant goes");
}

/// `plugin.remove` returns an error and keeps the plug-in when its grant can't be removed from
/// `grants.json`; a second one tries again. A grants file that doesn't hold a list doesn't stop it.
#[test]
fn a_grant_that_cannot_be_removed_stops_plugin_remove() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-remove");
    let id = "test.engine.permissions.removefail";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let recorded = std::fs::read(&grants).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = crate::store::grant_entries(&recorded).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();
    let data = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), b"{}").unwrap();

    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "it keeps its grant");
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store keeps the module the grant covers");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert!(data.join("settings.json").is_file(), "it keeps its settings");

    // Settings that can't be moved aside leave the grant recorded. A file at the aside path stops
    // the move on Unix, and a file held open in the folder stops it on Windows.
    std::fs::write(&grants, &recorded).unwrap();
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);
    std::fs::write(&aside, b"in the way").unwrap();
    #[cfg(windows)]
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::File::options().read(true).share_mode(1).open(data.join("settings.json")).unwrap()
    };
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    #[cfg(windows)]
    drop(held);
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the grant stays while the settings do");
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    let _ = std::fs::remove_file(&aside);

    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the second removal forgets the grant");

    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    std::fs::write(&grants, b"not a list").unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "a grants file that isn't a list approves nothing");
    assert!(!std::path::Path::new(&file).exists());
}

/// When `plugin.remove` can't remove the grant and can't move the settings back either, its error
/// says where the settings are, and the plug-in gets no storage until they are back. A second
/// removal that can't remove the grant moves them back rather than deleting them, and so does the
/// next load of the plug-in.
#[test]
fn settings_that_cannot_be_moved_back_are_reported_and_moved_back_later() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-move-back");
    let id = "test.engine.permissions.movebackfail";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let recorded = std::fs::read(&grants).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = crate::store::grant_entries(&recorded).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();
    let data = crate::store::plugin_data_dir(&folder.0, id);
    let settings = br#"{"zoom":"aGVsbG8="}"#;
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), settings).unwrap();
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);

    crate::store::MOVE_BACK_FAILS.set(true);
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    crate::store::MOVE_BACK_FAILS.set(false);
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(refused.contains(&aside.display().to_string()) && refused.contains("the folder is in use"), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert!(!data.exists());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings);
    assert!(vectorcraft_plugins::registry::storage(id).is_none(), "no empty settings take their place");

    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert_eq!(refused, crate::store::GRANT_NOT_REMOVED);
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), settings, "the second removal moves the settings back");
    assert!(!aside.exists());

    crate::store::MOVE_BACK_FAILS.set(true);
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    crate::store::MOVE_BACK_FAILS.set(false);
    assert!(!data.exists());

    // A folder of the plug-in's own made meanwhile is used, and a removal that can't remove the
    // grant leaves both.
    let newer = br#"{"zoom":"bmV3"}"#;
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), newer).unwrap();
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert_eq!(refused, crate::store::GRANT_NOT_REMOVED);
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), newer);
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings, "the kept settings stay");
    std::fs::remove_dir_all(&data).unwrap();

    // A restart loads the plug-in again with its settings.
    std::fs::write(&grants, &recorded).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in loads");
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), settings, "the load moves the settings back");
    assert!(!aside.exists());

    run(&mut next, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!data.exists() && !aside.exists(), "the removal deletes the settings");
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "nothing is left beside them");
}

/// What a deletion left in a plug-in's renamed settings folder, marked by the `.delete` file beside
/// it, isn't moved back. When that file can't be written, nothing is deleted, so the folder stays
/// whole and counts as kept settings.
#[test]
fn a_removal_deletes_only_folders_it_marked() {
    let folder = crate::tests_links::Folder::new("plugin-store-delete-mark");
    let id = "test.engine.permissions.deletemark";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let install = json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]});
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let data = crate::store::plugin_data_dir(&folder.0, id);
    let settings = br#"{"zoom":"aGVsbG8="}"#;
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);
    let mut marker = aside.clone().into_os_string();
    marker.push(".delete");

    // The next install starts with no settings, and its removal deletes the leftover and the mark.
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("settings.json"), settings).unwrap();
    std::fs::write(&marker, b"").unwrap();
    run(&mut next, "plugin.install", install.clone());
    assert!(!data.exists(), "a leftover of a deletion isn't moved back");
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "the leftover and its mark are deleted");

    // When the mark can't be written, the removal deletes nothing: installing the id again needs
    // `replace`, and a removal of the id that isn't installed deletes the folder.
    run(&mut next, "plugin.install", install.clone());
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), settings).unwrap();
    crate::store::MARKER_WRITE_FAILS.set(true);
    let removed = next.execute("plugin.remove", &json!({"id": id}));
    crate::store::MARKER_WRITE_FAILS.set(false);
    assert_eq!(removed.unwrap(), json!({"removed": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings, "nothing is deleted without the mark");
    assert!(!std::path::Path::new(&marker).exists());
    assert!(next.execute("plugin.install", &install).is_err(), "the kept settings go to a new install only with `replace`");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings);
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "the removal of the id deletes the kept folder");
}

/// Windows keeps a file another program holds open without delete sharing, so the last grant can't
/// be removed by deleting `grants.json`.
#[cfg(windows)]
#[test]
fn a_grants_file_that_cannot_be_deleted_keeps_the_stored_module() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let folder = crate::tests_links::Folder::new("plugin-store-held-grants");
    let id = "test.engine.permissions.heldgrants";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();

    let held = std::fs::File::options().read(true).share_mode(FILE_SHARE_READ).open(&grants).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    drop(held);
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");
    assert!(grants.exists());

    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// When the last grant can't be removed because `grants.json` can't be deleted, a replacing module
/// that asks for no permissions doesn't stay in the store: the store gets back the earlier module,
/// and the file is left as it was. The generation was raised before the removal and stays raised, so
/// the file left in place grants nothing and the module asks again.
#[test]
fn a_grants_file_whose_deletion_fails_keeps_the_stored_module() {
    let folder = crate::tests_links::Folder::new("plugin-store-undeletable-grants");
    let id = "test.engine.permissions.undeletablegrants";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let recorded = std::fs::read(&grants).unwrap();

    crate::store::GRANTS_DELETE_FAILS.set(true);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    crate::store::GRANTS_DELETE_FAILS.set(false);
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the file stays as it was");
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true, "the file left in place grants nothing");

    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists(), "a removal that can delete the file forgets the grant");
}

/// A plug-in folder that leaves a module listing no permissions under an id forgets the process's
/// grant for it, unless a later file in the folder leaves a granted module there. A grant kept in
/// the plug-in store stays, with the stored module it covers.
#[test]
fn a_plugin_folder_ends_the_process_grant_of_an_id_it_replaces() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let id = "test.engine.permissions.folderstale";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let dir = crate::tests_links::Folder::new("plugin-folder-stale-grant");
    std::fs::write(dir.0.join("a-plain.wasm"), &plain).unwrap();
    std::fs::write(dir.0.join("b-approved.wasm"), &approved).unwrap();
    let path = json!({"path": dir.0.display().to_string()});
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));

    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["failed"], json!([]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "the granted module is the one left");

    std::fs::remove_file(dir.0.join("b-approved.wasm")).unwrap();
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["loaded"], json!([id]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);
    run(&mut s, "plugin.remove", json!({"id": id}));

    let store = crate::tests_links::Folder::new("plugin-folder-stale-grant-store");
    let mut stored = session();
    stored.attach_plugin_store(store.0.clone());
    run(&mut stored, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let grants = store.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    run(&mut stored, "plugin.reload", path);
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the store still holds the module its grant covers");
    assert_eq!(run(&mut stored, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], false);
    run(&mut stored, "plugin.remove", json!({"id": id}));
}

/// A stored module whose bytes changed is skipped at the next start and stays in the store with its
/// grant, and the original bytes load again. A stored grant whose hash is malformed covers nothing.
/// With a store attached, a matching grant held only by the process doesn't count.
#[test]
fn a_stored_grant_covers_only_the_stored_module_it_recorded() {
    use vectorcraft_plugins::permissions::{Grant, GrantToken, Permission, sha256_hex};
    let folder = crate::tests_links::Folder::new("plugin-store-recorded-module");
    let id = "test.engine.permissions.storedbytes";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    let module = std::fs::read_dir(&folder.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "wasm"))
        .expect("the stored module");
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    let starts = || {
        forget();
        let mut next = session();
        next.attach_plugin_store(folder.0.clone());
        vectorcraft_plugins::registry::get(id).is_some()
    };
    assert!(starts(), "the module that was approved loads");
    std::fs::write(&module, &changed).unwrap();
    assert!(!starts(), "other bytes under the old grant are skipped");
    assert_eq!(std::fs::read(&module).unwrap(), changed, "the module stays in the store");
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "its grant stays");
    std::fs::write(&module, &first).unwrap();
    assert!(starts(), "the approved bytes load again");

    // A malformed hash is no wildcard, even for the module on disk.
    let upper = sha256_hex(&first).to_uppercase();
    for hash in ["", "00", upper.as_str()] {
        std::fs::write(
            &grants,
            crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": hash, "permissions": ["doc.read"]}]).to_string()),
        )
        .unwrap();
        assert!(!starts(), "hash {hash:?}");
    }

    // The store's grants are the ones that count, even when the process holds one for these bytes.
    std::fs::write(&grants, &recorded).unwrap();
    assert!(starts());
    let mut attached = session();
    attached.attach_plugin_store(folder.0.clone());
    vectorcraft_plugins::registry::grant(GrantToken::after_engine_approval(Grant {
        id: id.into(),
        hash: sha256_hex(&changed),
        permissions: vec![Permission::DocRead],
    }));
    let inspected = run(&mut attached, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&changed)}));
    assert_eq!(inspected["needsApproval"], true, "a grant held only by the process doesn't count with a store attached");
    run(&mut attached, "plugin.remove", json!({"id": id}));
}

/// A write to the plug-in store that fails leaves the store's module beside the grant that approved
/// it, so the old module loads at the next start: a module that can't be written doesn't update the
/// grant, and a grant that can't be written puts the old module back. On a first install, it takes
/// the new module out again.
#[test]
fn a_failed_store_write_keeps_the_stored_module_with_its_grant() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-failed-write");
    let id = "test.engine.permissions.failedwrite";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let first_data = vectorcraft_format::base64_encode(&first);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    let starts = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
        let mut next = session();
        next.attach_plugin_store(folder.0.clone());
        vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string())
    };
    let block = |path: &std::path::Path| {
        std::fs::remove_file(path).unwrap();
        std::fs::create_dir_all(path.join("block")).unwrap();
    };

    // The module can't be written: the grant stays the old module's.
    block(&module);
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the grant isn't updated for a module that wasn't stored");
    std::fs::remove_dir_all(&module).unwrap();
    std::fs::write(&module, &first).unwrap();
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads after a failed module write");

    // The grants file is full: the new grant, which is longer, doesn't fit.
    let cap = 1 << 20;
    let mut entries: Vec<Value> = crate::store::grant_entries(&recorded);
    entries.push(json!({"id": "", "pad": ""}));
    let room = cap - crate::store::signed_grants_file(&folder.0, &serde_json::to_string(&entries).unwrap()).len();
    *entries.last_mut().unwrap() = json!({"id": "", "pad": "x".repeat(room)});
    let full = crate::store::signed_grants_file(&folder.0, &serde_json::to_string(&entries).unwrap());
    assert_eq!(full.len(), cap);
    std::fs::write(&grants, &full).unwrap();
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), full, "not written past the cap");
    assert_eq!(std::fs::read(&module).unwrap(), first, "the old module is put back");
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads after a failed grant write");

    // The grants file can't be read, so nothing can be recorded in it.
    block(&grants);
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&module).unwrap(), first, "the old module is put back");
    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads once the grants are readable");

    // A stored module that can't be read back isn't replaced, so a failed grant write can't lose it.
    let oversized = vectorcraft_plugins::Limits::default().max_module_bytes as u64 + 1;
    std::fs::File::options().write(true).open(&module).unwrap().set_len(oversized).unwrap();
    block(&grants);
    run(&mut s, "plugin.install", update);
    assert_eq!(std::fs::metadata(&module).unwrap().len(), oversized, "the stored module stays");
    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    std::fs::write(&module, &first).unwrap();

    // A first install whose grant can't be written leaves no module without a grant in the store.
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!module.exists() && !grants.exists());
    std::fs::create_dir_all(grants.join("block")).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    assert!(!module.exists(), "the store holds no module its grants don't cover");
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
}

/// An install refused because another session revoked the grant its checks relied on writes
/// nothing: a store that kept the grant and lost the module gets no module back, its grants stay as
/// they were, and the plug-in installed with the id and the other session's revoke stand. `grant`
/// approves the module by itself.
#[test]
fn a_grant_revoked_while_installing_leaves_the_store_as_it_was() {
    use vectorcraft_plugins::permissions::sha256_hex;
    use vectorcraft_plugins::registry;
    let folder = crate::tests_links::Folder::new("plugin-store-revoked-while-installing");
    let id = "test.engine.permissions.revokedwhileinstalling";
    let bytes = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&bytes);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    std::fs::remove_file(&module).unwrap();

    crate::cmd::plugin::BEFORE_INSTALL.set(Some(Box::new(move || {
        registry::revoke(id);
    })));
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read"), "{e}");
    assert!(!module.exists(), "no module is stored");
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the grants stay as they were");
    assert!(registry::granted(id).is_none(), "the other session's revoke stands");
    assert_eq!(registry::get(id).map(|p| p.hash().to_string()), Some(sha256_hex(&bytes)), "the installed plug-in stays");

    crate::cmd::plugin::BEFORE_INSTALL.set(Some(Box::new(move || {
        registry::revoke(id);
    })));
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert_eq!(std::fs::read(&module).unwrap(), bytes, "a module `grant` approves is stored");
    assert!(registry::granted(id).is_some_and(|g| g.hash == sha256_hex(&bytes)));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in another session installs with the id while a module is checked takes the place the
/// checks found free. Without `replace`, or with `replace: false`, the install is refused, writes no
/// module and leaves that plug-in installed; with `replace: true` the module takes its place.
#[test]
fn a_plug_in_installed_while_installing_needs_replace() {
    use vectorcraft_plugins::permissions::sha256_hex;
    use vectorcraft_plugins::registry;
    let folder = crate::tests_links::Folder::new("plugin-store-installed-while-installing");
    let id = "test.engine.installedwhileinstalling";
    let ours = asking(id, "[]", &json!({"objects": []}));
    let theirs = asking(id, "[]", &json!({"objects": [], "documentData": null}));
    let data = vectorcraft_format::base64_encode(&ours);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    let installed = || registry::get(id).map(|p| p.hash().to_string());
    let meanwhile = |theirs: Vec<u8>| -> Option<Box<dyn FnOnce()>> {
        Some(Box::new(move || {
            registry::install_bytes(&theirs).unwrap();
        }))
    };

    for (replace, why) in [(Value::Null, "is another module than"), (json!(false), "is already installed")] {
        crate::cmd::plugin::BEFORE_INSTALL.set(meanwhile(theirs.clone()));
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "replace": replace})).unwrap_err().to_string();
        assert!(e.contains(why), "{replace}: {e}");
        assert!(!module.exists(), "{replace}: no module is stored");
        assert_eq!(installed(), Some(sha256_hex(&theirs)), "{replace}: the other plug-in stays installed");
        registry::remove(id);
    }

    crate::cmd::plugin::BEFORE_INSTALL.set(meanwhile(theirs));
    run(&mut s, "plugin.install", json!({"dataBase64": data, "replace": true}));
    assert_eq!(std::fs::read(&module).unwrap(), ours);
    assert_eq!(installed(), Some(sha256_hex(&ours)));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Without a plug-in store, the install's approval comes from the grant it read first, the one it
/// checks again before it installs: a grant another session records after that read approves
/// nothing, so a revoke that follows it can't be undone by the install recording it again.
#[test]
fn a_grant_recorded_and_revoked_while_installing_approves_nothing() {
    use vectorcraft_plugins::permissions::{Grant, GrantToken, Permission, sha256_hex};
    use vectorcraft_plugins::registry;
    let id = "test.engine.permissions.recordedwhileinstalling";
    let bytes = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&bytes);
    let mut s = session();
    let g = Grant { id: id.into(), hash: sha256_hex(&bytes), permissions: vec![Permission::DocRead] };
    crate::cmd::plugin::AFTER_GRANT_READ.set(Some(Box::new(move || {
        registry::grant(GrantToken::after_engine_approval(g));
    })));
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read"), "{e}");
    assert!(registry::get(id).is_none(), "nothing is installed");
    registry::revoke(id);
}

/// A `plugin.remove` in another session while a module is installed waits until the install has
/// written the store, then removes the module, its grant and the plug-in: nothing of it stays in
/// the store for the next launch.
#[test]
fn a_removal_while_installing_leaves_nothing_in_the_store() {
    use vectorcraft_plugins::registry;
    let folder = crate::tests_links::Folder::new("plugin-store-removed-while-installing");
    let id = "test.engine.permissions.removedwhileinstalling";
    let bytes = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&bytes);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    let grants = folder.0.join("grants.json");

    let (said, heard) = std::sync::mpsc::channel::<&'static str>();
    let (handed, remover) = std::sync::mpsc::channel();
    let dir = folder.0.clone();
    crate::cmd::plugin::BEFORE_STORE.set(Some(Box::new(move || {
        let thread = std::thread::spawn(move || {
            let tell = said.clone();
            crate::cmd::plugin::WHILE_WAITING.set(Some(Box::new(move || {
                let _ = tell.send("waiting");
            })));
            let mut other = session();
            other.attach_plugin_store(dir);
            let removed = other.execute("plugin.remove", &json!({"id": id})).map(|_| ()).map_err(|e| e.to_string());
            let _ = said.send("removed");
            removed
        });
        // The removal has finished, or waits for this install.
        let first = heard.recv().unwrap_or("gone");
        let _ = handed.send((first, thread));
    })));
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let (first, thread) = remover.recv().unwrap();
    assert_eq!(thread.join().unwrap(), Ok(()));
    assert!(registry::get(id).is_none() && registry::granted(id).is_none(), "the plug-in and its grant are gone");
    assert!(!module.exists(), "no module stays in the store");
    assert!(!std::fs::read_to_string(&grants).is_ok_and(|g| g.contains(id)), "no grant stays in the store");
    assert_eq!(first, "waiting", "the removal waits for the install");
}

/// Another session that attaches the plug-in store while a new version is installed loads the
/// stored plug-ins once the install has written the store: the version that runs is the one the
/// store keeps for the next launch.
#[test]
fn a_store_loaded_while_installing_loads_the_new_version() {
    use vectorcraft_plugins::permissions::sha256_hex;
    use vectorcraft_plugins::registry;
    let folder = crate::tests_links::Folder::new("plugin-store-loaded-while-installing");
    let id = "test.engine.permissions.loadedwhileinstalling";
    let old = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let new = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "documentData": null}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&old), "grant": ["doc.read"]}));

    let (said, heard) = std::sync::mpsc::channel::<&'static str>();
    let (handed, loader) = std::sync::mpsc::channel();
    let dir = folder.0.clone();
    crate::cmd::plugin::BEFORE_STORE.set(Some(Box::new(move || {
        let thread = std::thread::spawn(move || {
            let tell = said.clone();
            crate::cmd::plugin::WHILE_WAITING.set(Some(Box::new(move || {
                let _ = tell.send("waiting");
            })));
            session().attach_plugin_store(dir);
            let _ = said.send("loaded");
        });
        // The load has finished, or waits for this install.
        let first = heard.recv().unwrap_or("gone");
        let _ = handed.send((first, thread));
    })));
    let data = vectorcraft_format::base64_encode(&new);
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"], "replace": true}));
    let (first, thread) = loader.recv().unwrap();
    thread.join().unwrap();
    assert_eq!(registry::get(id).map(|p| p.hash().to_string()), Some(sha256_hex(&new)), "the new version runs");
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), new, "the store keeps the new version");
    assert_eq!(first, "waiting", "the load waits for the install");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant of `doc.read` to plug-in `id`, the module with the SHA-256 `hash`.
fn doc_read_grant(id: &str, hash: &str) -> vectorcraft_plugins::permissions::Grant {
    serde_json::from_value(json!({"id": id, "hash": hash, "permissions": ["doc.read"]})).unwrap()
}

/// A session that uses the plug-in store `dir` without loading it, as another process using the
/// same folder does: it doesn't wait for the installs and removals of this process's sessions.
fn other_process(dir: std::path::PathBuf) -> Session {
    let mut s = Session::new();
    s.plugin_dir = Some(dir);
    s
}

/// Waits on this thread for a turn on a plug-in store, telling `waiting` before it waits.
fn tell_when_waiting(waiting: std::sync::mpsc::Sender<&'static str>) {
    crate::store::WHILE_STORE_LOCKED.set(Some(Box::new(move || {
        let _ = waiting.send("waiting");
    })));
}

/// The ids `grants.json` in `dir` holds grants for, and the hash of each.
fn stored_grant_hashes(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let entries = crate::store::grant_entries(&std::fs::read(dir.join("grants.json")).unwrap());
    entries.iter().map(|e| (e["id"].as_str().unwrap().to_string(), e["hash"].as_str().unwrap().to_string())).collect()
}

/// An install into a `Plug-ins` folder that another process also installs the same id into keeps a
/// module with its own grant: the other process's install waits for this one's turn on the folder,
/// between this install's module and its grant, and then writes both of its own.
#[test]
fn installs_from_two_processes_into_one_store_keep_each_module_with_its_grant() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-installs-take-turns");
    let id = "test.engine.permissions.installsturns";
    let ours = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let theirs = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "documentData": null}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());

    let (said, heard) = std::sync::mpsc::channel::<&'static str>();
    let (handed, other) = std::sync::mpsc::channel();
    let (dir, module) = (folder.0.clone(), theirs.clone());
    crate::cmd::plugin::BEFORE_GRANT_STORED.set(Some(Box::new(move || {
        let thread = std::thread::spawn(move || {
            tell_when_waiting(said.clone());
            let other = other_process(dir);
            let turn = other.plugin_store_turn().unwrap();
            let stored = crate::cmd::plugin::keep_installed(&other, &turn, id, &module, Some(doc_read_grant(id, &sha256_hex(&module))));
            let _ = said.send(if stored { "stored" } else { "not stored" });
        });
        // The other install has written the store, or waits for this one.
        let first = heard.recv().unwrap_or("gone");
        let _ = handed.send((first, thread));
    })));
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&ours), "grant": ["doc.read"]}));
    let (first, thread) = other.recv().unwrap();
    thread.join().unwrap();

    let module = std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap();
    assert_eq!(stored_grant_hashes(&folder.0).get(id), Some(&sha256_hex(&module)), "the stored grant covers the stored module");
    assert_eq!(module, theirs, "the install that waited wrote last");
    assert_eq!(first, "waiting", "the other install waits for this one");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Grants written for two plug-ins into one `Plug-ins` folder by two processes are both kept: the
/// second write reads `grants.json` only once the first has written it.
#[test]
fn grants_written_by_two_processes_into_one_store_keep_both() {
    let folder = crate::tests_links::Folder::new("plugin-store-grants-take-turns");
    let (ours, theirs) = ("test.engine.permissions.grantturnsa", "test.engine.permissions.grantturnsb");
    let hash = vectorcraft_plugins::permissions::sha256_hex(b"module");
    let s = other_process(folder.0.clone());

    let (said, heard) = std::sync::mpsc::channel::<&'static str>();
    let (handed, other) = std::sync::mpsc::channel();
    let (dir, other_hash) = (folder.0.clone(), hash.clone());
    crate::store::AFTER_GRANTS_READ.set(Some(Box::new(move || {
        let thread = std::thread::spawn(move || {
            tell_when_waiting(said.clone());
            let written = other_process(dir).store_grant(theirs, Some(doc_read_grant(theirs, &other_hash)));
            let _ = said.send(if written { "written" } else { "not written" });
        });
        // The other write has written the file, or waits for this one.
        let first = heard.recv().unwrap_or("gone");
        let _ = handed.send((first, thread));
    })));
    assert!(s.store_grant(ours, Some(doc_read_grant(ours, &hash))), "the grant is written");
    let (first, thread) = other.recv().unwrap();
    thread.join().unwrap();

    let kept = stored_grant_hashes(&folder.0);
    assert_eq!(kept.keys().map(String::as_str).collect::<Vec<_>>(), [ours, theirs], "both grants are kept");
    assert_eq!(first, "waiting", "the other write waits for this one");
}

/// A setting a plug-in changes in one process and another setting it changes in a second process
/// using the same `Plug-ins` folder are both kept: the second process waits for the first's turn on
/// the folder and reads what it wrote. The second process is this test run again with
/// `VECTORCRAFT_TEST_PLUGIN_SETTINGS_STORE` naming the folder; it prints when it waits and when it
/// is done.
#[test]
fn settings_changed_by_two_processes_keep_both() {
    use std::io::{BufRead, Write};
    use vectorcraft_plugins::storage::Meter;
    const CHILD: &str = "VECTORCRAFT_TEST_PLUGIN_SETTINGS_STORE";
    const NAME: &str = "tests::plugins::settings_changed_by_two_processes_keep_both";
    let id = "test.engine.storagetwoprocesses";
    // Nothing a removal kept is moved back, so the storage is made without a turn on the folder.
    let attached = |s: &Session| {
        s.attach_plugin_storage(&crate::store::StoreTurn::unheld(), id);
        vectorcraft_plugins::registry::storage(id).unwrap()
    };
    if let Some(dir) = std::env::var_os(CHILD) {
        let st = attached(&other_process(std::path::PathBuf::from(dir)));
        crate::store::WHILE_STORE_LOCKED.set(Some(Box::new(|| {
            println!("settings-child-waiting");
            let _ = std::io::stdout().flush();
        })));
        st.set_setting("theirs", b"2", &mut Meter::unlimited()).unwrap();
        println!("settings-child-done");
        let _ = std::io::stdout().flush();
        return;
    }
    let folder = crate::tests_links::Folder::new("plugin-settings-two-processes");
    let st = attached(&other_process(folder.0.clone()));

    let (handed, child) = std::sync::mpsc::channel();
    let dir = folder.0.clone();
    crate::store::AFTER_SETTINGS_READ.set(Some(Box::new(move || {
        let mut process = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD, &dir)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = std::io::BufReader::new(process.stdout.take().unwrap()).lines();
        // The other process has written the settings, or waits for this change.
        let first = lines
            .by_ref()
            .map_while(std::io::Result::ok)
            .find_map(|l| ["waiting", "done"].into_iter().find(|said| l.contains(&format!("settings-child-{said}"))));
        let _ = handed.send((first, process, lines));
    })));
    st.set_setting("ours", b"1", &mut Meter::unlimited()).unwrap();
    let (first, mut process, lines) = child.recv().unwrap();
    let rest: Vec<String> = lines.map_while(std::io::Result::ok).collect();
    assert!(process.wait().unwrap().success(), "the other process failed: {rest:?}");

    let file: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(crate::store::plugin_data_dir(&folder.0, id).join("settings.json")).unwrap()).unwrap();
    assert_eq!(file.keys().map(String::as_str).collect::<Vec<_>>(), ["ours", "theirs"], "both settings are kept");
    assert_eq!(first, Some("waiting"), "the other process waits for this change");
    vectorcraft_plugins::registry::set_storage(id, None);
}

/// When the `Plug-ins` folder can't be locked (here `disabled.lock` is a folder), `plugin.install`
/// and `plugin.remove` change nothing and say why: the plug-in installed with the id, its grant and
/// the stored module stay as they were.
#[test]
fn a_store_that_cant_be_locked_refuses_installs_and_removals() {
    use vectorcraft_plugins::{permissions::sha256_hex, registry};
    let folder = crate::tests_links::Folder::new("plugin-store-not-locked");
    let id = "test.engine.permissions.notlocked";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let second = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "documentData": null}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    let lock = folder.0.join(crate::store::DISABLED_LOCK);
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    let grants = std::fs::read(folder.0.join("grants.json")).unwrap();

    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&second), "grant": ["doc.read"], "replace": true});
    let refused = s.execute("plugin.install", &update).unwrap_err().to_string();
    assert!(refused.contains(crate::store::STORE_NOT_LOCKED) && refused.contains("nothing was installed"), "{refused}");
    assert_eq!(registry::get(id).map(|p| p.hash().to_string()), Some(sha256_hex(&first)), "the installed plug-in stays");
    assert_eq!(registry::granted(id).map(|g| g.hash), Some(sha256_hex(&first)), "its grant stays");
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), first, "the store keeps its module");
    assert_eq!(std::fs::read(folder.0.join("grants.json")).unwrap(), grants, "and its grant");

    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::STORE_NOT_LOCKED) && refused.contains("nothing was removed"), "{refused}");
    assert!(registry::get(id).is_some(), "the plug-in stays installed");
    assert!(std::path::Path::new(&s.stored_plugin_file(id).unwrap()).exists(), "and stored");

    std::fs::remove_dir(&lock).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(registry::get(id).is_none());
}

/// Runs the test `name` of this module again in a process of its own → whether this is that
/// process. The list of disabled plug-ins is kept for the whole process, and the other tests here
/// read it again each time they attach a plug-in store.
fn in_own_process(name: &str) -> bool {
    const CHILD: &str = "VECTORCRAFT_TEST_PLUGIN_DISABLED_LIST";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("tests::plugins::{name}"), "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
        "{name} failed in its own process:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    false
}

/// An update whose grant can't be written, and whose old module can't be put back either, stays in
/// the store: a disabled plug-in is turned on, as for any update the store holds. The old grant
/// doesn't cover the new module, so the next start skips it until it is approved.
#[test]
fn an_update_left_in_the_store_by_a_failed_put_back_turns_a_disabled_plugin_on() {
    if !in_own_process("an_update_left_in_the_store_by_a_failed_put_back_turns_a_disabled_plugin_on") {
        return;
    }
    use vectorcraft_plugins::{permissions::sha256_hex, registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back-update");
    let id = "test.engine.permissions.putbackupdate";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let installed = s.execute("plugin.install", &update);
    crate::store::PUT_BACK_FAILS.set(false);
    assert_eq!(installed.unwrap()["hash"], sha256_hex(&changed), "the new module runs in this session");
    assert_eq!(registry::get(id).map(|p| p.hash().to_string()), Some(sha256_hex(&changed)));
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), changed, "the store holds the new module");
    assert!(safe_mode::disabled().is_empty(), "the module the store holds is turned on");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(safe_mode::parse_disabled_file(&saved).is_empty(), "nothing disabled is saved");

    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "the old grant doesn't cover the new module");
    safe_mode::set_disabled(vec![]);
}

/// An update whose grant can't be written, and whose put-back writes the old module and then
/// reports a failure, leaves the old module in the store: a disabled plug-in stays disabled, and the
/// next start doesn't load it.
#[test]
fn an_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off() {
    if !in_own_process("an_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off") {
        return;
    }
    use vectorcraft_plugins::{registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-put-back-fails-after-write");
    let id = "test.engine.permissions.putbacklate";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let disabled = safe_mode::disabled();
    assert!(!disabled.is_empty());
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(true);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let installed = s.execute("plugin.install", &update);
    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(false);
    installed.unwrap();
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), first, "the store holds the old module");
    assert_eq!(safe_mode::disabled(), disabled, "the plug-in stays disabled");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(!safe_mode::parse_disabled_file(&saved).is_empty(), "the disabled list on disk is kept");

    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "the disabled old module doesn't load");
    safe_mode::set_disabled(vec![]);
}

/// A module without permissions whose install can't remove the old grant, and whose put-back writes
/// the old module and then reports a failure, leaves the old module in the store: a disabled plug-in
/// stays disabled.
#[test]
fn a_plain_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off() {
    if !in_own_process("a_plain_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off") {
        return;
    }
    use vectorcraft_plugins::safe_mode;
    let folder = crate::tests_links::Folder::new("plugin-store-plain-put-back-fails-after-write");
    let id = "test.engine.permissions.plainputbacklate";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let disabled = safe_mode::disabled();
    assert!(!disabled.is_empty());
    let file = s.stored_plugin_file(id).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).remove(0);
    let full = crate::store::signed_grants_file(&folder.0, &format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")));
    std::fs::write(&grants, &full).unwrap();

    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(true);
    let installed = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(false);
    installed.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store holds the old module");
    assert_eq!(safe_mode::disabled(), disabled, "the plug-in stays disabled");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(!safe_mode::parse_disabled_file(&saved).is_empty(), "the disabled list on disk is kept");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    safe_mode::set_disabled(vec![]);
}

/// A first install whose grant can't be written, and whose module can't be taken out of the store
/// again, stays in the store: the plug-in's file is turned on. No grant covers the module, so the
/// next start skips it until it is approved.
#[test]
fn a_first_install_left_in_the_store_by_a_failed_put_back_turns_its_file_on() {
    if !in_own_process("a_first_install_left_in_the_store_by_a_failed_put_back_turns_its_file_on") {
        return;
    }
    use vectorcraft_plugins::{registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back-first");
    let id = "test.engine.permissions.putbackfirst";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let module = s.stored_plugin_file(id).unwrap();
    run(&mut s, "plugin.setEnabled", json!({"file": module, "enabled": false}));
    let grants = folder.0.join("grants.json");
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    let installed = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    crate::store::PUT_BACK_FAILS.set(false);
    installed.unwrap();
    assert!(registry::get(id).is_some(), "the module runs in this session");
    assert_eq!(std::fs::read(&module).unwrap(), first, "the store holds the module");
    assert!(safe_mode::disabled().is_empty(), "the file the store holds is turned on");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(safe_mode::parse_disabled_file(&saved).is_empty(), "nothing disabled is saved");

    std::fs::remove_dir_all(&grants).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "no grant covers the module");
    safe_mode::set_disabled(vec![]);
}

/// A plug-in folder reports a changed module as needing approval and still loads the other files;
/// granted on its own, the changed module loads from the folder. A session without a store uses the
/// grants held by the process.
#[test]
fn a_plugin_folder_skips_a_changed_module_until_it_is_approved() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let id = "test.engine.permissions.folderbytes";
    let plain = "test.engine.permissions.folderplain";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let dir = crate::tests_links::Folder::new("plugin-folder-changed-module");
    let file = dir.0.join("asks.wasm");
    std::fs::write(&file, &first).unwrap();
    std::fs::write(dir.0.join("plain.wasm"), asking(plain, "null", &json!({"objects": []}))).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    let path = json!({"path": dir.0.display().to_string()});
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!((r["loaded"].as_array().unwrap().len(), r["failed"].as_array().unwrap().len()), (2, 0), "{r}");

    std::fs::write(&file, &changed).unwrap();
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["loaded"], json!([plain]), "the other file still loads");
    let failed = r["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{r}");
    assert!(failed[0]["file"].as_str().unwrap().ends_with("asks.wasm"), "{r}");
    let why = failed[0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && why.contains("doc.read"), "{why}");
    let hash = vectorcraft_plugins::permissions::sha256_hex(&first);
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), hash, "the module installed earlier stays");

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read"], "replace": true}));
    let r = run(&mut s, "plugin.reload", path);
    assert_eq!(r["failed"], json!([]), "approved on its own, it loads: {r}");
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "plugin.remove", json!({"id": plain}));
}

/// The host enforces what a filter was granted: without `doc.write` an output that changes the
/// document is refused and nothing changes, without `doc.read` it is given no objects, and without
/// `state` it is given no data kept with the document and may not write any.
#[test]
fn a_filter_does_only_what_its_permissions_allow() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let filter = |s: &mut Session, pid: &str| s.execute("plugin.run", &json!({"id": pid, "ids": [id.0]}));
    let grant = |s: &mut Session, wasm: &[u8], perms: Value| {
        run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "grant": perms, "replace": true}));
    };
    // Denied `doc.write`: renaming the object is refused, and the document and history stay.
    let reader = "test.engine.permissions.reader";
    grant(&mut s, &asking(reader, r#"["doc.read"]"#, &json!({"objects": [{"id": id.0, "name": "Changed"}]})), json!(["doc.read"]));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = filter(&mut s, reader).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `doc.write`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Deleting it (leaving it out of the output) and adding objects are changes too.
    for out in [json!({"objects": []}), json!({"objects": [{"id": id.0}, {"path": triangle()}]})] {
        grant(&mut s, &asking(reader, r#"["doc.read"]"#, &out), json!(["doc.read"]));
        assert!(filter(&mut s, reader).is_err(), "{out}");
        assert_eq!(s.doc().unwrap().doc, before, "{out}");
    }
    // Returning the object unchanged changes nothing, so it may.
    grant(&mut s, &asking(reader, r#"["doc.read"]"#, &json!({"objects": [{"id": id.0}]})), json!(["doc.read"]));
    filter(&mut s, reader).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, before.layers);
    // Granted `doc.write` alone: it is given no objects, so its triangle is added beside the
    // rectangle rather than replacing it.
    let writer = "test.engine.permissions.writer";
    let manifest = format!(r#"{{"id":"{writer}","name":"W","kind":"filter","permissions":["doc.write"]}}"#);
    let added = json!({"objects": [{"path": triangle(), "name": "Given nothing"}]});
    let src = tpl::when_given(&manifest, tpl::Given::Input, r#"{"objects":[]}"#, &added.to_string());
    grant(&mut s, &wat::parse_str(src).unwrap(), json!(["doc.write"]));
    let made = filter(&mut s, writer).unwrap();
    let new = NodeId(made["ids"][0].as_u64().unwrap());
    assert_eq!(node(&s, new).name.as_deref(), Some("Given nothing"));
    assert!(s.doc().unwrap().doc.node(id).is_some(), "the rectangle it wasn't given stays");
    // Without `state`: kept data is not handed over, and writing some is refused.
    run(
        &mut s,
        "plugin.install",
        json!({"dataBase64": vectorcraft_format::base64_encode(&asking("test.engine.permissions.keeper", "null", &json!({"objects": [{"id": id.0}], "documentData": {"pen": 1}})))}),
    );
    filter(&mut s, "test.engine.permissions.keeper").unwrap();
    let stateless = "test.engine.permissions.keeper";
    let manifest = format!(r#"{{"id":"{stateless}","name":"K","kind":"filter","permissions":["doc.read","doc.write"]}}"#);
    let src =
        tpl::when_given(&manifest, tpl::Given::Params, r#""documentData":null"#, &json!({"objects": [{"id": id.0, "name": "No data"}]}).to_string());
    grant(&mut s, &wat::parse_str(src).unwrap(), json!(["doc.read", "doc.write"]));
    filter(&mut s, stateless).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("No data"), "given no kept data");
    let before = s.doc().unwrap().doc.clone();
    grant(
        &mut s,
        &asking(stateless, r#"["doc.read","doc.write"]"#, &json!({"objects": [{"id": id.0}], "documentData": {"pen": 2}})),
        json!(["doc.read", "doc.write"]),
    );
    let e = filter(&mut s, stateless).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `state`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().doc.plugin_data(stateless), Some(&json!({"pen": 1})));
}

/// A filter whose manifest lists `inputs` is also given the selected type, read-only: an echo
/// changes nothing, and an output that leaves the type out or changes it is refused with the
/// document and undo history as they were, handed in from a run made elsewhere too. A filter
/// without `inputs` is not given the type and so never deletes it. Installing one that lists
/// `inputs` needs the user's grant.
#[test]
fn read_only_kind_field_and_omission_refuse_without_undo() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let r = rect(&mut s, 10.0, "#ff0000");
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Hi"}))["id"].as_u64().unwrap());
    let reads =
        |id: &str| format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"inputs":["type"]}}"#);
    // Each case installs another module under the same id, in place of the last (`replace: true`).
    let install_wat = |s: &mut Session, src: &str| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true}))
    };
    let echo = "test.engine.inputs.echo";
    let module = tpl::module(&reads(echo), tpl::ECHO, "", 1);
    let data = vectorcraft_format::base64_encode(&wat::parse_str(&module).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read, doc.write"), "{e}");
    let listed = install_wat(&mut s, &module);
    assert_eq!(listed["inputs"], json!(["type"]));
    let p = |id: &str| json!({"id": id, "ids": [r.0, t.0]});
    let before = s.doc().unwrap().doc.clone();
    assert_eq!(run(&mut s, "plugin.run", p(echo))["ids"], json!([r.0, t.0]));
    assert_eq!(s.doc().unwrap().doc.layers, before.layers, "an echo changes nothing");
    for (output, says) in [
        (json!({"objects": [{"id": r.0}]}), format!("objects (id {}): a read-only object was left out", t.0)),
        (json!({"objects": [{"id": r.0}, {"id": t.0, "name": "Renamed"}]}), format!("objects[1] (id {}): read-only field 'name' changed", t.0)),
        (json!({"objects": [{"id": r.0}, {"id": t.0}, {"type": "type", "path": triangle()}]}), "`type` must be \"path\" or \"compound\"".into()),
    ] {
        let id = "test.engine.inputs.bad";
        install_wat(&mut s, &tpl::returning(&reads(id), &output.to_string()));
        let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
        let e = s.execute("plugin.run", &p(id)).unwrap_err().to_string();
        assert!(e.contains(&says), "{says}: {e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "{output}");
        // Handed in from a run made away from plugin.run, it is refused the same way.
        let made = FilterRun::prepare(&s, &p(id)).unwrap();
        let out = made.run().unwrap();
        made.hand_in(&mut s, out);
        let e = s.execute("plugin.run", &p(id)).unwrap_err().to_string();
        assert!(e.contains(&says), "handed in: {e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "handed in: {output}");
    }
    // Without `inputs` the type isn't given, so leaving it out keeps it.
    let legacy = "test.engine.inputs.legacy";
    install_wat(&mut s, &tpl::returning(&tpl::manifest(legacy, "filter", "{}"), &json!({"objects": [{"id": r.0, "name": "Kept"}]}).to_string()));
    assert_eq!(run(&mut s, "plugin.run", p(legacy))["ids"], json!([r.0]));
    assert_eq!(node(&s, r).name.as_deref(), Some("Kept"));
    assert!(matches!(node(&s, t).kind, vectorcraft_doc::NodeKind::Text(_)), "the type stays");
    // A manifest that lists `inputs` below `api` 2 doesn't install.
    let old = format!(r#"{{"id":"{echo}","name":"Old","kind":"filter","inputs":["type"]}}"#);
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::module(&old, tpl::ECHO, "", 1)).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("`inputs` needs `api` 2"), "{e}");
}

/// A filter at `api` 3 that lists type, images and symbol instances in `writes` changes a type
/// object's characters, an image's transform and opacity and the symbol an instance shows, as one
/// undo step that undo takes back and redo gives again, and the changes are kept when the document
/// is saved and opened. An echo records no undo step; a refused write, and a write by a filter that
/// lists the kinds in `inputs` alone, leave the document and the history as they were.
#[test]
fn a_filter_that_writes_type_images_and_instances_is_one_undo_step() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeKind, Symbol};
    use vectorcraft_geom::Affine;
    let mut s = session();
    let r = rect(&mut s, 10.0, "#ff0000");
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Hi"}))["id"].as_u64().unwrap());
    let (mut image, mut inst) = (NodeId(0), NodeId(0));
    s.edit("Set up", |d, _| {
        for name in ["Sun", "Moon"] {
            let art = Arc::new(Node::path(d.alloc_id(), triangle(), Appearance::default()));
            d.symbols.push(Symbol { name: name.into(), art, dynamic: false });
        }
        let layer = d.layers[0].id;
        let im = vectorcraft_doc::ImageObject {
            key: "none".into(),
            width: 4,
            height: 2,
            xf: Affine::translate((100.0, 20.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        image = d.insert(Some(layer), usize::MAX, n).unwrap();
        let n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Sun".into(), xf: Affine::translate((200.0, 20.0)) });
        inst = d.insert(Some(layer), usize::MAX, n).unwrap();
        Ok(())
    })
    .unwrap();
    let manifest = |id: &str, writes: &str| {
        format!(
            r#"{{"id":"{id}","name":"Revise","kind":"filter","api":3,"permissions":["doc.read","doc.write"],"inputs":["type","image","symbolInstance"]{writes}}}"#
        )
    };
    let all = r#","writes":["type","image","symbolInstance"]"#;
    let put = |s: &mut Session, src: &str| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true}))
    };
    let p = |id: &str| json!({"id": id, "ids": [r.0, t.0, image.0, inst.0]});
    let style = match &node(&s, t).kind {
        NodeKind::Text(x) => serde_json::to_value(&x.runs[0].style).unwrap(),
        _ => panic!("type"),
    };
    let edits = json!({"objects": [
        {"id": r.0},
        {"id": t.0, "text": {"runs": [{"text": "Bye now", "style": style}]}},
        {"id": image.0, "transform": [2, 0, 0, 2, 50, 60], "opacity": 0.25},
        {"id": inst.0, "symbolId": "Moon"},
    ]});
    let writer = "test.engine.writes.revise";
    let listed = put(&mut s, &tpl::returning(&manifest(writer, all), &edits.to_string()));
    assert_eq!((&listed["api"], &listed["writes"]), (&json!(3), &json!(["type", "image", "symbolInstance"])));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    assert_eq!(run(&mut s, "plugin.run", p(writer))["ids"], json!([r.0, t.0, image.0, inst.0]));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let check = |d: &vectorcraft_doc::Document| {
        let NodeKind::Text(x) = &d.node(t).unwrap().kind else { panic!("type") };
        assert_eq!(x.plain_text(), "Bye now");
        let NodeKind::Image(im) = &d.node(image).unwrap().kind else { panic!("image") };
        assert_eq!((im.xf, d.node(image).unwrap().opacity), (Affine::new([2.0, 0.0, 0.0, 2.0, 50.0, 60.0]), 0.25));
        assert!(
            matches!(&d.node(inst).unwrap().kind, NodeKind::SymbolInstance { symbol, xf } if symbol == "Moon" && *xf == Affine::translate((200.0, 20.0)))
        );
    };
    let after = s.doc().unwrap().doc.clone();
    check(&after);
    assert!(matches!(&after.node(t).unwrap().kind, NodeKind::Text(x) if x.cached_bounds.is_some()), "the type is laid out again");
    assert_eq!(s.doc().unwrap().selection.objects, [r, t, image, inst]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.layers, before.layers);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().doc.layers, after.layers);
    check(&vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap());
    // An echo changes nothing and records no undo step.
    let echo = "test.engine.writes.echo";
    put(&mut s, &tpl::module(&manifest(echo, all), tpl::ECHO, "", 1));
    let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    run(&mut s, "plugin.run", p(echo));
    assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "an echo records no undo step");
    // Refused writes leave the document and the history as they were, also when a valid change
    // comes before the refused one.
    let bad = "test.engine.writes.bad";
    let retyped = json!({"id": t.0, "text": {"runs": [{"text": "Hello again", "style": style}]}});
    for (writes, output, says) in [
        (
            all,
            json!({"objects": [{"id": r.0}, retyped, {"id": image.0}, {"id": inst.0, "symbolId": "Nowhere"}]}),
            format!("objects[3] (id {}).symbolId: 'Nowhere' is not a symbol in this document", inst.0),
        ),
        (
            all,
            json!({"objects": [{"id": r.0}, {"id": t.0}, {"id": image.0, "opacity": 1.5}, {"id": inst.0}]}),
            format!("objects[2] (id {}).opacity: must be a number from 0 to 1", image.0),
        ),
        ("", edits.clone(), format!("objects[1] (id {}): read-only field 'text.kind' changed", t.0)),
    ] {
        put(&mut s, &tpl::returning(&manifest(bad, writes), &output.to_string()));
        let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
        let e = s.execute("plugin.run", &p(bad)).unwrap_err().to_string();
        assert!(e.contains(&says), "{says}: {e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "{output}");
    }
}

/// The manifest of a filter at `api` 3 with id `id` that asks for the document information
/// `context` (a JSON list), with `doc.read` and `doc.write`, and `rest` (keys after a comma, or
/// nothing).
fn context_manifest(id: &str, context: &str, rest: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":3,"permissions":["doc.read","doc.write"],"context":{context}{rest}}}"#)
}

/// Installs `src` granted `doc.read` and `doc.write`, in place of any module with its id.
fn install_reading(s: &mut Session, src: &str) -> Value {
    let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true}))
}

/// A filter at `api` 3 whose manifest lists `context` is given the document's settings, artboards,
/// layers and swatches in `_context.document` with nothing selected: plain values, hidden and
/// locked layers with their flags, the active artboard and the current layer marked, and the
/// document's name without its folder. A filter that doesn't list `context` is given none, and one
/// that lists it without `doc.read` or below `api` 3 doesn't install.
#[test]
fn an_unselected_filter_reads_the_document_context_it_asks_for() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    run(&mut s, "artboard.new", json!({"x": 500, "y": 0, "width": 100, "height": 50}));
    let hidden = run(&mut s, "layer.new", json!({"name": "Hidden", "visible": false}))["id"].as_u64().unwrap();
    let locked = run(&mut s, "layer.newSublayer", json!({"parent": hidden, "name": "Locked", "locked": true}))["id"].as_u64().unwrap();
    run(&mut s, "layer.setCurrent", json!({"id": locked}));
    run(&mut s, "swatch.new", json!({"name": "Tone", "color": "#ff0000", "global": true}));
    run(&mut s, "select.none", json!({}));
    s.doc_mut().unwrap().path = Some("/private/folder/Poster.vectorcraft".into());
    let second = s.doc().unwrap().doc.artboards[1].id;
    s.doc_mut().unwrap().active_artboard = Some(second);
    let id = "test.engine.context";
    let all = r#"["settings","artboards","layers","swatches"]"#;
    let installed = install_reading(&mut s, &tpl::returning(&context_manifest(id, all, ""), r#"{"objects":[]}"#));
    assert_eq!(installed["context"], json!(["settings", "artboards", "layers", "swatches"]));
    let swatches = run(&mut s, "swatch.list", json!({}))["swatches"].clone();
    assert!(s.doc().unwrap().selection.objects.is_empty());
    let given = FilterRun::prepare(&s, &json!({"id": id})).unwrap().params_json()["_context"]["document"].clone();
    let st = s.doc().unwrap();
    let d = &st.doc;
    assert_eq!(given["revision"], json!(st.revision));
    let board = &d.artboards[1];
    let settings = json!({
        "name": "Poster.vectorcraft", "colorMode": "rgb", "units": d.units.label(), "width": board.rect.width(),
        "height": board.rect.height(), "bleed": d.setup.bleed, "rasterEffectsPpi": d.raster_ppi(), "activeArtboard": second,
    });
    assert_eq!(given["settings"], settings);
    let boards: Vec<Value> = d
        .artboards
        .iter()
        .enumerate()
        .map(|(i, a)| json!({"id": a.id, "index": i, "name": a.name, "rect": [a.rect.x0, a.rect.y0, a.rect.x1, a.rect.y1], "active": i == 1}))
        .collect();
    assert_eq!((boards.len(), &given["artboards"]), (2, &json!(boards)));
    let layers = given["layers"].as_array().unwrap();
    let rows: Vec<(String, u64, bool, bool, bool)> = layers
        .iter()
        .map(|l| {
            (l["name"].as_str().unwrap().to_string(), l["depth"].as_u64().unwrap(), l["visible"] == true, l["locked"] == true, l["current"] == true)
        })
        .collect();
    let bottom = d.layers[0].display_name();
    assert_eq!(rows, [(bottom, 0, true, false, false), ("Hidden".into(), 0, false, false, false), ("Locked".into(), 1, true, true, true)]);
    assert_eq!((&layers[0]["parentId"], &layers[1]["sublayers"], &layers[2]["parentId"]), (&Value::Null, &json!(1), &json!(hidden)));
    assert_eq!((&layers[1]["id"], &layers[2]["id"]), (&json!(hidden), &json!(locked)));
    assert_eq!(given["swatches"], swatches, "as swatch.list lists them");
    assert!(given["swatches"].as_array().unwrap().iter().any(|w| w["name"] == "Tone" && w["global"] == true));
    assert!(!given.to_string().contains("/private/folder"), "no path: {given}");
    // Only the sections asked for.
    let some = "test.engine.context.some";
    install_reading(&mut s, &tpl::returning(&context_manifest(some, r#"["layers"]"#, ""), r#"{"objects":[]}"#));
    let given = FilterRun::prepare(&s, &json!({"id": some})).unwrap().params_json()["_context"]["document"].clone();
    let keys: Vec<&String> = given.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(given.get("layers").is_some() && given.get("revision").is_some());
    // A filter without `context` is given none.
    let legacy = "test.engine.context.legacy";
    install_reading(&mut s, &tpl::returning(&tpl::manifest(legacy, "filter", "{}"), r#"{"objects":[]}"#));
    assert!(FilterRun::prepare(&s, &json!({"id": legacy})).unwrap().params_json()["_context"].get("document").is_none());
    // Without `doc.read`, or below `api` 3, it doesn't install.
    let needs = "`context` is for a filter, or a live effect with `commands`, at `api` 3 or later with the permission `doc.read`";
    for manifest in [
        format!(r#"{{"id":"{some}","name":"x","kind":"filter","api":3,"permissions":["doc.write"],"context":["layers"]}}"#),
        format!(r#"{{"id":"{some}","name":"x","kind":"filter","api":2,"permissions":["doc.read"],"context":["layers"]}}"#),
    ] {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap());
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true})).unwrap_err();
        assert!(e.to_string().contains(needs), "{manifest}: {e}");
    }
}

/// A plug-in command, of a filter or of a live effect, whose manifest lists `context` is given it
/// too, with nothing selected.
#[test]
fn a_plugin_command_reads_the_document_context_without_a_selection() {
    let mut s = session();
    let made = json!({"objects": [{"path": triangle(), "name": "Saw layers"}]}).to_string();
    for (id, kind, permissions) in [
        ("test.engine.context.command", "filter", r#"["doc.read","doc.write"]"#),
        ("test.engine.context.effectcommand", "effect", r#"["doc.read","doc.write","effect.render"]"#),
    ] {
        let manifest = format!(
            r#"{{"id":"{id}","name":"Test {id}","kind":"{kind}","api":3,"permissions":{permissions},"context":["layers"],"commands":[{{"id":"look","label":"Look"}}]}}"#
        );
        let src = tpl::when_given_else(&manifest, tpl::Given::Params, r#""layers":["#, &made, r#"{"objects":[]}"#);
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        let grant: Vec<Value> = serde_json::from_str(permissions).unwrap();
        run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": grant, "replace": true}));
        run(&mut s, "select.none", json!({}));
        let ids = run(&mut s, &format!("plugin.{id}.look"), json!({}))["ids"].clone();
        assert_eq!(ids.as_array().map(Vec::len), Some(1), "{kind}: {ids}");
        assert_eq!(node(&s, NodeId(ids[0].as_u64().unwrap())).name.as_deref(), Some("Saw layers"), "{kind}");
    }
}

/// A document context holds at most [`cmd::plugin::MAX_CONTEXT_ITEMS`] rows in
/// [`cmd::plugin::MAX_CONTEXT_BYTES`] bytes: past either, the run fails before the plug-in starts,
/// with the document and undo history as they were. Each row comes off the run's instruction
/// budget.
#[test]
fn the_document_context_is_capped_and_its_rows_cost_instructions() {
    use cmd::plugin::{CONTEXT_FUEL_PER_ITEM, FilterRun, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS};
    let mut s = session();
    let id = "test.engine.context.cap";
    install_reading(&mut s, &tpl::returning(&context_manifest(id, r#"["swatches"]"#, ""), r#"{"objects":[]}"#));
    let p = json!({"id": id});
    let listed = run(&mut s, "swatch.list", json!({}))["swatches"].as_array().unwrap().len();
    let add = |s: &mut Session, n: usize| {
        s.edit("Set up", |d, _| {
            let base = d.swatches.last().unwrap().clone();
            let at = d.swatches.len();
            d.swatches.extend((at..at + n).map(|i| vectorcraft_color::Swatch { name: format!("Many {i}"), ..base.clone() }));
            Ok(())
        })
        .unwrap();
    };
    add(&mut s, MAX_CONTEXT_ITEMS - listed);
    let full = FilterRun::prepare(&s, &p).unwrap();
    assert_eq!(full.params_json()["_context"]["document"]["swatches"].as_array().map(Vec::len), Some(MAX_CONTEXT_ITEMS));
    assert_eq!(full.limits().fuel_base, full.plugin().limits().fuel_base - MAX_CONTEXT_ITEMS as u64 * CONTEXT_FUEL_PER_ITEM);
    let too_large = "the document information the plug-in asks for is too large to hand over";
    add(&mut s, 1);
    let refused = |s: &mut Session| {
        let Err(e) = FilterRun::prepare(s, &p) else { panic!("prepared past the cap") };
        assert!(e.to_string().contains(too_large), "{e}");
        let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
        let e = s.execute("plugin.run", &p).unwrap_err().to_string();
        assert!(e.contains(too_large), "{e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo));
    };
    refused(&mut s);
    // A few rows past the byte cap are refused as well.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    s.edit("Set up", |d, _| {
        d.swatches.last_mut().unwrap().name = "n".repeat(MAX_CONTEXT_BYTES);
        Ok(())
    })
    .unwrap();
    refused(&mut s);
}

/// A filter that writes type, run while typing is in progress: the plug-in is given the typed
/// text, so an echo keeps it, with Real-time Drawing on and off. An output that is refused leaves
/// the document and the history as they were; with Real-time Drawing on the typing stays in
/// progress, and off it is kept as its own step before the plug-in is given its input.
#[test]
fn a_filter_that_writes_type_is_given_the_typing_in_progress() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let manifest = |id: &str| {
        format!(
            r#"{{"id":"{id}","name":"Retype","kind":"filter","api":3,"permissions":["doc.read","doc.write"],"inputs":["type"],"writes":["type"]}}"#
        )
    };
    let put = |s: &mut Session, src: &str| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true}))
    };
    let plain = |s: &Session, t: NodeId| match &node(s, t).kind {
        NodeKind::Text(x) => x.plain_text(),
        _ => panic!("type"),
    };
    let labels = |s: &Session| s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
    let typing = |real_time: bool| {
        let mut s = session();
        s.prefs.real_time_drawing = real_time;
        let v = ViewInfo::default();
        let t = NodeId(run(&mut s, "text.create", json!({"x": 100, "y": 100, "text": "A", "size": 20}))["id"].as_u64().unwrap());
        s.select_tool("type", v).unwrap();
        let b = s.doc().unwrap().doc.bounds_of(&[t], false).unwrap();
        for k in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(k, b.x1 - 1.0, b.center().y), v).unwrap();
        }
        s.tool_text("B", v).unwrap();
        assert_eq!(plain(&s, t), if real_time { "AB" } else { "A" }, "real time {real_time}");
        (s, t)
    };
    let bang = "test.engine.writes.typing.bang";
    let bad = "test.engine.writes.typing.bad";
    for real_time in [false, true] {
        // The plug-in adds "!" to the text it is given: "AB" or, given only "A", "A".
        let (mut s, t) = typing(real_time);
        let style = match &node(&s, t).kind {
            NodeKind::Text(x) => serde_json::to_value(&x.runs[0].style).unwrap(),
            _ => panic!("type"),
        };
        let retyped = |text: &str| json!({"objects": [{"id": t.0, "text": {"runs": [{"text": text, "style": style}]}}]}).to_string();
        put(&mut s, &tpl::when_given_else(&manifest(bang), tpl::Given::Input, r#""AB""#, &retyped("AB!"), &retyped("A!")));
        run(&mut s, "plugin.run", json!({"id": bang, "ids": [t.0]}));
        assert_eq!(plain(&s, t), "AB!", "real time {real_time}: the plug-in is given the typed text");
        let steps = labels(&s);
        assert_eq!(steps[steps.len() - 2..], ["Typing", "Retype"], "real time {real_time}");

        let (mut s, t) = typing(real_time);
        put(&mut s, &tpl::returning(&manifest(bad), &json!({"objects": [{"id": t.0, "transform": [0, 0, 0, 0, 0, 0]}]}).to_string()));
        let undo = labels(&s);
        let e = s.execute("plugin.run", &json!({"id": bad, "ids": [t.0]})).unwrap_err().to_string();
        assert!(e.contains("transform: flattens the object to a line or a point"), "{e}");
        assert_eq!(plain(&s, t), "AB", "real time {real_time}");
        if real_time {
            assert_eq!(labels(&s), undo, "the refused output leaves the history as it was");
            assert!(s.in_interaction(), "and the typing in progress");
        } else {
            assert_eq!(labels(&s)[undo.len()..], ["Typing"], "only the typing is kept");
        }
    }
}

/// A run past its memory budget fails before the plug-in starts, with the document and undo
/// history as they were: a read-only object whose other fields don't fit, a path and a compound
/// path that don't fit, and parameters past the input's half of the cap, with objects selected or
/// with none. The module has one page of memory, the most it may have, so a run that started would
/// report that the plug-in ran out of memory instead.
#[test]
fn a_run_past_the_memory_budget_fails_before_the_plugin_starts() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeKind};
    use vectorcraft_geom::FillRule;
    use vectorcraft_plugins::{Limits, Plugin, registry};
    let id = "test.engine.inputs.budget";
    let params: Vec<String> = (0..9).map(|i| format!(r#""p{i}":{{"type":"string","maxLength":4096}}"#)).collect();
    let manifest = format!(
        r#"{{"id":"{id}","name":"Budget","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"inputs":["type"],"params":{{{}}}}}"#,
        params.join(",")
    );
    let module = wat::parse_str(tpl::module(&manifest, "unreachable", "", 1)).unwrap();
    // One page of memory: the input and the parameters share 32 KiB. The module lists permissions,
    // so its grant is recorded first.
    let plugin = Plugin::load(&module, Limits { max_memory_bytes: 1 << 16, ..Limits::default() }).unwrap();
    registry::grant(vectorcraft_plugins::permissions::GrantToken::after_engine_approval(vectorcraft_plugins::permissions::Grant {
        id: id.into(),
        hash: plugin.hash().to_string(),
        permissions: plugin.manifest().permissions(),
    }));
    registry::install(plugin).unwrap();
    let mut s = session();
    let small = rect(&mut s, 10.0, "#ff0000");
    let long = rect(&mut s, 30.0, "#00ff00");
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Hi"}))["id"].as_u64().unwrap());
    let mut compound = NodeId(0);
    s.edit("Set up", |d, _| {
        d.node_mut(t).unwrap().name = Some("n".repeat(40_000));
        let points: Vec<Point> = (0..4_000).map(|i| Point::new(f64::from(i), 0.0)).collect();
        let outline = PathData::single(SubPath::polyline(&points, false));
        if let NodeKind::Path { path, .. } = &mut d.node_mut(long).unwrap().kind {
            *path = outline.clone();
        }
        let member = Node::path(d.alloc_id(), outline, Appearance::default());
        let n = Node::new(d.alloc_id(), NodeKind::Compound { children: vec![Arc::new(member)], rule: FillRule::NonZero });
        let layer = d.layers[0].id;
        compound = d.insert(Some(layer), usize::MAX, n).unwrap();
        Ok(())
    })
    .unwrap();
    let padding: Value = (0..9).map(|i| (format!("p{i}"), json!("x".repeat(4096)))).collect::<serde_json::Map<_, _>>().into();
    let too_large = "memory budget (the selection is too large to hand over)";
    let named = format!("memory budget (object {} needs", t.0);
    let with_padding = |ids: Vec<u64>| {
        let mut p = padding.clone();
        p["id"] = json!(id);
        p["ids"] = json!(ids);
        p
    };
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    for (p, says) in [
        (json!({"id": id, "ids": [t.0]}), named.as_str()),
        (json!({"id": id, "ids": [long.0]}), too_large),
        (json!({"id": id, "ids": [compound.0]}), too_large),
        (with_padding(vec![small.0]), too_large),
        (with_padding(vec![]), too_large),
    ] {
        let e = s.execute("plugin.run", &p).unwrap_err().to_string();
        assert!(e.contains(says), "{says}: {e}");
        assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len()), before, "{says}");
    }
    assert!(registry::remove(id) && registry::revoke(id));
}

/// What the user granted is kept in the plug-in store with the module: the next session installs
/// the plug-in again without asking. A stored plug-in that lists permissions nobody granted it is
/// skipped, even when another store granted it, and so is one in the Additional Plug-ins Folder
/// until it is granted on its own.
#[test]
fn granted_permissions_are_kept_and_folders_skip_the_rest() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let folder = crate::tests_links::Folder::new("plugin-store-grants");
    let id = "test.engine.permissions.stored";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    assert!(folder.0.join("grants.json").is_file());
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store, as granted");
    // Only a store's own grants count: the same module in another store without a grant is
    // skipped, and a grant given elsewhere is not written to that store.
    let other = crate::tests_links::Folder::new("plugin-store-grants-other");
    std::fs::write(other.0.join("copy.wasm"), &wasm).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut elsewhere = session();
    elsewhere.attach_plugin_store(other.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "granted in another store only");
    let e = elsewhere.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "a grant kept in another store doesn't count here: {e}");
    let lone = "test.engine.permissions.lone";
    let lone_wasm = asking(lone, r#"["doc.read"]"#, &json!({"objects": []}));
    run(&mut elsewhere, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&lone_wasm), "grant": ["doc.read"]}));
    let kept = std::fs::read_to_string(other.0.join("grants.json")).unwrap();
    assert!(kept.contains(lone) && !kept.contains(id), "{kept}");
    run(&mut elsewhere, "plugin.remove", json!({"id": lone}));
    assert!(!other.0.join("grants.json").exists(), "the last grant removed takes the file with it");
    // Without the grant it is skipped, and the file stays.
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    std::fs::remove_file(folder.0.join("grants.json")).unwrap();
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "nobody granted it");
    assert_eq!(store_files(&folder.0), 1, "the module stays in the store");

    // The Additional Plug-ins Folder skips it with a note until it is granted.
    let dir = crate::tests_links::Folder::new("plugin-folder-grants");
    std::fs::write(dir.0.join("asks.wasm"), &wasm).unwrap();
    let r = run(&mut third, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([]));
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && why.contains("doc.read"), "{why}");
    run(&mut third, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    let r = run(&mut third, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([id]));
    run(&mut third, "plugin.remove", json!({"id": id}));
    // With a store attached, only that store's grants load it from the folder: a grant given in a
    // session without a store doesn't count.
    let mut storeless = session();
    run(&mut storeless, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    vectorcraft_plugins::registry::remove(id);
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "granted in the process");
    let empty = crate::tests_links::Folder::new("plugin-store-grants-empty");
    let mut scoped = session();
    scoped.attach_plugin_store(empty.0.clone());
    let r = run(&mut scoped, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    vectorcraft_plugins::registry::revoke(id);
    assert_eq!(r["loaded"], json!([]), "a grant outside this store doesn't load it");
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval"), "{why}");
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "not installed");
    // The preference's folder is loaded again for a session whose store holds other grants.
    let pref_dir = crate::tests_links::Folder::new("plugin-folder-pref-grants");
    std::fs::write(pref_dir.0.join("asks.wasm"), &wasm).unwrap();
    let pref = json!({"key": "pluginsFolder", "value": pref_dir.0.display().to_string()});
    run(&mut scoped, "prefs.set", pref.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "nothing granted in this store");
    let granting = crate::tests_links::Folder::new("plugin-store-grants-pref");
    let mut granted = session();
    granted.attach_plugin_store(granting.0.clone());
    run(&mut granted, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    run(&mut granted, "prefs.set", pref);
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "loaded under this store's grant");
    run(&mut granted, "plugin.remove", json!({"id": id}));
}

/// The registry installs a module that lists permissions only under a grant the process holds, so
/// a start and a `plugin.reload` record the store's grant for the module they load. A module in the
/// folder that nobody granted is reported and skipped, and records nothing.
#[test]
fn stored_and_reload_grants_still_install() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let store = crate::tests_links::Folder::new("plugin-store-grants-reload");
    let id = "test.engine.permissions.reload";
    let lone = "test.engine.permissions.reload.lone";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(store.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    forget();
    let mut next = session();
    next.attach_plugin_store(store.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store");
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    forget();
    let dir = crate::tests_links::Folder::new("plugin-folder-grants-reload");
    std::fs::write(dir.0.join("granted.wasm"), &wasm).unwrap();
    std::fs::write(dir.0.join("lone.wasm"), asking(lone, r#"["doc.read"]"#, &json!({"objects": []}))).unwrap();
    let r = run(&mut next, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([id]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "the store's grant is the process's too");
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && r["failed"][0]["file"].as_str().unwrap().ends_with("lone.wasm"), "{r}");
    assert!(vectorcraft_plugins::registry::get(lone).is_none() && vectorcraft_plugins::registry::granted(lone).is_none());
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// A grant covers the module it was given to: another module under the id, listing the same
/// permissions, isn't installed from the store at the next start, a folder load reports it as
/// needing approval, and `plugin.install` asks for `grant` again, even with `replace: true`. It
/// doesn't get the settings kept for the id until the user grants it. A kept grant without a hash
/// covers no module.
#[test]
fn a_grant_covers_only_the_module_it_was_given_to() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let folder = crate::tests_links::Folder::new("plugin-store-grant-hash");
    let id = "test.engine.granthash";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    // The stored module is replaced on disk by one that reads the settings.
    let other = storing(id, READS);
    let stored: Vec<_> =
        std::fs::read_dir(&folder.0).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "wasm")).collect();
    assert_eq!(stored.len(), 1, "{stored:?}");
    std::fs::write(&stored[0], &other).unwrap();
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    vectorcraft_plugins::registry::set_storage(id, None);
    let no_storage = |how: &str| {
        assert!(vectorcraft_plugins::registry::storage(id).is_none(), "{how} gives another module the id's settings and cache");
    };
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "another module isn't loaded under the grant");
    no_storage("the next start");
    let dir = crate::tests_links::Folder::new("plugin-folder-grant-hash");
    std::fs::write(dir.0.join("other.wasm"), &other).unwrap();
    let r = run(&mut next, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([]));
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval"), "{why}");
    no_storage("a folder load");
    let data = vectorcraft_format::base64_encode(&other);
    let e = next.execute("plugin.install", &json!({"dataBase64": data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "{e}");
    no_storage("a refused install");
    // Granted, it reads the settings the first module set.
    run(&mut next, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));

    // A kept grant that records no hash, or an empty one, covers no module: the stored module isn't
    // installed at the next start and `plugin.inspect` asks for approval.
    for entry in [json!({"id": id, "permissions": ["settings", "cache"]}), json!({"id": id, "hash": "", "permissions": ["settings", "cache"]})] {
        std::fs::write(folder.0.join("grants.json"), crate::store::signed_grants_file(&folder.0, &json!([entry]).to_string())).unwrap();
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
        let mut again = session();
        again.attach_plugin_store(folder.0.clone());
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "a grant without a hash doesn't load the module: {entry}");
        assert_eq!(run(&mut again, "plugin.inspect", json!({"dataBase64": data}))["needsApproval"], json!(true), "{entry}");
        next = again;
    }
    run(&mut next, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}));
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// A grants file entry this version can't use (a permission it doesn't know, an invalid id) is
/// skipped without dropping the other grants, and recording a grant writes it back as it was,
/// except an entry for the plug-in being granted or removed. A grants file that doesn't parse as a
/// list is never overwritten.
#[test]
fn grants_this_version_cant_read_are_kept() {
    let folder = crate::tests_links::Folder::new("plugin-store-grants-unknown");
    let kept = "test.engine.permissions.kept";
    let added = "test.engine.permissions.added";
    let later = "test.engine.permissions.later";
    let empty = json!({"objects": []});
    let kept_wasm = asking(kept, r#"["doc.read"]"#, &empty);
    let added_wasm = asking(added, r#"["doc.read"]"#, &empty);
    let later_wasm = asking(later, r#"["doc.read"]"#, &empty);
    let future = json!({"id": later, "hash": "00", "permissions": ["doc.read", "ui.panel"]});
    let invalid = json!({"id": "bad id/", "hash": "00", "permissions": ["doc.read"]});
    let grant = json!({"id": kept, "hash": vectorcraft_plugins::permissions::sha256_hex(&kept_wasm), "permissions": ["doc.read"]});
    let grants = folder.0.join("grants.json");
    std::fs::write(&grants, crate::store::signed_grants_file(&folder.0, &json!([future, grant, invalid]).to_string())).unwrap();
    std::fs::write(folder.0.join("kept.wasm"), &kept_wasm).unwrap();
    let entries = || crate::store::grant_entries(&std::fs::read(&grants).unwrap());
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(kept).is_some(), "the entries this version can't read don't drop the grant next to them");
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    let now = entries();
    assert_eq!(now.len(), 4, "{now:?}");
    assert!(now.contains(&future) && now.contains(&invalid), "written back as they were: {now:?}");
    assert!(now.iter().any(|e| e["id"] == added), "{now:?}");
    run(&mut s, "plugin.remove", json!({"id": added}));
    assert_eq!(entries().len(), 3);
    // Granting the plug-in an entry names replaces that entry.
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&later_wasm), "grant": ["doc.read"]}));
    let now = entries();
    assert!(!now.contains(&future), "{now:?}");
    assert_eq!(now.iter().filter(|e| e["id"] == later).count(), 1, "{now:?}");
    run(&mut s, "plugin.remove", json!({"id": later}));
    run(&mut s, "plugin.remove", json!({"id": kept}));
    assert_eq!(entries(), vec![invalid.clone()], "the last usable grant removed leaves the entry it can't read");

    // A file that doesn't hold a list is left as it is: installing still works in this session.
    for broken in [&b"[{\"id\": "[..], &b"{\"id\": \"x\"}"[..]] {
        std::fs::write(&grants, broken).unwrap();
        run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
        assert!(vectorcraft_plugins::registry::get(added).is_some());
        assert_eq!(std::fs::read(&grants).unwrap(), broken, "not overwritten");
        run(&mut s, "plugin.remove", json!({"id": added}));
        assert_eq!(std::fs::read(&grants).unwrap(), broken, "not overwritten by a removal either");
    }

    // The file stays within the 1 MiB it is read up to: compact when pretty-printing would pass it,
    // and left as it is when even compact would.
    let cap = 1 << 20;
    let many: Vec<Value> = (0..14_000).map(|i| json!({"id": format!("x.{i}"), "hash": "00", "permissions": ["ui.panel"]})).collect();
    let compact = crate::store::signed_grants_file(&folder.0, &serde_json::to_string(&many).unwrap());
    assert!(compact.len() < cap && serde_json::to_vec_pretty(&many).unwrap().len() > cap);
    std::fs::write(&grants, &compact).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    let written = std::fs::read(&grants).unwrap();
    assert!(written.len() <= cap, "{}", written.len());
    assert_eq!(entries().len(), many.len() + 1);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm)}));
    assert_eq!(inspected["needsApproval"], json!(false), "the compact file is read back");
    run(&mut s, "plugin.remove", json!({"id": added}));
    let full = crate::store::signed_grants_file(
        &folder.0,
        &json!([{"id": "x.full", "hash": "0".repeat(cap - 250), "permissions": ["ui.panel"]}]).to_string(),
    );
    assert!(full.len() < cap);
    std::fs::write(&grants, &full).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    assert_eq!(std::fs::read(&grants).unwrap(), full, "not written past the cap");
    run(&mut s, "plugin.remove", json!({"id": added}));
}

/// Tests: the key store `store` on this thread until the guard is dropped.
struct ThreadKeys;

impl ThreadKeys {
    fn using(store: impl crate::store::grant_keys::GrantKeyStore + 'static) -> Self {
        crate::store::grant_keys::THREAD_STORE.set(Some(std::sync::Arc::new(store)));
        ThreadKeys
    }
}

impl Drop for ThreadKeys {
    fn drop(&mut self) {
        crate::store::grant_keys::THREAD_STORE.set(None);
    }
}

/// A key store that is locked or can't be reached.
struct UnreachableKeys;

impl crate::store::grant_keys::GrantKeyStore for UnreachableKeys {
    fn key(&self) -> std::result::Result<Option<crate::store::grant_keys::GrantKey>, crate::store::grant_keys::KeyStoreError> {
        Err(crate::store::grant_keys::KeyStoreError("the key store is locked".into()))
    }

    fn set_key(&self, _: &crate::store::grant_keys::GrantKey) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        Err(crate::store::grant_keys::KeyStoreError("the key store is locked".into()))
    }

    fn generation(&self, _: &std::path::Path) -> std::result::Result<Option<u64>, crate::store::grant_keys::KeyStoreError> {
        Err(crate::store::grant_keys::KeyStoreError("the key store is locked".into()))
    }

    fn set_generation(&self, _: &std::path::Path, _: u64) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        Err(crate::store::grant_keys::KeyStoreError("the key store is locked".into()))
    }
}

/// A memory key store the test keeps a handle on. While `file_generations_fail` is set, it fails to
/// keep the generation that the grants file in the folder is at, and keeps every other one.
struct SharedKeys {
    keys: std::sync::Arc<crate::store::grant_keys::MemoryGrantKeys>,
    file_generations_fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SharedKeys {
    fn new(keys: &std::sync::Arc<crate::store::grant_keys::MemoryGrantKeys>) -> Self {
        SharedKeys { keys: keys.clone(), file_generations_fail: std::sync::Arc::default() }
    }
}

impl crate::store::grant_keys::GrantKeyStore for SharedKeys {
    fn key(&self) -> std::result::Result<Option<crate::store::grant_keys::GrantKey>, crate::store::grant_keys::KeyStoreError> {
        crate::store::grant_keys::GrantKeyStore::key(self.keys.as_ref())
    }

    fn set_key(&self, key: &crate::store::grant_keys::GrantKey) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        crate::store::grant_keys::GrantKeyStore::set_key(self.keys.as_ref(), key)
    }

    fn generation(&self, folder: &std::path::Path) -> std::result::Result<Option<u64>, crate::store::grant_keys::KeyStoreError> {
        crate::store::grant_keys::GrantKeyStore::generation(self.keys.as_ref(), folder)
    }

    fn set_generation(&self, folder: &std::path::Path, generation: u64) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        if self.file_generations_fail.load(std::sync::atomic::Ordering::SeqCst)
            && std::fs::read(folder.join("grants.json")).ok().and_then(|bytes| generation_of(&bytes)) == Some(generation)
        {
            return Err(crate::store::grant_keys::KeyStoreError("the key store can't keep it".into()));
        }
        crate::store::grant_keys::GrantKeyStore::set_generation(self.keys.as_ref(), folder, generation)
    }
}

/// The generation the grants file `bytes` is at.
fn generation_of(bytes: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Value>(bytes).ok()?["generation"].as_u64()
}

/// The generation `keys` keeps for the plug-in store `dir`.
fn kept_generation(keys: &crate::store::grant_keys::MemoryGrantKeys, dir: &std::path::Path) -> Option<u64> {
    crate::store::grant_keys::GrantKeyStore::generation(keys, &crate::store::plugin_store_key(dir)).unwrap()
}

/// Whether the stored plug-in `id` loads from the plug-in store `dir` at a new start, and whether
/// `plugin.inspect` of `data` (its module) then asks for approval.
fn starts_from(dir: &std::path::Path, id: &str, data: &str) -> (bool, bool) {
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(dir.to_path_buf());
    let asks = run(&mut next, "plugin.inspect", json!({"dataBase64": data}))["needsApproval"] == json!(true);
    (vectorcraft_plugins::registry::get(id).is_some(), asks)
}

/// The grants file as VectorCraft wrote it keeps its grants; once it is changed outside VectorCraft
/// it grants nothing, and its plug-ins ask again: a grant given more permissions, a grant added, the
/// MAC taken out or cut short, a MAC made with another key, the file cut short, and the file copied
/// into another plug-in folder.
#[test]
fn a_grants_file_changed_outside_vectorcraft_grants_nothing() {
    use crate::store::grant_keys::{GrantKey, MemoryGrantKeys};
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-signed");
    let copy = crate::tests_links::Folder::new("plugin-store-signed-copy");
    let id = "test.engine.permissions.signed";
    let added = "test.engine.permissions.signedadded";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let added_wasm = asking(added, r#"["doc.read"]"#, &json!({"objects": []}));
    let (data, added_data) = (vectorcraft_format::base64_encode(&wasm), vectorcraft_format::base64_encode(&added_wasm));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let added_file = folder.0.join(format!("{}.wasm", crate::store::file_stem(added)));
    std::fs::write(&added_file, &added_wasm).unwrap();
    let written = std::fs::read(&grants).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the file as written keeps its grant");

    let file: Value = serde_json::from_slice(&written).unwrap();
    let with = |change: &dyn Fn(&mut Value)| {
        let mut file = file.clone();
        change(&mut file);
        serde_json::to_vec(&file).unwrap()
    };
    // At the generation kept for the folder, so that only the MAC tells the file apart.
    let generation = file["generation"].as_u64().unwrap();
    let other_key = {
        let keys = MemoryGrantKeys::with_key(GrantKey::from_bytes([7; 32]));
        crate::store::grant_keys::GrantKeyStore::set_generation(&keys, &crate::store::plugin_store_key(&folder.0), generation).unwrap();
        let _other = ThreadKeys::using(keys);
        crate::store::signed_grants_file(&folder.0, &file["grants"].to_string())
    };
    let tag = file["mac"].as_str().unwrap().to_string();
    let changed: [(&str, Vec<u8>); 6] = [
        ("a permission added", with(&|f| f["grants"][0]["permissions"] = json!(["doc.read", "doc.write"]))),
        (
            "a grant added",
            with(&|f| f["grants"].as_array_mut().unwrap().push(json!({"id": added, "hash": sha256_hex(&added_wasm), "permissions": ["doc.read"]}))),
        ),
        ("the MAC taken out", with(&|f| _ = f.as_object_mut().unwrap().remove("mac"))),
        ("the MAC cut short", with(&|f| f["mac"] = json!(tag[..32]))),
        ("a MAC made with another key", other_key),
        ("the file cut short", written[..written.len() - 2].to_vec()),
    ];
    for (what, bytes) in changed {
        std::fs::write(&grants, &bytes).unwrap();
        assert_eq!(starts_from(&folder.0, id, &data), (false, true), "{what}");
        assert_eq!(starts_from(&folder.0, added, &added_data), (false, true), "{what}");
        assert_eq!(std::fs::read(&grants).unwrap(), bytes, "{what}: the file is left as it is");
    }

    std::fs::write(&grants, &written).unwrap();
    std::fs::write(copy.0.join("copied.wasm"), &wasm).unwrap();
    std::fs::write(copy.0.join("grants.json"), &written).unwrap();
    crate::store::grant_keys::set_current_generation(&crate::store::plugin_store_key(&copy.0), generation).unwrap();
    assert_eq!(starts_from(&copy.0, id, &data), (false, true), "copied into another folder");
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the file left as written keeps its grant");
    std::fs::remove_file(&added_file).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grants file in the form an older version wrote, a plain list, grants nothing. Approving a
/// plug-in again replaces it with a signed file that holds that grant alone; removing a plug-in takes
/// its entry out and leaves the rest unsigned.
#[test]
fn an_unsigned_grants_file_is_replaced_by_the_next_approval() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-unsigned");
    let id = "test.engine.permissions.unsigned";
    let other = "test.engine.permissions.unsignedother";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let other_wasm = asking(other, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let entry = |id: &str, wasm: &[u8]| json!({"id": id, "hash": sha256_hex(wasm), "permissions": ["doc.read"]});
    let unsigned = serde_json::to_vec(&json!([entry(id, &wasm), entry(other, &other_wasm)])).unwrap();
    std::fs::write(&grants, &unsigned).unwrap();
    std::fs::write(folder.0.join(format!("{}.wasm", crate::store::file_stem(id))), &wasm).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "an unsigned list grants nothing");
    assert_eq!(std::fs::read(&grants).unwrap(), unsigned, "and is left as it is");

    // `other` isn't installed, so the removal reports it as not found after removing its entry.
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.execute("plugin.remove", &json!({"id": other})).is_err());
    assert_eq!(crate::store::grant_entries(&std::fs::read(&grants).unwrap()), vec![entry(id, &wasm)], "the removal takes its entry out");
    assert!(std::fs::read(&grants).unwrap().starts_with(b"["), "and leaves the rest unsigned");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true));

    std::fs::write(&grants, &unsigned).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"], "replace": true}));
    assert_eq!(crate::store::grant_entries(&std::fs::read(&grants).unwrap()), vec![entry(id, &wasm)], "only the new grant is kept");
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the approval counts at the next start");
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists());
}

/// With no key in the key store, a signed grants file grants nothing; the next approval makes a key
/// and signs a file that holds that grant alone.
#[test]
fn a_grants_file_without_its_key_is_replaced_by_the_next_approval() {
    use crate::store::grant_keys::{GrantKey, GrantKeyStore, MemoryGrantKeys};
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-no-key");
    let id = "test.engine.permissions.nokey";
    let other = "test.engine.permissions.nokeyother";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let list = json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}, {"id": other, "hash": "0".repeat(64), "permissions": ["doc.read"]}]);
    let signed = {
        let _old = ThreadKeys::using(MemoryGrantKeys::with_key(GrantKey::from_bytes([3; 32])));
        crate::store::signed_grants_file(&folder.0, &list.to_string())
    };
    std::fs::write(&grants, &signed).unwrap();
    std::fs::write(folder.0.join(format!("{}.wasm", crate::store::file_stem(id))), &wasm).unwrap();
    let keys = std::sync::Arc::new(MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "no key, no grant");
    assert!(keys.key().unwrap().is_none(), "reading makes no key");
    assert_eq!(std::fs::read(&grants).unwrap(), signed, "the file is left as it is");

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"], "replace": true}));
    assert!(keys.key().unwrap().is_some(), "the approval makes a key");
    let ids: Vec<Value> = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).iter().map(|e| e["id"].clone()).collect();
    assert_eq!(ids, vec![json!(id)], "only the new grant is kept");
    assert_eq!(starts_from(&folder.0, id, &data), (true, false));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// When the key store can't be reached, no grants file grants anything and none is written: an
/// approval holds for the session only, the stored module isn't replaced, and a grant the file may
/// hold can't be removed.
#[test]
fn an_unreachable_key_store_keeps_approvals_in_the_session() {
    let folder = crate::tests_links::Folder::new("plugin-store-unreachable-keys");
    let id = "test.engine.permissions.unreachable";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let written = std::fs::read(&grants).unwrap();
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());

    let locked = ThreadKeys::using(UnreachableKeys);
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the stored grant can't be checked");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read"], "replace": true}));
    assert_eq!(
        vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()),
        Some(vectorcraft_plugins::permissions::sha256_hex(&changed)),
        "it runs in this session"
    );
    assert_eq!(std::fs::read(&grants).unwrap(), written, "the grants file is left as it is");
    assert_eq!(std::fs::read(&module).unwrap(), wasm, "the store keeps the module its grant covers");
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert_eq!(std::fs::read(&grants).unwrap(), written);

    drop(locked);
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the grant counts once the key store can be reached");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists());
}

/// While the key store can't be reached, removing a plug-in fails and leaves it installed when no
/// earlier signed copy of the grants file can be kept from granting it again: with the grants file
/// gone, and with a grants file that doesn't name the plug-in. Once the key store can be reached, the
/// removal raises the generation, and the copy put back grants nothing.
#[test]
fn a_removal_the_key_store_cannot_make_last_fails() {
    let folder = crate::tests_links::Folder::new("plugin-store-unreachable-remove");
    let id = "test.engine.permissions.unreachableremove";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    let written = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();

    let locked = ThreadKeys::using(UnreachableKeys);
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "with the grants file gone: {refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some() && module.exists(), "the plug-in stays installed");
    drop(locked);
    run(&mut s, "plugin.remove", json!({"id": id}));
    std::fs::write(&grants, &written).unwrap();
    std::fs::write(&module, &wasm).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the copy put back grants nothing");
    std::fs::remove_file(&grants).unwrap();
    std::fs::remove_file(&module).unwrap();

    let other = "test.engine.permissions.unreachableremoveother";
    let plain = vectorcraft_format::base64_encode(&asking(id, "[]", &json!({"objects": []})));
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    run(&mut s, "plugin.install", json!({"dataBase64": plain}));
    let names: Vec<Value> = crate::store::grant_entries(&std::fs::read(&grants).unwrap()).iter().map(|e| e["id"].clone()).collect();
    assert_eq!(names, vec![json!(other)], "the grants file doesn't name the plug-in");
    let locked = ThreadKeys::using(UnreachableKeys);
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "with a grants file that can't be checked: {refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some() && module.exists(), "the plug-in stays installed");
    drop(locked);
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "plugin.remove", json!({"id": other}));
}

/// A stored plug-in that didn't load, because the grants file of an older version doesn't count, and
/// that keeps no settings or cache, is removed with its module and its entry, and the removal
/// reports it removed. A removal whose stored module can't be deleted fails and keeps its settings.
/// Removing an invalid id leaves the kept generation as it is.
#[test]
fn removing_a_stored_plugin_that_did_not_load_reports_it_removed() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-unloaded-remove");
    let id = "test.engine.permissions.unloadedremove";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let module = folder.0.join(format!("{}.wasm", crate::store::file_stem(id)));
    std::fs::write(&grants, serde_json::to_vec(&json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}])).unwrap()).unwrap();
    std::fs::write(&module, &wasm).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the unsigned list grants nothing");

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert_eq!(s.execute("plugin.remove", &json!({"id": id})).ok(), Some(json!({"removed": id})));
    assert!(!module.exists(), "the module is removed");
    assert!(!grants.exists(), "and its entry");
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err(), "a second removal finds nothing");

    // A stored module that can't be deleted (here a folder in its place), of a plug-in that keeps
    // settings, fails the removal, and the settings stay for the next try.
    std::fs::create_dir_all(module.join("held")).unwrap();
    let data_dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&data_dir).unwrap();
    let setting = data_dir.join("settings.json");
    std::fs::write(&setting, b"{}").unwrap();
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains("can't delete"), "{refused}");
    assert!(module.exists());
    assert_eq!(std::fs::read(&setting).ok().as_deref(), Some(&b"{}"[..]), "the settings are moved back");
    std::fs::remove_dir_all(&module).unwrap();
    assert_eq!(s.execute("plugin.remove", &json!({"id": id})).ok(), Some(json!({"removed": id})), "a second removal deletes them");
    assert!(!data_dir.exists());

    // An invalid id names no grant: its removal doesn't raise the generation.
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::with_key(crate::store::grant_keys::GrantKey::from_bytes([3; 32])));
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    for invalid in ["", "not an id"] {
        assert!(s.execute("plugin.remove", &json!({"id": invalid})).is_err(), "{invalid:?} isn't found");
        assert_eq!(kept_generation(&keys, &folder.0), None, "{invalid:?}: the generation is left as it is");
    }
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    assert_eq!(kept_generation(&keys, &folder.0), Some(2), "a valid id's removal still retires signed copies");
}

/// Removing an installed plug-in whose stored module can't be deleted fails, and the plug-in stays
/// installed with its settings and its stored grant; a second removal, once the module can be
/// deleted, removes it.
#[test]
fn removing_an_installed_plugin_whose_module_cant_be_deleted_keeps_it_installed() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::with_key(crate::store::grant_keys::GrantKey::from_bytes([5; 32])));
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let folder = crate::tests_links::Folder::new("plugin-store-installed-remove-held");
    let id = "test.engine.permissions.installedremoveheld";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let module = folder.0.join(format!("{}.wasm", crate::store::file_stem(id)));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert!(module.is_file(), "the module is stored");
    assert!(s.holds_plugin_grant(id), "and its grant");
    let stored_grant = s.stored_plugin_grant(id);
    assert!(stored_grant.is_some(), "in the signed grants file");
    let data_dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&data_dir).unwrap();
    let setting = data_dir.join("settings.json");
    std::fs::write(&setting, b"{}").unwrap();

    // A folder in the module's place can't be deleted as a file.
    std::fs::remove_file(&module).unwrap();
    std::fs::create_dir_all(module.join("held")).unwrap();
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains("can't delete"), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert!(module.exists());
    assert!(s.stored_plugin_grant(id) == stored_grant, "the stored grant is kept for the next launch");
    assert_eq!(std::fs::read(&setting).ok().as_deref(), Some(&b"{}"[..]), "the settings are moved back");

    std::fs::remove_dir_all(&module).unwrap();
    assert_eq!(s.execute("plugin.remove", &json!({"id": id})).ok(), Some(json!({"removed": id})), "a second removal removes it");
    assert!(vectorcraft_plugins::registry::get(id).is_none() && !module.exists() && !data_dir.exists());
    assert!(!s.holds_plugin_grant(id), "and its grant");
}

/// A MAC is made over the generation, the grants and the folder, and only the whole MAC of the same
/// key, generation, grants and folder verifies. A key's bytes are never printed.
#[test]
fn a_grants_mac_verifies_only_for_its_key_grants_and_folder() {
    use crate::store::grant_keys::{GrantKey, sign, verify};
    let (key, other) = (GrantKey::from_bytes([1; 32]), GrantKey::from_bytes([2; 32]));
    let folder = std::path::Path::new("/plug-ins");
    let tag = sign(&key, folder, 4, b"[]").unwrap();
    assert_eq!(tag.len(), 64);
    assert!(verify(&key, folder, 4, b"[]", &tag));
    assert!(!verify(&other, folder, 4, b"[]", &tag), "another key");
    assert!(!verify(&key, std::path::Path::new("/other"), 4, b"[]", &tag), "another folder");
    assert!(!verify(&key, folder, 3, b"[]", &tag), "another generation");
    assert!(!verify(&key, folder, 4, b"[ ]", &tag), "other grants");
    assert!(!verify(&key, folder, 4, b"[]", &tag[..62]), "a MAC cut short");
    assert!(!verify(&key, folder, 4, b"[]", &tag.to_uppercase()), "upper case");
    assert!(!verify(&key, folder, 4, b"[]", ""));
    assert_eq!(format!("{key:?}"), "GrantKey(..)");
}

/// A key store where another process keeps its key between this one's read and its write, and
/// that write does not replace it.
struct KeptFirstElsewhere {
    reads: std::sync::Mutex<u32>,
}

impl crate::store::grant_keys::GrantKeyStore for KeptFirstElsewhere {
    fn key(&self) -> std::result::Result<Option<crate::store::grant_keys::GrantKey>, crate::store::grant_keys::KeyStoreError> {
        let mut reads = self.reads.lock().unwrap();
        *reads += 1;
        // signing_key and create_key each read once before the write.
        Ok((*reads > 2).then(|| crate::store::grant_keys::GrantKey::from_bytes([7; 32])))
    }

    fn set_key(&self, _: &crate::store::grant_keys::GrantKey) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        Ok(())
    }

    fn generation(&self, _: &std::path::Path) -> std::result::Result<Option<u64>, crate::store::grant_keys::KeyStoreError> {
        Ok(None)
    }

    fn set_generation(&self, _: &std::path::Path, _: u64) -> std::result::Result<(), crate::store::grant_keys::KeyStoreError> {
        Ok(())
    }
}

#[test]
fn a_new_grants_key_is_the_one_the_key_store_kept() {
    use crate::store::grant_keys::{GrantKey, sign, verify};
    let _keys = ThreadKeys::using(KeptFirstElsewhere { reads: std::sync::Mutex::new(0) });
    let key = crate::store::grant_keys::signing_key().unwrap();
    assert_eq!(key, GrantKey::from_bytes([7; 32]), "the key kept first signs");
    let folder = std::path::Path::new("/plug-ins");
    assert!(verify(&GrantKey::from_bytes([7; 32]), folder, 1, b"[]", &sign(&key, folder, 1, b"[]").unwrap()));
}

/// An earlier copy of the grants file put back over the current one grants nothing: after one
/// plug-in's grant is revoked while another's stays, and after the last grant is revoked and the
/// file removed.
#[test]
fn an_earlier_grants_file_put_back_grants_nothing() {
    let folder = crate::tests_links::Folder::new("plugin-store-earlier-copy");
    let id = "test.engine.permissions.earlier";
    let other = "test.engine.permissions.earlierother";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let other_wasm = asking(other, r#"["doc.read"]"#, &json!({"objects": []}));
    let (data, other_data) = (vectorcraft_format::base64_encode(&wasm), vectorcraft_format::base64_encode(&other_wasm));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    run(&mut s, "plugin.install", json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    let both = std::fs::read(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": other}));
    let one = std::fs::read(&grants).unwrap();

    std::fs::write(&grants, &both).unwrap();
    assert_eq!(starts_from(&folder.0, other, &other_data), (false, true), "the revoked grant doesn't come back");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the earlier copy grants nothing");
    std::fs::write(&grants, &one).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the current file keeps its grant");

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists());
    std::fs::write(&grants, &one).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the last revoked grant doesn't come back");

    // A grant revoked from a file edited outside VectorCraft doesn't come back with a signed copy
    // made before the edit either.
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let signed = std::fs::read(&grants).unwrap();
    let mut edited: Value = serde_json::from_slice(&signed).unwrap();
    edited["mac"] = json!("0".repeat(64));
    std::fs::write(&grants, serde_json::to_vec(&edited).unwrap()).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists());
    std::fs::write(&grants, &signed).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the grant revoked from the edited file doesn't come back");
}

/// A grant revoked while the grants file is gone, or while another file that grants nothing (one
/// without a MAC, or one that doesn't hold a list) is in its place, doesn't come back with the last
/// signed file put back: the revoke raises the generation. A revoke that a signed file without an
/// entry for the plug-in already covers, or one while the key store holds no key, leaves the
/// generation as it is.
#[test]
fn a_grant_revoked_without_a_signed_file_doesnt_come_back_with_one() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let folder = crate::tests_links::Folder::new("plugin-store-revoke-without-file");
    let id = "test.engine.permissions.revokedgone";
    let other = "test.engine.permissions.revokedgoneother";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant(other, None), "nothing to revoke in an empty folder");
    assert_eq!(kept(&folder.0), None, "without a key nothing is signed, so no generation is kept");
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let signed = std::fs::read(&grants).unwrap();
    assert_eq!(kept(&folder.0), Some(3));
    assert!(s.store_grant(other, None));
    assert_eq!(kept(&folder.0), Some(3), "the signed file names no grant for it, so nothing is raised");
    assert_eq!(starts_from(&folder.0, id, &data), (true, false));

    // The file is gone when the grant is revoked.
    std::fs::remove_file(&grants).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant(id, None));
    assert_eq!(kept(&folder.0), Some(5));
    std::fs::write(&grants, &signed).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the grant revoked while the file was gone doesn't come back");

    // A file without a MAC that names no grant for it is in place.
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let signed = std::fs::read(&grants).unwrap();
    std::fs::write(&grants, b"[]").unwrap();
    assert!(s.store_grant(id, None));
    assert_eq!(std::fs::read(&grants).unwrap(), b"[]", "the unsigned file is left as it is");
    std::fs::write(&grants, &signed).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the grant revoked over an unsigned file doesn't come back");

    // A file that doesn't hold a list is in place: it is left as it is, and the revoke holds once it
    // raises the generation.
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let signed = std::fs::read(&grants).unwrap();
    std::fs::write(&grants, br#""not a list""#).unwrap();
    assert!(s.store_grant(id, None));
    assert_eq!(std::fs::read(&grants).unwrap(), br#""not a list""#, "the file is left as it is");
    std::fs::write(&grants, &signed).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the grant revoked over a file without a list doesn't come back");
}

/// Every signed write of the grants file raises the generation by two and is at the generation after
/// that, which the key store then keeps, and the file stays signed; removing the last grant raises
/// the generation too.
#[test]
fn a_signed_grants_write_raises_the_generation_and_stays_signed() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let folder = crate::tests_links::Folder::new("plugin-store-generation");
    let id = "test.engine.permissions.generation";
    let other = "test.engine.permissions.generationother";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert_eq!((generation_of(&std::fs::read(&grants).unwrap()), kept(&folder.0)), (Some(3), Some(3)));
    assert_eq!(starts_from(&folder.0, id, &data), (true, false));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    assert_eq!((generation_of(&std::fs::read(&grants).unwrap()), kept(&folder.0)), (Some(6), Some(6)));
    run(&mut s, "plugin.remove", json!({"id": other}));
    assert_eq!((generation_of(&std::fs::read(&grants).unwrap()), kept(&folder.0)), (Some(9), Some(9)));
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the file stays signed");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!grants.exists());
    assert_eq!(kept(&folder.0), Some(11), "removing the file raises the generation");
}

/// Each plug-in folder has a generation of its own.
#[test]
fn two_plugin_folders_keep_their_own_generations() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let first = crate::tests_links::Folder::new("plugin-store-generation-a");
    let second = crate::tests_links::Folder::new("plugin-store-generation-b");
    let ids = ["test.engine.permissions.foldera", "test.engine.permissions.folderaother", "test.engine.permissions.folderb"];
    let data: Vec<String> = ids.iter().map(|id| vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})))).collect();
    let mut a = session();
    a.attach_plugin_store(first.0.clone());
    run(&mut a, "plugin.install", json!({"dataBase64": data[0], "grant": ["doc.read"]}));
    run(&mut a, "plugin.install", json!({"dataBase64": data[1], "grant": ["doc.read"]}));
    let mut b = session();
    b.attach_plugin_store(second.0.clone());
    run(&mut b, "plugin.install", json!({"dataBase64": data[2], "grant": ["doc.read"]}));
    assert_eq!((kept(&first.0), kept(&second.0)), (Some(6), Some(3)));
    assert_eq!(starts_from(&first.0, ids[0], &data[0]), (true, false));
    assert_eq!(starts_from(&second.0, ids[2], &data[2]), (true, false));
    let (mut a, mut b) = (session(), session());
    a.attach_plugin_store(first.0.clone());
    b.attach_plugin_store(second.0.clone());
    run(&mut a, "plugin.remove", json!({"id": ids[0]}));
    run(&mut a, "plugin.remove", json!({"id": ids[1]}));
    run(&mut b, "plugin.remove", json!({"id": ids[2]}));
}

/// A key store that holds a key and no generation for the folder, as one does the first time this
/// version runs, reads generation 0: a file signed at 0 keeps its grants, and the next write is at 3.
#[test]
fn a_key_without_a_generation_reads_generation_zero() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::with_key(crate::store::grant_keys::GrantKey::from_bytes([5; 32])));
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let folder = crate::tests_links::Folder::new("plugin-store-generation-zero");
    let id = "test.engine.permissions.generationzero";
    let other = "test.engine.permissions.generationzeroother";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    assert_eq!(crate::store::grant_keys::current_generation(&crate::store::plugin_store_key(&folder.0)), Ok(0));
    let file = crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}]).to_string());
    assert_eq!(generation_of(&file), Some(0));
    std::fs::write(&grants, &file).unwrap();
    std::fs::write(folder.0.join(format!("{}.wasm", crate::store::file_stem(id))), &wasm).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the file at generation 0 keeps its grant");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    run(&mut s, "plugin.install", json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    assert_eq!(generation_of(&std::fs::read(&grants).unwrap()), Some(3));
    assert_eq!(starts_from(&folder.0, id, &data), (true, false));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.remove", json!({"id": other}));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant revoked while the file signed at generation 0 is gone doesn't come back with that file
/// put back: with a key kept, the revoke raises the generation from 0.
#[test]
fn a_grant_revoked_while_a_generation_zero_file_is_gone_doesnt_come_back() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::with_key(crate::store::grant_keys::GrantKey::from_bytes([6; 32])));
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let folder = crate::tests_links::Folder::new("plugin-store-generation-zero-revoke");
    let id = "test.engine.permissions.generationzerorevoke";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let grants = folder.0.join("grants.json");
    let file = crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}]).to_string());
    assert_eq!(generation_of(&file), Some(0));
    std::fs::write(folder.0.join(format!("{}.wasm", crate::store::file_stem(id))), &wasm).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant(id, None));
    assert_eq!(kept_generation(&keys, &folder.0), Some(2));
    std::fs::write(&grants, &file).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the file at generation 0 grants nothing");
}

/// When the key store can't keep the generation of a write, the file written is one generation ahead
/// of the kept one and grants nothing, so its plug-ins ask again; so does the earlier file, since
/// the generation was raised past it before the write, and a file behind the kept generation.
#[test]
fn a_grants_file_whose_generation_was_not_kept_grants_nothing() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let shared = SharedKeys::new(&keys);
    let fail = shared.file_generations_fail.clone();
    let _keys = ThreadKeys::using(shared);
    let folder = crate::tests_links::Folder::new("plugin-store-generation-unkept");
    let store = crate::store::plugin_store_key(&folder.0);
    let id = "test.engine.permissions.generationunkept";
    let other = "test.engine.permissions.generationunkeptother";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let written = std::fs::read(&grants).unwrap();

    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    _ = s.execute("plugin.install", &json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    let ahead = std::fs::read(&grants).unwrap();
    let kept = crate::store::grant_keys::GrantKeyStore::generation(keys.as_ref(), &store).unwrap();
    assert_eq!((generation_of(&ahead), kept), (Some(6), Some(5)), "the file is one generation ahead");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "and grants nothing");
    assert_eq!(starts_from(&folder.0, other, &other_data), (false, true));

    std::fs::write(&grants, &written).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the earlier file grants nothing");
    crate::store::grant_keys::GrantKeyStore::set_generation(keys.as_ref(), &store, 3).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (true, false), "the file at the kept generation keeps its grant");
    crate::store::grant_keys::GrantKeyStore::set_generation(keys.as_ref(), &store, 4).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "a file one generation behind grants nothing");
}

/// A revoke over a signed file whose generation wasn't kept raises the generation past the file's,
/// so the file left in place still grants nothing.
#[test]
fn a_revoke_over_a_file_ahead_of_its_generation_leaves_it_granting_nothing() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let shared = SharedKeys::new(&keys);
    let fail = shared.file_generations_fail.clone();
    let _keys = ThreadKeys::using(shared);
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let folder = crate::tests_links::Folder::new("plugin-store-revoke-ahead");
    let id = "test.engine.permissions.revokeahead";
    let other = "test.engine.permissions.revokeaheadother";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    _ = s.execute("plugin.install", &json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    let ahead = std::fs::read(&grants).unwrap();
    assert_eq!((generation_of(&ahead), kept(&folder.0)), (Some(6), Some(5)));

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant("test.engine.permissions.revokeaheadnone", None));
    assert_eq!(std::fs::read(&grants).unwrap(), ahead, "the file is left as it is");
    assert_eq!(kept(&folder.0), Some(7), "the generation is past the file's");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the file still grants nothing");
    assert_eq!(starts_from(&folder.0, other, &other_data), (false, true));
}

/// A signed write raises the generation before it writes the file, so a signed write whose generation
/// wasn't kept also leaves the earlier file granting nothing; a revoke over that earlier file put back
/// is then not at the generation of the unkept write, and a copy of that write put back after the
/// revoke grants nothing.
#[test]
fn a_revoke_after_a_write_whose_generation_wasnt_kept_keeps_that_write_granting_nothing() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let shared = SharedKeys::new(&keys);
    let fail = shared.file_generations_fail.clone();
    let _keys = ThreadKeys::using(shared);
    let folder = crate::tests_links::Folder::new("plugin-store-revoke-after-unkept");
    let ids = ["test.engine.permissions.afterunkept", "test.engine.permissions.afterunkeptother", "test.engine.permissions.afterunkeptthird"];
    let data: Vec<String> = ids.iter().map(|id| vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})))).collect();
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data[0], "grant": ["doc.read"]}));
    run(&mut s, "plugin.install", json!({"dataBase64": data[1], "grant": ["doc.read"]}));
    let earlier = std::fs::read(&grants).unwrap();
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    _ = s.execute("plugin.install", &json!({"dataBase64": data[2], "grant": ["doc.read"]}));
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    let unkept = std::fs::read(&grants).unwrap();

    std::fs::write(&grants, &earlier).unwrap();
    assert_eq!(starts_from(&folder.0, ids[0], &data[0]), (false, true), "the earlier file grants nothing");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant(ids[1], None));
    std::fs::write(&grants, &unkept).unwrap();
    assert_eq!(starts_from(&folder.0, ids[1], &data[1]), (false, true), "the revoked grant doesn't come back");
    assert_eq!(starts_from(&folder.0, ids[2], &data[2]), (false, true), "the unkept write grants nothing");
    std::fs::remove_file(&grants).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    for id in ids {
        _ = s.execute("plugin.remove", &json!({"id": id}));
    }
}

/// A grant whose signed file fits the 1 MiB limit at the generation it is written at is kept, also
/// when the file would not fit with a longer generation number.
#[test]
fn a_grants_file_that_fits_at_its_generation_is_written() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let folder = crate::tests_links::Folder::new("plugin-store-grants-fit");
    let id = "test.engine.permissions.grantsfit";
    let grant = doc_read_grant(id, &sha256_hex(b"module"));
    let grants = folder.0.join("grants.json");
    // The file as it is written: compact, since pretty-printed it is larger than the limit.
    let size = |pad: &str| serde_json::to_vec(&json!({"version": 1, "generation": 3, "grants": [grant, pad], "mac": "0".repeat(64)})).unwrap().len();
    let pad = "a".repeat((1 << 20) - size(""));
    assert_eq!(size(&pad), 1 << 20);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    std::fs::write(&grants, crate::store::signed_grants_file(&folder.0, &json!([pad]).to_string())).unwrap();
    assert!(s.store_grant(id, Some(grant)), "the grant is kept");
    let written = std::fs::read(&grants).unwrap();
    assert_eq!((written.len(), generation_of(&written)), (1 << 20, Some(3)));
    assert_eq!(kept_generation(&keys, &folder.0), Some(3));
}

/// A grants file edited to carry the largest generations doesn't stop a revoke and doesn't move the
/// generation the key store keeps near the limit, so later grants are still kept: the generation
/// comes from the key store alone.
#[test]
fn a_grants_file_at_the_largest_generations_doesnt_stop_a_revoke() {
    use vectorcraft_plugins::permissions::sha256_hex;
    for carried in [u64::MAX, u64::MAX - 1] {
        let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
        let _keys = ThreadKeys::using(SharedKeys::new(&keys));
        let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
        let folder = crate::tests_links::Folder::new(&format!("plugin-store-largest-generation-{}", u64::MAX - carried));
        let id = "test.engine.permissions.largestgeneration";
        let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
        let data = vectorcraft_format::base64_encode(&wasm);
        let grants = folder.0.join("grants.json");
        let edited = |entries: Value| json!({"version": 1, "generation": carried, "grants": entries, "mac": "0".repeat(64)}).to_string();
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
        assert_eq!(kept(&folder.0), Some(3));

        // The edited file names the grant, which the revoke takes out.
        std::fs::write(&grants, edited(json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}]))).unwrap();
        run(&mut s, "plugin.remove", json!({"id": id}));
        assert!(!grants.exists(), "generation {carried}: the entry is taken out");
        assert_eq!(kept(&folder.0), Some(5), "generation {carried}: the kept generation is raised from its own value");

        // The edited file names no grant for it.
        std::fs::write(&grants, edited(json!([]))).unwrap();
        assert!(s.store_grant(id, None), "generation {carried}: the revoke is recorded");
        assert_eq!(kept(&folder.0), Some(7), "generation {carried}: the kept generation is raised from its own value");

        run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
        assert_eq!((generation_of(&std::fs::read(&grants).unwrap()), kept(&folder.0)), (Some(10), Some(10)), "generation {carried}");
        assert_eq!(starts_from(&folder.0, id, &data), (true, false), "generation {carried}: a later grant is kept");
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        run(&mut s, "plugin.remove", json!({"id": id}));
    }
}

/// A signed write whose generation wasn't kept leaves a file one generation past the kept one. A
/// revoke raises the generation past that file too, so a copy of it put back grants nothing: a revoke
/// from a file without a MAC, one while the file is gone, one over a file that doesn't hold a list,
/// and one that removes the last grant of a signed file.
#[test]
fn a_write_whose_generation_wasnt_kept_grants_nothing_after_a_revoke() {
    use vectorcraft_plugins::permissions::sha256_hex;
    for path in ["unsigned", "gone", "not-a-list", "signed"] {
        let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
        let shared = SharedKeys::new(&keys);
        let fail = shared.file_generations_fail.clone();
        let _keys = ThreadKeys::using(shared);
        let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
        let folder = crate::tests_links::Folder::new(&format!("plugin-store-unkept-copy-{path}"));
        let id = "test.engine.permissions.unkeptcopy";
        let other = "test.engine.permissions.unkeptcopyother";
        let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
        let other_wasm = asking(other, r#"["doc.read"]"#, &json!({"objects": []}));
        let (data, other_data) = (vectorcraft_format::base64_encode(&wasm), vectorcraft_format::base64_encode(&other_wasm));
        let grants = folder.0.join("grants.json");
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
        fail.store(true, std::sync::atomic::Ordering::SeqCst);
        _ = s.execute("plugin.install", &json!({"dataBase64": other_data, "grant": ["doc.read"]}));
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let unkept = std::fs::read(&grants).unwrap();
        let before = kept(&folder.0).unwrap();
        assert_eq!(generation_of(&unkept), Some(before + 1), "{path}: the file is one generation past the kept one");

        let other_grant = json!([{"id": other, "hash": sha256_hex(&other_wasm), "permissions": ["doc.read"]}]).to_string();
        match path {
            "unsigned" => std::fs::write(&grants, &other_grant).unwrap(),
            "gone" => std::fs::remove_file(&grants).unwrap(),
            "not-a-list" => std::fs::write(&grants, br#""not a list""#).unwrap(),
            _ => std::fs::write(&grants, crate::store::signed_grants_file(&folder.0, &other_grant)).unwrap(),
        }
        assert!(s.store_grant(other, None), "{path}");
        std::fs::write(&grants, &unkept).unwrap();
        assert_eq!(starts_from(&folder.0, other, &other_data), (false, true), "{path}: the revoked grant doesn't come back");
        assert_eq!(starts_from(&folder.0, id, &data), (false, true), "{path}: the copy grants nothing");
        assert_eq!(kept(&folder.0), Some(before + 2), "{path}: the generation is past the file whose generation wasn't kept");
        std::fs::remove_file(&grants).unwrap();
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        _ = s.execute("plugin.remove", &json!({"id": id}));
        _ = s.execute("plugin.remove", &json!({"id": other}));
    }
}

/// A signed write that fails after raising the generation leaves the generation raised: the file
/// left in place, signed at the generation before the raise, grants nothing and its plug-ins ask
/// again, and so does a copy of it put back after a later write.
#[test]
fn a_grants_write_that_fails_leaves_the_generation_raised() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let folder = crate::tests_links::Folder::new("plugin-store-write-fails");
    let id = "test.engine.permissions.writefails";
    let other = "test.engine.permissions.writefailsother";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let other_data = vectorcraft_format::base64_encode(&asking(other, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let earlier = std::fs::read(&grants).unwrap();
    assert_eq!((generation_of(&earlier), kept(&folder.0)), (Some(3), Some(3)));

    crate::store::GRANTS_WRITE_FAILS.set(true);
    let grant = vectorcraft_plugins::permissions::Grant {
        id: other.into(),
        hash: vectorcraft_plugins::permissions::sha256_hex(b"module"),
        permissions: vec![vectorcraft_plugins::permissions::Permission::DocRead],
    };
    let written = s.store_grant(other, Some(grant));
    crate::store::GRANTS_WRITE_FAILS.set(false);
    assert!(!written, "the write reports a failure");
    assert_eq!(std::fs::read(&grants).unwrap(), earlier, "the file is left as it was");
    assert_eq!(kept(&folder.0), Some(5), "the generation stays raised");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the file left in place grants nothing");

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": other_data, "grant": ["doc.read"]}));
    assert_eq!((generation_of(&std::fs::read(&grants).unwrap()), kept(&folder.0)), (Some(8), Some(8)), "the next write is past the raise");
    assert_eq!(starts_from(&folder.0, other, &other_data), (true, false), "the next write keeps its grant");
    std::fs::write(&grants, &earlier).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the copy from before the raise grants nothing");
    std::fs::remove_file(&grants).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    _ = s.execute("plugin.remove", &json!({"id": id}));
    _ = s.execute("plugin.remove", &json!({"id": other}));
}

/// A revoke whose signed grants file can't be removed leaves the generation raised: the grant left in
/// the file doesn't come back, and a later removal takes the file away.
#[test]
fn a_revoke_whose_grants_file_cant_be_removed_doesnt_bring_the_grant_back() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let kept = |dir: &std::path::Path| kept_generation(&keys, dir);
    let folder = crate::tests_links::Folder::new("plugin-store-remove-fails");
    let id = "test.engine.permissions.removefails";
    let data = vectorcraft_format::base64_encode(&asking(id, r#"["doc.read"]"#, &json!({"objects": []})));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    let recorded = std::fs::read(&grants).unwrap();

    crate::store::GRANTS_DELETE_FAILS.set(true);
    let revoked = s.store_grant(id, None);
    crate::store::GRANTS_DELETE_FAILS.set(false);
    assert!(!revoked, "the removal reports a failure");
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the file is left as it was");
    assert_eq!(kept(&folder.0), Some(5), "the generation stays raised");
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the revoked grant doesn't come back");

    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(s.store_grant(id, None), "a later removal is recorded");
    assert!(!grants.exists(), "and takes the file away");
    std::fs::write(&grants, &recorded).unwrap();
    assert_eq!(starts_from(&folder.0, id, &data), (false, true), "the copy put back grants nothing");
    std::fs::remove_file(&grants).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    _ = s.execute("plugin.remove", &json!({"id": id}));
}

/// The key store's generation for a folder only goes up: a lower one is refused and the kept one
/// stays, while the same one and a higher one are kept.
#[test]
fn the_kept_grants_generation_is_never_lowered() {
    let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
    let _keys = ThreadKeys::using(SharedKeys::new(&keys));
    let folder = crate::tests_links::Folder::new("plugin-store-generation-lowered");
    let store = crate::store::plugin_store_key(&folder.0);
    assert!(crate::store::grant_keys::set_current_generation(&store, 5).is_ok());
    assert!(crate::store::grant_keys::set_current_generation(&store, 3).is_err(), "a lower generation is refused");
    assert_eq!(crate::store::grant_keys::current_generation(&store), Ok(5), "the kept one stays");
    assert!(crate::store::grant_keys::set_current_generation(&store, 5).is_ok(), "the same one is kept");
    assert!(crate::store::grant_keys::set_current_generation(&store, 6).is_ok());
    assert_eq!(crate::store::grant_keys::current_generation(&store), Ok(6));
}

/// A kept generation within two of the largest 64-bit value isn't raised: a signed write leaves the
/// file and the generation as they are, and the file keeps its grant. Three below it, the write goes
/// to the largest value.
#[test]
fn a_kept_generation_near_the_largest_value_isnt_raised() {
    use vectorcraft_plugins::permissions::{Grant, Permission, sha256_hex};
    for (kept_before, written) in [(u64::MAX - 2, None), (u64::MAX - 3, Some(u64::MAX))] {
        let keys = std::sync::Arc::new(crate::store::grant_keys::MemoryGrantKeys::default());
        let _keys = ThreadKeys::using(SharedKeys::new(&keys));
        let folder = crate::tests_links::Folder::new(&format!("plugin-store-generation-near-max-{}", u64::MAX - kept_before));
        let store = crate::store::plugin_store_key(&folder.0);
        let id = "test.engine.permissions.nearmax";
        let other = "test.engine.permissions.nearmaxother";
        let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
        let data = vectorcraft_format::base64_encode(&wasm);
        let grants = folder.0.join("grants.json");
        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
        crate::store::grant_keys::GrantKeyStore::set_generation(keys.as_ref(), &store, kept_before).unwrap();
        let file =
            crate::store::signed_grants_file(&folder.0, &json!([{"id": id, "hash": sha256_hex(&wasm), "permissions": ["doc.read"]}]).to_string());
        std::fs::write(&grants, &file).unwrap();
        assert_eq!(starts_from(&folder.0, id, &data), (true, false), "{kept_before}: the file grants");

        let mut s = session();
        s.attach_plugin_store(folder.0.clone());
        let grant = Grant { id: other.into(), hash: sha256_hex(b"module"), permissions: vec![Permission::DocRead] };
        assert_eq!(s.store_grant(other, Some(grant)), written.is_some(), "{kept_before}");
        match written {
            None => {
                assert_eq!(std::fs::read(&grants).unwrap(), file, "{kept_before}: the file is left as it is");
                assert_eq!(kept_generation(&keys, &folder.0), Some(kept_before), "{kept_before}: the generation too");
                assert_eq!(starts_from(&folder.0, id, &data), (true, false), "{kept_before}: the file keeps its grant");
            }
            Some(generation) => {
                assert_eq!(
                    (generation_of(&std::fs::read(&grants).unwrap()), kept_generation(&keys, &folder.0)),
                    (Some(generation), Some(generation))
                );
                assert_eq!(starts_from(&folder.0, id, &data), (true, false), "{kept_before}: the new file keeps the grant");
            }
        }
        std::fs::remove_file(&grants).unwrap();
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    }
}

/// The user agreed once: a heavy filter kept in the plug-in store is installed again at the next
/// start without asking, with its budget.
#[test]
fn a_stored_heavy_filter_keeps_its_budget() {
    let folder = crate::tests_links::Folder::new("plugin-store-heavy");
    let id = "test.engine.storedheavy";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let plugin = vectorcraft_plugins::registry::get(id).expect("installed again from the store");
    assert_eq!(plugin.limits().fuel_base, vectorcraft_plugins::HEAVY_FUEL);
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// The app runs a heavy filter away from the session ([`cmd::plugin::FilterRun`]) and hands its
/// output in: `plugin.run` with the same params applies it, as one undo step, without running the
/// plug-in again. Only once, and only for the same run: objects that changed since, or the plug-in
/// installed again, run the plug-in instead.
#[test]
fn a_filter_run_made_elsewhere_is_applied_by_plugin_run() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.handin";
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    let p = json!({"id": id, "ids": []});
    let named = |s: &Session, ids: &Value| s.doc().unwrap().doc.node(NodeId(ids[0].as_u64().unwrap())).unwrap().name.clone();
    // What it computed, as the app hands it in; here marked, so the test sees which output applied.
    let r = FilterRun::prepare(&s, &p).unwrap();
    assert!(r.current(&s));
    let output = String::from_utf8(r.run().unwrap()).unwrap().replace("Plug-in", "Handed in");
    r.hand_in(&mut s, output.clone().into_bytes());
    let undo = s.doc().unwrap().history.undo.len();
    let made = run(&mut s, "plugin.run", p.clone())["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Handed in"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Once only: the next run runs the plug-in.
    let made = run(&mut s, "plugin.run", p.clone())["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"));
    // Made for objects that changed since, it isn't theirs.
    let obj = rect(&mut s, 10.0, "#ff0000");
    let on = json!({"id": id, "ids": [obj.0]});
    let stale = FilterRun::prepare(&s, &on).unwrap();
    run(&mut s, "paint.setFill", json!({"ids": [obj.0], "color": "#00ff00"}));
    assert!(!stale.current(&s), "the document changed");
    stale.hand_in(&mut s, output.clone().into_bytes());
    let made = run(&mut s, "plugin.run", on)["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"), "made for objects that changed since");
    // A plug-in installed again is another one.
    let again = FilterRun::prepare(&s, &p).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    assert!(!again.current(&s));
    again.hand_in(&mut s, output.into_bytes());
    let made = run(&mut s, "plugin.run", p)["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"), "made by the plug-in installed before");
}

/// A heavy filter run away from the session reads what it keeps with the document
/// (`_context.documentData`) and hands its `documentData` back with its output: `plugin.run`
/// applies both as one undo step.
#[test]
fn a_filter_run_made_elsewhere_keeps_its_document_data() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.handindata";
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","budget":"heavy"}}"#);
    let install = |s: &mut Session, src: String| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true, "replace": true}));
    };
    // It first keeps the pens it found with the document.
    install(&mut s, tpl::returning(&manifest, &json!({"objects": [], "documentData": {"pens": 2}}).to_string()));
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    // Given those pens, it adds one, in the background; the output handed in is marked, so the
    // test sees it is the one that applied.
    let next = json!({"objects": [{"path": triangle(), "name": "Pen 3"}], "documentData": {"pens": 3}}).to_string();
    install(&mut s, tpl::when_given(&manifest, tpl::Given::Params, r#""documentData":{"pens":2}"#, &next));
    let p = json!({"id": id, "ids": []});
    let r = FilterRun::prepare(&s, &p).unwrap();
    let output = String::from_utf8(r.run().unwrap()).unwrap().replace("Pen 3", "Handed in");
    r.hand_in(&mut s, output.into_bytes());
    let undo = s.doc().unwrap().history.undo.len();
    let made = run(&mut s, "plugin.run", p)["ids"].clone();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.node(NodeId(made[0].as_u64().unwrap())).unwrap().name.as_deref(), Some("Handed in"));
    assert_eq!(st.doc.plugin_data(id), Some(&json!({"pens": 3})));
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.plugin_data(id), Some(&json!({"pens": 2})), "undone with its objects");
}

/// A plug-in of API level 2 that uses the host imports lists `progress`, which the user grants like
/// any permission; a filter run made elsewhere then reports its progress, as a share, to the
/// progress the caller shows, and sees the caller's Cancel.
#[test]
fn a_level_2_filter_installs_with_progress_granted_and_reports_to_the_caller() {
    use crate::cmd::fileio::progress::{self, Progress};
    use crate::cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.level2";
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write","progress"]}}"#);
    let wasm = wat::parse_str(tpl::cooperating(&manifest, 1000, r#"{"objects":[]}"#, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&wasm);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data}));
    assert_eq!((&inspected["needsApproval"], &inspected["permissions"]), (&json!(true), &json!(["doc.read", "doc.write", "progress"])));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read", "doc.write"]})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read, doc.write, progress"), "{e}");
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write", "progress"]}));
    let shown = Progress::new();
    let r = FilterRun::prepare(&s, &json!({"id": id})).unwrap();
    assert!(progress::scope(&shown, || r.run()).is_ok());
    assert_eq!((shown.counts(), shown.counts_share()), ((999, 1000), true));
    // Cancelled, it stops on its first step, and what it returns then is discarded.
    shown.cancel();
    let cancelled = progress::scope(&shown, || r.run());
    assert!(matches!(cancelled, Err(EngineError::Cancelled)), "{cancelled:?}");
    assert_eq!(shown.counts(), (0, 1000));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// An approval of the module `wasm` with id `id`, its `permissions` and, if `heavy`, its budget.
fn approval(id: &str, wasm: &[u8], permissions: &[&str], heavy: bool) -> crate::cmd::plugin::InstallApproval {
    crate::cmd::plugin::InstallApproval {
        id: id.into(),
        hash: vectorcraft_plugins::permissions::sha256_hex(wasm),
        permissions: permissions.iter().map(|p| p.to_string()).collect(),
        heavy,
    }
}

/// In a session that wasn't started to accept them, `grant` and `allowHeavy` are refused whichever
/// way the command runs, a batch included, until an approval covers the module, its id, SHA-256 and
/// permissions. An approval is spent by the first install, whatever it does. A hash already approved
/// still installs.
#[test]
fn a_request_cannot_grant_what_only_a_person_approves() {
    let mut s = strict_session();
    let id = "test.engine.approval";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let refused = "take effect only in a session started with --allow-plugin-grants";
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("approve them in the desktop app") && e.contains("--allow-plugin-grants"), "{e}");
    let batch = json!({"commands": [{"command": "plugin.install", "params": {"dataBase64": data, "grant": ["doc.read"]}}]});
    let e = s.execute("command.batch", &batch).unwrap_err().to_string();
    assert!(e.contains(refused), "a batch can't grant: {e}");
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    // Approved for other bytes, for another id or for other permissions: refused, and spent.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [{"name": "x"}]}));
    for wrong in [
        approval(id, &other, &["doc.read"], false),
        approval("test.engine.approval.other", &wasm, &["doc.read"], false),
        approval(id, &wasm, &["doc.read", "state"], false),
    ] {
        s.approve_plugin_install(wrong);
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
        assert!(e.contains(refused), "{e}");
    }
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    // The approval for this module installs it once.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "the approval was spent: {e}");
    // A hash already approved installs without one.
    run(&mut s, "plugin.install", json!({"dataBase64": data}));
    run(&mut s, "plugin.remove", json!({"id": id}));

    // A refused install spends the approval too.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"], "hash": "0"})).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    // So does one refused for its params.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    s.execute("plugin.install", &json!({"dataBase64": data, "grant": "doc.read"})).unwrap_err();
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");

    // The heavy budget needs its own agreement.
    let heavy_id = "test.engine.approval.heavy";
    let heavy_wasm = heavy(heavy_id, 1);
    let heavy_data = vectorcraft_format::base64_encode(&heavy_wasm);
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data, "allowHeavy": true})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    // Without a `permissions` list it has the ones its kind implies.
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": heavy_data}));
    let implied: Vec<&str> = inspected["permissions"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    s.approve_plugin_install(approval(heavy_id, &heavy_wasm, &implied, false));
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data, "allowHeavy": true})).unwrap_err().to_string();
    assert!(e.contains(refused), "an approval without the heavy budget doesn't cover it: {e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data})).unwrap_err().to_string();
    assert!(e.contains("approve it in the desktop app"), "{e}");
    s.approve_plugin_install(approval(heavy_id, &heavy_wasm, &implied, true));
    run(&mut s, "plugin.install", json!({"dataBase64": heavy_data, "allowHeavy": true}));
    run(&mut s, "plugin.remove", json!({"id": heavy_id}));

    // A session started to accept them takes them from requests.
    s.allow_plugin_grants_from_requests();
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module a person approved in the desktop app, recorded in the store's `grants.json`, installs
/// from another session with that store and no flag and no `grant`. Other bytes under its id are
/// refused there, with or without `grant`.
#[test]
fn a_module_approved_in_the_app_installs_from_a_strict_session_with_its_store() {
    let folder = crate::tests_links::Folder::new("plugin-approval-store");
    let id = "test.engine.approval.store";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let mut desktop = strict_session();
    desktop.attach_plugin_store(folder.0.clone());
    desktop.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    run(&mut desktop, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert!(std::fs::read_to_string(folder.0.join("grants.json")).unwrap().contains(id), "recorded in the store");

    // Another process starts with an empty registry and the store's grants.
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    forget();
    let mut headless = strict_session();
    headless.attach_plugin_store(folder.0.clone());
    forget();
    run(&mut headless, "plugin.install", json!({"dataBase64": data}));
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));

    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [{"name": "x"}]}));
    let other_data = vectorcraft_format::base64_encode(&other);
    let e = headless.execute("plugin.install", &json!({"dataBase64": other_data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("approve them in the desktop app"), "{e}");
    let e = headless.execute("plugin.install", &json!({"dataBase64": other_data, "replace": true, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains("take effect only in a session started with --allow-plugin-grants"), "{e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    run(&mut headless, "plugin.remove", json!({"id": id}));
}

/// A filter run the caller cancels (MCP's `notifications/cancelled`, the app's Cancel) stops between
/// two slices of its budget and changes nothing.
#[test]
fn a_cancelled_filter_run_changes_nothing() {
    let mut s = session();
    let id = "test.engine.cancelled";
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1 << 40)), "allowHeavy": true}));
    let before = s.doc().unwrap().doc.clone();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || s.execute("plugin.run", &json!({"id": id})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(s.doc().unwrap().doc, before);
}

/// Preferences › Plug-ins & Scratch Disks › Plug-in Instruction Budget: with Normal a light filter
/// runs out of instructions and changes nothing; with Large the same installed filter finishes, as
/// one undo step, without being installed again. The time budget, Cancel and a heavy filter's
/// larger budget stay as they are, and setting Normal again applies to the next run.
#[test]
fn the_plugin_instruction_budget_preference_sets_a_filter_run_s_instructions() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.largebudget";
    let output = json!({"objects": [{"path": triangle(), "name": "Plug-in"}]}).to_string();
    // More steps than a light budget pays for, fewer than 5 times it.
    install_wat(&mut s, &tpl::spinning(&tpl::manifest(id, "filter", "{}"), 20_000_000, &output));
    let p = json!({"id": id, "ids": []});
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute("plugin.run", &p).unwrap_err().to_string();
    assert!(e.contains("instruction budget"), "{e}");
    assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&before, undo));

    run(&mut s, "prefs.set", json!({"key": "pluginInstructionBudget", "value": "large"}));
    let light = vectorcraft_plugins::Limits::default();
    let large = FilterRun::prepare(&s, &p).unwrap().limits().clone();
    assert_eq!(large, vectorcraft_plugins::Limits { fuel_base: vectorcraft_plugins::LARGE_FUEL, ..light.clone() });
    assert_eq!((large.fuel_per_byte, large.wall_time_ms), (light.fuel_per_byte, light.wall_time_ms));
    // Cancel still stops it, and it changes nothing.
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || s.execute("plugin.run", &p));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&before, undo));
    let made = run(&mut s, "plugin.run", p.clone())["ids"].clone();
    assert_eq!(made.as_array().map(Vec::len), Some(1));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);

    // A heavy filter keeps its larger budget with either choice.
    let heavy_id = "test.engine.largebudget.heavy";
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(heavy_id, 1)), "allowHeavy": true}));
    let hp = json!({"id": heavy_id, "ids": []});
    for budget in ["large", "normal"] {
        run(&mut s, "prefs.set", json!({"key": "pluginInstructionBudget", "value": budget}));
        let limits = FilterRun::prepare(&s, &hp).unwrap().limits().clone();
        let heavy_limits = (vectorcraft_plugins::HEAVY_FUEL, vectorcraft_plugins::HEAVY_WALL_TIME_MS);
        assert_eq!((limits.fuel_base, limits.wall_time_ms), heavy_limits, "{budget}");
    }

    // Normal again: the next run runs out of instructions.
    let undo = s.doc().unwrap().history.undo.len();
    let e = s.execute("plugin.run", &p).unwrap_err().to_string();
    assert!(e.contains("instruction budget"), "{e}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "plugin.remove", json!({"id": heavy_id}));
}

/// The manifest of a filter `id` named "Cmd Test" of `kind` with `commands` (a JSON list).
fn with_commands(id: &str, kind: &str, commands: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"{kind}","commands":{commands}}}"#)
}

/// Two commands: Grow, with a parameter, and Tidy, whose `menu` names another menu.
const GROW_AND_TIDY: &str = r#"[{"id":"grow","label":"Grow","params":{"by":{"type":"number","min":0,"max":10,"default":2}}},
    {"id":"tidy","label":"Tidy","menu":"File"}]"#;

/// A plug-in that adds a triangle when it runs as its command `grow` (`_context.command`) and
/// returns its input unchanged otherwise.
fn grower(id: &str) -> String {
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    tpl::when_given(&with_commands(id, "filter", GROW_AND_TIDY), tpl::Given::Params, r#""command":"grow""#, triangle)
}

/// The command listing's entry for `id`.
fn listed(s: &Session, id: &str) -> Option<crate::cmd::CommandInfo> {
    s.commands().into_iter().find(|c| c.id == id)
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

/// A plug-in's commands are commands of their own while it is installed: listed under Object ›
/// Plug-ins › its name whatever menu the manifest names, run with their checked parameters and
/// `_context.command` as one undo step named after them, and gone once it is removed or replaced
/// by a module without them.
#[test]
fn plugin_commands_are_registered_run_and_removed() {
    let mut s = session();
    let id = "test.engine.cmds";
    let (grow, tidy) = ("plugin.test.engine.cmds.grow", "plugin.test.engine.cmds.tidy");
    let info = install_wat(&mut s, &grower(id));
    assert_eq!(info["commands"][0]["command"], grow);
    assert_eq!(info["commands"][1]["label"], "Tidy");
    for (cmd, label) in [(grow, "Grow"), (tidy, "Tidy")] {
        let c = listed(&s, cmd).unwrap_or_else(|| panic!("{cmd} is listed"));
        assert_eq!((c.label, c.menu.as_slice()), (label, &["Object", "Plug-ins", "Cmd Test"][..]), "{cmd}");
        assert!(c.enabled, "{cmd}");
    }
    assert!(crate::cmd::find_command(grow).is_some());

    run(&mut s, "select.none", json!({}));
    let (before, undo) = (objects(&s), s.doc().unwrap().history.undo.len());
    let r = run(&mut s, grow, json!({"by": 3}));
    assert_eq!((r["plugin"].as_str(), r["command"].as_str()), (Some(id), Some("grow")));
    assert_eq!(objects(&s), before + 1, "the command added its triangle");
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Grow"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(objects(&s), before);
    // Its parameters are checked before it runs.
    let e = s.execute(grow, &json!({"by": "lots"})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { cmd, .. } if cmd == grow), "{e}");
    assert_eq!(objects(&s), before);
    // Another command of the same plug-in is told apart by its id.
    run(&mut s, tidy, json!({}));
    assert_eq!(objects(&s), before, "tidy returns its input unchanged");

    // Replaced by a module without commands, its commands are gone; installed again, back.
    install_wat(&mut s, &tpl::module(&tpl::manifest(id, "filter", "{}"), tpl::ECHO, "", 1));
    assert!(listed(&s, grow).is_none() && crate::cmd::find_command(grow).is_none());
    install_wat(&mut s, &grower(id));
    assert!(listed(&s, grow).is_some() && crate::cmd::find_command(grow).is_some());
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(listed(&s, grow).is_none() && listed(&s, tidy).is_none());
    assert!(crate::cmd::find_command(grow).is_none() && crate::cmd::find_command(tidy).is_none());
    assert!(s.execute(grow, &json!({})).is_err(), "removed, it no longer runs");
}

/// At API level 2 a command's `menu` lists it in that menu with its plug-in's name after its
/// label, and `plugin.list` gives its `menu` with `after` escaped. A command whose label reads like
/// a built-in command of that menu, or like a command a plug-in with a lower id places there,
/// stays in its plug-in's submenu, and `plugin.list` marks it `placed: false`. Replaced by a module
/// whose command has no `menu`, a command is back in its submenu.
#[test]
fn placed_plugin_commands_are_listed_in_their_menu() {
    let mut s = session();
    let placing = |id: &str, name: &str, commands: &str| {
        let manifest = format!(r#"{{"id":"{id}","name":"{name}","api":2,"version":"1","kind":"filter","commands":{commands}}}"#);
        tpl::returning(&manifest, r#"{"objects":[]}"#)
    };
    let (ant, bee, cat) = ("test.engine.place.ant", "test.engine.place.bee", "test.engine.place.cat");
    let low = |id: &str| crate::cmd::plugin::command_id(id, "low");
    let menu_of = |s: &Session, command: &str| listed(s, command).map(|c| c.menu.iter().map(|m| m.to_string()).collect::<Vec<_>>());
    let in_select = json!([{"id": "low", "label": "Low Area", "menu": {"path": ["Select"], "after": "\u{202E}Inverse"}}]).to_string();
    let in_select = in_select.as_str();

    let info = install_wat(&mut s, &placing(ant, "Tidy", in_select));
    let menu = &info["commands"][0]["menu"];
    assert_eq!(menu["path"], json!(["Select"]));
    assert!(menu["after"].as_str().is_some_and(|a| a.ends_with("Inverse") && !a.contains('\u{202E}')), "escaped: {menu}");
    assert!(menu.get("placed").is_none(), "{menu}");
    let c = listed(&s, &low(ant)).expect("listed");
    assert_eq!((c.label, menu_of(&s, &low(ant))), ("Low Area (Tidy)", Some(vec!["Select".to_string()])));

    // Another plug-in of the same name placing the same label at the same spot stays in its submenu.
    let info = install_wat(&mut s, &placing(bee, "Tidy", in_select));
    assert_eq!(info["commands"][0]["menu"]["placed"], json!(false));
    assert_eq!(listed(&s, &low(bee)).map(|c| c.label), Some("Low Area"));
    assert_eq!(menu_of(&s, &low(bee)), Some(vec!["Object".into(), "Plug-ins".into(), "Tidy".into()]));
    assert_eq!(menu_of(&s, &low(ant)), Some(vec!["Select".to_string()]));

    // `Paste (in Place)` reads like Edit's Paste in Place; Keep is placed.
    let edit = r#"[{"id":"paste","label":"Paste","menu":{"path":["Edit"]}},{"id":"keep","label":"Keep","menu":{"path":["Edit"]}}]"#;
    let info = install_wat(&mut s, &placing(cat, "in Place", edit));
    assert_eq!(info["commands"][0]["menu"]["placed"], json!(false));
    assert!(info["commands"][1]["menu"].get("placed").is_none());
    let paste = crate::cmd::plugin::command_id(cat, "paste");
    assert_eq!(menu_of(&s, &paste), Some(vec!["Object".into(), "Plug-ins".into(), "in Place".into()]));
    assert_eq!(menu_of(&s, &crate::cmd::plugin::command_id(cat, "keep")), Some(vec!["Edit".to_string()]));
    // `Stop Global (Edit)` reads like the other label of Select's Start Global Edit.
    let dog = "test.engine.place.dog";
    let info = install_wat(&mut s, &placing(dog, "Edit", r#"[{"id":"stop","label":"Stop Global","menu":{"path":["Select"]}}]"#));
    assert_eq!(info["commands"][0]["menu"]["placed"], json!(false));
    run(&mut s, "plugin.remove", json!({"id": dog}));
    // Object's Show All has one label, so `Hide (All)` is placed beside it.
    let all = "test.engine.place.all";
    let info = install_wat(&mut s, &placing(all, "All", r#"[{"id":"hide","label":"Hide","menu":{"path":["Object"]}}]"#));
    assert!(info["commands"][0]["menu"].get("placed").is_none(), "{}", info["commands"][0]);
    assert_eq!(menu_of(&s, &crate::cmd::plugin::command_id(all, "hide")), Some(vec!["Object".to_string()]));
    run(&mut s, "plugin.remove", json!({"id": all}));
    // `Hide (Edges)` reads like the desktop app's View › Hide Edges, with or without the desktop app.
    let edges = "test.engine.place.edges";
    let info = install_wat(&mut s, &placing(edges, "Edges", r#"[{"id":"hide","label":"Hide","menu":{"path":["View"]}}]"#));
    assert_eq!(info["commands"][0]["menu"]["placed"], json!(false));
    run(&mut s, "plugin.remove", json!({"id": edges}));
    // Inspecting a plug-in that is not installed reports what installing it gives: of two commands
    // with one label at one spot, the second stays in its submenu.
    let twice = r#"[{"id":"a","label":"Low","menu":{"path":["Select"]}},{"id":"b","label":"Low","menu":{"path":["Select"]}}]"#;
    let data = vectorcraft_format::base64_encode(&wat::parse_str(placing("test.engine.place.eel", "Eel", twice)).unwrap());
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data}));
    assert!(inspected["commands"][0]["menu"].get("placed").is_none(), "{inspected}");
    assert_eq!(inspected["commands"][1]["menu"]["placed"], json!(false), "{inspected}");

    // Replaced by a module whose command has no `menu`, Ant's command is back in its submenu and
    // Bee's takes the spot.
    install_wat(&mut s, &placing(ant, "Tidy", r#"[{"id":"low","label":"Low Area"}]"#));
    assert_eq!(menu_of(&s, &low(ant)), Some(vec!["Object".into(), "Plug-ins".into(), "Tidy".into()]));
    assert_eq!(menu_of(&s, &low(bee)), Some(vec!["Select".to_string()]));
    for id in [ant, bee, cat] {
        run(&mut s, "plugin.remove", json!({"id": id}));
    }
    assert!(listed(&s, &low(bee)).is_none());
}

/// A command run is journaled like any other, so an action records it with its parameters and
/// plays it back.
#[test]
fn actions_record_and_play_plugin_commands() {
    let mut s = session();
    let id = "test.engine.cmds.actions";
    let grow = crate::cmd::plugin::command_id(id, "grow");
    install_wat(&mut s, &grower(id));
    run(&mut s, "select.none", json!({}));
    let before = objects(&s);
    let start = s.journal.len();
    run(&mut s, &grow, json!({"by": 4}));
    let steps = s.journal_for_action(start);
    assert_eq!(steps, [(grow.clone(), json!({"by": 4}))]);
    run(&mut s, "edit.undo", json!({}));
    let commands: Vec<Value> = steps.iter().map(|(c, p)| json!({"command": c, "params": p})).collect();
    // The triangle a run adds is selected, and the next run would be given it: deselect between.
    let none = json!({"command": "select.none", "params": {}});
    run(&mut s, "command.batch", json!({"label": "Grow Twice", "commands": [commands[0].clone(), none, commands[0].clone()]}));
    assert_eq!(objects(&s), before + 2, "both steps ran");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A command that traps leaves the document and its history as they were, and the error names
/// the plug-in and the command.
#[test]
fn a_failing_plugin_command_changes_nothing_and_names_itself() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let id = "test.engine.cmds.trap";
    install_wat(&mut s, &tpl::module(&with_commands(id, "filter", r#"[{"id":"boom","label":"Boom"}]"#), "unreachable", "", 1));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "boom"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("“Cmd Test” › Boom: ") && e.contains("trapped"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A command that runs out of its instruction budget leaves the document and its history as they
/// were, and the error names the plug-in and the command.
#[test]
fn a_plugin_command_out_of_budget_changes_nothing_and_names_itself() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let id = "test.engine.cmds.spin";
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    // More steps than a light budget pays for (as the plug-in crate's budget test counts them).
    install_wat(&mut s, &tpl::spinning(&with_commands(id, "filter", r#"[{"id":"spin","label":"Spin"}]"#), 100_000_000, triangle));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "spin"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("“Cmd Test” › Spin: ") && e.contains("instruction budget"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// An `ids` that is not a list, or that holds an entry that is not an object id, is an error naming
/// `ids` for a plug-in command and for `plugin.run`, and is never read as the selection; left out
/// or null, `ids` is the selection.
#[test]
fn a_malformed_ids_is_refused_not_read_as_the_selection() {
    let mut s = session();
    let id = "test.engine.cmds.ids";
    let grow = crate::cmd::plugin::command_id(id, "grow");
    install_wat(&mut s, &grower(id));
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let cases = [
        (grow.as_str(), json!(5)),
        (grow.as_str(), json!([1, "x"])),
        (grow.as_str(), json!([-1])),
        ("plugin.run", json!("all")),
        ("plugin.run", json!([1.5])),
    ];
    for (cmd, ids) in cases {
        let mut p = json!({"ids": ids});
        if cmd == "plugin.run" {
            p["id"] = json!(id);
        }
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { cmd: c, .. } if c == cmd), "{cmd} {ids}: {e}");
        let said = e.to_string();
        assert!(
            said.contains("ids must be an array of non-negative integer object ids") || said.contains("not a non-negative integer object id"),
            "{cmd} {ids}: {e}"
        );
        assert_eq!(s.doc().unwrap().doc, before, "{cmd} {ids}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{cmd} {ids}");
    }
    // Left out or null, the command runs on the selection.
    for p in [json!({}), json!({"ids": null})] {
        let r = run(&mut s, &grow, p.clone());
        assert_eq!(r["command"], "grow", "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc().unwrap().doc, before, "{p}");
    }
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A live effect's commands are listed under Effect › Plug-ins and run under its permissions (with
/// commands it gets `doc.write` unless its list leaves it out), and labels show bidirectional
/// characters escaped. A malformed
/// `commands` entry is refused at install, and the error names it.
#[test]
fn effect_commands_labels_and_malformed_entries() {
    let mut s = session();
    let id = "test.engine.cmds.effect";
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    install_wat(&mut s, &tpl::returning(&with_commands(id, "effect", "[{\"id\":\"flip\",\"label\":\"Flip\u{202E}txt\"}]"), triangle));
    let c = listed(&s, &crate::cmd::plugin::command_id(id, "flip")).expect("listed");
    assert_eq!((c.label, c.menu.as_slice()), ("Flip\\u{202E}txt", &["Effect", "Plug-ins", "Cmd Test"][..]));
    run(&mut s, "select.none", json!({}));
    // Without `permissions`, a live effect with commands may change the document: its command adds
    // the triangle, as one undo step.
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let out = run(&mut s, &crate::cmd::plugin::command_id(id, "flip"), json!({}));
    assert_eq!(out["ids"].as_array().map(Vec::len), Some(1), "{out}");
    assert_ne!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "plugin.remove", json!({"id": id}));
    // One that lists its permissions without `doc.write` is refused the change, and the document
    // stays as it was.
    let listed = format!(
        r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"effect","permissions":["doc.read","effect.render"],"commands":[{{"id":"flip","label":"Flip"}}]}}"#
    );
    let wasm = wat::parse_str(tpl::returning(&listed, triangle)).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(&wasm);
    let grant = json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read", "effect.render"], "hash": hash});
    run(&mut s, "plugin.install", grant);
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "flip"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `doc.write`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));

    let bad = id.replace("effect", "bad");
    let wasm = wat::parse_str(tpl::module(&with_commands(&bad, "filter", r#"[{"id":"ok","label":"A"},{"id":"Bad","label":"B"}]"#), tpl::ECHO, "", 1));
    let e = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm.unwrap())})).unwrap_err().to_string();
    assert!(e.contains("command `Bad`"), "{e}");
    assert!(vectorcraft_plugins::registry::get(&bad).is_none());
}

/// A filter `id` of API level 2 that lists `settings` and `cache`, whose `vc_run` runs `first`
/// (instructions that trap unless each storage call returns what it should), with the keys "zoom"
/// at 0x2000 and "tile" at 0x2004 and the value "hello" at 0x2100.
fn storing(id: &str, first: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["settings","cache"]}}"#);
    let data = format!(r#"{} (data (i32.const 0x2000) "zoomtile") (data (i32.const 0x2100) "hello")"#, tpl::STORAGE_IMPORTS);
    wat::parse_str(tpl::with_imports(&manifest, &data, first, r#"{"objects":[]}"#)).unwrap()
}

/// Sets "zoom" to "hello" and writes "hel" to the cache entry "tile".
const WRITES: &str = "(if (call $vc_settings_set (i32.const 0x2000) (i32.const 4) (i32.const 0x2100) (i32.const 5)) (then unreachable))
    (if (call $vc_cache_write (i32.const 0x2004) (i32.const 4) (i32.const 0x2100) (i32.const 3)) (then unreachable))";

/// Traps unless "zoom" is "hello".
const READS: &str =
    "(if (i32.ne (call $vc_settings_get (i32.const 0x2000) (i32.const 4) (i32.const 0x3000) (i32.const 16)) (i32.const 5)) (then unreachable))
    (if (i32.ne (i32.load (i32.const 0x3000)) (i32.load (i32.const 0x2100))) (then unreachable))";

fn install_storing(s: &mut Session, id: &str, first: &str) -> Value {
    let data = vectorcraft_format::base64_encode(&storing(id, first));
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"]}))
}

/// [`install_storing`] in place of another module with the same id (`replace: true`).
fn replace_storing(s: &mut Session, id: &str, first: &str) -> Value {
    let data = vectorcraft_format::base64_encode(&storing(id, first));
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}))
}

/// A plug-in's settings and cache are kept in the plug-in store under its id: the next session
/// has them, replacing the plug-in keeps them, another plug-in can't read them, `plugin.clearCache`
/// with its id empties its cache alone and `plugin.remove` deletes both. The caps and the bytes
/// each call is charged follow ([`plugin_storage_is_capped_and_survives_a_damaged_settings_file`],
/// [`plugin_storage_calls_are_charged_the_files_they_read_and_write`]) in the same test.
/// `plugin.clearCache` without an id empties the cache of every plug-in installed in the process,
/// so it is tested in a binary of its own (`tests/plugin_clear_cache.rs`).
#[test]
fn plugins_keep_settings_and_a_cache_of_their_own_in_the_store() {
    let folder = crate::tests_links::Folder::new("plugin-storage");
    let (id, other) = ("test.engine.storing", "test.engine.storing.other");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    assert!(dir.join("settings.json").is_file());
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));

    // A restart: installed again from the store, and replaced by a module that reads.
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let listed = run(&mut next, "plugin.list", json!({}));
    let entry = listed["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
    assert_eq!(entry["cacheBytes"], json!(3));
    assert_eq!(replace_storing(&mut next, id, READS)["cacheBytes"], json!(3));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));

    // Another plug-in has settings of its own: "zoom" isn't set there.
    install_storing(&mut next, other, READS);
    assert!(next.execute("plugin.run", &json!({"id": other, "ids": []})).is_err());

    // Clearing the cache leaves the settings, and the other plug-in's cache.
    replace_storing(&mut next, other, WRITES);
    run(&mut next, "plugin.run", json!({"id": other, "ids": []}));
    assert_eq!(run(&mut next, "plugin.clearCache", json!({"id": id})), json!({"cleared": [id], "bytesFreed": 3}));
    assert_eq!(run(&mut next, "plugin.info", json!({"id": id}))["cacheBytes"], json!(0));
    assert_eq!(run(&mut next, "plugin.info", json!({"id": other}))["cacheBytes"], json!(3));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));
    assert!(next.execute("plugin.clearCache", &json!({"id": "test.engine.storing.missing"})).is_err());

    run(&mut next, "plugin.remove", json!({"id": id}));
    assert!(!dir.exists(), "removed with the plug-in");
    run(&mut next, "plugin.remove", json!({"id": other}));
    plugin_storage_is_capped_and_survives_a_damaged_settings_file();
    plugin_storage_calls_are_charged_the_files_they_read_and_write();
}

/// A `storage` folder in the plug-in store that is a link leads out of the store: the plug-in
/// installs without `replace` although a folder for its id is there, its storage calls fail as
/// unavailable, and removing the plug-in deletes nothing the link leads to.
#[test]
fn plugin_storage_behind_a_link_is_left_alone() {
    let folder = crate::tests_links::Folder::new("plugin-storage-link");
    let outside = crate::tests_links::Folder::new("plugin-storage-outside");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&outside.0, folder.0.join("storage")).is_ok();
    // Without Developer Mode or elevation Windows makes no symbolic link, but a junction.
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&outside.0, folder.0.join("storage")).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(folder.0.join("storage"))
            .arg(&outside.0)
            .output()
            .is_ok_and(|o| o.status.success());
    #[cfg(not(any(unix, windows)))]
    let made = false;
    if !made {
        eprintln!("skipped: symbolic links can't be made here");
        return;
    }
    let id = "test.engine.storagelink";
    let kept = crate::store::plugin_data_dir(&folder.0, id).join("settings.json");
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"{}").unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let m = &mut vectorcraft_plugins::storage::Meter::unlimited();
    assert_eq!(st.set_setting("zoom", b"hello", m), Err(vectorcraft_plugins::storage::StorageError::Unavailable));
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(kept.is_file(), "left where the link leads");
}

/// `plugin.remove` with the id of a plug-in that isn't installed deletes the settings and cache the
/// plug-in store still keeps for it; with neither, it fails as before.
#[test]
fn removing_an_id_that_is_not_installed_deletes_its_storage() {
    let folder = crate::tests_links::Folder::new("plugin-storage-orphan");
    let id = "test.engine.storageorphan";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("settings.json"), b"{}").unwrap();
    std::fs::write(dir.join("cache").join("tile.bin"), b"abc").unwrap();
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(run(&mut s, "plugin.remove", json!({"id": id})), json!({"removed": id}));
    assert!(!dir.exists(), "the storage of an id that isn't installed is deleted");
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err(), "nothing left to remove");
}

/// The `plugin.*` commands that the tests running every command with `{}` run
/// ([`crate::runs_with_empty_params`]) leave the cache of an installed plug-in as it is, so
/// those tests can run beside the ones here that check a cache. Each command runs with `{}` in a
/// document with two rectangles, one or both selected, as `every_command_survives_empty_params`
/// runs it.
#[test]
fn plugin_commands_run_with_empty_params_keep_other_plugins_caches() {
    let folder = crate::tests_links::Folder::new("plugin-storage-empty-params");
    let id = "test.engine.storageemptyparams";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));
    let mut ran = vec![];
    for all in [false, true] {
        for c in crate::cmd::command_specs().iter().filter(|c| c.id.starts_with("plugin.") && crate::runs_with_empty_params(c.id)) {
            let mut other = session();
            let a = rect(&mut other, 10.0, "#ff0000");
            rect(&mut other, 40.0, "#0000ff");
            if all {
                run(&mut other, "select.all", json!({}));
            } else {
                run(&mut other, "select.set", json!({"ids": [a.0]}));
            }
            let _ = other.execute(c.id, &json!({}));
            ran.push(c.id);
            assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3), "after {}", c.id);
        }
    }
    assert!(ran.contains(&"plugin.list"), "{ran:?}");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// The settings file and the cache are checked: a file that doesn't read is moved aside and the
/// settings start empty; a write past a cap fails and changes nothing; cache entries whose names
/// differ only in case are two files; a removed plug-in's storage takes no more writes.
fn plugin_storage_is_capped_and_survives_a_damaged_settings_file() {
    use vectorcraft_plugins::storage::{self, Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-caps");
    let id = "test.engine.storagecaps";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&dir).unwrap();
    let m = &mut Meter::unlimited();

    // A truncated settings file.
    std::fs::write(dir.join("settings.json"), br#"{"zoom": "aGVsbG"#).unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), None);
    assert!(dir.join("settings.unreadable.json").is_file() && !dir.join("settings.json").exists());
    st.set_setting("zoom", b"hello", m).unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), Some(b"hello".to_vec()));
    // Damaged again, it replaces the file moved aside before.
    std::fs::write(dir.join("settings.json"), b"[").unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), None);
    assert_eq!(std::fs::read(dir.join("settings.unreadable.json")).unwrap(), b"[");
    st.set_setting("zoom", b"hello", m).unwrap();
    plugin_storage_writes_are_atomic(&*st, &dir, &folder.0);

    // At most 256 keys: a new one is refused, an existing one still changes.
    let keys: std::collections::BTreeMap<String, String> = (0..storage::MAX_SETTINGS).map(|i| (format!("k{i}"), "eA==".to_string())).collect();
    std::fs::write(dir.join("settings.json"), serde_json::to_vec(&keys).unwrap()).unwrap();
    assert_eq!(st.set_setting("one-more", b"x", m), Err(StorageError::Full));
    st.set_setting("k0", b"y", m).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), Some(b"y".to_vec()));
    plugin_storage_skips_oversize_files(&*st, &dir);

    // At most 1 MiB in all: eleven values of 64 KiB fit, a twelfth doesn't, and nothing changes.
    let big = vec![7_u8; storage::MAX_SETTING_BYTES];
    let keys: std::collections::BTreeMap<String, String> = (0..11).map(|i| (format!("k{i}"), vectorcraft_format::base64_encode(&big))).collect();
    std::fs::write(dir.join("settings.json"), serde_json::to_vec(&keys).unwrap()).unwrap();
    assert_eq!(st.set_setting("k11", &big, m), Err(StorageError::Full));
    assert_eq!((st.setting("k11", m).unwrap(), st.setting("k0", m).unwrap()), (None, Some(big.clone())));

    // The cache: entry size, then the total, here nearly reached by a file already there.
    assert_eq!(st.set_cache_entry("big", &vec![0; storage::MAX_CACHE_ENTRY + 1], m), Err(StorageError::Full));
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::File::create(dir.join("cache").join("filler.bin")).unwrap().set_len(storage::MAX_CACHE_BYTES - 2).unwrap();
    // A temporary file a failed write left behind counts toward no cap, and clearing removes it.
    let leftover = dir.join("cache").join(".tile.bin.1-0.tmp");
    std::fs::write(&leftover, b"tmp").unwrap();
    assert_eq!(st.set_cache_entry("tile", b"abc", m), Err(StorageError::Full));
    assert_eq!(st.cache_entry("tile", m).unwrap(), None);
    st.set_cache_entry("tile", b"ab", m).unwrap();
    assert_eq!(st.cache_bytes(), storage::MAX_CACHE_BYTES);
    assert_eq!(st.clear_cache().unwrap(), storage::MAX_CACHE_BYTES + 3);
    assert!(!leftover.exists());

    // Names differing in case are two entries.
    st.set_cache_entry("Tile", b"1", m).unwrap();
    st.set_cache_entry("tile", b"22", m).unwrap();
    assert_eq!((st.cache_entry("Tile", m).unwrap(), st.cache_entry("tile", m).unwrap()), (Some(b"1".to_vec()), Some(b"22".to_vec())));
    assert_eq!(st.cache_bytes(), 3);

    // More items in the cache folder than a write looks at: the write is refused whatever the
    // items it looked at hold, and clearing still reaches every entry.
    let cache = dir.join("cache");
    for i in 0..crate::store::MAX_CACHE_SCAN {
        std::fs::create_dir(cache.join(format!("d{i:05}"))).unwrap();
    }
    std::fs::File::create(cache.join("z.bin")).unwrap().set_len(storage::MAX_CACHE_BYTES).unwrap();
    assert_eq!(st.set_cache_entry("more", b"1", m), Err(StorageError::Full));
    assert_eq!(st.clear_cache().unwrap(), storage::MAX_CACHE_BYTES + 3);
    assert!(!cache.join("z.bin").exists());

    // An id that differs only in case has storage of its own, which removing this one leaves.
    let upper_id = "test.engine.StorageCaps";
    s.attach_plugin_storage(&s.plugin_store_turn().unwrap(), upper_id);
    let upper = vectorcraft_plugins::registry::storage(upper_id).unwrap();
    assert_eq!(upper.setting("k0", m).unwrap(), None);
    upper.set_setting("k0", b"upper", m).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), Some(big));
    // So do ids whose `_` and `.` could be read as each other's escapes.
    let folders: std::collections::BTreeSet<_> =
        ["a.b", "a_0b", "a_b", "A.b", "a__b", "a._b", "a_.b"].iter().map(|id| crate::store::plugin_data_dir(&folder.0, id)).collect();
    assert_eq!(folders.len(), 7, "{folders:?}");

    // Removing the plug-in closes the storage a run still holds, even one from before the plug-in
    // was replaced, so nothing is written back.
    s.attach_plugin_storage(&s.plugin_store_turn().unwrap(), id);
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert_eq!(st.set_setting("zoom", b"again", m), Err(StorageError::Unavailable));
    assert_eq!(st.set_cache_entry("tile", b"again", m), Err(StorageError::Unavailable));
    assert!(!dir.exists());
    assert_eq!(upper.setting("k0", m).unwrap(), Some(b"upper".to_vec()));
    s.remove_plugin_storage(&s.plugin_store_turn().unwrap(), upper_id, || true).unwrap();
}

/// Files larger than the storage reads are not read: a settings file larger than 1 MiB, although
/// each of its values would fit, is moved aside and the settings start empty; a cache entry file
/// larger than an entry may be fails as unavailable. `st` is the storage in `dir`.
fn plugin_storage_skips_oversize_files(st: &dyn vectorcraft_plugins::storage::Storage, dir: &std::path::Path) {
    use vectorcraft_plugins::storage::{self, Meter, StorageError};
    let m = &mut Meter::unlimited();
    let value = vectorcraft_format::base64_encode(&vec![7_u8; storage::MAX_SETTING_BYTES]);
    let keys: std::collections::BTreeMap<String, String> = (0..13).map(|i| (format!("k{i}"), value.clone())).collect();
    let oversize = serde_json::to_vec(&keys).unwrap();
    assert!(oversize.len() > storage::MAX_SETTINGS_BYTES);
    std::fs::write(dir.join("settings.json"), &oversize).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), None);
    assert!(dir.join("settings.unreadable.json").is_file() && !dir.join("settings.json").exists());
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    let huge = dir.join("cache").join("huge.bin");
    std::fs::File::create(&huge).unwrap().set_len(storage::MAX_CACHE_ENTRY as u64 + 1).unwrap();
    assert_eq!(st.cache_entry("huge", m), Err(StorageError::Unavailable));
    std::fs::remove_file(&huge).unwrap();
}

/// Writes are atomic: a new file takes the old one's name, so another name for the old file (a hard
/// link) keeps its content. `st` is the storage in `dir` and holds the setting "zoom"; `outside` is
/// a folder outside it.
fn plugin_storage_writes_are_atomic(st: &dyn vectorcraft_plugins::storage::Storage, dir: &std::path::Path, outside: &std::path::Path) {
    let m = &mut vectorcraft_plugins::storage::Meter::unlimited();
    let settings_copy = outside.join("settings-copy.json");
    let before = std::fs::read(dir.join("settings.json")).unwrap();
    std::fs::hard_link(dir.join("settings.json"), &settings_copy).unwrap();
    st.set_setting("zoom", b"world", m).unwrap();
    assert_eq!(std::fs::read(&settings_copy).unwrap(), before);
    assert_eq!(st.setting("zoom", m).unwrap(), Some(b"world".to_vec()));
    st.set_cache_entry("tile", b"abc", m).unwrap();
    let entry_copy = outside.join("tile-copy.bin");
    let entry = std::fs::read_dir(dir.join("cache")).unwrap().map(|e| e.unwrap().path()).find(|p| p.is_file()).unwrap();
    std::fs::hard_link(&entry, &entry_copy).unwrap();
    st.set_cache_entry("tile", b"xyz", m).unwrap();
    assert_eq!(std::fs::read(&entry_copy).unwrap(), b"abc");
    assert_eq!(st.cache_entry("tile", m).unwrap(), Some(b"xyz".to_vec()));
    assert!(st.remove_cache_entry("tile").unwrap());
    std::fs::remove_file(&settings_copy).unwrap();
    std::fs::remove_file(&entry_copy).unwrap();
}

/// Gives the file at `path` the modified time `when`, whatever the file system's clock says.
fn set_modified(path: &std::path::Path, when: std::time::SystemTime) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

/// Each storage call is charged the bytes it reads and writes, before it does: a settings call the
/// whole settings file when it reads the file and the whole file when it writes it, a cache call the
/// entry it reads or writes. A call that would pass what its meter has left fails and changes nothing.
fn plugin_storage_calls_are_charged_the_files_they_read_and_write() {
    use vectorcraft_plugins::storage::{Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-meter");
    let id = "test.engine.storagemeter";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let file = crate::store::plugin_data_dir(&folder.0, id).join("settings.json");
    let size = || std::fs::metadata(&file).unwrap().len();
    st.set_setting("big", &[7; 60_000], &mut Meter::unlimited()).unwrap();
    st.set_setting("small", b"x", &mut Meter::unlimited()).unwrap();

    // A file another window changed is read whole, even for a one-byte setting.
    let touch = || set_modified(&file, std::fs::metadata(&file).unwrap().modified().unwrap() + std::time::Duration::from_secs(1));
    touch();
    let before = size();
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("small", &mut m).unwrap(), Some(b"x".to_vec()));
    assert_eq!(m.used(), before);
    // Setting it writes the file again, and so does deleting one.
    let mut m = Meter::unlimited();
    st.set_setting("small", b"yz", &mut m).unwrap();
    assert_eq!(m.used(), size());
    let mut m = Meter::unlimited();
    assert!(st.delete_setting("small", &mut m).unwrap());
    assert_eq!(m.used(), size());
    st.set_setting("small", b"y", &mut Meter::unlimited()).unwrap();

    // Less left than the file: nothing is read. Enough to read it but not to write it again:
    // nothing changes.
    touch();
    let mut m = Meter::new(size() - 1);
    assert_eq!(st.setting("small", &mut m), Err(StorageError::Budget));
    assert_eq!(m.used(), 0);
    assert_eq!(st.set_setting("small", b"z", &mut Meter::new(size())), Err(StorageError::Budget));
    assert_eq!(st.setting("small", &mut Meter::unlimited()).unwrap(), Some(b"y".to_vec()));

    // A cache write is charged the entry it writes, a read the entry it reads.
    let mut m = Meter::unlimited();
    st.set_cache_entry("tile", &[1; 1000], &mut m).unwrap();
    assert_eq!(m.used(), 1000);
    let mut m = Meter::unlimited();
    assert_eq!(st.cache_entry("tile", &mut m).unwrap().map(|v| v.len()), Some(1000));
    assert_eq!(m.used(), 1000);
    assert_eq!(st.cache_entry("tile", &mut Meter::new(999)), Err(StorageError::Budget));
    assert_eq!(st.set_cache_entry("tile", &[2; 10], &mut Meter::new(9)), Err(StorageError::Budget));
    assert_eq!(st.cache_entry("tile", &mut Meter::unlimited()).unwrap().map(|v| v.len()), Some(1000));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A session with the plug-in `id` installed in a new store → its storage and the path of its
/// settings file.
fn storage_in(
    s: &mut Session,
    folder: &crate::tests_links::Folder,
    id: &str,
) -> (std::sync::Arc<dyn vectorcraft_plugins::storage::Storage>, std::path::PathBuf) {
    s.attach_plugin_store(folder.0.clone());
    install_storing(s, id, "");
    (vectorcraft_plugins::registry::storage(id).unwrap(), crate::store::plugin_data_dir(&folder.0, id).join("settings.json"))
}

/// The settings stay in memory while the file keeps its size and modified time: a read doesn't open
/// the file, a set to the value already there doesn't write it, a real change is in the file when
/// the call returns, and a file with another time is read again.
#[test]
fn plugin_settings_are_read_from_memory_until_the_file_changes() {
    use vectorcraft_plugins::storage::{Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-kept");
    let id = "test.engine.storagekept";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    st.set_setting("zoom", b"hello", &mut Meter::unlimited()).unwrap();
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();

    // Other bytes of the same size and the same time: the storage can't tell, so what it answers
    // shows that it did not read the file.
    let other =
        std::fs::read_to_string(&file).unwrap().replace(&vectorcraft_format::base64_encode(b"hello"), &vectorcraft_format::base64_encode(b"world"));
    std::fs::write(&file, &other).unwrap();
    set_modified(&file, written);
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"hello".to_vec()));
    assert_eq!(m.used(), 0, "answered from memory");

    // Setting the value it has writes nothing: the file still holds the other bytes.
    let mut m = Meter::unlimited();
    st.set_setting("zoom", b"hello", &mut m).unwrap();
    assert_eq!(m.used(), 0, "nothing to write");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), other);

    // Another time: read again.
    set_modified(&file, written + std::time::Duration::from_secs(1));
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"world".to_vec()));
    assert_eq!(m.used(), other.len() as u64);

    // A change reaches the file before the call returns, and a failed one is not kept.
    st.set_setting("tile", b"x", &mut Meter::unlimited()).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"tile\""));
    assert_eq!(st.set_setting("tile", b"y", &mut Meter::new(0)), Err(StorageError::Budget));
    assert_eq!(st.setting("tile", &mut Meter::unlimited()).unwrap(), Some(b"x".to_vec()));
    assert!(st.delete_setting("tile", &mut Meter::unlimited()).unwrap());
    assert!(!std::fs::read_to_string(&file).unwrap().contains("\"tile\""));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// `plugin.remove` drops what was kept: a file put back with the removed settings' size and time is
/// read from disk by the storage of the id installed again.
#[test]
fn removing_a_plugin_drops_its_settings_from_memory() {
    use vectorcraft_plugins::storage::Meter;
    let folder = crate::tests_links::Folder::new("plugin-storage-kept-removed");
    let id = "test.engine.storagekeptremoved";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    st.set_setting("zoom", b"hello", &mut Meter::unlimited()).unwrap();
    let (bytes, written) = (std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap());
    run(&mut s, "plugin.remove", json!({"id": id}));

    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    set_modified(&file, written);
    s.attach_plugin_storage(&s.plugin_store_turn().unwrap(), id);
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"hello".to_vec()));
    assert_eq!(m.used(), bytes.len() as u64, "read from the file");
    s.remove_plugin_storage(&s.plugin_store_turn().unwrap(), id, || true).unwrap();
}

/// The cache's total stays in memory while the cache folder keeps its time; a write and a clear
/// update it, and removing an entry drops it. A file whose length changes behind the storage leaves
/// the folder's time alone, so the total it reports shows that it did not list the folder.
#[test]
fn plugin_cache_total_is_kept_and_follows_writes_removals_and_clears() {
    use vectorcraft_plugins::storage::Meter;
    let folder = crate::tests_links::Folder::new("plugin-storage-kept-cache");
    let id = "test.engine.storagekeptcache";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    let cache = file.parent().unwrap().join("cache");
    let m = &mut Meter::unlimited();
    let grow = |name: &str, len: u64| std::fs::File::options().write(true).open(cache.join(name)).unwrap().set_len(len).unwrap();

    st.set_cache_entry("tile", b"abc", m).unwrap();
    grow("tile.bin", 10);
    assert_eq!(st.cache_bytes(), 3, "the write set the total");
    // A write lists the folder for its caps, and starts from what it finds.
    st.set_cache_entry("more", b"12345", m).unwrap();
    grow("more.bin", 5);
    assert_eq!(st.cache_bytes(), 15);
    assert!(st.remove_cache_entry("more").unwrap());
    assert_eq!(st.cache_bytes(), 10, "after a removal the total comes from the folder");
    grow("tile.bin", 20);
    assert_eq!(st.cache_bytes(), 10, "and is kept again");
    assert_eq!(st.clear_cache().unwrap(), 20);
    assert_eq!(st.cache_bytes(), 0);

    // A folder deleted behind the storage is noticed.
    st.set_cache_entry("tile", b"abc", m).unwrap();
    std::fs::remove_dir_all(&cache).unwrap();
    assert_eq!(st.cache_bytes(), 0);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module with other bytes than the plug-in installed with its id, or, with none installed, one
/// whose id has settings and a cache in the plug-in store, installs only with `replace: true`, and
/// then keeps them; the same module installs again without it, and `replace` must be true or false.
#[test]
fn another_module_under_an_id_with_settings_needs_replace() {
    let folder = crate::tests_links::Folder::new("plugin-storage-replace");
    let id = "test.engine.storingreplace";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    let hash = run(&mut s, "plugin.info", json!({"id": id}))["hash"].clone();
    // The same module again.
    install_storing(&mut s, id, WRITES);

    let reads = vectorcraft_format::base64_encode(&storing(id, READS));
    let install = |s: &mut Session, extra: Value| {
        let mut p = json!({"dataBase64": reads, "grant": ["settings", "cache"]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("plugin.install", &p)
    };
    let Err(e) = install(&mut s, json!({})) else { panic!("another module took the plug-in's place") };
    assert!(e.to_string().contains(&format!("“Test {id}” is another module than the plug-in with id \"{id}\"")), "{e}");
    assert!(e.to_string().contains("`replace: true`"), "{e}");
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsReplace"], json!(true));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["hash"], hash);
    for bad in [json!({"replace": "yes"}), json!({"replace": 1})] {
        let Err(e) = install(&mut s, bad) else { panic!("a bad `replace` was taken") };
        assert!(e.to_string().contains("`replace` must be true or false"), "{e}");
    }
    assert!(install(&mut s, json!({"replace": false})).is_err());
    install(&mut s, json!({"replace": true})).unwrap();
    // It reads what the one it replaced set.
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));

    // Not installed, but with settings in the store: another module needs `replace` too.
    vectorcraft_plugins::registry::remove(id);
    let writes = vectorcraft_format::base64_encode(&storing(id, WRITES));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": writes}))["needsReplace"], json!(true));
    assert!(s.execute("plugin.install", &json!({"dataBase64": writes, "grant": ["settings", "cache"]})).is_err());
    run(&mut s, "plugin.install", json!({"dataBase64": writes, "grant": ["settings", "cache"], "replace": true}));
    run(&mut s, "plugin.remove", json!({"id": id}));
    // Once removed, with its settings and cache, it installs as a first install does.
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsReplace"], json!(false));
    install(&mut s, json!({})).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A filter that calls commands runs on the document with the edit in progress kept first: a move
/// still being previewed is its own undo step, and the filter is given the moved rectangle, which
/// its empty output then removes.
#[test]
fn a_filter_that_calls_commands_is_given_the_edit_in_progress() {
    let id = "test.engine.callsettle";
    let mut s = session();
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"]}}"#);
    let wasm = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true}));
    let paths = |s: &Session| s.doc().unwrap().doc.layers.first().and_then(|l| l.children()).map_or(0, |c| c.len());
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}));
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 7, "dy": 0})).unwrap();
    run(&mut s, "plugin.run", json!({"id": id}));
    assert!(!s.in_interaction());
    assert_eq!(paths(&s), 0, "the filter was given the moved rectangle and removed it");
    let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
    assert_eq!(labels, [format!("Test {id}").as_str(), "Move"]);
    run(&mut s, "edit.undo", json!({}));
    let x = run(&mut s, "document.inspect", json!({}))["layers"][0]["children"][0]["bounds"]["x"].clone();
    assert_eq!(x, json!(17.0), "undoing the filter brings back the moved rectangle");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Installs a filter `id` that calls commands and hands back what it is given, with `commands` (a
/// JSON list of command specs) in its manifest.
fn install_echo_calling(s: &mut Session, id: &str, commands: &str) {
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"],"commands":{commands}}}"#
    );
    let wasm = wat::parse_str(tpl::module(&manifest, tpl::ECHO, "", 1)).unwrap();
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true}));
}

/// With Real-time Drawing off, a 50 × 50 rectangle at (10, 10) whose move by 7 pt is still being
/// dragged: the tool has stored the move, which the document does not hold yet.
fn deferred_move(s: &mut Session) -> NodeId {
    let r = NodeId(run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    run(s, "prefs.set", json!({"key": "realTimeDrawing", "value": false}));
    let move_by = vectorcraft_tools::Action::Preview("object.move".into(), json!({"dx": 7, "dy": 0}));
    s.apply_actions(vec![vectorcraft_tools::Action::Begin("Move".into()), move_by]).unwrap();
    assert_eq!(node(s, r).geometric_bounds().unwrap().x0, 10.0, "the move is not applied while it is dragged");
    r
}

/// The rectangle an echoing run during [`deferred_move`] left: moved, with the interaction ended and
/// the move the last undo step, since the run handed the rectangle back as it was and so records
/// none.
fn assert_moved_once(s: &Session, r: NodeId) {
    assert!(!s.in_interaction());
    assert_eq!(node(s, r).geometric_bounds().unwrap().x0, 17.0, "the filter was given the moved rectangle");
    let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Move", "Rectangle"]);
}

/// With Real-time Drawing off, `plugin.run` of a filter that calls commands during a drag applies
/// the dragged move first and gives the filter the moved rectangle, which it hands back as it was.
#[test]
fn a_filter_that_calls_commands_is_given_a_deferred_move() {
    let id = "test.engine.callsdeferred";
    let mut s = session();
    install_echo_calling(&mut s, id, "[]");
    let r = deferred_move(&mut s);
    run(&mut s, "plugin.run", json!({"id": id}));
    assert_moved_once(&s, r);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in command that calls commands, run with Real-time Drawing off during a drag, is given
/// the moved rectangle as `plugin.run` is.
#[test]
fn a_plugin_command_that_calls_commands_is_given_a_deferred_move() {
    let id = "test.engine.callsdeferredcmd";
    let mut s = session();
    install_echo_calling(&mut s, id, r#"[{"id":"same","label":"Same"}]"#);
    let r = deferred_move(&mut s);
    run(&mut s, &format!("plugin.{id}.same"), json!({}));
    assert_moved_once(&s, r);
    run(&mut s, "plugin.remove", json!({"id": id}));
}
