//! Fill-only open-path Eraser regressions through the public engine and CPU renderer.
// Integration tests may unwrap and panic on failure (AGENTS.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::{FillRule, PathData, Point};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    s
}

fn path(s: &Session, id: NodeId) -> &PathData {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { path, .. } => path,
        _ => panic!("the fixture is a path"),
    }
}

fn paths(s: &Session) -> Vec<PathData> {
    let mut paths = vec![];
    s.doc().unwrap().doc.walk(|node| {
        if let NodeKind::Path { path, .. } = &node.kind {
            paths.push(path.clone());
        }
    });
    paths
}

// The varied setting necessarily selects erase_varied, while a circular nib keeps the
// same deterministic swept area as the fixed tip. No random input or device is involved.
fn erase(s: &mut Session, mut params: Value, varied: bool) -> Value {
    if varied {
        params["angleMode"] = json!("pressure");
        params["angleVariation"] = json!(30);
        params["roundness"] = json!(100);
    }
    s.execute("path.eraseRegion", &params).unwrap()
}

fn eraser_preserves_the_fill_of_an_open_rectangle_case(varied: bool) {
    let samples = [(110.0, 125.0), (190.0, 125.0), (110.0, 175.0), (190.0, 175.0)];
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    for closed in [true, false] {
        let mut s = session();
        let id = NodeId(
            s.execute(
                "path.create",
                &json!({"closed": closed, "anchors": [
                    {"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        assert_eq!(path(&s, id).is_closed(), closed);
        let before = render(&s);
        for (x, y) in samples.into_iter().chain([(150.0, 150.0)]) {
            assert_eq!(before.pixel(x as u32, y as u32), [0, 0, 0, 255], "closed={closed}: original fill at ({x}, {y})");
        }

        // The explicit command fixes the tip at 10 pt; its round ends are outside the rectangle.
        erase(&mut s, json!({"points": [[50, 150], [250, 150]], "size": 10}), varied);
        let remaining: Vec<PathData> = paths(&s);
        let filled =
            |x, y| remaining.iter().any(|path| vectorcraft_geom::hit::fill_contains(&path.to_bezpath(), FillRule::NonZero, Point::new(x, y)));
        let after = render(&s);
        for (x, y) in samples {
            assert!(filled(x, y), "closed={closed}: retained fill missing at ({x}, {y})");
            assert_eq!(after.pixel(x as u32, y as u32), [0, 0, 0, 255], "closed={closed}: retained pixel at ({x}, {y})");
        }
        assert!(!filled(150.0, 150.0), "closed={closed}: eraser band still filled");
        assert_eq!(after.pixel(150, 150)[3], 0, "closed={closed}: erased pixel");
        let area: f64 = remaining.iter().map(|path| vectorcraft_pathops::area(path, FillRule::NonZero)).sum();
        assert!((area - 9000.0).abs() < 1.0, "closed={closed}: retained area {area}");
    }
}

#[test]
fn eraser_preserves_the_fill_of_an_open_rectangle() {
    eraser_preserves_the_fill_of_an_open_rectangle_case(false);
}

#[test]
fn eraser_preserves_the_fill_of_an_open_rectangle_varied_tip() {
    eraser_preserves_the_fill_of_an_open_rectangle_case(true);
}

fn eraser_keeps_open_compound_holes_with_both_fill_rules_case(varied: bool) {
    for rule in ["nonZero", "evenOdd"] {
        let mut s = session();
        let outer = NodeId(
            s.execute(
                "path.create",
                &json!({"anchors": [
                    {"x": 100, "y": 100}, {"x": 300, "y": 100}, {"x": 300, "y": 300}, {"x": 100, "y": 300}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        let inner = NodeId(
            s.execute(
                "path.create",
                &json!({"anchors": [
                    {"x": 150, "y": 150}, {"x": 150, "y": 250}, {"x": 250, "y": 250}, {"x": 250, "y": 150}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        for id in [outer, inner] {
            s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [outer.0, inner.0]})).unwrap();
        let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("path.setFillRule", &json!({"rule": rule})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        let members = match &s.doc().unwrap().doc.node(compound).unwrap().kind {
            NodeKind::Compound { children, .. } => children,
            other => panic!("expected compound path, got {other:?}"),
        };
        assert!(members.iter().all(|member| !member.path_data().unwrap().is_closed()), "{rule}: both source contours remain open");
        let before =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert!(before.pixel(120, 140)[3] > 240, "{rule}: ring fills before Eraser");
        assert_eq!(before.pixel(200, 200)[3], 0, "{rule}: the compound hole is empty before Eraser");
        let before_miss = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let miss_history = s.doc().unwrap().history.undo.len();
        let miss = erase(&mut s, json!({"points": [[200, 200]], "size": 10}), varied);
        assert_eq!(miss["ids"], json!([]), "{rule}: a stroke wholly inside the hole misses");
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_miss);
        assert_eq!(s.doc().unwrap().history.undo.len(), miss_history, "{rule}: the hole miss adds no history");

        erase(&mut s, json!({"points": [[80, 120], [320, 120]], "size": 10}), varied);
        let after =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert_eq!(after.pixel(120, 120)[3], 0, "{rule}: swept band is empty");
        assert!(after.pixel(120, 140)[3] > 240, "{rule}: retained ring is filled");
        assert_eq!(after.pixel(200, 200)[3], 0, "{rule}: compound hole remains empty");
    }
}

#[test]
fn eraser_keeps_open_compound_holes_with_both_fill_rules() {
    eraser_keeps_open_compound_holes_with_both_fill_rules_case(false);
}

#[test]
fn eraser_keeps_open_compound_holes_with_both_fill_rules_varied_tip() {
    eraser_keeps_open_compound_holes_with_both_fill_rules_case(true);
}

fn eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie_case(varied: bool) {
    let mut s = session();
    let id = NodeId(
        s.execute(
            "path.create",
            &json!({"anchors": [
                {"x": 100, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}, {"x": 200, "y": 100}
            ]}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert!(!path(&s, id).is_closed(), "the bowtie is open");
    assert!(path(&s, id).subpaths.first().unwrap().signed_area().abs() < 1e-9, "the bowtie has zero signed area");
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let before = render(&s);
    let before_area: f64 = before.pixels.as_chunks::<4>().0.iter().map(|rgba| f64::from(rgba[3]) / 255.0).sum();
    assert!((before_area - 5000.0).abs() < 2.0, "the open bowtie fills both lobes: {before_area}");
    for y in [110, 140, 175] {
        assert!(before.pixel(150, y)[3] > 240, "original lobe pixel y={y}");
    }

    erase(&mut s, json!({"points": [[50, 125], [250, 125]], "size": 10}), varied);
    let after = render(&s);
    let after_area: f64 = after.pixels.as_chunks::<4>().0.iter().map(|rgba| f64::from(rgba[3]) / 255.0).sum();
    assert!((after_area - 4500.0).abs() < 2.0, "only the 500 px² sweep strip is removed: {after_area}");
    assert!(after.pixel(150, 110)[3] > 240);
    assert!(after.pixel(150, 140)[3] > 240);
    assert!(after.pixel(150, 175)[3] > 240);
    assert_eq!(after.pixel(150, 125)[3], 0, "the swept strip is empty");
    let area: f64 = paths(&s).iter().map(|path| vectorcraft_pathops::area(path, FillRule::NonZero)).sum();
    assert!((area - 4500.0).abs() < 1.0, "retained bowtie area {area}");
}

#[test]
fn eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie() {
    eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie_case(false);
}

#[test]
fn eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie_varied_tip() {
    eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie_case(true);
}

#[test]
fn eraser_keeps_painted_stroke_open_paths_on_the_outline_route() {
    for varied in [false, true] {
        let mut s = session();
        let id = NodeId(
            s.execute(
                "path.create",
                &json!({"anchors": [
                    {"x": 100, "y": 100}, {"x": 200, "y": 100},
                    {"x": 200, "y": 200}, {"x": 100, "y": 200}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 4})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        assert!(!path(&s, id).is_closed());
        let appearance = &s.doc().unwrap().doc.node(id).unwrap().appearance;
        assert_eq!(appearance.stroke_width(), 4.0);
        assert!(!appearance.stroke_paint().is_none());
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let history = s.doc().unwrap().history.undo.len();
        let selection = s.doc().unwrap().selection.clone();
        let before_image =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert_eq!(before_image.pixel(150, 150), [0, 0, 0, 255], "the interior really has visible fill");
        assert!(before_image.pixel(150, 100)[3] > 240, "the open path has painted visible stroke");
        // This sweep lies inside the fill, at least 30 pt from any painted outline.
        // Fill-only scope must not extend to this painted-stroke path or wrap it.
        let miss = erase(&mut s, json!({"points": [[130, 150], [170, 150]], "size": 10}), varied);
        assert_eq!(miss["ids"], json!([]), "varied={varied}: the existing outline route misses");
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before, "varied={varied}: no replacement or wrapper");
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        assert_eq!(s.doc().unwrap().selection, selection);
        let after_image =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert_eq!(after_image.pixels, before_image.pixels);
    }
}

// The Alt-drag marquee dispatches the same public Eraser command with `rect` instead of `points`.
fn marquee_rectangle(s: &mut Session, closed: bool, stroked: bool) -> NodeId {
    let id = NodeId(
        s.execute(
            "path.create",
            &json!({"closed": closed, "anchors": [
                {"x": 100, "y": 100}, {"x": 200, "y": 100},
                {"x": 200, "y": 200}, {"x": 100, "y": 200}
            ]}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
    if stroked {
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 4})).unwrap();
    } else {
        s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
    }
    id
}

#[test]
fn eraser_marquee_preserves_open_fill_like_closed_and_undo() {
    for closed in [true, false] {
        let mut s = session();
        let id = marquee_rectangle(&mut s, closed, false);
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        assert_eq!(path(&s, id).is_closed(), closed);
        let render = |s: &Session| {
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
        };
        let before_image = render(&s);
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        let retained = [(110, 125), (190, 125), (110, 175), (190, 175)];
        for (x, y) in retained.into_iter().chain([(150, 150)]) {
            assert_eq!(before_image.pixel(x, y), [0, 0, 0, 255], "closed={closed}: original fill at ({x}, {y})");
        }
        s.execute("path.eraseRegion", &json!({"rect": [50, 145, 200, 10]})).unwrap();
        let after_image = render(&s);
        for (x, y) in retained {
            assert_eq!(after_image.pixel(x, y), [0, 0, 0, 255], "closed={closed}: marquee lost retained fill at ({x}, {y})");
        }
        assert_eq!(after_image.pixel(150, 150)[3], 0, "closed={closed}: marquee band is empty");
        let remaining = paths(&s);
        let area: f64 = remaining.iter().map(|p| vectorcraft_pathops::area(p, FillRule::NonZero)).sum();
        assert!((area - 9000.0).abs() < 1.0, "closed={closed}: marquee retained area {area}");
        assert_eq!(s.doc().unwrap().history.undo.len(), history + 1, "one marquee is one edit");
        let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let after_selection = s.doc().unwrap().selection.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before, "closed={closed}: Undo restores the original document");
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after);
        assert_eq!(s.doc().unwrap().selection, after_selection);
    }
}

#[test]
fn eraser_marquee_keeps_compound_holes_and_outline_controls() {
    for rule in ["nonZero", "evenOdd"] {
        let mut s = session();
        let mut ids = vec![];
        for anchors in [
            json!([{"x": 100, "y": 100}, {"x": 300, "y": 100}, {"x": 300, "y": 300}, {"x": 100, "y": 300}]),
            json!([{"x": 150, "y": 150}, {"x": 150, "y": 250}, {"x": 250, "y": 250}, {"x": 250, "y": 150}]),
        ] {
            let id = s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
            s.execute("paint.setFill", &json!({"ids": [id], "color": "#000000"})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
            ids.push(id);
        }
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("path.setFillRule", &json!({"rule": rule})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        let members = match &s.doc().unwrap().doc.node(compound).unwrap().kind {
            NodeKind::Compound { children, .. } => children,
            other => panic!("expected compound, got {other:?}"),
        };
        assert!(members.iter().all(|n| !n.path_data().unwrap().is_closed()));
        let render = |s: &Session| {
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
        };
        let before_image = render(&s);
        assert!(before_image.pixel(120, 120)[3] > 240, "{rule}: original ring is filled");
        assert_eq!(before_image.pixel(200, 200)[3], 0, "{rule}: original hole is empty");
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        let miss = s.execute("path.eraseRegion", &json!({"rect": [180, 180, 40, 40]})).unwrap();
        assert_eq!(miss["ids"], json!([]), "{rule}: a marquee wholly inside the hole misses");
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        let result = s.execute("path.eraseRegion", &json!({"rect": [80, 115, 240, 10]})).unwrap();
        assert!(!result["ids"].as_array().unwrap().is_empty());
        for id in result["ids"].as_array().unwrap() {
            assert!(
                matches!(s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind, NodeKind::Compound { .. }),
                "{rule}: no wrapper replaces the compound pieces"
            );
        }
        let after_image = render(&s);
        assert_eq!(after_image.pixel(120, 120)[3], 0, "{rule}: marquee band is empty");
        assert!(after_image.pixel(120, 140)[3] > 240, "{rule}: ring outside the marquee remains filled");
        assert_eq!(after_image.pixel(200, 200)[3], 0, "{rule}: compound hole stays empty");
        let area: f64 = after_image.pixels.as_chunks::<4>().0.iter().map(|rgba| f64::from(rgba[3]) / 255.0).sum();
        assert!((area - 28000.0).abs() < 2.0, "{rule}: marquee removes only its 2000 px² strip: {area}");
        assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
        assert_eq!(s.doc().unwrap().selection, selection);
    }

    // Filled AND painted-stroke open art keeps the existing outline route, so an interior
    // marquee that touches none of its three painted edges is a genuine no-op.
    let mut s = session();
    let id = marquee_rectangle(&mut s, false, true);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let miss = s.execute("path.eraseRegion", &json!({"rect": [130, 145, 40, 10]})).unwrap();
    assert_eq!(miss["ids"], json!([]));
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);

    // Knife's target collection must not inherit the Eraser-only open-fill area extension.
    let mut s = session();
    let id = marquee_rectangle(&mut s, false, false);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let miss = s.execute("path.knife", &json!({"points": [[50, 150], [250, 150]]})).unwrap();
    assert_eq!(miss["ids"], json!([]));
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
}

// A mask is a role of one first child, not a collection of ordinary sibling pieces.
struct ClipMaskFixture {
    group: NodeId,
    mask: NodeId,
    art: NodeId,
    compound: bool,
    rule: FillRule,
    appearance: Value,
    art_json: Value,
}

fn blue_clip_art(s: &mut Session) -> NodeId {
    let art = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [art.0], "color": "#0000ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [art.0], "none": true})).unwrap();
    art
}

fn painted_clip(s: &mut Session, art: NodeId, mask: NodeId, compound: bool) -> ClipMaskFixture {
    s.execute("select.set", &json!({"ids": [art.0, mask.0]})).unwrap();
    let group = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(mask).unwrap().appearance.fill_paint().is_none(), "Make clears the clipping path's paint");
    // The reported area route requires visible fill on the mask AFTER Make, which clears it.
    s.execute("paint.setFill", &json!({"ids": [mask.0], "color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [mask.0], "none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [mask.0]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![mask], "only the mask is an Eraser target");
    let node = s.doc().unwrap().doc.node(mask).unwrap();
    assert!(!node.appearance.fill_paint().is_none(), "the restored visible fill admits the area route");
    assert!(node.appearance.stroke_paint().is_none(), "no painted stroke admits an outline fallback");
    let rule = match &node.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        other => panic!("expected a path or compound mask, got {other:?}"),
    };
    let fixture = ClipMaskFixture {
        group,
        mask,
        art,
        compound,
        rule,
        appearance: serde_json::to_value(&node.appearance).unwrap(),
        art_json: serde_json::to_value(s.doc().unwrap().doc.node(art).unwrap()).unwrap(),
    };
    assert_clip_identity(s, &fixture);
    fixture
}

fn assert_clip_identity(s: &Session, c: &ClipMaskFixture) {
    let d = &s.doc().unwrap().doc;
    let children = match &d.node(c.group).unwrap().kind {
        NodeKind::Group { children, clip: true } => children,
        other => panic!("expected the original clip group, got {other:?}"),
    };
    assert_eq!(children.iter().map(|n| n.id).collect::<Vec<_>>(), [c.mask, c.art], "the one mask stays first; no painted siblings or promoted art");
    let mask = d.node(c.mask).unwrap();
    let rule = match (&mask.kind, c.compound) {
        (NodeKind::Path { rule, clipping: true, .. }, false) | (NodeKind::Compound { rule, .. }, true) => *rule,
        (other, _) => panic!("the original mask kind/role changed: {other:?}"),
    };
    assert_eq!(rule, c.rule);
    assert_eq!(serde_json::to_value(&mask.appearance).unwrap(), c.appearance);
    assert_eq!(serde_json::to_value(d.node(c.art).unwrap()).unwrap(), c.art_json, "the clipped art itself is untouched");
}

fn assert_clip_pixels(s: &Session, blue: &[(u32, u32)], clear: &[(u32, u32)]) {
    let image = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    for &(x, y) in blue {
        assert_eq!(image.pixel(x, y), [0, 0, 255, 255], "clipped blue art at ({x}, {y}), not a painted mask sibling");
    }
    for &(x, y) in clear {
        assert_eq!(image.pixel(x, y)[3], 0, "transparent clip region at ({x}, {y})");
    }
}

fn clip_eraser(s: &mut Session, mode: &str, full: bool) -> Value {
    if mode == "rect" {
        let rect = if full { json!([50, 50, 300, 300]) } else { json!([50, 145, 200, 10]) };
        s.execute("path.eraseRegion", &json!({"rect": rect})).unwrap()
    } else {
        assert!(matches!(mode, "fixed" | "varied"));
        let params = if full { json!({"points": [[150, 150]], "size": 300}) } else { json!({"points": [[50, 150], [250, 150]], "size": 10}) };
        erase(s, params, mode == "varied")
    }
}

fn fully_erase_clip_and_restore(s: &mut Session, c: &ClipMaskFixture, mode: &str) {
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    clip_eraser(s, mode, true);
    let image = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    assert!(image.pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0), "empty geometry must hide all clipped art, not promote it");
    assert_clip_identity(s, c);
    match &s.doc().unwrap().doc.node(c.mask).unwrap().kind {
        NodeKind::Path { path, .. } => assert!(path.is_empty()),
        NodeKind::Compound { children, .. } => assert!(children.is_empty()),
        other => panic!("empty mask retains its kind: {other:?}"),
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    let empty = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let empty_selection = s.doc().unwrap().selection.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), empty);
    assert_eq!(s.doc().unwrap().selection, empty_selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    assert_clip_identity(s, c);
}

#[test]
fn eraser_clipping_split_and_empty_masks_keep_their_single_role() {
    for closed in [false, true] {
        for mode in ["fixed", "varied", "rect"] {
            let mut s = session();
            let art = blue_clip_art(&mut s);
            let mask = marquee_rectangle(&mut s, closed, false);
            let c = painted_clip(&mut s, art, mask, false);
            assert_eq!(path(&s, mask).is_closed(), closed, "the original mask's open/closed state is real");
            assert_clip_pixels(&s, &[(110, 125), (190, 125), (110, 175), (190, 175), (150, 150)], &[(90, 150)]);
            let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
            let selection = s.doc().unwrap().selection.clone();
            let history = s.doc().unwrap().history.undo.len();
            clip_eraser(&mut s, mode, false);
            assert_clip_pixels(&s, &[(110, 125), (190, 125), (110, 175), (190, 175)], &[(150, 150), (90, 150)]);
            assert_clip_identity(&s, &c);
            assert_eq!(s.doc().unwrap().history.undo.len(), history + 1, "closed={closed}, mode={mode}: one erase edit");
            let split = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
            let split_selection = s.doc().unwrap().selection.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), history);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), split);
            assert_eq!(s.doc().unwrap().selection, split_selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
            fully_erase_clip_and_restore(&mut s, &c, mode);
        }
    }
}

#[test]
fn eraser_clipping_compound_holes_and_empty_masks_keep_their_rule() {
    for rule in ["nonZero", "evenOdd"] {
        let mut s = session();
        let art = blue_clip_art(&mut s);
        let mut ids = vec![];
        for anchors in [
            json!([{"x": 100, "y": 100}, {"x": 300, "y": 100}, {"x": 300, "y": 300}, {"x": 100, "y": 300}]),
            json!([{"x": 150, "y": 150}, {"x": 150, "y": 250}, {"x": 250, "y": 250}, {"x": 250, "y": 150}]),
        ] {
            let id = s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
            s.execute("paint.setFill", &json!({"ids": [id], "color": "#000000"})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
            ids.push(id);
        }
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        let mask = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("path.setFillRule", &json!({"rule": rule})).unwrap();
        let c = painted_clip(&mut s, art, mask, true);
        assert_eq!(c.rule, if rule == "evenOdd" { FillRule::EvenOdd } else { FillRule::NonZero });
        let members = match &s.doc().unwrap().doc.node(mask).unwrap().kind {
            NodeKind::Compound { children, .. } => children,
            other => panic!("expected compound, got {other:?}"),
        };
        assert!(members.iter().all(|member| !member.path_data().unwrap().is_closed()), "the source contours are open");
        assert_clip_pixels(&s, &[(120, 110), (120, 120), (120, 140)], &[(200, 200), (90, 110)]);
        // First witness the independent zero-mask promotion defect in actual pixels.
        let original = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let original_selection = s.doc().unwrap().selection.clone();
        let original_history = s.doc().unwrap().history.undo.len();
        fully_erase_clip_and_restore(&mut s, &c, "rect");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), original);
        assert_eq!(s.doc().unwrap().selection, original_selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), original_history);
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        s.execute("path.eraseRegion", &json!({"rect": [80, 115, 240, 10]})).unwrap();
        assert_clip_identity(&s, &c);
        assert_clip_pixels(&s, &[(120, 110), (120, 140)], &[(120, 120), (200, 200), (90, 110)]);
        assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
        let split = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let split_selection = s.doc().unwrap().selection.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), split);
        assert_eq!(s.doc().unwrap().selection, split_selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
        // Member IDs may be rebuilt by set_geometry; only the compound mask's own role is stable.
    }
}

#[test]
fn eraser_clipping_path_eraser_keeps_mask_but_removes_ordinary_art() {
    let mut s = session();
    let art = blue_clip_art(&mut s);
    let mask = marquee_rectangle(&mut s, false, false);
    let c = painted_clip(&mut s, art, mask, false);
    assert!(!path(&s, mask).is_closed());
    assert_clip_pixels(&s, &[(190, 110), (190, 150), (190, 190)], &[(90, 150)]);
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    s.execute("path.eraseSegments", &json!({"points": [[50, 150], [250, 150]], "width": 10})).unwrap();
    // The two open outline pieces close as triangles for clipping, not as an area subtraction.
    assert_clip_pixels(&s, &[(190, 110), (190, 190)], &[(190, 150), (90, 150)]);
    assert_clip_identity(&s, &c);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    let split = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let split_selection = s.doc().unwrap().selection.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), split);
    assert_eq!(s.doc().unwrap().selection, split_selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);

    // The same shared apply_plan must still REMOVE wholly erased ordinary art.
    let mut s = session();
    let ordinary = marquee_rectangle(&mut s, false, false);
    s.execute("select.set", &json!({"ids": [ordinary.0]})).unwrap();
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    s.execute("path.eraseSegments", &json!({"points": [[50, 150], [250, 150]], "width": 300})).unwrap();
    assert!(s.doc().unwrap().doc.node(ordinary).is_none(), "ordinary art does not retain an empty mask node");
    assert!(s.doc().unwrap().selection.is_empty());
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(ordinary).is_none());
    assert!(s.doc().unwrap().selection.is_empty());
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
}

// Bounds-fitted fill gradients are page-space paints, not Along-stroke gradients.
fn eraser_gradient_fill(s: &mut Session, id: NodeId, placed: bool) -> Value {
    let mut gradient = json!({"kind": "linear", "angle": 90, "dither": false, "stops": [
        {"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}
    ]});
    if placed {
        gradient["start"] = json!([150, 200]);
        gradient["end"] = json!([150, 100]);
    }
    s.execute("paint.setFill", &json!({"ids": [id.0], "gradient": gradient})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let vectorcraft_doc::color::Paint::Gradient(g) = s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() else {
        panic!("the public paint command must install a gradient fill");
    };
    assert_eq!(g.geom.is_some(), placed, "the fixture distinguishes fitted and explicitly placed gradients");
    assert!(!g.gradient.dither, "exact retained-pixel comparisons do not use noise");
    serde_json::to_value(g.geom).unwrap()
}

fn eraser_gradient_image(s: &Session) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
}

fn eraser_gradient_region(s: &mut Session, mode: &str, trim: bool, miss: bool) -> Value {
    if mode == "rect" {
        let rect = if miss {
            json!([10, 10, 10, 10])
        } else if trim {
            json!([50, 90, 200, 20])
        } else {
            json!([50, 145, 200, 10])
        };
        s.execute("path.eraseRegion", &json!({"rect": rect})).unwrap()
    } else {
        assert!(matches!(mode, "fixed" | "varied"));
        let params = if miss {
            json!({"points": [[20, 20]], "size": 10})
        } else if trim {
            json!({"points": [[50, 100], [250, 100]], "size": 20})
        } else {
            json!({"points": [[50, 150], [250, 150]], "size": 10})
        };
        erase(s, params, mode == "varied")
    }
}

fn eraser_gradient_rectangle_case(closed: bool, mode: &str, trim: bool, placed: bool) {
    let mut s = session();
    let id = marquee_rectangle(&mut s, closed, false);
    let placed_geom = eraser_gradient_fill(&mut s, id, placed);
    assert_eq!(path(&s, id).is_closed(), closed);
    let before_image = eraser_gradient_image(&s);
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    for y in [125, 175] {
        assert_eq!(before_image.pixel(150, y)[3], 255, "the retained sample is inside the original fill");
    }
    assert_ne!(before_image.pixel(150, 125), before_image.pixel(150, 175), "the original ramp varies vertically");
    let erased_y = if trim { 105 } else { 150 };
    assert_eq!(before_image.pixel(150, erased_y)[3], 255);

    let miss = eraser_gradient_region(&mut s, mode, trim, true);
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "a miss must not pin or replace the document");
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);

    let result = eraser_gradient_region(&mut s, mode, trim, false);
    let ids = result["ids"].as_array().unwrap();
    assert_eq!(ids.len(), if trim { 1 } else { 2 }, "closed={closed}, mode={mode}, trim={trim}: actual piece count");
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(150, erased_y)[3], 0, "the erase operation really removes the intended band");
    for y in [125, 175] {
        assert_eq!(
            after_image.pixel(150, y),
            before_image.pixel(150, y),
            "closed={closed}, mode={mode}, trim={trim}, placed={placed}: retained gradient color at (150, {y})"
        );
    }
    if placed {
        for id in ids {
            let node = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
            let vectorcraft_doc::color::Paint::Gradient(g) = node.appearance.fill_paint() else { panic!("the placed fill remains a gradient") };
            assert_eq!(serde_json::to_value(g.geom).unwrap(), placed_geom, "an explicit vector is not refitted");
        }
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1, "one erase is one edit");
    let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let after_selection = s.doc().unwrap().selection.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after);
    assert_eq!(s.doc().unwrap().selection, after_selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    assert_eq!(eraser_gradient_image(&s).pixels, after_image.pixels, "Redo restores the exact rendered result");
}

#[test]
fn eraser_gradient_retains_page_colors_for_split_trim_and_placed_fill() {
    // A legal already-placed vector is the positive control for the shared pinning seam.
    eraser_gradient_rectangle_case(false, "fixed", false, true);
    for closed in [false, true] {
        for mode in ["fixed", "varied", "rect"] {
            for trim in [false, true] {
                eraser_gradient_rectangle_case(closed, mode, trim, false);
            }
        }
    }
}

fn eraser_gradient_compound_fixture(s: &mut Session, rule: &str) -> (NodeId, NodeId, NodeId) {
    let mut ids = vec![];
    for anchors in [
        json!([{"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}]),
        json!([{"x": 140, "y": 140}, {"x": 140, "y": 160}, {"x": 160, "y": 160}, {"x": 160, "y": 140}]),
    ] {
        let id = s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [id], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
        ids.push(id);
    }
    let outer = NodeId(*ids.first().unwrap());
    let inner = NodeId(*ids.get(1).unwrap());
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("path.setFillRule", &json!({"rule": rule})).unwrap();
    eraser_gradient_fill(s, compound, false);
    let owner = s.doc().unwrap().doc.node(compound).unwrap();
    let members = match &owner.kind {
        NodeKind::Compound { children, .. } => children,
        other => panic!("expected a compound, got {other:?}"),
    };
    assert_eq!(members.len(), 2);
    for member in members {
        assert!(!member.path_data().unwrap().is_closed(), "the compound's source contours are genuinely open");
        assert!(member.appearance.fill_paint().is_none() && member.appearance.stroke_paint().is_none(), "the parent alone owns the paint");
    }
    let bounds = owner.geometric_bounds().unwrap();
    assert_eq!(bounds, vectorcraft_geom::Rect::new(100.0, 100.0, 200.0, 200.0));
    (outer, inner, compound)
}

fn eraser_gradient_compound_case(outline: bool, rule: &str) {
    let mut s = session();
    let (outer, inner, compound) = eraser_gradient_compound_fixture(&mut s, rule);
    let before_image = eraser_gradient_image(&s);
    let retained = if outline { vec![(190, 175)] } else { vec![(150, 125), (150, 175)] };
    for &(x, y) in &retained {
        assert_eq!(before_image.pixel(x, y)[3], 255, "{rule}: retained source interior");
    }
    assert_ne!(before_image.pixel(190, 125), before_image.pixel(190, 175), "the parent paint is a nonconstant vertical ramp");
    assert_eq!(before_image.pixel(155, 155)[3], 0, "the source hole is empty");
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let inner_before = serde_json::to_value(s.doc().unwrap().doc.node(inner).unwrap()).unwrap();
    let miss = if outline {
        s.execute("path.eraseSegments", &json!({"points": [[20, 20]], "width": 20})).unwrap()
    } else {
        s.execute("path.eraseRegion", &json!({"rect": [145, 145, 10, 10]})).unwrap()
    };
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "{rule}: no-op must not pin the owner");
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert_eq!(s.doc().unwrap().selection, selection);

    if outline {
        // Path Eraser plans the outer leaf, but its Compound parent supplies the gradient.
        s.execute("path.eraseSegments", &json!({"points": [[50, 100], [250, 100]], "width": 20})).unwrap();
        let after_bounds = s.doc().unwrap().doc.node(compound).unwrap().geometric_bounds().unwrap();
        assert!((after_bounds.y0 - 110.0).abs() < 1e-5, "the edited extremal outline shrinks the parent fit: {after_bounds:?}");
        assert_eq!(serde_json::to_value(s.doc().unwrap().doc.node(inner).unwrap()).unwrap(), inner_before, "the inner member itself was not edited");
        let hole = Point::new(155.0, 155.0);
        assert!(
            vectorcraft_geom::hit::fill_contains(&path(&s, outer).to_bezpath(), FillRule::NonZero, hole),
            "the sample remains inside the outer fill"
        );
        assert!(
            vectorcraft_geom::hit::fill_contains(&path(&s, inner).to_bezpath(), FillRule::NonZero, hole),
            "the unchanged inner contour still makes a hole there"
        );
    } else {
        s.execute("path.eraseRegion", &json!({"rect": [50, 145, 200, 10]})).unwrap();
    }
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(155, 155)[3], 0, "{rule}: the un-erased part of the hole stays empty");
    if !outline {
        assert_eq!(after_image.pixel(190, 150)[3], 0, "the marquee actually removes filled art across the ring");
        assert_eq!(after_image.pixel(150, 142)[3], 0, "the hole above the erased band remains empty");
    }
    for &(x, y) in &retained {
        assert_eq!(
            after_image.pixel(x, y),
            before_image.pixel(x, y),
            "{rule}, outline={outline}: parent-owned gradient must keep its original color at ({x}, {y})"
        );
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let after_selection = s.doc().unwrap().selection.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after);
    assert_eq!(s.doc().unwrap().selection, after_selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    assert_eq!(eraser_gradient_image(&s).pixels, after_image.pixels);
}

#[test]
fn eraser_gradient_compound_owner_keeps_page_colors_and_holes() {
    for outline in [true, false] {
        for rule in ["nonZero", "evenOdd"] {
            eraser_gradient_compound_case(outline, rule);
        }
    }
}

fn eraser_container_gradient_fixture(s: &mut Session, placed: bool, nested: bool) -> (NodeId, NodeId, NodeId, Value) {
    let mut ids = vec![];
    for anchors in [
        json!([{"x": 100, "y": 100}, {"x": 150, "y": 100}, {"x": 150, "y": 140}, {"x": 100, "y": 140}]),
        json!([{"x": 170, "y": 160}, {"x": 220, "y": 160}, {"x": 220, "y": 200}, {"x": 170, "y": 200}]),
    ] {
        let id = s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [id], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
        ids.push(id);
    }
    let edited = NodeId(ids[0]);
    let sibling = NodeId(ids[1]);
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let inner = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let owner = if nested { NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap()) } else { inner };
    s.execute("appearance.addFill", &json!({"ids": [owner.0], "target": "object"})).unwrap();
    let mut gradient = json!({"kind": "linear", "angle": 90, "dither": false, "stops": [
        {"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}
    ]});
    if placed {
        gradient["start"] = json!([160, 200]);
        gradient["end"] = json!([160, 100]);
    }
    s.execute("appearance.setItem", &json!({"ids": [owner.0], "target": "object", "index": 0, "gradient": gradient})).unwrap();
    let d = &s.doc().unwrap().doc;
    let n = d.node(owner).unwrap();
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert_eq!(n.appearance.items.len(), 1, "the Group owns one fill row");
    assert_eq!(n.appearance.contents_at(), 0, "the own fill paints above the children's black fills");
    let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the Group itself owns the gradient") };
    assert_eq!(g.geom.is_some(), placed);
    assert!(!g.gradient.dither);
    let geom = serde_json::to_value(g.geom).unwrap();
    assert_eq!(n.geometric_bounds().unwrap(), vectorcraft_geom::Rect::new(100.0, 100.0, 220.0, 200.0));
    for id in [edited, sibling] {
        assert!(!d.node(id).unwrap().appearance.has_unplaced_gradient(), "the gradient is not on a leaf");
    }
    assert_eq!(d.parent_of(edited), Some(inner));
    if nested {
        assert_eq!(d.parent_of(inner), Some(owner));
        assert!(d.node(inner).unwrap().appearance.items.is_empty(), "the intervening Group has no own paint");
    }
    s.execute("select.set", &json!({"ids": [edited.0]})).unwrap();
    (edited, sibling, owner, geom)
}

fn eraser_container_gradient_restore(s: &mut Session, before: &Value, selection: &vectorcraft_doc::Selection, history: usize) {
    let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let after_selection = s.doc().unwrap().selection.clone();
    let after_pixels = eraser_gradient_image(s).pixels;
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1, "one erase remains one edit");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), *before);
    assert_eq!(&s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after);
    assert_eq!(s.doc().unwrap().selection, after_selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    assert_eq!(eraser_gradient_image(s).pixels, after_pixels);
}

fn eraser_container_gradient_case(placed: bool, nested: bool) {
    let mut s = session();
    let (edited, sibling, owner, geom) = eraser_container_gradient_fixture(&mut s, placed, nested);
    let before_image = eraser_gradient_image(&s);
    assert_eq!(before_image.pixel(125, 105)[3], 255, "the extremal child really paints the erase region");
    assert_eq!(before_image.pixel(190, 175)[3], 255);
    assert_ne!(before_image.pixel(190, 175), before_image.pixel(190, 185), "the owner's paint varies on the untouched sibling");
    assert_ne!(before_image.pixel(190, 175), [0, 0, 0, 255], "the Group's top paint actually covers the black child");
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let sibling_before = serde_json::to_value(before.node(sibling).unwrap()).unwrap();
    let ancestry = before.ancestry(edited).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let miss = eraser_gradient_region(&mut s, "rect", true, true);
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "a miss must not replace any ancestor");
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let result = eraser_gradient_region(&mut s, "rect", true, false);
    assert_eq!(result["ids"].as_array().unwrap().len(), 1, "trim keeps one piece, not a split");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.ancestry(edited).unwrap(), ancestry, "the child and every original container retain their roles");
    assert_eq!(serde_json::to_value(d.node(sibling).unwrap()).unwrap(), sibling_before, "the sibling itself was not edited");
    let n = d.node(owner).unwrap();
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert!((n.geometric_bounds().unwrap().y0 - 110.0).abs() < 1e-5, "the owner's original minimum y really shrinks");
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(125, 105)[3], 0, "the intended band is erased through the Group's paint");
    assert_eq!(
        after_image.pixel(190, 175),
        before_image.pixel(190, 175),
        "placed={placed}, nested={nested}: erasing one child must not recolor its untouched sibling"
    );
    if placed {
        let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the own fill stays a gradient") };
        assert_eq!(serde_json::to_value(g.geom).unwrap(), geom, "an explicitly placed owner is not refitted");
    }
    eraser_container_gradient_restore(&mut s, &before_json, &selection, history);
}

#[test]
fn eraser_container_gradient_keeps_untouched_sibling_colors() {
    // Positive control runs before the unchanged-product fitted-paint failure.
    eraser_container_gradient_case(true, false);
    eraser_container_gradient_case(false, false);
    eraser_container_gradient_case(false, true);
}

fn eraser_container_effect_gradient_case(group: bool, placed: bool) {
    let mut s = session();
    let (edited, owner, geom) = if group {
        let (edited, _, owner, geom) = eraser_container_gradient_fixture(&mut s, placed, false);
        (edited, owner, geom)
    } else {
        let id = marquee_rectangle(&mut s, true, false);
        let geom = eraser_gradient_fill(&mut s, id, placed);
        (id, id, geom)
    };
    s.execute(
        "effect.apply",
        &json!({
            "ids": [owner.0], "target": "object", "item": null,
            "effect": "path.offsetPath", "params": {"offset": 10, "joins": "round"}
        }),
    )
    .unwrap();
    let d = &s.doc().unwrap().doc;
    let n = d.node(owner).unwrap();
    assert_eq!(n.appearance.effects.len(), 1);
    let effect = &n.appearance.effects[0];
    assert_eq!(effect.id, "path.offsetPath");
    assert!(effect.visible);
    assert_eq!(effect.params["offset"].as_f64(), Some(10.0));
    assert_eq!(effect.params["joins"], json!("round"));
    let effect_before = serde_json::to_value(&n.appearance.effects).unwrap();
    let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the effect owner really owns the gradient") };
    assert_eq!(g.geom.is_some(), placed);
    assert!(!g.gradient.dither);
    assert_eq!(g.gradient.stops.len(), 2);
    assert_ne!(g.gradient.stops[0].color, g.gradient.stops[1].color, "the own paint is a real nonconstant ramp before rendering");
    let original_bounds = n.geometric_bounds().unwrap();
    assert_eq!((original_bounds.y0, original_bounds.y1), (100.0, 200.0));
    let before_image = eraser_gradient_image(&s);
    assert_eq!(before_image.pixel(125, 105)[3], 255);
    assert_eq!(before_image.pixel(190, 175)[3], 255);
    assert_eq!(before_image.pixel(190, 185)[3], 255);
    assert_ne!(before_image.pixel(190, 175), before_image.pixel(190, 185), "the rendered own gradient is nonconstant");
    assert_eq!(before_image.pixel(190, 205)[3], 255, "the live positive offset paints beyond the source's y=200 bounds");
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let ancestry = before.ancestry(edited).unwrap();
    let miss = s.execute("path.eraseRegion", &json!({"rect": [10, 10, 10, 10]})).unwrap();
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);

    // Remove 30 pt of source height: the remaining +10 pt live offset cannot
    // repaint the cleared (125,105) pixel. The offset itself remains live.
    let result = s.execute("path.eraseRegion", &json!({"rect": [50, 80, 200, 50]})).unwrap();
    assert_eq!(result["ids"].as_array().unwrap().len(), 1, "this is a one-piece extremal trim");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.ancestry(edited).unwrap(), ancestry);
    let n = d.node(owner).unwrap();
    assert!((n.geometric_bounds().unwrap().y0 - 130.0).abs() < 1e-5, "the original owner bounds really shrink");
    assert_eq!(serde_json::to_value(&n.appearance.effects).unwrap(), effect_before, "the edit must not flatten or remove the live effect");
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(125, 105)[3], 0, "the +10 offset cannot repaint this deliberately deep erased band");
    assert_eq!(
        after_image.pixel(190, 175),
        before_image.pixel(190, 175),
        "group={group}, placed={placed}: freezing the raw Node box must not recolor retained effected art"
    );
    if placed {
        let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the placed fill remains a gradient") };
        assert_eq!(serde_json::to_value(g.geom).unwrap(), geom);
    }
    eraser_container_gradient_restore(&mut s, &before_json, &selection, history);
}

#[test]
fn eraser_container_gradient_keeps_evaluated_offset_fit() {
    // Establish ordinary legal placement through the SAME live offset first.
    eraser_container_effect_gradient_case(true, true);
    eraser_container_effect_gradient_case(false, true);
    eraser_container_effect_gradient_case(true, false);
    eraser_container_effect_gradient_case(false, false);
}

fn eraser_placement_ramp(placed: bool, bottom: f64, top: f64) -> Value {
    let mut gradient = json!({"kind": "linear", "angle": 90, "dither": false, "stops": [
        {"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}
    ]});
    if placed {
        gradient["start"] = json!([160, bottom]);
        gradient["end"] = json!([160, top]);
    }
    gradient
}

fn eraser_placement_effects(s: &Session, owner: NodeId) -> Value {
    let n = s.doc().unwrap().doc.node(owner).unwrap();
    json!({"object": n.appearance.effects,
        "items": n.appearance.items.iter().map(|item| item.effects()).collect::<Vec<_>>()})
}

fn eraser_placement_trim(
    s: &mut Session,
    edited: NodeId,
    owner: NodeId,
    clear: (u32, u32),
) -> (vectorcraft_render::Rendered, vectorcraft_render::Rendered) {
    let before_image = eraser_gradient_image(s);
    assert_eq!(before_image.pixel(clear.0, clear.1)[3], 255, "the actual effected art paints the erase witness");
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let ancestry = before.ancestry(edited).unwrap();
    let effects = eraser_placement_effects(s, owner);
    let bounds = before.node(owner).unwrap().geometric_bounds().unwrap();
    assert_eq!((bounds.y0, bounds.y1), (100.0, 200.0));
    let miss = s.execute("path.eraseRegion", &json!({"rect": [10, 10, 10, 10]})).unwrap();
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let result = s.execute("path.eraseRegion", &json!({"rect": [50, 80, 200, 50]})).unwrap();
    assert_eq!(result["ids"].as_array().unwrap().len(), 1, "the extremal trim keeps one piece");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.ancestry(edited).unwrap(), ancestry);
    assert!((d.node(owner).unwrap().geometric_bounds().unwrap().y0 - 130.0).abs() < 1e-5);
    assert_eq!(eraser_placement_effects(s, owner), effects, "stored object and item effects stay live and identical");
    let after_image = eraser_gradient_image(s);
    assert_eq!(after_image.pixel(clear.0, clear.1)[3], 0, "the witness really leaves the effected art");
    eraser_container_gradient_restore(s, &before_json, &selection, history);
    (before_image, after_image)
}

fn eraser_placement_matches(
    placed: &(vectorcraft_render::Rendered, vectorcraft_render::Rendered),
    fitted: &(vectorcraft_render::Rendered, vectorcraft_render::Rendered),
    samples: &[(u32, u32)],
) {
    for &(x, y) in samples {
        assert_eq!(placed.0.pixel(x, y)[3], 255);
        assert_eq!(placed.1.pixel(x, y)[3], 255, "the equivalent placed control retains the sample");
        assert_eq!(fitted.0.pixel(x, y), placed.0.pixel(x, y), "the public placed vector is equivalent BEFORE the edit at ({x},{y})");
        assert_eq!(
            fitted.1.pixel(x, y),
            placed.1.pixel(x, y),
            "fitted and equivalent placed paint follow the SAME live effects AFTER the edit at ({x},{y})"
        );
    }
    assert_ne!(placed.0.pixel(samples[0].0, samples[0].1), placed.0.pixel(samples[1].0, samples[1].1));
}

fn eraser_placement_affine_scene(placed: bool) -> (vectorcraft_render::Rendered, vectorcraft_render::Rendered) {
    let mut s = session();
    let (edited, _, owner, _) = eraser_container_gradient_fixture(&mut s, false, false);
    // Offset10 makes the original aggregate fit y=90..210. The explicit control
    // fixes THAT frame before the following Transform carries it, not the raw box.
    s.execute(
        "appearance.setItem",
        &json!({"ids": [owner.0], "target": "object", "index": 0,
        "gradient": eraser_placement_ramp(placed, 210.0, 90.0)}),
    )
    .unwrap();
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
        "effect": "path.offsetPath", "params": {"offset": 10, "joins": "round"}}),
    )
    .unwrap();
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
        "effect": "distort.transform", "params": {"scaleH": 100, "scaleV": 150,
            "reference": 8, "objects": true, "copies": 0, "random": false,
            "rotate": 0, "moveH": 0, "moveV": 0, "patterns": false, "strokes": false}}),
    )
    .unwrap();
    let n = s.doc().unwrap().doc.node(owner).unwrap();
    assert_eq!(n.appearance.effects.len(), 2);
    let transform = &n.appearance.effects[1];
    assert_eq!(transform.id, "distort.transform");
    assert!(transform.visible);
    assert_eq!(transform.params["scaleV"].as_f64(), Some(150.0));
    assert_eq!(transform.params["reference"].as_f64(), Some(8.0));
    assert_eq!(transform.params["objects"].as_bool(), Some(true));
    assert_eq!(transform.params["copies"].as_f64(), Some(0.0));
    let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("own Group gradient missing") };
    assert_eq!(g.geom.is_some(), placed);
    // Reference8 is the bottom-right of the effected aggregate (230,210),
    // unchanged by the top trim. V150 sends the original y90 edge to y30;
    // without this actual affine action pixel(125,60) would be transparent.
    eraser_placement_trim(&mut s, edited, owner, (125, 60))
}

#[test]
fn eraser_placement_control_affine_carry_matches_equivalent_placed() {
    let placed = eraser_placement_affine_scene(true);
    let fitted = eraser_placement_affine_scene(false);
    eraser_placement_matches(&placed, &fitted, &[(190, 175), (190, 185)]);
}

fn eraser_placement_item_stroke_scene(placed: bool) -> (vectorcraft_render::Rendered, vectorcraft_render::Rendered) {
    let mut s = session();
    let id = marquee_rectangle(&mut s, true, false);
    s.execute("paint.setFill", &json!({"ids": [id.0], "none": true})).unwrap();
    s.execute("stroke.set", &json!({"ids": [id.0], "weight": 10})).unwrap();
    // The row's +10 geometry expands y100..200 to y90..210; Within stroke
    // weight10 fits its gradient to the inflated y85..215 paint box.
    s.execute(
        "paint.setStroke",
        &json!({"ids": [id.0],
        "gradient": eraser_placement_ramp(placed, 215.0, 85.0)}),
    )
    .unwrap();
    let index = s
        .doc()
        .unwrap()
        .doc
        .node(id)
        .unwrap()
        .appearance
        .items
        .iter()
        .position(|item| matches!(item, vectorcraft_doc::AppearanceItem::Stroke(_)))
        .unwrap();
    s.execute("paint.editGradient", &json!({"ids": [id.0], "item": index, "stroke": true, "strokeMode": "within"})).unwrap();
    s.execute(
        "effect.apply",
        &json!({"ids": [id.0], "target": "object", "item": index,
        "effect": "path.offsetPath", "params": {"offset": 10, "joins": "round"}}),
    )
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert!(matches!(n.kind, NodeKind::Path { .. }));
    assert!(n.appearance.fill_paint().is_none());
    assert!(n.appearance.effects.is_empty(), "this effect belongs only to the stroke row");
    let vectorcraft_doc::AppearanceItem::Stroke(st) = &n.appearance.items[index] else { panic!("public stroke row missing") };
    assert_eq!(st.width, 10.0);
    assert_eq!(st.gradient_mode, vectorcraft_doc::StrokeGradientMode::Within);
    assert!(st.path_gradient().is_none(), "the test excludes Along/Across path-coordinate paint");
    assert_eq!(st.effects.len(), 1);
    assert_eq!(st.effects[0].id, "path.offsetPath");
    assert!(st.effects[0].visible);
    let vectorcraft_doc::color::Paint::Gradient(g) = &st.paint else { panic!("stroke gradient missing") };
    assert_eq!(g.geom.is_some(), placed);
    assert!(!g.gradient.dither);
    eraser_placement_trim(&mut s, id, id, (150, 90))
}

#[test]
fn eraser_placement_control_item_within_stroke_matches_equivalent_placed() {
    let placed = eraser_placement_item_stroke_scene(true);
    let fitted = eraser_placement_item_stroke_scene(false);
    eraser_placement_matches(&placed, &fitted, &[(210, 175), (210, 185)]);
}

fn eraser_mixed_gradient_hidden_stroke(s: &mut Session, id: NodeId, rasterize: bool) -> usize {
    let index = s.doc().unwrap().doc.node(id).unwrap().appearance.items.iter().position(|item| !item.is_fill()).unwrap();
    s.execute(
        "appearance.setItem",
        &json!({"ids": [id.0], "index": index, "weight": 4, "gradient": {
            "kind": "linear", "stops": [{"offset": 0, "color": "#00ff00"}, {"offset": 1, "color": "#ff00ff"}]
        }}),
    )
    .unwrap();
    let mode = if rasterize { "within" } else { "along" };
    s.execute("paint.editGradient", &json!({"ids": [id.0], "item": index, "stroke": true, "strokeMode": mode})).unwrap();
    if rasterize {
        s.execute("effect.apply", &json!({"ids": [id.0], "target": "object", "item": index, "effect": "rasterize", "params": {}})).unwrap();
    }
    s.execute("appearance.setItem", &json!({"ids": [id.0], "index": index, "visible": false})).unwrap();
    index
}

fn eraser_mixed_gradient_rows_case(placed: bool, rasterize: bool) -> ([[u8; 4]; 2], [[u8; 4]; 2]) {
    let mut s = session();
    let id = marquee_rectangle(&mut s, false, false);
    let placed_geom = eraser_gradient_fill(&mut s, id, placed);
    let index = eraser_mixed_gradient_hidden_stroke(&mut s, id, rasterize);
    let node = s.doc().unwrap().doc.node(id).unwrap();
    assert!(!path(&s, id).is_closed(), "the fixture exercises the fill-only open-path route");
    assert!(node.appearance.has_drawn_fill());
    let hidden = node.appearance.items[index].clone();
    let vectorcraft_doc::AppearanceItem::Stroke(stroke) = &hidden else { panic!("the explicit row is a stroke") };
    assert!(!stroke.visible);
    assert_eq!(stroke.width, 4.0, "the hidden row has a genuine positive stroke weight");
    assert!(matches!(stroke.paint, vectorcraft_doc::color::Paint::Gradient(_)));
    assert_eq!(stroke.path_gradient().is_some(), !rasterize, "the two cases isolate the two owner-wide guard branches");
    assert_eq!(
        stroke.gradient_mode,
        if rasterize { vectorcraft_doc::StrokeGradientMode::Within } else { vectorcraft_doc::StrokeGradientMode::Along }
    );
    assert_eq!(stroke.effects.iter().any(|e| e.visible && e.id == "rasterize"), rasterize);
    assert!(
        node.appearance
            .items
            .iter()
            .all(|item| { !matches!(item, vectorcraft_doc::AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0) }),
        "no visible painted stroke may change Eraser's area eligibility"
    );
    let vectorcraft_doc::color::Paint::Gradient(fill) = node.appearance.fill_paint() else { panic!("the visible fill is a gradient") };
    assert_eq!(fill.geom.is_some(), placed);
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let before_image = eraser_gradient_image(&s);
    let before_colors = [before_image.pixel(150, 125), before_image.pixel(150, 175)];
    assert!(before_colors.iter().all(|color| color[3] == 255));
    assert_ne!(before_colors[0], before_colors[1], "the visible fill is a genuine nonconstant ramp");
    assert_eq!(before_image.pixel(150, 150)[3], 255);

    let miss = eraser_gradient_region(&mut s, "rect", false, true);
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);

    let result = eraser_gradient_region(&mut s, "rect", false, false);
    let ids = result["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2, "the marquee splits the filled area into two retained pieces");
    for value in ids {
        let node = s.doc().unwrap().doc.node(NodeId(value.as_u64().unwrap())).unwrap();
        assert_eq!(node.appearance.items[index], hidden, "the unsupported hidden row stays byte-for-byte unchanged");
        if placed {
            let vectorcraft_doc::color::Paint::Gradient(fill) = node.appearance.fill_paint() else { panic!("the fill stays a gradient") };
            assert_eq!(serde_json::to_value(fill.geom).unwrap(), placed_geom);
        }
    }
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(150, 150)[3], 0, "the intended fill band is erased");
    let after_colors = [after_image.pixel(150, 125), after_image.pixel(150, 175)];
    eraser_container_gradient_restore(&mut s, &before_json, &selection, history);
    (before_colors, after_colors)
}

#[test]
fn eraser_mixed_gradient_rows_keep_visible_fill_placement() {
    let placed_along = eraser_mixed_gradient_rows_case(true, false);
    assert_eq!(placed_along.0, placed_along.1, "an explicitly placed fill with hidden Along stroke is the legitimate control");
    let placed_rasterize = eraser_mixed_gradient_rows_case(true, true);
    assert_eq!(placed_rasterize.0, placed_rasterize.1, "an explicitly placed fill with a hidden Rasterize row remains unchanged");
    // Finish both fitted scenes, including metadata/no-op/Undo controls, before the shared color assertion.
    let fitted_along = eraser_mixed_gradient_rows_case(false, false);
    let fitted_rasterize = eraser_mixed_gradient_rows_case(false, true);
    assert!(
        fitted_along.0 == fitted_along.1 && fitted_rasterize.0 == fitted_rasterize.1,
        "hidden rows must not suppress the visible fill's placement: Along {fitted_along:?}; Rasterize {fitted_rasterize:?}"
    );
}

fn eraser_endpoint_region(s: &mut Session, mode: &str, miss: bool) -> Value {
    if mode == "rect" {
        let rect = if miss { json!([55, 145, 10, 10]) } else { json!([150, 140, 20, 20]) };
        s.execute("path.eraseRegion", &json!({"rect": rect})).unwrap()
    } else {
        let points = if miss { json!([[60, 150]]) } else { json!([[160, 150]]) };
        erase(s, json!({"points": points, "size": 20}), mode == "varied")
    }
}

fn eraser_keeps_unused_endpoint_handles_out_of_fill_case(mode: &str) {
    for (closed, stroked) in [(false, false), (true, false), (false, true)] {
        let mut s = session();
        let id = NodeId(
            s.execute(
                "path.create",
                &json!({"closed": closed, "anchors": [
                    {"x": 100, "y": 100, "in": [0, 100]}, {"x": 200, "y": 100},
                    {"x": 200, "y": 200}, {"x": 100, "y": 200, "out": [0, 200]}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        if stroked {
            s.execute("paint.setFill", &json!({"ids": [id.0], "none": true})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
            s.execute("stroke.set", &json!({"ids": [id.0], "weight": 4})).unwrap();
        } else {
            s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let sp = &path(&s, id).subpaths[0];
        assert_eq!(sp.closed, closed);
        assert_eq!(sp.anchors.first().unwrap().h_in, Point::new(0.0, 100.0));
        assert_eq!(sp.anchors.last().unwrap().h_out, Point::new(0.0, 200.0));
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        let before_image = eraser_gradient_image(&s);
        assert_eq!(before_image.pixel(60, 150)[3], if closed { 255 } else { 0 }, "{mode}: fixture closure");
        assert_eq!(before_image.pixel(160, 150)[3], if stroked { 0 } else { 255 }, "{mode}: interior witness");

        let result = eraser_endpoint_region(&mut s, mode, false);
        let after_image = eraser_gradient_image(&s);
        assert_eq!(after_image.pixel(160, 150)[3], 0, "{mode}: erased interior");
        assert_eq!(after_image.pixel(60, 150), before_image.pixel(60, 150), "{mode}: no new bulge or lost closed curve");
        if stroked {
            assert_eq!(result["ids"], json!([]), "{mode}: the open outline misses an interior dab");
            assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), history);
            continue;
        }
        assert_eq!(after_image.pixel(180, 180), [0, 0, 0, 255], "{mode}: retained fill");
        assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before, "{mode}: original geometry and endpoint handles restored");
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        if !closed {
            let miss = eraser_endpoint_region(&mut s, mode, true);
            assert_eq!(miss["ids"], json!([]), "{mode}: the phantom bulge cannot count as an intersection");
            assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
            assert_eq!(s.doc().unwrap().selection, selection);
            assert_eq!(s.doc().unwrap().history.undo.len(), history);
        }
    }
}

#[test]
fn eraser_keeps_unused_endpoint_handles_out_of_fill_marquee() {
    eraser_keeps_unused_endpoint_handles_out_of_fill_case("rect");
}

#[test]
fn eraser_keeps_unused_endpoint_handles_out_of_fill_fixed_tip() {
    eraser_keeps_unused_endpoint_handles_out_of_fill_case("fixed");
}

#[test]
fn eraser_keeps_unused_endpoint_handles_out_of_fill_varied_tip() {
    eraser_keeps_unused_endpoint_handles_out_of_fill_case("varied");
}

fn eraser_copying_container_gradient_before_colors(before_image: &vectorcraft_render::Rendered) -> [[u8; 4]; 2] {
    let before_colors = [before_image.pixel(190, 175), before_image.pixel(190, 185)];
    assert!(before_colors.iter().all(|color| color[3] == 255));
    assert_ne!(before_colors[0], before_colors[1], "the original sibling draws a nonconstant gradient");
    assert_ne!(before_colors[0], [0, 0, 0, 255], "the Group's own fill covers the black child");
    assert_eq!(before_image.pixel(125, 105)[3], 255, "the original extremal child paints the trim witness");
    assert_eq!(before_image.pixel(190, 295)[3], 255, "the translated sibling copy really paints");
    before_colors
}

fn eraser_copying_container_gradient_child_offset(s: &mut Session, sibling: NodeId, owner: NodeId, placed: bool) {
    s.execute(
        "effect.apply",
        &json!({"ids": [sibling.0], "target": "object", "item": null,
            "effect": "path.offsetPath", "params": {"offset": 10, "joins": "round"}}),
    )
    .unwrap();
    // The untouched sibling's live offset makes the evaluated source box
    // [100,100,230,210]; the Group's raw source box is still [100,100,220,200].
    let mut gradient = eraser_placement_ramp(placed, 210.0, 100.0);
    if placed {
        gradient["start"] = json!([165, 210]);
        gradient["end"] = json!([165, 100]);
    }
    s.execute("appearance.setItem", &json!({"ids": [owner.0], "target": "object", "index": 0, "gradient": gradient})).unwrap();
    let sibling_node = s.doc().unwrap().doc.node(sibling).unwrap();
    assert_eq!(sibling_node.appearance.effects.len(), 1);
    assert_eq!(sibling_node.appearance.effects[0].id, "path.offsetPath");
    assert!(sibling_node.appearance.effects[0].visible);
    assert_eq!(sibling_node.appearance.effects[0].params["offset"].as_f64(), Some(10.0));
}

fn eraser_copying_container_gradient_scene(placed: bool) -> ([[u8; 4]; 2], [[u8; 4]; 2]) {
    let mut s = session();
    let (edited, sibling, owner, geom) = eraser_container_gradient_fixture(&mut s, placed, false);
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
            "effect": "stylize.dropShadow", "params": {"x": 10, "y": 0, "blur": 0}}),
    )
    .unwrap();
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
            "effect": "distort.transform", "params": {"scaleH": 100, "scaleV": 100,
                "reference": 8, "objects": true, "copies": 1, "random": false,
                "rotate": 0, "moveH": 0, "moveV": 120, "patterns": false, "strokes": false}}),
    )
    .unwrap();
    let n = s.doc().unwrap().doc.node(owner).unwrap();
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert_eq!(n.appearance.effects.len(), 2);
    assert_eq!(n.appearance.effects[0].id, "stylize.dropShadow");
    assert!(n.appearance.effects[0].visible);
    assert_eq!(n.appearance.effects[1].id, "distort.transform");
    assert!(n.appearance.effects[1].visible);
    assert_eq!(n.appearance.effects[1].params["copies"].as_f64(), Some(1.0));
    assert_eq!(n.appearance.effects[1].params["moveV"].as_f64(), Some(120.0));
    let effects = serde_json::to_value(&n.appearance.effects).unwrap();
    let before_image = eraser_gradient_image(&s);
    let before_colors = eraser_copying_container_gradient_before_colors(&before_image);
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let sibling_before = serde_json::to_value(before.node(sibling).unwrap()).unwrap();
    let ancestry = before.ancestry(edited).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let miss = eraser_gradient_region(&mut s, "rect", true, true);
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "a miss cannot pin an ancestor");
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let result = eraser_gradient_region(&mut s, "rect", true, false);
    assert_eq!(result["ids"].as_array().unwrap().len(), 1, "the extremal trim keeps one piece");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.ancestry(edited).unwrap(), ancestry);
    assert_eq!(serde_json::to_value(d.node(sibling).unwrap()).unwrap(), sibling_before, "the sibling itself is unchanged");
    let n = d.node(owner).unwrap();
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert!((n.geometric_bounds().unwrap().y0 - 110.0).abs() < 1e-5);
    assert_eq!(serde_json::to_value(&n.appearance.effects).unwrap(), effects, "both effects stay live and unchanged");
    if placed {
        let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the own fill stays a gradient") };
        assert_eq!(serde_json::to_value(g.geom).unwrap(), geom, "the explicit source fit must not be rewritten");
    }
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(125, 105)[3], 0, "the intended band was erased");
    let after_colors = [after_image.pixel(190, 175), after_image.pixel(190, 185)];
    assert!(after_colors.iter().all(|color| color[3] == 255));
    eraser_container_gradient_restore(&mut s, &before_json, &selection, history);
    (before_colors, after_colors)
}

#[test]
fn eraser_copying_container_gradient_keeps_source_fit_on_untouched_sibling() {
    // The source is painted before its translated copy is made. The fixture's explicit
    // [160,200] -> [160,100] fit is equivalent to the source's original fitted paint.
    let placed = eraser_copying_container_gradient_scene(true);
    let fitted = eraser_copying_container_gradient_scene(false);
    assert_eq!(placed.0, placed.1, "the legitimate explicitly placed source fit preserves the untouched sibling");
    assert_eq!(fitted.0, placed.0, "both source fits must draw the same original sibling before editing");
    assert_eq!(fitted.1, fitted.0, "pinning a copying container must not recolor an untouched original sibling");
    assert_eq!(fitted.1, placed.1, "the fitted edit must match the equivalent explicitly placed source fit");
}

fn eraser_copying_container_gradient_owner_effects(n: &vectorcraft_doc::Node, nested_copy: bool) {
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert_eq!(n.appearance.effects.len(), if nested_copy { 3 } else { 2 });
    assert_eq!(n.appearance.effects[0].id, "stylize.dropShadow");
    assert!(n.appearance.effects[0].visible);
    for (index, effect) in n.appearance.effects.iter().enumerate().skip(1) {
        assert_eq!(effect.id, "distort.transform");
        assert!(effect.visible);
        assert_eq!(effect.params["copies"].as_f64(), Some(1.0));
        let move_v = if nested_copy && index == 1 { 60.0 } else { 120.0 };
        assert_eq!(effect.params["moveV"].as_f64(), Some(move_v));
    }
}

fn eraser_copying_container_gradient_variant_scene(placed: bool, nested_copy: bool) -> ([[u8; 4]; 2], [[u8; 4]; 2]) {
    let mut s = session();
    let (edited, sibling, owner, _) = eraser_container_gradient_fixture(&mut s, placed, false);
    if !nested_copy {
        eraser_copying_container_gradient_child_offset(&mut s, sibling, owner, placed);
    }
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
            "effect": "stylize.dropShadow", "params": {"x": 10, "y": 0, "blur": 0}}),
    )
    .unwrap();
    if nested_copy {
        s.execute(
            "effect.apply",
            &json!({"ids": [owner.0], "target": "object", "item": null,
                "effect": "distort.transform", "params": {"scaleH": 100, "scaleV": 100,
                    "reference": 8, "objects": true, "copies": 1, "random": false,
                    "rotate": 0, "moveH": 0, "moveV": 60, "patterns": false, "strokes": false}}),
        )
        .unwrap();
    }
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null,
            "effect": "distort.transform", "params": {"scaleH": 100, "scaleV": 100,
                "reference": 8, "objects": true, "copies": 1, "random": false,
                "rotate": 0, "moveH": 0, "moveV": 120, "patterns": false, "strokes": false}}),
    )
    .unwrap();
    let n = s.doc().unwrap().doc.node(owner).unwrap();
    eraser_copying_container_gradient_owner_effects(n, nested_copy);
    let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the Group owns the gradient") };
    assert_eq!(g.geom.is_some(), placed);
    let geom = serde_json::to_value(g.geom).unwrap();
    let effects = serde_json::to_value(&n.appearance.effects).unwrap();
    let before_image = eraser_gradient_image(&s);
    let before_colors = eraser_copying_container_gradient_before_colors(&before_image);
    let before = s.doc().unwrap().doc.clone();
    let before_json = serde_json::to_value(&before).unwrap();
    let sibling_before = serde_json::to_value(before.node(sibling).unwrap()).unwrap();
    let ancestry = before.ancestry(edited).unwrap();
    let selection = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let miss = eraser_gradient_region(&mut s, "rect", true, true);
    assert_eq!(miss["ids"], json!([]));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "a miss cannot pin an ancestor");
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_json);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    let result = eraser_gradient_region(&mut s, "rect", true, false);
    assert_eq!(result["ids"].as_array().unwrap().len(), 1, "the extremal trim keeps one piece");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.ancestry(edited).unwrap(), ancestry);
    assert_eq!(serde_json::to_value(d.node(sibling).unwrap()).unwrap(), sibling_before, "the sibling itself is unchanged");
    let n = d.node(owner).unwrap();
    assert!(matches!(n.kind, NodeKind::Group { clip: false, .. }));
    assert!((n.geometric_bounds().unwrap().y0 - 110.0).abs() < 1e-5);
    assert_eq!(serde_json::to_value(&n.appearance.effects).unwrap(), effects, "both effects stay live and unchanged");
    if placed {
        let vectorcraft_doc::color::Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("the own fill stays a gradient") };
        assert_eq!(serde_json::to_value(g.geom).unwrap(), geom, "the explicit source fit must not be rewritten");
    }
    let after_image = eraser_gradient_image(&s);
    assert_eq!(after_image.pixel(125, 105)[3], 0, "the intended band was erased");
    let after_colors = [after_image.pixel(190, 175), after_image.pixel(190, 185)];
    assert!(after_colors.iter().all(|color| color[3] == 255));
    eraser_container_gradient_restore(&mut s, &before_json, &selection, history);
    (before_colors, after_colors)
}

#[test]
fn eraser_copying_container_gradient_keeps_evaluated_child_source_fit() {
    let placed = eraser_copying_container_gradient_variant_scene(true, false);
    let fitted = eraser_copying_container_gradient_variant_scene(false, false);
    assert_eq!(placed.0, placed.1, "the explicitly placed evaluated-child source fit preserves the untouched sibling");
    assert_eq!(fitted.0, placed.0, "both paints fit the evaluated source before copying");
    assert_eq!(fitted.1, fitted.0, "pinning uses evaluated child geometry, not the raw prefix box");
    assert_eq!(fitted.1, placed.1, "fitted paint matches the equivalent evaluated-source placement after editing");
}

#[test]
fn eraser_copying_container_gradient_keeps_source_fit_through_earlier_copy() {
    // Both copying effects are eligible: the outer split still leaves the earlier
    // copying Transform in its prefix. Own paint is fitted before either copy.
    let placed = eraser_copying_container_gradient_variant_scene(true, true);
    let fitted = eraser_copying_container_gradient_variant_scene(false, true);
    assert_eq!(placed.0, placed.1, "the explicit original-source fit is unchanged by the trim");
    assert_eq!(fitted.0, placed.0, "the original source fit is equivalent before both copy stages");
    assert_eq!(fitted.1, fitted.0, "pinning follows the earlier eligible copy split as rendering does");
    assert_eq!(fitted.1, placed.1, "both live copy stages preserve the same source placement after editing");
}

#[test]
fn eraser_keeps_group_painted_open_paths_on_the_outline_route() {
    for mode in ["fixed", "varied", "marquee"] {
        for group_stroke in [false, true] {
            let mut s = session();
            let id = NodeId(
                s.execute(
                    "path.create",
                    &json!({"anchors": [
                        {"x": 100, "y": 100}, {"x": 200, "y": 100},
                        {"x": 200, "y": 200}, {"x": 100, "y": 200}
                    ]}),
                )
                .unwrap()["id"]
                    .as_u64()
                    .unwrap(),
            );
            s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#0000ff"})).unwrap();
            s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
            if group_stroke {
                s.execute("appearance.addStroke", &json!({"ids": [group.0], "target": "object"})).unwrap();
                s.execute(
                    "appearance.setItem",
                    &json!({"ids": [group.0], "target": "object", "index": 0,
                    "color": "#ff0000", "weight": 4}),
                )
                .unwrap();
            }
            s.execute("select.none", &json!({})).unwrap();
            assert!(!path(&s, id).is_closed());
            let d = &s.doc().unwrap().doc;
            assert_eq!(d.parent_of(id), Some(group));
            assert!(d.node(id).unwrap().appearance.stroke_paint().is_none());
            assert_eq!(d.node(group).unwrap().appearance.stroke_width(), if group_stroke { 4.0 } else { 0.0 });
            let render = |s: &Session| {
                vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
            };
            let before_image = render(&s);
            assert_eq!(before_image.pixel(150, 150), [0, 0, 255, 255]);
            if group_stroke {
                assert_eq!(before_image.pixel(150, 100), [255, 0, 0, 255], "the Group really paints the child's open outline");
                assert_eq!(before_image.pixel(99, 125)[3], 0, "the missing closing edge is not stroked");
            }
            let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
            let history = s.doc().unwrap().history.undo.len();
            let selection = s.doc().unwrap().selection.clone();
            let paint = s.paint.clone();
            // This ordinary Eraser gesture hits fill, at least 30 pt from every real outline.
            let result = match mode {
                "marquee" => s.execute("path.eraseRegion", &json!({"rect": [130, 145, 40, 10]})).unwrap(),
                _ => erase(&mut s, json!({"points": [[130, 150], [170, 150]], "size": 10}), mode == "varied"),
            };
            if group_stroke {
                assert_eq!(result["ids"], json!([]), "{mode}: a Group-painted open outline keeps main's miss behavior");
                assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
                assert_eq!(s.doc().unwrap().history.undo.len(), history);
                assert_eq!(s.doc().unwrap().selection, selection);
                assert_eq!(render(&s).pixels, before_image.pixels);
            } else {
                assert!(!result["ids"].as_array().unwrap().is_empty(), "{mode}: the fill-only Group control still cuts as an area");
                let after_image = render(&s);
                assert_eq!(after_image.pixel(150, 150)[3], 0);
                assert_eq!(after_image.pixel(120, 125), [0, 0, 255, 255]);
            }
            assert_eq!(s.paint, paint);
        }
    }
}

#[test]
fn eraser_ancestor_stroke_classification_keeps_container_context() {
    for container in ["nested", "layer", "compound", "clip"] {
        for stroke in ["painted", "hidden", "none", "zero"] {
            eraser_ancestor_stroke_case(container, stroke);
        }
    }
}

fn eraser_ancestor_stroke_case(container: &str, stroke: &str) {
    let mut s = eraser_ancestor_stroke_scene(container, stroke);
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let before_image = render(&s);
    assert_eq!(before_image.pixel(150, 150), [0, 0, 255, 255], "{container}/{stroke}: actual filled input");
    let outline_route = stroke == "painted" && container != "clip";
    if outline_route {
        assert_eq!(before_image.pixel(150, 100), [255, 0, 0, 255], "{container}: actual ancestor stroke");
    } else if container == "clip" && stroke == "painted" {
        assert_eq!(before_image.pixel(150, 90), [255, 0, 0, 255], "the ancestor paints the clipping outline");
        assert_eq!(before_image.pixel(150, 100), [0, 0, 255, 255], "the ancestor does not stroke the clipped child's outline");
    }
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let history = s.doc().unwrap().history.undo.len();
    let selection = s.doc().unwrap().selection.clone();
    let paint = s.paint.clone();
    let result = erase(&mut s, json!({"points": [[130, 150], [170, 150]], "size": 10}), false);
    if outline_route {
        assert_eq!(result["ids"], json!([]), "{container}/{stroke}: preserve main's open-outline miss");
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(render(&s).pixels, before_image.pixels);
    } else {
        assert!(!result["ids"].as_array().unwrap().is_empty(), "{container}/{stroke}: the eligible fill still cuts");
        let after_image = render(&s);
        assert_eq!(after_image.pixel(150, 150)[3], 0, "{container}/{stroke}");
        assert_eq!(after_image.pixel(120, 125), [0, 0, 255, 255], "{container}/{stroke}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(render(&s).pixels, before_image.pixels);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(render(&s).pixel(150, 150)[3], 0);
    }
    assert_eq!(s.paint, paint);
}

fn eraser_ancestor_stroke_scene(container: &str, stroke: &str) -> Session {
    let mut s = session();
    let id = NodeId(
        s.execute(
            "path.create",
            &json!({"anchors": [
                {"x": 100, "y": 100}, {"x": 200, "y": 100},
                {"x": 200, "y": 200}, {"x": 100, "y": 200}
            ]}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#0000ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let mut subject = id;
    if container == "compound" {
        let other = s
            .execute(
                "path.create",
                &json!({"anchors": [
                    {"x": 220, "y": 100}, {"x": 240, "y": 100},
                    {"x": 240, "y": 120}, {"x": 220, "y": 120}
                ]}),
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        s.execute("paint.setFill", &json!({"ids": [other], "color": "#0000ff"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [other], "none": true})).unwrap();
        s.execute("select.set", &json!({"ids": [id.0, other]})).unwrap();
        subject = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        let NodeKind::Compound { children, .. } = &s.doc().unwrap().doc.node(subject).unwrap().kind else {
            panic!("the public Make command produces a compound path")
        };
        assert_eq!(children.len(), 2);
        assert!(children.iter().all(|child| !child.path_data().unwrap().is_closed()));
    } else if container == "clip" {
        let mask = s.execute("shape.rectangle", &json!({"x": 90, "y": 90, "width": 120, "height": 120})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id.0, mask]})).unwrap();
        subject = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        assert!(s.doc().unwrap().doc.node(subject).unwrap().clips());
    }
    let owner = if container == "layer" {
        s.doc().unwrap().doc.layer_of(id).unwrap()
    } else {
        s.execute("select.set", &json!({"ids": [subject.0]})).unwrap();
        let inner = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
        if container == "nested" { NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap()) } else { inner }
    };
    s.execute("appearance.addStroke", &json!({"ids": [owner.0], "target": "object"})).unwrap();
    s.execute(
        "appearance.setItem",
        &json!({"ids": [owner.0], "target": "object", "index": 0,
        "color": "#ff0000", "weight": if stroke == "zero" { 0 } else { 4 }, "visible": stroke != "hidden"}),
    )
    .unwrap();
    if stroke == "none" {
        s.execute("appearance.setItem", &json!({"ids": [owner.0], "target": "object", "index": 0, "none": true})).unwrap();
    }
    // Select only the original open child (or the two-member compound). In the clip
    // control, neither the mask nor the other container geometry is an Eraser target.
    let selected = if container == "compound" { subject } else { id };
    s.execute("select.set", &json!({"ids": [selected.0]})).unwrap();
    s
}

// Pinning-only checks: no Eraser operation or other geometry edit is performed below.
fn eraser_pinning_only_same_pixels(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, label: &str) {
    let changed = (0..300).find_map(|y| (0..300).find(|&x| a.pixel(x, y) != b.pixel(x, y)).map(|x| (x, y, a.pixel(x, y), b.pixel(x, y))));
    assert_eq!(changed, None, "{label}: first changed pixel");
}

fn eraser_pinning_only_nested_scene(placed: bool, parent_offset: bool) -> (Session, NodeId) {
    let mut s = session();
    let (_, _, owner, _) = eraser_container_gradient_fixture(&mut s, false, false);
    s.execute("select.set", &json!({"ids": [owner.0]})).unwrap();
    let outer = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    if parent_offset {
        s.execute(
            "effect.apply",
            &json!({"ids": [outer.0], "target": "object", "item": null,
                "effect": "path.offsetPath", "params": {"offset": 10, "joins": "round"}}),
        )
        .unwrap();
    }
    // These are the retained ordinary fit and the existing +10 pt evaluated fit.
    let (bottom, top) = if parent_offset { (210.0, 90.0) } else { (200.0, 100.0) };
    s.execute(
        "appearance.setItem",
        &json!({"ids": [owner.0], "target": "object", "index": 0,
            "gradient": eraser_placement_ramp(placed, bottom, top)}),
    )
    .unwrap();
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(owner), Some(outer), "INPUT: the gradient owner is nested in the effect owner");
    assert_eq!(doc.node(owner).unwrap().geometric_bounds().unwrap(), vectorcraft_geom::Rect::new(100.0, 100.0, 220.0, 200.0));
    assert!(doc.node(owner).unwrap().appearance.effects.is_empty(), "INPUT: the offset belongs to the enclosing Group only");
    let effects = &doc.node(outer).unwrap().appearance.effects;
    assert_eq!(effects.len(), usize::from(parent_offset));
    if parent_offset {
        assert_eq!(effects[0].id, "path.offsetPath");
        assert!(effects[0].visible);
        assert_eq!(effects[0].params["offset"].as_f64(), Some(10.0));
        assert_eq!(effects[0].params["joins"], json!("round"));
    }
    let vectorcraft_doc::color::Paint::Gradient(g) = doc.node(owner).unwrap().appearance.fill_paint() else {
        panic!("INPUT: the nested Group owns the gradient")
    };
    assert_eq!(g.geom.is_some(), placed);
    assert!(!g.gradient.dither);
    (s, owner)
}

fn eraser_pinning_only_nested_case(parent_offset: bool) {
    let (s, owner) = eraser_pinning_only_nested_scene(false, parent_offset);
    let (placed, placed_owner) = eraser_pinning_only_nested_scene(true, parent_offset);
    let before = eraser_gradient_image(&s);
    let explicit = eraser_gradient_image(&placed);
    assert_eq!(before.pixel(190, 175)[3], 255, "INPUT: the retained sibling is painted");
    assert_eq!(before.pixel(190, 185)[3], 255);
    assert_ne!(before.pixel(190, 175), before.pixel(190, 185), "INPUT: the fitted ramp is nonconstant");
    if parent_offset {
        assert_eq!(before.pixel(190, 205)[3], 255, "INPUT: the enclosing positive offset reaches beyond the raw box");
    }
    eraser_pinning_only_same_pixels(&before, &explicit, "INPUT: the independently placed control matches before pinning");
    let placed_doc = &placed.doc().unwrap().doc;
    assert!(
        vectorcraft_render::pinned_appearance_for_geometry_edit(placed_doc, placed_doc.node(placed_owner).unwrap()).is_none(),
        "INPUT: an already explicit placement needs no pinning"
    );
    let label =
        if parent_offset { "PINNING_ONLY: nested Group under positive Offset Path" } else { "CONTROL: nested Group without enclosing effects" };
    eraser_pinning_only_store(&s.doc().unwrap().doc, owner, &before, !parent_offset, label);
}

fn eraser_pinning_only_store(doc: &vectorcraft_doc::Document, owner: NodeId, before: &vectorcraft_render::Rendered, required: bool, label: &str) {
    let original = serde_json::to_value(doc).unwrap();
    let node = doc.node(owner).unwrap();
    let Some(pinned) = vectorcraft_render::pinned_appearance_for_geometry_edit(doc, node) else {
        assert!(!required, "INPUT: the ordinary control must pin reproducibly: {label}");
        assert_eq!(serde_json::to_value(doc).unwrap(), original);
        println!("PINNING_FALLBACK: {label} remains fitted; no pixel failure established");
        return;
    };
    assert_eq!(serde_json::to_value(doc).unwrap(), original, "INPUT: the public helper leaves its input unchanged");
    let vectorcraft_doc::color::Paint::Gradient(g) = pinned.fill_paint() else { panic!("INPUT: returned paint is a gradient") };
    assert!(g.geom.is_some(), "INPUT: the returned fitted gradient has an explicit placement");
    // The fixed linear fixture permits only stored gradient geometry to change, not effects or other row data.
    let mut normalized = pinned.clone();
    assert_eq!(normalized.items.len(), node.appearance.items.len());
    for (after, before) in normalized.items.iter_mut().zip(&node.appearance.items) {
        if let (vectorcraft_doc::color::Paint::Gradient(a), vectorcraft_doc::color::Paint::Gradient(b)) = (after.paint_mut(), before.paint()) {
            a.geom = b.geom;
        }
    }
    assert_eq!(serde_json::to_value(&normalized).unwrap(), serde_json::to_value(&node.appearance).unwrap(), "INPUT: only placement changes");
    let mut only_paint = doc.clone();
    only_paint.node_mut(owner).unwrap().appearance = pinned.into();
    let mut restored = only_paint.clone();
    restored.node_mut(owner).unwrap().appearance = node.appearance.clone();
    assert_eq!(serde_json::to_value(&restored).unwrap(), original, "INPUT: geometry, effects and every other source field are unchanged");
    let after = vectorcraft_render::Renderer::new().render(&only_paint, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    eraser_pinning_only_same_pixels(before, &after, label);
}

#[test]
fn eraser_pinning_only_nested_gradient_matches_unchanged_render() {
    // Establish the ordinary nested Group without an enclosing geometry effect FIRST.
    eraser_pinning_only_nested_case(false);
    // One retained positive offset, no cut, parameter search or timing measurement.
    eraser_pinning_only_nested_case(true);
}

fn eraser_pinning_only_text_clip_case(stroked: bool, no_stroke: Option<&vectorcraft_render::Rendered>) -> vectorcraft_render::Rendered {
    assert_eq!(stroked, no_stroke.is_some(), "INPUT: the subject follows its no-stroke control");
    // Original clippaint.rs text, paint, font defaults and geometry; one retained ordinary scale.
    let mut s = session();
    let art = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 30, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [art], "color": "#ff0000"})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 10, "y": 120, "text": "Clip", "size": 72})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.all", &json!({})).unwrap();
    let owner = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [text.0], "color": "#0000ff"})).unwrap();
    if stroked {
        s.execute("paint.setStroke", &json!({"ids": [text.0], "color": "#00ff00"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [text.0], "weight": 2})).unwrap();
    } else {
        s.execute("paint.setStroke", &json!({"ids": [text.0], "none": true})).unwrap();
    }
    s.execute("appearance.addFill", &json!({"ids": [owner.0], "target": "object"})).unwrap();
    s.execute(
        "appearance.setItem",
        &json!({"ids": [owner.0], "target": "object", "index": 0, "gradient": eraser_placement_ramp(false, 200.0, 100.0)}),
    )
    .unwrap();
    s.execute(
        "effect.apply",
        &json!({"ids": [owner.0], "target": "object", "item": null, "effect": "distort.transform",
            "params": {"scaleH": 100, "scaleV": 150, "reference": 8, "objects": true, "copies": 0,
                "random": false, "rotate": 0, "moveH": 0, "moveV": 0, "patterns": false, "strokes": false}}),
    )
    .unwrap();
    let doc = &s.doc().unwrap().doc;
    let node = doc.node(owner).unwrap();
    assert!(node.clips(), "INPUT: the source is a clipping Group");
    assert_eq!(node.children().unwrap()[0].id, text);
    let t = doc.node(text).unwrap();
    let NodeKind::Text(type_) = &t.kind else { panic!("INPUT: the clipping object is type") };
    assert_eq!(type_.plain_text(), "Clip");
    assert_eq!(t.clip_paint().stroke.is_some(), stroked, "INPUT: only the character-stroke paint distinguishes the control");
    let bounds = t.geometric_bounds().unwrap();
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0, "INPUT: actual font setup supplies nonempty glyph geometry");
    let effects = &node.appearance.effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].id, "distort.transform");
    assert!(effects[0].visible);
    for (key, expected) in [("scaleH", 100.0), ("scaleV", 150.0), ("reference", 8.0), ("copies", 0.0)] {
        assert_eq!(effects[0].params[key].as_f64(), Some(expected), "INPUT: accepted Transform {key}");
    }
    let before = eraser_gradient_image(&s);
    let background = before.pixel(299, 299);
    assert_eq!(background[3], 0, "INPUT: the retained background sample is clear");
    if let Some(control) = no_stroke {
        assert_eq!(control.pixel(299, 299), background, "INPUT: both unchanged renders use the same background");
    }
    let mut ramp = std::collections::BTreeSet::new();
    let mut green = 0;
    let mut outside_green = 0;
    for y in 0..300 {
        for x in 0..300 {
            let p = before.pixel(x, y);
            if p[3] == 255 && p[0] > 0 && p[2] > 0 && p[1] == 0 {
                ramp.insert(p);
            }
            let painted_stroke = p[3] > 0 && p[1] > 200 && p[0] < 60 && p[2] < 60;
            green += usize::from(painted_stroke);
            outside_green += usize::from(painted_stroke && no_stroke.is_some_and(|control| control.pixel(x, y) == background));
        }
    }
    assert!(ramp.len() > 1, "INPUT: the fitted container gradient paints actual nonconstant glyph pixels");
    assert_eq!(green > 0, stroked, "INPUT: the painted-character stroke is actually visible only in the subject");
    if stroked {
        assert!(outside_green > 0, "INPUT: the actual character stroke paints where the no-stroke control has only clear background");
    }
    let label =
        if stroked { "PINNING_ONLY: painted clipping text under nonuniform scale" } else { "CONTROL: same glyph/scale without painted-clip wrapper" };
    eraser_pinning_only_store(doc, owner, &before, !stroked, label);
    before
}

#[test]
fn eraser_pinning_only_painted_text_clip_matches_unchanged_render() {
    let control = eraser_pinning_only_text_clip_case(false, None);
    eraser_pinning_only_text_clip_case(true, Some(&control));
}

#[test]
fn eraser_pinning_only_keeps_hidden_geometry_and_raster_ancestors_supported() {
    for raster in [false, true] {
        let (mut s, owner) = eraser_pinning_only_nested_scene(false, !raster);
        let outer = s.doc().unwrap().doc.parent_of(owner).unwrap();
        if raster {
            s.execute(
                "effect.apply",
                &json!({"ids": [outer.0], "target": "object", "item": null, "effect": "blur.gaussian", "params": {"radius": 3}}),
            )
            .unwrap();
        }
        let mut doc = (*s.doc().unwrap().doc).clone();
        if !raster {
            let ancestor = doc.node_mut(outer).unwrap();
            ancestor.appearance.effects[0].visible = false;
        }
        let effect = &doc.node(outer).unwrap().appearance.effects[0];
        assert_eq!(doc.node(outer).unwrap().appearance.effects.len(), 1);
        assert_eq!(effect.visible, raster);
        if raster {
            assert_eq!(effect.id, "blur.gaussian");
            assert_eq!(effect.params["radius"].as_f64(), Some(3.0));
        } else {
            assert_eq!(effect.id, "path.offsetPath");
            assert_eq!(effect.params["offset"].as_f64(), Some(10.0));
        }
        let before = vectorcraft_render::Renderer::new().render(&doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert_eq!(before.pixel(190, 175)[3], 255, "INPUT: ordinary gradient art remains visible");
        assert_ne!(before.pixel(190, 175), before.pixel(190, 185), "INPUT: the paint is nonconstant");
        let label = if raster { "CONTROL: visible Gaussian ancestor" } else { "CONTROL: hidden Offset Path ancestor" };
        eraser_pinning_only_store(&doc, owner, &before, true, label);
    }
}
