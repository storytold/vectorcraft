//! Filled open-path Eraser regressions through the public engine and CPU renderer.
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

/// #6582: an open contour fills its implicitly closed area. Erasing a strip removes only that
/// strip, just as for the closed rectangle, rather than changing its fill to diagonal chords.
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

fn total_line_length(paths: &[PathData], x: f64) -> f64 {
    paths
        .iter()
        .flat_map(|path| &path.subpaths)
        .filter(|subpath| !subpath.closed && subpath.anchors.len() >= 2 && subpath.anchors.iter().all(|anchor| (anchor.p.x - x).abs() < 0.01))
        .map(|subpath| subpath.anchors.windows(2).map(|pair| pair[0].p.distance(pair[1].p)).sum::<f64>())
        .sum()
}

#[test]
fn eraser_keeps_mixed_fill_and_zero_area_strokes_with_one_undo() {
    for filled_open in [true, false] {
        let mut s = session();
        let id = NodeId(s.execute("path.create", &json!({"anchors": [{"x": 100, "y": 100}]})).unwrap()["id"].as_u64().unwrap());
        s.execute(
            "path.setAnchors",
            &json!({"id": id.0, "subpaths": [
                {"closed": !filled_open, "anchors": [{"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}]},
                {"closed": false, "anchors": [{"x": 250, "y": 100}, {"x": 250, "y": 200}]},
                {"closed": false, "anchors": [{"x": 250, "y": 220}, {"x": 250, "y": 270}]}
            ]}),
        )
        .unwrap();
        if filled_open {
            s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        } else {
            s.execute("paint.setFill", &json!({"ids": [id.0], "none": true})).unwrap();
        }
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 2})).unwrap();
        s.execute("select.none", &json!({})).unwrap();

        let before_miss = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let miss_history = s.doc().unwrap().history.undo.len();
        let miss = s.execute("path.eraseRegion", &json!({"points": [[50, 20], [250, 20]], "size": 10})).unwrap();
        assert_eq!(miss["ids"], json!([]));
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_miss, "a non-intersecting Eraser leaves the document unchanged");
        assert_eq!(s.doc().unwrap().history.undo.len(), miss_history, "a miss adds no history entry");

        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let before_selection = s.doc().unwrap().selection.clone();
        let before_history = s.doc().unwrap().history.undo.len();
        s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
        let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let after_selection = s.doc().unwrap().selection.clone();
        assert_eq!(s.doc().unwrap().history.undo.len(), before_history + 1);

        let remaining = paths(&s);
        let line_length = total_line_length(&remaining, 250.0);
        assert!((line_length - 140.0).abs() < 0.1, "filled_open={filled_open}: retained open strokes {line_length}");
        assert!(
            remaining
                .iter()
                .flat_map(|path| &path.subpaths)
                .filter(|sp| sp.anchors.iter().all(|a| (a.p.x - 250.0).abs() < 0.01))
                .all(|sp| !sp.closed)
        );
        let image =
            vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        for (x, y) in [(250, 120), (250, 180), (250, 240)] {
            let pixel = image.pixel(x, y);
            assert!(pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60 && pixel[3] > 200, "retained red stroke at ({x},{y}): {pixel:?}");
        }
        if filled_open {
            let area: f64 = remaining.iter().map(|path| vectorcraft_pathops::area(path, FillRule::NonZero)).sum();
            assert!((area - 9000.0).abs() < 1.0, "retained fill area {area}");
            for (x, y) in [(110, 125), (190, 125), (110, 175), (190, 175)] {
                assert!(image.pixel(x, y)[3] > 240, "retained fill pixel ({x},{y})");
            }
            assert_eq!(image.pixel(150, 150)[3], 0, "the fill erase band is empty");
        } else {
            let outline_length: f64 = remaining.iter().map(PathData::length).sum();
            assert!((outline_length - 520.0).abs() < 1.0, "unfilled closed rectangle and two open lines retain 520 pt of stroke: {outline_length}");
            assert!(remaining.iter().flat_map(|path| &path.subpaths).all(|subpath| !subpath.closed), "unfilled outline survivors stay open");
            assert_eq!(image.pixel(150, 145)[3], 0, "outline splitting does not add a stroke across the rectangle interior");
        }

        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before, "one Undo restores the exact document");
        assert_eq!(s.doc().unwrap().selection, before_selection);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after, "one Redo restores the exact erased document");
        assert_eq!(s.doc().unwrap().selection, after_selection);
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

/// Even-Odd can cancel a nonzero signed area: that open path still has an erasable stroke.
#[test]
fn eraser_cuts_the_outline_when_even_odd_cancels_an_open_paths_fill() {
    let mut s = session();
    let made = s.execute("path.create", &json!({"d": "M100 100H200V200H100V100H200V200H100V100"})).unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    s.execute("path.setFillRule", &json!({"rule": "evenOdd"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let original = path(&s, id);
    assert!(!original.is_closed());
    assert!(original.subpaths.first().unwrap().signed_area().abs() > 10000.0);
    assert!(vectorcraft_pathops::area(original, FillRule::EvenOdd) < 1e-9, "Even-Odd cancels the two laps");
    assert!((original.length() - 800.0).abs() < 1e-9);

    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
    let length: f64 = paths(&s).iter().map(PathData::length).sum();
    assert!((length - 760.0).abs() < 0.1, "both laps lose their crossed stroke portions: {length}");
}

/// Hidden fills retain open outlines, while a lower visible fill beneath None still paints.
#[test]
fn eraser_hidden_fill_keeps_only_the_explicit_open_outline() {
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    for lower_fill in [false, true] {
        let mut s = session();
        let id = NodeId(
            s.execute(
                "path.create",
                &json!({"anchors": [
                    {"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}
                ]}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 2})).unwrap();
        if lower_fill {
            // The basic black Fill remains visible underneath this added top Fill set to None.
            s.execute("appearance.addFill", &json!({"ids": [id.0]})).unwrap();
            s.execute("appearance.setItem", &json!({"ids": [id.0], "index": 2, "none": true})).unwrap();
        } else {
            // Basic appearance order is Fill 0, Stroke 1; hiding Fill 0 leaves only open strokes.
            s.execute("appearance.setItem", &json!({"ids": [id.0], "index": 0, "visible": false})).unwrap();
        }
        s.execute("select.none", &json!({})).unwrap();
        let before = render(&s);
        if lower_fill {
            assert!(before.pixel(150, 125)[3] > 240, "the lower black fill remains visible under top None");
        } else {
            assert_eq!(before.pixel(150, 125)[3], 0, "hidden fill leaves the interior empty");
            assert_eq!(before.pixel(100, 125)[3], 0, "the open path has no implicit left stroke");
        }
        assert!(before.pixel(200, 125)[0] > 200 && before.pixel(200, 125)[1] < 60, "the explicit right stroke is visible");

        s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
        let after = render(&s);
        if lower_fill {
            assert!(after.pixel(150, 125)[3] > 240, "visible lower fill survives above the erased band");
            assert!(after.pixel(150, 175)[3] > 240, "visible lower fill survives below the erased band");
            assert_eq!(after.pixel(150, 150)[3], 0, "the Eraser band is empty");
        } else {
            let survivors = paths(&s);
            assert!(survivors.iter().all(|path| !path.is_closed()), "unfilled survivors remain open");
            let length: f64 = survivors.iter().map(PathData::length).sum();
            assert!((length - 290.0).abs() < 0.1, "only the crossed explicit right edge loses 10 pt: {length}");
            assert_eq!(after.pixel(150, 125)[3], 0, "hidden fill leaves the interior empty");
            assert_eq!(after.pixel(100, 125)[3], 0, "the missing implicit left stroke remains absent");
        }
    }
}

/// The original compound's cancelled outline stays grouped beside its filled contour.
#[test]
fn eraser_even_odd_cancellation_survives_beside_a_filled_subpath() {
    let mut s = session();
    let id = NodeId(
        s.execute("path.create", &json!({"d": "M100 100H200V200H100V100H200V200H100V100 M230 100H280V200H230"})).unwrap()["id"].as_u64().unwrap(),
    );
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [id.0], "weight": 2})).unwrap();
    s.execute("path.setFillRule", &json!({"ids": [id.0], "rule": "evenOdd"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(path(&s, id).subpaths.len(), 2, "one node holds both open subpaths");
    assert!(path(&s, id).subpaths.iter().all(|subpath| !subpath.closed));
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let before = render(&s);
    assert_eq!(before.pixel(150, 125)[3], 0, "the twice-traced open subpath cancels under Even-Odd");
    assert_eq!(before.pixel(150, 175)[3], 0, "the cancelled lower run is empty before Eraser");
    assert!(before.pixel(250, 125)[3] > 240, "the disjoint open rectangle fills");

    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [290, 150]], "size": 10})).unwrap();
    let after = render(&s);
    assert_eq!(after.pixel(150, 175)[3], 0, "the cancelled interior stays empty below the sweep");
    assert!(after.pixel(250, 125)[3] > 240, "the disjoint filled subpath survives");
    assert_eq!(after.pixel(250, 150)[3], 0, "the swept band through the filled subpath is empty");
    for (x, y) in [(100, 125), (200, 125), (230, 125), (280, 125)] {
        let pixel = after.pixel(x, y);
        assert!(pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60 && pixel[3] > 200, "open outline survivor ({x},{y}): {pixel:?}");
    }
}
