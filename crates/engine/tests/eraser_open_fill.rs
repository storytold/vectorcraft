//! Fill-only open-path Eraser regressions through the public engine and CPU renderer.
// Integration tests may unwrap and panic on failure (AGENTS.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
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

#[test]
fn eraser_preserves_the_fill_of_an_open_rectangle() {
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
        s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
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
fn eraser_keeps_open_compound_holes_with_both_fill_rules() {
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
        let miss = s.execute("path.eraseRegion", &json!({"points": [[200, 200]], "size": 10})).unwrap();
        assert_eq!(miss["ids"], json!([]), "{rule}: a stroke wholly inside the hole misses");
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_miss);
        assert_eq!(s.doc().unwrap().history.undo.len(), miss_history, "{rule}: the hole miss adds no history");

        s.execute("path.eraseRegion", &json!({"points": [[80, 120], [320, 120]], "size": 10})).unwrap();
        let after =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        assert_eq!(after.pixel(120, 120)[3], 0, "{rule}: swept band is empty");
        assert!(after.pixel(120, 140)[3] > 240, "{rule}: retained ring is filled");
        assert_eq!(after.pixel(200, 200)[3], 0, "{rule}: compound hole remains empty");
    }
}

#[test]
fn eraser_removes_only_the_swept_strip_from_a_zero_signed_area_open_bowtie() {
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

    s.execute("path.eraseRegion", &json!({"points": [[50, 125], [250, 125]], "size": 10})).unwrap();
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
