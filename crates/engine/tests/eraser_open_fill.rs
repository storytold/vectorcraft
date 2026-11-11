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

/// Geometric area of paths that own a visible fill, excluding stroke-only children.
/// This does not infer inherited paint from compound paths or containers.
fn own_filled_path_area(s: &Session) -> f64 {
    let mut area = 0.0;
    s.doc().unwrap().doc.walk(|node| {
        if let NodeKind::Path { path, rule, .. } = &node.kind
            && node
                .appearance
                .items
                .iter()
                .any(|item| matches!(item, vectorcraft_doc::AppearanceItem::Fill(fill) if fill.visible && !fill.paint.is_none()))
        {
            area += vectorcraft_pathops::area(path, *rule);
        }
    });
    area
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
            let area = own_filled_path_area(&s);
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
    assert_eq!(render_erased(&s).pixel(150, 125)[3], 0, "the cancelled interior is transparent before Eraser");

    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
    let length: f64 = paths(&s).iter().map(PathData::length).sum();
    assert!((length - 760.0).abs() < 0.1, "both laps lose their crossed stroke portions: {length}");
    assert_eq!(render_erased(&s).pixel(150, 125)[3], 0, "Eraser does not paint the cancelled interior outside its sweep");
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
    assert_eq!(before.pixel(230, 125), [0, 0, 0, 255], "the second contour has fill but no explicit left stroke");
    assert_eq!(before.pixel(229, 125), [0, 0, 0, 0], "there is no stroke outside its implicit closing edge");

    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [290, 150]], "size": 10})).unwrap();
    let after = render(&s);
    assert_eq!(after.pixel(150, 175)[3], 0, "the cancelled interior stays empty below the sweep");
    assert!(after.pixel(250, 125)[3] > 240, "the disjoint filled subpath survives");
    assert_eq!(after.pixel(250, 150)[3], 0, "the swept band through the filled subpath is empty");
    for (x, y) in [(230, 125), (229, 125)] {
        assert_eq!(after.pixel(x, y), before.pixel(x, y), "the implicit closing edge stays unstroked at ({x},{y})");
    }
    for (x, y) in [(100, 125), (200, 125), (280, 125)] {
        let pixel = after.pixel(x, y);
        assert!(pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60 && pixel[3] > 200, "open outline survivor ({x},{y}): {pixel:?}");
    }
}
#[test]
fn eraser_preserves_the_missing_edge_of_a_filled_open_stroke() {
    let mut failures = vec![];
    for (opacity, expected_alpha) in [(100, 255u8), (50, 128u8)] {
        for closed in [true, false] {
            let (mut s, id) = painted_open_rectangle(closed);
            s.execute("transparency.set", &json!({"ids": [id.0], "opacity": opacity})).unwrap();
            s.execute("select.none", &json!({})).unwrap();
            let mut check = |ok: bool, message: String| {
                if !ok {
                    failures.push(format!("closed={closed} opacity={opacity}: {message}"));
                }
            };
            let before = render_erased(&s);
            for y in [125, 175] {
                check(before.pixel(150, y)[3].abs_diff(expected_alpha) <= 1, format!("original fill at y={y}: {:?}", before.pixel(150, y)));
            }
            let before_left = before.pixel(99, 125);
            if closed {
                check(before_left[0] > before_left[1] && before_left[3] > 0, format!("closed control's explicit red left edge: {before_left:?}"));
            } else {
                check(before_left[3] == 0, format!("open source has no implicit left stroke: {before_left:?}"));
            }
            let before_overlap = before.pixel(199, 125);
            check(before_overlap[3].abs_diff(expected_alpha) <= 1, format!("object opacity is applied once before Eraser: {before_overlap:?}"));
            check(before_overlap[0] > before_overlap[1] && before_overlap[3] > 0, format!("visible fill and red stroke overlap: {before_overlap:?}"));
            s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
            let after = render_erased(&s);
            for y in [125, 175] {
                check(
                    after.pixel(150, y) == before.pixel(150, y),
                    format!("retained fill at y={y}: before {:?}, after {:?}", before.pixel(150, y), after.pixel(150, y)),
                );
            }
            check(after.pixel(150, 150)[3] == 0, format!("Eraser band is empty: {:?}", after.pixel(150, 150)));
            let after_left = after.pixel(99, 125);
            check(after_left == before_left, format!("left-edge stroke: before {before_left:?}, after {after_left:?}"));
            let after_overlap = after.pixel(199, 125);
            check(after_overlap == before_overlap, format!("unaffected fill/stroke overlap: before {before_overlap:?}, after {after_overlap:?}"));
            check(after_overlap[0] > after_overlap[1] && after_overlap[3] > 0, format!("explicit right stroke remains visible: {after_overlap:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn render_erased(s: &Session) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
}

fn painted_open_rectangle(closed: bool) -> (Session, NodeId) {
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
    s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [id.0], "weight": 2})).unwrap();
    (s, id)
}

fn assert_eraser_history(s: &mut Session, before: &serde_json::Value, before_selection: &vectorcraft_doc::Selection) {
    let after = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let after_selection = s.doc().unwrap().selection.clone();
    let after_pixels = render_erased(s).pixels;
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), *before, "one Undo restores the exact source");
    assert_eq!(s.doc().unwrap().selection, *before_selection);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after, "one Redo restores the exact erased document");
    assert_eq!(s.doc().unwrap().selection, after_selection);
    assert_eq!(render_erased(s).pixels, after_pixels, "Redo restores the rendered appearance");
}

#[test]
fn eraser_keeps_object_metadata_and_mask_once_around_separate_fill_and_stroke() {
    let (mut s, id) = painted_open_rectangle(false);
    let mask = NodeId(s.execute("shape.rectangle", &json!({"x": 80, "y": 80, "width": 140, "height": 140})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [mask.0], "color": "#ffffff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [mask.0], "none": true})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({"ids": [id.0, mask.0]})).unwrap();
    s.execute(
        "transparency.set",
        &json!({"ids": [id.0], "opacity": 50, "blend": "multiply", "isolate": true, "knockout": "off", "knockoutShape": true}),
    )
    .unwrap();
    s.execute("attributes.set", &json!({"ids": [id.0], "note": "Eraser source", "showCenter": true})).unwrap();
    // A stored object axis is independent of this axis-aligned fixture's geometry.
    s.edit("Fixture metadata", |doc, _| {
        let n = doc.node_mut(id).unwrap();
        n.name = Some("Filled open stroke".into());
        n.bbox_angle = 17.0;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let original = s.doc().unwrap().doc.node(id).unwrap().clone();
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let before_selection = s.doc().unwrap().selection.clone();
    let before_history = s.doc().unwrap().history.undo.len();
    let before_image = render_erased(&s);
    assert!(before_image.pixel(150, 125)[3].abs_diff(128) <= 1);
    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), before_history + 1);
    let root = s.doc().unwrap().doc.node(id).unwrap();
    let NodeKind::Group { children, clip: false } = &root.kind else { panic!("fill and stroke share one ordinary object wrapper") };
    assert_eq!(
        (&root.name, root.opacity, root.blend, root.isolate, root.knockout, root.knockout_shape, root.bbox_angle),
        (&original.name, original.opacity, original.blend, original.isolate, original.knockout, original.knockout_shape, original.bbox_angle)
    );
    assert_eq!(root.extras(), original.extras(), "the mask and Attributes remain on the object");
    assert!(!children.is_empty());
    for child in children {
        assert!(child.name.is_none() && child.has_default_transparency(), "paint lanes do not repeat object metadata or transparency");
        assert_eq!(child.bbox_angle, 0.0);
        assert_eq!(child.extras(), &vectorcraft_doc::NodeExtras::default());
    }
    let after = render_erased(&s);
    for (x, y) in [(99, 125), (150, 125), (150, 175), (199, 125)] {
        assert_eq!(after.pixel(x, y), before_image.pixel(x, y), "object appearance outside the sweep at ({x},{y})");
    }
    assert_eq!(after.pixel(150, 150)[3], 0);
    assert_eraser_history(&mut s, &before, &before_selection);
}

#[test]
fn eraser_keeps_a_painted_clipping_source_on_the_legacy_path_route() {
    let mut results = vec![];
    for closed in [true, false] {
        let mut s = session();
        let art = NodeId(s.execute("shape.rectangle", &json!({"x": 90, "y": 90, "width": 60, "height": 120})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setFill", &json!({"ids": [art.0], "color": "#ff0000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [art.0], "none": true})).unwrap();
        let clip = NodeId(
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
        let group = NodeId(s.execute("object.clippingMask.make", &json!({"ids": [art.0, clip.0]})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setFill", &json!({"ids": [clip.0], "color": "#0000ff"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [clip.0], "color": "#00ff00"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [clip.0], "weight": 2})).unwrap();
        s.execute("select.set", &json!({"ids": [clip.0]})).unwrap();
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let before_selection = s.doc().unwrap().selection.clone();
        let image = render_erased(&s);
        assert_eq!(image.pixel(110, 125), [255, 0, 0, 255], "clipped art paints above the clipping fill");
        assert_eq!(image.pixel(180, 125), [0, 0, 255, 255], "clipping fill paints behind the art");
        assert_eq!(image.pixel(199, 125), [0, 255, 0, 255], "clipping stroke paints over the fill");
        assert_eq!(image.pixel(90, 125)[3], 0, "art outside the clip is hidden");
        s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let NodeKind::Group { children, clip: true } = &doc.node(group).unwrap().kind else { panic!("the clipping group is retained") };
        let source = children.first().unwrap();
        assert_eq!(source.id, clip);
        assert!(matches!(source.kind, NodeKind::Path { .. }), "the actual clipping source stays a path");
        let paint = source.clip_paint();
        assert!(paint.fill.is_some() && paint.stroke.is_some(), "the clipping source keeps its fill and stroke paint");
        assert_eq!(render_erased(&s).pixel(150, 150)[3], 0, "the clipping region loses the sweep band");
        // Legacy fill Booleans close both source contours. This deliberately pins that existing
        // painted-clip behavior against the unchanged closed-source route, not the ordinary fix.
        results.push(render_erased(&s).pixels);
        assert_eraser_history(&mut s, &before, &before_selection);
    }
    assert_eq!(results[0], results[1], "open clipping sources retain the existing closed-fill Boolean behavior");
}

#[test]
fn eraser_separates_an_ordinary_open_stroke_inside_a_clipping_group() {
    let (mut s, art) = painted_open_rectangle(false);
    let clip = NodeId(s.execute("shape.rectangle", &json!({"x": 80, "y": 80, "width": 140, "height": 140})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.clippingMask.make", &json!({"ids": [art.0, clip.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
    let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let before_selection = s.doc().unwrap().selection.clone();
    let original_clip = s.doc().unwrap().doc.node(clip).unwrap().clone();
    let before_image = render_erased(&s);
    assert_eq!(before_image.pixel(99, 125)[3], 0);
    assert_eq!(before_image.pixel(150, 125), [0, 0, 0, 255]);
    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(clip).unwrap(), &original_clip, "the actual clipping source is untouched");
    let after = render_erased(&s);
    for (x, y) in [(99, 125), (150, 125), (150, 175), (199, 125)] {
        assert_eq!(after.pixel(x, y), before_image.pixel(x, y), "ordinary clipped art retains its appearance at ({x},{y})");
    }
    assert_eq!(after.pixel(150, 150)[3], 0);
    assert_eraser_history(&mut s, &before, &before_selection);
}

#[test]
fn eraser_keeps_a_live_outline_strokes_original_appearance_context() {
    let mut results = vec![];
    for closed in [true, false] {
        let (mut s, id) = painted_open_rectangle(closed);
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 10})).unwrap();
        s.execute("effect.apply", &json!({"ids": [id.0], "effect": "path.outlineStroke", "item": null, "params": {}})).unwrap();
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let original = s.doc().unwrap().doc.node(id).unwrap();
        assert_eq!(original.appearance.stroke().unwrap().width, 10.0);
        assert!(original.appearance.effects.iter().any(|effect| effect.visible && effect.id == "path.outlineStroke"));
        let before = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let before_selection = s.doc().unwrap().selection.clone();
        s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
        let after = render_erased(&s);
        assert!(after.pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0), "the live effect still paints");
        results.push(after.pixels);
        assert_eraser_history(&mut s, &before, &before_selection);
    }
    // The legacy fill Booleans produce identical closed survivors and retain the 10 pt context.
    assert_eq!(results[0], results[1], "an open source with a live effect retains the closed legacy rendering");
}

#[test]
fn eraser_preserves_a_translucent_fill_above_the_stroke() {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{AppearanceItem, FillLayer};
    let (mut s, id) = painted_open_rectangle(false);
    s.edit("Fixture top fill", |doc, _| {
        let node = doc.node_mut(id).unwrap();
        assert!(matches!(node.appearance.items.last(), Some(AppearanceItem::Stroke(_))));
        let mut fill = FillLayer::new(Paint::solid(Color::WHITE));
        fill.opacity = 0.5;
        node.appearance.items.push(AppearanceItem::Fill(fill));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = render_erased(&s);
    let near = |actual: [u8; 4], expected: [u8; 4]| actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 1);
    assert!(near(before.pixel(199, 125), [255, 128, 128, 255]), "half-opacity white covers red: {:?}", before.pixel(199, 125));
    assert!(near(before.pixel(150, 125), [128, 128, 128, 255]), "half-opacity white covers black: {:?}", before.pixel(150, 125));
    assert_eq!(before.pixel(99, 125), [0, 0, 0, 0]);
    s.execute("path.eraseRegion", &json!({"points": [[50, 150], [250, 150]], "size": 10})).unwrap();
    let after = render_erased(&s);
    for (x, y) in [(199, 125), (150, 125), (99, 125)] {
        assert_eq!(after.pixel(x, y), before.pixel(x, y), "paint order and item opacity survive at ({x},{y})");
    }
    assert_eq!(after.pixel(150, 150)[3], 0, "the swept band is empty");
}
