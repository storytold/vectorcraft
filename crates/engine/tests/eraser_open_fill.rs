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
