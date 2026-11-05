//! Select by Artboards.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;

const C: &str = "addon.select.byArtboards";

/// A document with three 100 × 100 artboards in a row, 50 pt apart: 0 at x 0, 1 at x 150, 2 at x 300.
fn three_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    for x in [150, 300] {
        s.execute("artboard.new", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn pick(s: &mut Session, p: Value) -> Vec<NodeId> {
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute(C, &p).unwrap();
    let mut ids = s.doc().unwrap().selection.objects.clone();
    assert_eq!(r["count"], json!(ids.len()));
    ids.sort();
    ids
}

#[test]
fn selects_by_overlap_inside_and_outside_several_artboards() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0); // inside 0
    let ab = rect(&mut s, 90.0, 40.0, 80.0, 20.0); // across 0 and 1
    let b = rect(&mut s, 160.0, 10.0, 20.0, 20.0); // inside 1
    let c = rect(&mut s, 310.0, 10.0, 20.0, 20.0); // inside 2
    let off = rect(&mut s, 0.0, 200.0, 20.0, 20.0); // on no board
    let edge = rect(&mut s, 0.0, 0.0, 100.0, 100.0); // flush with board 0
    let want = |mut v: Vec<NodeId>| {
        v.sort();
        v
    };
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1]})), want(vec![a, ab, b, edge]));
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "overlap"})), want(vec![a, ab, b, edge]));
    // Wholly within one board, edges included; straddling two boards is not inside either.
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "inside"})), want(vec![a, b, edge]));
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "outside"})), want(vec![c, off]));
    assert_eq!(pick(&mut s, json!({"artboards": [2], "relation": "outside"})), want(vec![a, ab, b, off, edge]));
    // One board does what All on Active Artboard does.
    for i in 0..3u64 {
        s.execute("select.allOnArtboard", &json!({"artboard": i})).unwrap();
        let mut on = s.doc().unwrap().selection.objects.clone();
        on.sort();
        assert_eq!(pick(&mut s, json!({"artboards": [i]})), on, "board {i}");
    }
    // A board listed twice counts once.
    assert_eq!(pick(&mut s, json!({"artboards": [2, 2]})), vec![c]);
}

#[test]
fn hidden_locked_and_isolated_art_follow_select_all() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let hidden = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let locked = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    let far = rect(&mut s, 0.0, 300.0, 20.0, 20.0);
    s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![a]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "outside"})), vec![far]);
    // Art held by a locked artboard is left out.
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![]);
    s.execute("artboard.setProps", &json!({"index": 0, "locked": false})).unwrap();
    // In isolation mode only the isolated group's objects are looked at.
    let b = rect(&mut s, 20.0, 50.0, 10.0, 10.0);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.isolate", &json!({"id": g})).unwrap();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "inside"})), both);
    assert_eq!(pick(&mut s, json!({"artboards": [1]})), vec![]);
}

#[test]
fn bad_params_are_errors_and_leave_the_selection() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    for p in [
        json!({}),
        json!({"artboards": []}),
        json!({"artboards": [3]}),
        json!({"artboards": [-1]}),
        json!({"artboards": [0.5]}),
        json!({"artboards": "0"}),
        json!({"artboards": [0], "relation": "near"}),
        json!({"artboards": [0], "relation": 1}),
        json!({"artboards": [0], "relatoin": "inside"}),
    ] {
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        assert!(s.execute(C, &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![a], "{p}");
    }
}

#[test]
fn a_symbol_instance_is_measured_by_its_art() {
    // The symbol's art, (100, 60) to (140, 80), drawn 50 pt up and left: (50, 10) to (90, 30), on
    // the 200 × 120 artboard, while the 20 × 20 square around the instance's origin lies off it.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Far"><rect x="100" y="60" width="40" height="20" fill="#f00"/></symbol></defs><use href="#Far" x="-50" y="-50"/></svg>"##;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "far.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let mut instance = None;
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. }) {
            instance = Some(n.id);
        }
    });
    let instance = instance.unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![instance]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "inside"})), vec![instance]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "outside"})), vec![]);
}

/// Select by Link Status.
mod by_link_status {
    use serde_json::{Value, json};
    use vectorcraft_doc::{Document, NodeId};

    use crate::Session;
    use crate::tests_links::{BLUE, Folder, RED, place, png, session, write};

    const C: &str = "addon.select.byLinkStatus";

    /// One linked image whose file stays as it is (`ok`), one whose file changes (`modified`),
    /// one whose file goes (`missing`), an embedded one, and a placed document whose file goes
    /// (missing too), bottom to top.
    struct Links {
        s: Session,
        ok: NodeId,
        modified: NodeId,
        missing: NodeId,
        embedded: NodeId,
        document: NodeId,
        _dir: Folder,
    }

    fn links(name: &str) -> Links {
        let dir = Folder::new(name);
        let (a, b, c, e, d) = (dir.file("ok.png"), dir.file("changes.png"), dir.file("goes.png"), dir.file("kept.png"), dir.file("page.vectorcraft"));
        write(&a, &png(10, 10, RED));
        write(&b, &png(12, 10, RED));
        write(&c, &png(14, 10, BLUE));
        write(&e, &png(16, 10, BLUE));
        write(&d, &vectorcraft_format::save_file(&Document::new(80.0, 50.0)));
        let mut s = session();
        let (ok, modified, missing) = (place(&mut s, &a), place(&mut s, &b), place(&mut s, &c));
        let embedded = NodeId(s.execute("file.place", &json!({"path": e, "at": [100, 100], "link": false})).unwrap()["ids"][0].as_u64().unwrap());
        let document = place(&mut s, &d);
        write(&b, &png(13, 10, BLUE));
        std::fs::remove_file(&c).unwrap();
        std::fs::remove_file(&d).unwrap();
        Links { s, ok, modified, missing, embedded, document, _dir: dir }
    }

    fn run(s: &mut Session, p: Value) -> Value {
        s.execute(C, &p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
        v.sort();
        v
    }

    fn selection(s: &Session) -> Vec<NodeId> {
        sorted(s.doc().unwrap().selection.objects.clone())
    }

    fn ids(v: &Value) -> Vec<NodeId> {
        sorted(v.as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect())
    }

    #[test]
    fn each_status_selects_its_images_and_placed_documents() {
        let Links { mut s, ok, modified, missing, embedded, document, _dir } = links("select-status");
        // As the Links panel lists them.
        let listed = s.execute("links.list", &json!({})).unwrap();
        let status = |id: NodeId| listed["links"].as_array().unwrap().iter().find(|r| r["id"] == id.0).unwrap()["status"].clone();
        assert_eq!([ok, modified, missing, embedded, document].map(status), ["ok", "modified", "missing", "embedded", "missing"].map(|s| json!(s)));
        let undo = s.doc().unwrap().history.undo.len();
        for (status, want) in [("linked", vec![ok]), ("modified", vec![modified]), ("missing", vec![missing, document]), ("embedded", vec![embedded])]
        {
            s.execute("select.none", &json!({})).unwrap();
            let r = run(&mut s, json!({"status": status}));
            assert_eq!((ids(&r["selected"]), r["count"].as_u64()), (sorted(want.clone()), Some(want.len() as u64)), "{status}: {r}");
            assert_eq!(selection(&s), sorted(want), "{status}");
            assert_eq!(r["skipped"], json!([]));
            assert!(r.get("zoom").is_none());
        }
        // Several at once, in stacking order (bottom first); a status listed twice counts once.
        let r = run(&mut s, json!({"status": ["missing", "linked", "missing"]}));
        assert_eq!(r["selected"], json!([ok.0, missing.0, document.0]));
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "only the selection changes");
    }

    #[test]
    fn ids_add_and_zoom() {
        let Links { mut s, ok, modified, embedded, document, _dir, .. } = links("select-options");
        // Only among the ids given; other objects among them (a rectangle) are passed over.
        let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap();
        let r = run(&mut s, json!({"status": "missing", "ids": [document.0, ok.0, rect]}));
        assert_eq!(r["selected"], json!([document.0]));
        assert_eq!(selection(&s), vec![document]);
        // Added to the selection.
        let r = run(&mut s, json!({"status": "embedded", "add": true}));
        assert_eq!((r["count"].as_u64(), r["selected"].clone()), (Some(1), json!([embedded.0])));
        assert_eq!(selection(&s), sorted(vec![document, embedded]));
        // Replacing it by default.
        run(&mut s, json!({"status": "modified"}));
        assert_eq!(selection(&s), vec![modified]);
        // Nothing to select: added, the selection stays; replacing, it empties.
        let r = run(&mut s, json!({"status": "linked", "ids": [embedded.0], "add": true}));
        assert_eq!((r["count"].as_u64(), r["selected"].clone(), r["skipped"].clone()), (Some(0), json!([]), json!([])));
        assert_eq!(selection(&s), vec![modified]);
        run(&mut s, json!({"status": "linked", "ids": [embedded.0]}));
        assert_eq!(selection(&s), vec![]);
        // Zoom is reported for the caller (the desktop app zooms).
        assert_eq!(run(&mut s, json!({"status": "linked", "zoom": true}))["zoom"], true);
        assert!(run(&mut s, json!({"status": "linked", "zoom": false})).get("zoom").is_none());
    }

    #[test]
    fn hidden_locked_template_and_isolated_ones_are_skipped_with_the_reason() {
        let dir = Folder::new("select-skipped");
        let files: Vec<String> = (0..6).map(|i| dir.file(&format!("img{i}.png"))).collect();
        for (i, f) in files.iter().enumerate() {
            write(f, &png(10 + i as u32, 10, RED));
        }
        let mut s = session();
        let [free, hidden, locked, in_group, mate] = [0, 1, 2, 3, 4].map(|i| place(&mut s, &files[i]));
        s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
        s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
        // Inside a locked group.
        s.execute("select.set", &json!({"ids": [in_group.0]})).unwrap();
        let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.setProps", &json!({"ids": [g], "locked": true})).unwrap();
        // On a template layer.
        let layer = s.execute("layer.new", &json!({"name": "Trace"})).unwrap()["id"].as_u64().unwrap();
        let on_template = place(&mut s, &files[5]);
        s.execute("layer.setProps", &json!({"id": layer, "template": true})).unwrap();
        let r = run(&mut s, json!({"status": "linked"}));
        assert_eq!(ids(&r["selected"]), sorted(vec![free, mate]), "{r}");
        let mut reasons: Vec<(u64, String)> =
            r["skipped"].as_array().unwrap().iter().map(|k| (k["id"].as_u64().unwrap(), k["reason"].as_str().unwrap().to_string())).collect();
        reasons.sort();
        let mut want: Vec<(u64, String)> = [(hidden, "hidden"), (locked, "locked"), (in_group, "locked"), (on_template, "template")]
            .into_iter()
            .map(|(id, why)| (id.0, why.to_string()))
            .collect();
        want.sort();
        assert_eq!(reasons, want, "{r}");
        assert!(reasons.iter().all(|(_, why)| ["hidden", "locked", "template", "isolation"].contains(&why.as_str())));
        // In isolation mode, those outside the isolated group are left out.
        s.execute("select.set", &json!({"ids": [mate.0]})).unwrap();
        let iso = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.isolate", &json!({"id": iso})).unwrap();
        let r = run(&mut s, json!({"status": "linked"}));
        assert_eq!(r["selected"], json!([mate.0]), "{r}");
        let why = |id: NodeId| r["skipped"].as_array().unwrap().iter().find(|k| k["id"] == id.0).map(|k| k["reason"].clone());
        assert_eq!(why(free), Some(json!("isolation")));
        assert_eq!(why(hidden), Some(json!("isolation")));
    }

    #[test]
    fn bad_params_are_errors_and_leave_the_selection() {
        let Links { mut s, ok, _dir, .. } = links("select-bad");
        for p in [
            json!({}),
            json!({"status": null}),
            json!({"status": []}),
            json!({"status": "lost"}),
            json!({"status": ["linked", "lost"]}),
            json!({"status": 3}),
            json!({"status": [true]}),
            json!({"status": "linked", "add": "yes"}),
            json!({"status": "linked", "zoom": 1}),
            json!({"status": "linked", "ids": "1"}),
            json!({"status": "linked", "ids": [999_999]}),
            json!({"status": "linked", "ids": [-1]}),
            json!({"status": "linked", "stauts": "missing"}),
        ] {
            s.execute("select.set", &json!({"ids": [ok.0]})).unwrap();
            assert!(s.execute(C, &p).is_err(), "{p}");
            assert_eq!(s.doc().unwrap().selection.objects, vec![ok], "{p}");
        }
        // Without a document it is off, not a crash.
        assert!(Session::new().execute(C, &json!({"status": "linked"})).is_err());
    }
}
