//! Synthetic version-12 regression documents exercise placement and clipping, independently of
//! the pinned real-file corpus. These are reader fixtures, never claimed as Affinity-authored.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use kurbo::Rect;
use vectorcraft_affinity::synth::{self, F, Method, tag};
use vectorcraft_doc::{Document, Node, NodeKind};

fn field(t: &[u8; 4], f: F) -> (vectorcraft_affinity::stream::Tag, F) {
    (tag(t), f)
}

fn red(id: u32) -> F {
    F::Def(
        id,
        vec![tag(b"FDsc")],
        vec![field(
            b"FDeF",
            F::Def(
                id + 1,
                vec![tag(b"FilS")],
                vec![field(
                    b"Colr",
                    F::Def(
                        id + 2,
                        vec![tag(b"RGBA")],
                        vec![field(b"_col", F::Struct([1f32, 0.0, 0.0, 1.0].into_iter().flat_map(f32::to_le_bytes).collect()))],
                    ),
                )],
            ),
        )],
    )
}

fn shape(id: u32, name: &str, board: bool, ellipse: bool, bounds: [f64; 4], transform: [f64; 6], children: Vec<F>) -> F {
    F::Def(
        id,
        vec![tag(b"ShpN")],
        vec![
            field(b"Desc", F::Str(name.into())),
            field(b"ABEn", F::Bool(board)),
            field(b"Shpe", F::Def(id + 1, vec![tag(if ellipse { b"ShpE" } else { b"ShNR" })], vec![])),
            field(b"ShpB", F::F64s(bounds.to_vec())),
            field(b"Xfrm", F::F64s(transform.to_vec())),
            field(b"BFFl", F::Shared(vec![red(id + 2)])),
            field(b"Chld", F::Shared(children)),
        ],
    )
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

fn document(spreads: Vec<Vec<F>>, dpi: f64, method: Method) -> Vec<u8> {
    let spreads = spreads
        .into_iter()
        .enumerate()
        .map(|(i, nodes)| {
            F::Def(1000 + i as u32, vec![tag(b"Sprd")], vec![field(b"SprB", F::F64s(vec![0.0, 0.0, 200.0, 100.0])), field(b"Chld", F::Shared(nodes))])
        })
        .collect();
    let doc = synth::stream(&[
        field(b"UVCn", F::Obj(tag(b"UVCn"), vec![field(b"UPPI", F::F64(dpi))])),
        field(b"DocR", F::Def(2000, vec![tag(b"DocN")], vec![field(b"Chld", F::Shared(spreads))])),
    ]);
    synth::container(&[("doc.dat", &doc, method)], None)
}

fn find<'a>(nodes: &'a [std::sync::Arc<Node>], name: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|n| {
        if n.name.as_deref() == Some(name) && matches!(n.kind, NodeKind::Group { .. }) {
            Some(n.as_ref())
        } else {
            n.children().and_then(|c| find(c, name))
        }
    })
}

#[test]
fn nested_boards_keep_order_and_negative_origins_at_document_resolution() {
    let inner = shape(20, "Inner", true, false, [0.0, 0.0, 20.0, 10.0], [1.0, 0.0, 10.0, 0.0, 1.0, 5.0], vec![]);
    let outer = shape(10, "Outer", true, false, [0.0, 0.0, 100.0, 60.0], [1.0, 0.0, -40.0, 0.0, 1.0, -20.0], vec![inner]);
    let last = shape(30, "Last", true, false, [0.0, 0.0, 40.0, 80.0], [1.0, 0.0, 150.0, 0.0, 1.0, 0.0], vec![]);
    for method in [Method::Stored, Method::Zlib, Method::Zstd] {
        let loaded = vectorcraft_engine::cmd::fileio::load("boards.af", &document(vec![vec![outer.clone(), last.clone()]], 144.0, method)).unwrap();
        let boards = &loaded.doc.artboards;
        assert_eq!(boards.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Outer", "Inner", "Last"]);
        assert_eq!(boards.iter().map(|b| b.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(boards[0].rect, Rect::new(-20.0, -10.0, 30.0, 20.0));
        assert_eq!(boards[1].rect, Rect::new(-15.0, -7.5, -5.0, -2.5));
        assert_eq!(boards[2].rect, Rect::new(75.0, 0.0, 95.0, 40.0));
        let reopened = vectorcraft_format::load_file(&vectorcraft_format::save_file(&loaded.doc)).unwrap();
        assert_eq!(reopened.doc.artboards, loaded.doc.artboards);
        assert_eq!(reopened.doc.layers, loaded.doc.layers);
    }
}

fn pixel(d: &Document, x: usize, y: usize) -> [u8; 4] {
    let img = vectorcraft_render::Renderer::new().render_region(d, d.artboards[0].rect, 1.0, false);
    img.to_straight()[(y * img.width as usize + x) * 4..][..4].try_into().unwrap()
}

#[test]
fn rotated_board_clips_to_its_outline_instead_of_its_export_rectangle() {
    let child = shape(20, "Oversized", false, false, [-100.0, -100.0, 200.0, 200.0], IDENTITY, vec![]);
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let board = shape(10, "Diamond", true, false, [0.0, 0.0, 80.0, 80.0], [c, -c, 60.0, c, c, 0.0], vec![child]);
    let loaded = vectorcraft_engine::cmd::fileio::load("rotated.af", &document(vec![vec![board]], 72.0, Method::Zstd)).unwrap();
    let d = &loaded.doc;
    let g = find(&d.layers, "Diamond").unwrap();
    let NodeKind::Path { path, clipping: true, .. } = &g.children().unwrap()[0].kind else { panic!() };
    let a = &path.subpaths[0].anchors;
    assert!((a[0].p.x - 60.0).abs() < 1e-8 && a[0].p.y.abs() < 1e-8);
    assert!((a[1].p.x - (60.0 + 80.0 * c)).abs() < 1e-8);
    assert_eq!(pixel(d, 2, 2), [0, 0, 0, 0], "bbox corners are outside the diamond");
    assert_eq!(pixel(d, 55, 55), [255, 0, 0, 255]);
}

#[test]
fn converted_ellipse_artboard_keeps_curved_clipping() {
    let child = shape(20, "Oversized", false, false, [-20.0, -20.0, 120.0, 120.0], IDENTITY, vec![]);
    let board = shape(10, "Ellipse", true, true, [0.0, 0.0, 100.0, 100.0], IDENTITY, vec![child]);
    let loaded = vectorcraft_engine::cmd::fileio::load("ellipse.af", &document(vec![vec![board]], 72.0, Method::Stored)).unwrap();
    assert_eq!(pixel(&loaded.doc, 2, 2), [0, 0, 0, 0]);
    assert_eq!(pixel(&loaded.doc, 50, 50), [255, 0, 0, 255]);
}

#[test]
fn invalid_board_keeps_children_and_reports_the_loss() {
    let child = shape(20, "Retained", false, false, [1.0, 2.0, 10.0, 20.0], IDENTITY, vec![]);
    let board = shape(10, "Invalid", true, false, [0.0, 0.0, 0.0, 100.0], IDENTITY, vec![child]);
    let loaded = vectorcraft_engine::cmd::fileio::load("invalid.af", &document(vec![vec![board]], 72.0, Method::Stored)).unwrap();
    assert!(loaded.warnings.iter().any(|w| w.contains("artboard without valid bounds")));
    assert!(!loaded.preview_only);
    assert_eq!(loaded.doc.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 100.0));
    let invalid = find(&loaded.doc.layers, "Invalid").unwrap();
    assert_eq!(invalid.children().unwrap()[0].name.as_deref(), Some("Retained"));
}

#[test]
fn several_spreads_preserve_each_board_and_leave_room_between_spreads() {
    let a = shape(10, "First", true, false, [0.0, 0.0, 100.0, 60.0], IDENTITY, vec![]);
    let b = shape(20, "Second", true, false, [0.0, 0.0, 100.0, 60.0], IDENTITY, vec![]);
    let loaded = vectorcraft_engine::cmd::fileio::load("spreads.af", &document(vec![vec![a], vec![b]], 72.0, Method::Stored)).unwrap();
    assert_eq!(loaded.doc.artboards.len(), 2);
    assert_eq!(loaded.doc.artboards[0].rect, Rect::new(0.0, 0.0, 100.0, 60.0));
    assert_eq!(loaded.doc.artboards[1].rect, Rect::new(136.0, 0.0, 236.0, 60.0));
}

/// Coordinate fields use setProps rather than the drag tool's artboard.move command.
#[test]
fn coordinate_edits_move_imported_clipping_groups_and_undo_together() {
    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_geom::Vec2;

    for current in [false, true] {
        let child = shape(20, "Oversized", false, false, [-10.0, -10.0, 110.0, 70.0], IDENTITY, vec![]);
        let mut first = shape(10, "First", true, false, [0.0, 0.0, 100.0, 60.0], IDENTITY, vec![child]);
        if current {
            let F::Def(_, _, fields) = &mut first else { panic!() };
            fields.retain(|(t, _)| *t != tag(b"ABEn"));
            fields.push(field(b"phrp", F::Def(18, vec![tag(b"aprp")], vec![])));
        }
        let second = shape(30, "Second", true, false, [0.0, 0.0, 40.0, 60.0], [1.0, 0.0, 200.0, 0.0, 1.0, 0.0], vec![]);
        let loaded = vectorcraft_engine::cmd::fileio::load("synthetic.af", &document(vec![vec![first, second]], 72.0, Method::Zstd)).unwrap();
        let mut s = Session::new();
        s.add_document(loaded.doc, None);
        let before = s.doc().unwrap().doc.clone();
        let first_bounds = find(&before.layers, "First").unwrap().geometric_bounds().unwrap();
        let second_node = find(&before.layers, "Second").unwrap().clone();
        let child_bounds = find(&before.layers, "First").unwrap().children().unwrap()[1].geometric_bounds().unwrap();
        let delta = Vec2::new(13.0, -7.0);
        s.execute("artboard.setProps", &json!({"index": 0, "x": 13, "y": -7, "moveArt": true})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        assert_eq!(after.artboards[0].rect, before.artboards[0].rect + delta);
        let first = find(&after.layers, "First").unwrap();
        assert_eq!(first.geometric_bounds().unwrap(), first_bounds + delta);
        assert_eq!(first.children().unwrap()[1].geometric_bounds().unwrap(), child_bounds + delta);
        assert_eq!(find(&after.layers, "Second").unwrap(), &second_node);
        assert_eq!(after.artboards[1], before.artboards[1]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *after);
    }
}
