#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Shape as _;

use super::super::outline::OVERLAY_MAX_ANCHORS;
use super::*;

fn id_of(made: &serde_json::Value) -> NodeId {
    NodeId(made["id"].as_u64().unwrap())
}

fn blank() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// A curve with handles, a live rectangle, a group, point type and a symbol instance, all selected.
fn scene() -> (VectorcraftApp, NodeId) {
    let mut app = blank();
    let curve = id_of(&app.run("path.create", json!({"d": "M40 40 C60 10 90 10 110 40 S160 70 180 40 L180 90"})).unwrap());
    app.run("shape.rectangle", json!({"x": 200.5, "y": 30.25, "width": 60, "height": 40})).unwrap();
    let a = id_of(&app.run("shape.ellipse", json!({"x": 40, "y": 120, "width": 50, "height": 30})).unwrap());
    let b = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 120, "width": 30, "height": 30})).unwrap());
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("text.create", json!({"x": 200, "y": 150, "text": "Mark", "size": 20})).unwrap();
    let art = id_of(&app.run("shape.rectangle", json!({"x": 280, "y": 200, "width": 20, "height": 20})).unwrap());
    app.run("symbol.new", json!({"name": "Mark", "ids": [art.0]})).unwrap();
    app.run("select.all", json!({})).unwrap();
    (app, curve)
}

/// The outlines as (colour, path boxes, key box, revision), for comparing before and after a change.
type Seen = Vec<(Color32, Vec<vectorcraft_geom::Rect>, Option<vectorcraft_geom::Rect>, u64)>;

fn seen(app: &VectorcraftApp) -> Seen {
    let st = app.session.active().unwrap();
    let boxes = |paths: &[BezPath]| paths.iter().map(|p| p.bounding_box()).collect::<Vec<_>>();
    outlines(app, st).iter().map(|o| (o.color, boxes(&o.paths), o.key.as_ref().map(|k| k.bounding_box()), o.revision)).collect()
}

fn revisions(seen: &Seen) -> Vec<u64> {
    seen.iter().map(|o| o.3).collect()
}

fn marks(app: &VectorcraftApp) -> Anchors {
    anchors(app, app.session.active().unwrap(), app.session.tool_id(), false)
}

fn all_selected(selection: &PathSelection) -> usize {
    selection.anchors.len() + selection.others.iter().map(|(_, knots)| knots.len()).sum::<usize>()
}

/// Every change of the document, the selection, the tool or the preferences gives the outlines and anchors of the
/// new state, and the outline's revision follows the paths, the key and the colour it is given.
fn no_stale_marks_after_a_change(tool: &str) {
    let (mut app, curve) = scene();
    app.select_tool(tool);
    let all = app.session.active().unwrap().selection.objects.clone();
    let ids = |list: &[NodeId]| json!({"ids": list.iter().map(|i| i.0).collect::<Vec<_>>()});
    let start = seen(&app);
    assert!(!start.is_empty(), "the selection is outlined");

    // An edit moves the curve's outline: the paths and the revision change; undo returns both.
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    app.run("object.nudge", json!({"dx": 12, "dy": -5})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    let edited = seen(&app);
    assert_ne!(edited, start, "after an edit");
    assert_ne!(revisions(&edited), revisions(&start), "an edit changes the revision");
    app.run("edit.undo", json!({})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_eq!(seen(&app), start, "after an undo the outlines are the first ones, revision included");
    app.run("edit.redo", json!({})).unwrap();
    app.run("select.set", ids(&all)).unwrap();
    assert_eq!(seen(&app), edited, "after a redo");

    // A layer colour changes the colour and the revision, not the paths.
    let layer = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"id": layer.0, "color": "#ff00aa"})).unwrap();
    let recolored = seen(&app);
    assert_ne!(recolored.iter().map(|o| o.0).collect::<Vec<_>>(), edited.iter().map(|o| o.0).collect::<Vec<_>>(), "a layer colour");
    assert_ne!(revisions(&recolored), revisions(&edited), "a colour changes the revision");

    // Fewer objects, then the key object.
    app.run("select.set", ids(&all[..2])).unwrap();
    let fewer = seen(&app);
    assert_eq!(fewer.iter().map(|o| o.1.len()).sum::<usize>(), 2, "one outline per selected object");
    let keyless = revisions(&fewer);
    app.session.active_mut().unwrap().selection.key = Some(curve);
    let keyed = seen(&app);
    assert!(keyed.iter().any(|o| o.2.is_some()), "the key object's outline");
    assert_ne!(revisions(&keyed), keyless, "a key object changes the revision");
    app.session.active_mut().unwrap().selection.key = None;

    // The tool decides what anchors show, and leaves the outlines as they are.
    app.select_tool(if tool == "selection" { "directSelection" } else { "selection" });
    assert_eq!(seen(&app), fewer, "the other tool outlines the same paths");
    app.select_tool(tool);

    // Anchors and a segment of the curve are the selection's own state, not a command's.
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    app.session.active_mut().unwrap().selection.anchors.insert(curve, [(0, 1)].into_iter().collect());
    let one = marks(&app).selection;
    assert_eq!((one.path, one.anchors.clone()), (Some(curve.0), vec![KnotRef::new(0, 1)]), "one anchor of the curve");
    app.session.active_mut().unwrap().selection.anchors.insert(curve, [(0, 2), (0, 3)].into_iter().collect());
    assert_eq!(marks(&app).selection.anchors, vec![KnotRef::new(0, 2), KnotRef::new(0, 3)], "other anchors of the curve");
    app.session.active_mut().unwrap().selection.segments.insert(curve, [(0, 2)].into_iter().collect());
    let segment = marks(&app).selection;
    assert_eq!((segment.segment, segment.anchors.len()), (Some(KnotRef::new(0, 2)), 0), "a segment hollows the anchors");

    // The display preferences.
    app.session.active_mut().unwrap().selection.segments.clear();
    app.session.prefs.anchor_size = 6;
    app.session.prefs.handle_style = "hollow".into();
    let options = marks(&app).options;
    assert_eq!((options.anchor_size, options.handles), (Some(8.0), HandleStyle::Hollow));
    app.session.prefs.show_handles_multiple_anchors = false;
    assert_eq!(marks(&app).handles, HandlesShown::OneAnchor);
    app.session.prefs.show_handles_multiple_anchors = true;
    assert_eq!(marks(&app).handles, HandlesShown::Selected);
}

#[test]
fn no_stale_marks_after_a_change_with_the_selection_tool() {
    no_stale_marks_after_a_change("selection");
}

#[test]
fn no_stale_marks_after_a_change_with_the_direct_selection_tool() {
    no_stale_marks_after_a_change("directSelection");
}

/// Direct Selection edits anchors, and so draws the anchors of a partial selection; the Selection tool, a whole one.
#[test]
fn only_the_chosen_anchors_of_a_partial_selection_are_selected() {
    let (mut app, curve) = scene();
    app.select_tool("directSelection");
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    let NodeKind::Path { path, .. } = &app.session.active().unwrap().doc.node(curve).unwrap().kind else { panic!("a path") };
    assert_eq!(marks(&app).selection.anchors.len(), path.anchor_count(), "selected whole: every anchor");
    app.run("select.anchors", json!({"id": curve.0, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    assert_eq!(marks(&app).selection.anchors, vec![KnotRef::new(0, 2)], "partial: the chosen anchor");
}

/// The instance's outline comes from its symbol's art, which is not part of the instance's node.
#[test]
fn an_edit_of_a_symbol_outlines_its_instance_again() {
    let (mut app, _) = scene();
    let instance = {
        let st = app.session.active().unwrap();
        st.selection.objects.iter().copied().find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::SymbolInstance { .. }))).unwrap()
    };
    app.run("select.set", json!({"ids": [instance.0]})).unwrap();
    let before = seen(&app);
    app.run("symbol.edit", json!({"name": "Mark"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("object.scale", json!({"sx": 3, "sy": 2})).unwrap();
    app.run("symbol.exitEditing", json!({})).unwrap();
    app.run("select.set", json!({"ids": [instance.0]})).unwrap();
    let after = seen(&app);
    assert_ne!(before[0].1, after[0].1, "the instance is outlined around its edited art");
    assert_ne!(revisions(&before), revisions(&after), "and the revision follows");
}

/// A clip group of a wide rectangle and a square: the group is `(group, wide, square)`.
fn clip_group() -> (VectorcraftApp, NodeId, NodeId) {
    let mut app = blank();
    let wide = id_of(&app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap());
    let square = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap());
    app.run("select.set", json!({"ids": [wide.0, square.0]})).unwrap();
    let group = id_of(&app.run("object.clippingMask.make", json!({})).unwrap());
    (app, group, wide)
}

#[test]
fn a_selected_clip_group_shows_its_clipping_path_not_its_hidden_art() {
    let (mut app, group, _) = clip_group();
    for tool in ["selection", "directSelection", "groupSelection"] {
        app.select_tool(tool);
        app.run("select.set", json!({"ids": [group.0]})).unwrap();
        let shown = seen(&app);
        assert_eq!(
            shown.iter().map(|o| o.1.clone()).collect::<Vec<_>>(),
            vec![vec![vectorcraft_geom::Rect::new(100.0, 100.0, 200.0, 200.0)]],
            "{tool}"
        );
    }
}

#[test]
fn the_art_a_clip_group_clips_shows_whole_when_picked() {
    let (mut app, _, wide) = clip_group();
    app.select_tool("groupSelection");
    app.run("select.set", json!({"ids": [wide.0]})).unwrap();
    assert_eq!(seen(&app)[0].1, vec![vectorcraft_geom::Rect::new(0.0, 0.0, 300.0, 300.0)]);
}

#[test]
fn a_clip_group_over_a_huge_path_shows_its_clipping_path() {
    let mut app = blank();
    let n = OVERLAY_MAX_ANCHORS + 10;
    let points = (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
    let huge = id_of(&app.run("path.create", json!({"d": format!("M20 20 {points} Z")})).unwrap());
    let square = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap());
    app.run("select.set", json!({"ids": [huge.0, square.0]})).unwrap();
    let group = id_of(&app.run("object.clippingMask.make", json!({})).unwrap());
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [group.0]})).unwrap();
    assert_eq!(
        seen(&app)[0].1,
        vec![vectorcraft_geom::Rect::new(100.0, 100.0, 200.0, 200.0)],
        "the square's outline, not the bounds of the huge art"
    );
    assert_eq!(marks(&app).paths.iter().map(|p| p.anchor_count()).sum::<usize>(), 4, "and its four anchors");
}

#[test]
fn a_huge_selection_shows_its_bounds_instead_of_every_outline() {
    let mut app = blank();
    let points = |n: usize| (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
    app.run("path.create", json!({"d": format!("M20 20 {} Z", points(OVERLAY_MAX_ANCHORS + 10))})).unwrap();
    let color = c32(app.session.active().unwrap().doc.layer_color(app.session.active().unwrap().selection.objects[0]));
    let huge = seen(&app);
    assert_eq!((huge.len(), huge[0].0, huge[0].1.len()), (1, color, 1), "one bounds path in the layer colour");
    let st = app.session.active().unwrap();
    assert!(outlines(&app, st)[0].paths[0].elements().len() < 10, "a rectangle, not the outline");
    assert!(marks(&app).paths.is_empty(), "no anchors either");
    assert!(dots(&app, st).origins.is_empty());
    app.run("edit.undo", json!({})).unwrap();
    app.run("path.create", json!({"d": format!("M20 20 {} Z", points(200))})).unwrap();
    let st = app.session.active().unwrap();
    assert!(outlines(&app, st)[0].paths[0].elements().len() > 200, "a path of few points is outlined whole");
    assert_eq!(marks(&app).paths.len(), 1);
}

#[test]
fn the_key_object_has_its_own_outline() {
    let (mut app, curve) = scene();
    let ids = app.session.active().unwrap().selection.objects.clone();
    app.run("select.key", json!({"id": curve.0})).unwrap();
    let st = app.session.active().unwrap();
    let want = node_outline(st.doc.node(curve).unwrap(), &st.doc.symbol_boxes());
    let color = c32(st.doc.layer_color(curve));
    let out = outlines(&app, st);
    let key = out.iter().find(|o| o.color == color).and_then(|o| o.key.as_ref()).unwrap();
    assert_eq!(key.bounding_box(), want.bounding_box());
    assert_eq!(out.iter().filter(|o| o.key.is_some()).count(), 1, "one key outline");
    assert_eq!(out.iter().map(|o| o.paths.len()).sum::<usize>(), ids.len(), "the selection's own outlines stay");
}

/// A file with a 40 × 40 px red PNG in a new temporary folder named `name`.
fn red_png(name: &str) -> std::path::PathBuf {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let png = vectorcraft_testkit::temp_dir(name).join("red.png");
    let image = Image::from_raw(40, 40, ChannelLayout::Rgba, SampleType::U8, [200u8, 40, 40, 255].repeat(40 * 40)).unwrap();
    std::fs::write(&png, craft_image_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    png
}

/// The two lines from corner to corner of `[tl, tr, br, bl]`.
fn cross_of([tl, tr, br, bl]: [Point; 4]) -> Vec<PathEl> {
    vec![PathEl::MoveTo(tl), PathEl::LineTo(br), PathEl::MoveTo(tr), PathEl::LineTo(bl)]
}

/// The outline of `[tl, tr, br, bl]`: its corners joined by straight lines.
fn frame_of([tl, tr, br, bl]: [Point; 4]) -> Vec<PathEl> {
    vec![PathEl::MoveTo(tl), PathEl::LineTo(tr), PathEl::LineTo(br), PathEl::LineTo(bl), PathEl::ClosePath]
}

/// A selected linked image and a selected placed document are crossed from corner to corner, on a turned or
/// distorted image along its frame; a selected embedded image shows its outline alone, along its frame when turned or
/// distorted. The key object's outline and the hover outline are not crossed (#9080).
#[test]
fn a_selected_linked_image_is_crossed_and_an_embedded_one_is_not() {
    let png = red_png("selection-linked");
    let placed = png.with_file_name("art.vectorcraft");
    std::fs::write(&placed, vectorcraft_format::save_file(&vectorcraft_doc::Document::new(60.0, 40.0))).unwrap();
    let mut app = blank();
    let place = |app: &mut VectorcraftApp, path: &std::path::Path, link: bool, at: [f64; 2]| {
        NodeId(app.run("file.place", json!({"path": path, "link": link, "at": at})).unwrap()["ids"][0].as_u64().unwrap())
    };
    let linked = place(&mut app, &png, true, [100.0, 120.0]);
    let embedded = place(&mut app, &png, false, [300.0, 120.0]);
    let document = place(&mut app, &placed, true, [200.0, 230.0]);
    let corners = |app: &VectorcraftApp, id: NodeId| {
        let n = app.session.active().unwrap().doc.node(id).unwrap();
        match &n.kind {
            NodeKind::PlacedDocument(p) => p.corners(),
            _ => n.image_corners().unwrap(),
        }
    };
    // Whether the selection's outlines hold a path made of `els`.
    let drawn = |app: &VectorcraftApp, els: &[PathEl]| {
        outlines(app, app.session.active().unwrap()).iter().any(|o| o.paths.iter().any(|p| p.elements() == els))
    };
    // Whether `id` is crossed, and whether it is outlined along its frame.
    let marked = |app: &VectorcraftApp, id: NodeId| (drawn(app, &cross_of(corners(app, id))), drawn(app, &frame_of(corners(app, id))));
    app.run("select.all", json!({})).unwrap();
    assert_eq!(marked(&app, linked), (true, true), "a linked image");
    assert!(drawn(&app, &cross_of(corners(&app, document))), "a placed document");
    assert_eq!(marked(&app, embedded), (false, true), "an embedded image");
    assert_eq!(outlines(&app, app.session.active().unwrap()).iter().map(|o| o.paths.len()).sum::<usize>(), 5, "three outlines and two crosses");
    app.run("select.set", json!({"ids": [linked.0, embedded.0]})).unwrap();
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    assert_eq!(marked(&app, linked), (true, true), "a turned linked image, along its frame");
    assert_eq!(marked(&app, embedded), (false, true), "a turned embedded image, outlined along its frame");
    // Move each image's bottom right corner up its right side, so its corners no longer make a parallelogram.
    app.session
        .edit("Distort", |doc, _| {
            for id in [linked, embedded] {
                if let Some(n) = doc.node_mut(id)
                    && let Some([tl, tr, br, bl]) = n.image_corners()
                {
                    n.set_image_corners([tl, tr, br.lerp(tr, 0.4), bl]).map_err(|e| vectorcraft_engine::EngineError::Other(format!("{e:?}")))?;
                }
            }
            Ok(())
        })
        .unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!([linked, embedded].iter().all(|id| doc.node(*id).unwrap().is_distorted_image()), "both images are distorted");
    assert_eq!(marked(&app, linked), (true, true), "a distorted linked image, corner to corner");
    assert_eq!(marked(&app, embedded), (false, true), "a distorted embedded image, outlined along its frame");
    // The key object's outline is its frame alone, and so is the hover outline.
    for id in [linked, embedded] {
        app.run("select.key", json!({"id": id.0})).unwrap();
        let st = app.session.active().unwrap();
        let key = outlines(&app, st).into_iter().find_map(|o| o.key).unwrap();
        assert_eq!(key.elements(), frame_of(corners(&app, id)).as_slice(), "the key object's outline of {id:?}");
        assert_eq!(outline_of(st, id).unwrap().path.elements(), frame_of(corners(&app, id)).as_slice(), "the hover outline of {id:?}");
    }
}

/// The selection's outlines count the ends of the lines that cross linked images toward the overlay's limit; the
/// hover outline and the key object's outline, which draw no such lines, do not (#9080).
#[test]
fn the_overlay_limit_counts_crossing_lines_only_where_they_are_drawn() {
    use super::super::outline::{too_many_anchors, too_many_marks};
    let image = |link: Option<vectorcraft_doc::LinkInfo>| {
        let im = vectorcraft_doc::ImageObject {
            key: "k".into(),
            width: 10,
            height: 10,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link,
            placement: Default::default(),
            pixels: None,
        };
        vectorcraft_doc::Node::new(NodeId(1), NodeKind::Image(Box::new(im)))
    };
    // Four corners each fit; with the four ends of the crossing lines they do not.
    let count = OVERLAY_MAX_ANCHORS / 6;
    let linked = vec![image(Some(vectorcraft_doc::LinkInfo { path: "/a.png".into(), ..Default::default() })); count];
    let embedded = vec![image(None); count];
    assert!(!too_many_anchors(&linked) && too_many_marks(&linked), "linked images");
    assert!(!too_many_anchors(&embedded) && !too_many_marks(&embedded), "embedded images");
}

/// A selected group outlines, marks and crosses only the members the canvas draws: a hidden member and a linked image
/// in a hidden group inside it leave no outline, anchors or crossing lines, in the selection's marks and in the hover
/// outline. Hiding them shows the same as deleting them.
#[test]
fn a_selected_group_leaves_its_hidden_members_unmarked() {
    let png = red_png("selection-hidden");
    let mut app = blank();
    let left = id_of(&app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    let right = id_of(&app.run("shape.rectangle", json!({"x": 330, "y": 230, "width": 20, "height": 20})).unwrap());
    let hidden = id_of(&app.run("shape.rectangle", json!({"x": 120, "y": 100, "width": 40, "height": 40})).unwrap());
    let linked = app.run("file.place", json!({"path": png, "link": true, "at": [240.0, 150.0]})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let inner = id_of(&app.run("object.group", json!({})).unwrap());
    app.run("select.set", json!({"ids": [left.0, right.0, hidden.0, inner.0]})).unwrap();
    let group = id_of(&app.run("object.group", json!({})).unwrap());
    // With the group selected after the commands `edit`: its outline paths, the paths that show anchors with their
    // anchor count, and its hover outline.
    let shown = |app: &mut VectorcraftApp, edit: &[(&str, serde_json::Value)]| {
        for (command, params) in edit {
            app.run(command, params.clone()).unwrap();
        }
        app.run("select.set", json!({"ids": [group.0]})).unwrap();
        let st = app.session.active().unwrap();
        let outlined: Vec<Vec<PathEl>> = outlines(app, st).into_iter().flat_map(|o| o.paths).map(|p| p.elements().to_vec()).collect();
        let anchored = marks(app).paths.iter().map(|p| p.anchor_count()).collect::<Vec<_>>();
        let hovered = outline_of(st, group).map(|h| h.path.elements().to_vec());
        (outlined, anchored, hovered)
    };
    let both = shown(&mut app, &[]);
    let without = shown(&mut app, &[("select.set", json!({"ids": [hidden.0, inner.0]})), ("edit.clear", json!({}))]);
    let with_hidden = shown(&mut app, &[("edit.undo", json!({})), ("select.set", json!({"ids": [hidden.0, inner.0]})), ("object.hide", json!({}))]);
    assert_ne!(both, without, "the marks show every member");
    assert_eq!(without.1, vec![4, 4], "the anchors of the two visible members");
    assert_eq!(with_hidden, without, "hidden members are not marked");
}

/// The overlay's limit counts the points of the members the highlight outlines: a hidden member of a group, a blend, a
/// repeat or an intertwine, and the art of a hidden sublayer, do not count toward it.
#[test]
fn the_overlay_limit_leaves_out_hidden_members() {
    use super::super::outline::{too_many_anchors, too_many_marks};
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, LayerColor, Node};
    use vectorcraft_geom::SubPath;
    let points: Vec<Point> = (0..=OVERLAY_MAX_ANCHORS).map(|i| Point::new(i as f64, 0.0)).collect();
    let many = Arc::new(Node::path(NodeId(2), PathData::single(SubPath::polyline(&points, false)), Appearance::default()));
    let few = Arc::new(Node::path(
        NodeId(3),
        vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::default(),
    ));
    let mut wrong = vec![];
    for visible in [true, false] {
        let mut member = (*many).clone();
        member.visible = visible;
        let members = || vec![Arc::new(member.clone()), few.clone()];
        let group = Node::group(NodeId(1), members());
        let blend = Node::new(NodeId(6), NodeKind::Blend { children: members(), spec: Default::default() });
        let repeat = Node::new(NodeId(7), NodeKind::Repeat(vectorcraft_doc::RepeatSpec::grid(members(), 0.0, 0.0)));
        let intertwine = Node::new(NodeId(8), NodeKind::Intertwine { children: members() });
        let mut sublayer = Node::layer(NodeId(5), "Sublayer", LayerColor::Preset(1));
        sublayer.visible = visible;
        if let Some(children) = sublayer.children_mut() {
            children.push(many.clone());
        }
        let mut layer = Node::layer(NodeId(4), "Layer", LayerColor::Preset(0));
        if let Some(children) = layer.children_mut() {
            children.extend([Arc::new(sublayer), few.clone()]);
        }
        for (n, what) in [(&group, "a group"), (&blend, "a blend"), (&repeat, "a repeat"), (&intertwine, "an intertwine"), (&layer, "a layer")] {
            if too_many_anchors([n]) != visible || too_many_marks([n]) != visible {
                wrong.push(format!("{what}, member visible: {visible}"));
            }
        }
    }
    assert!(wrong.is_empty(), "the limit is crossed only by visible points: {wrong:?}");
}

#[test]
fn the_anchors_of_a_container_selected_whole_draw_selected() {
    let (mut app, _) = scene();
    let group = {
        let st = app.session.active().unwrap();
        st.selection.objects.iter().copied().find(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Group { .. }))).unwrap()
    };
    app.select_tool("directSelection");
    app.run("select.set", json!({"ids": [group.0]})).unwrap();
    let shown = marks(&app);
    // The ellipse's four anchors and the square's four.
    assert_eq!(all_selected(&shown.selection), 8);
    assert_eq!(shown.paths.len(), 2);
}

#[test]
fn point_type_baselines_run_under_the_line_whatever_its_alignment() {
    for justify in ["left", "center", "right"] {
        let mut app = blank();
        let id = id_of(&app.run("text.create", json!({"x": 300, "y": 150, "text": "Baseline marker", "size": 24, "justify": justify})).unwrap());
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        let st = app.session.active().unwrap();
        let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
        let line = outlines(&app, st)[0].paths[0].bounding_box();
        assert!((line.y0 - 150.0).abs() < 0.5 && (line.y1 - 150.0).abs() < 0.5, "{justify}: the baseline is level at the anchor's height");
        assert!((line.x0 - b.x0).abs() < 3.0 && (line.x1 - b.x1).abs() < 3.0, "{justify}: the baseline runs {line:?}, the type {b:?}");
        assert_eq!(dots(&app, st).origins.len(), 1, "{justify}: the origin dot");
    }
}

#[test]
fn point_type_hover_shows_baseline_only_and_selection_shows_both_box_and_baseline() {
    let mut app = blank();
    let id = id_of(&app.run("text.create", json!({"x": 200, "y": 150, "text": "Lorem ipsum", "size": 36})).unwrap());
    app.run("select.none", json!({})).unwrap();
    app.select_tool("selection");
    let over = Point::new(220.0, 140.0);
    let st = app.session.active().unwrap();
    let hovered = hover(&app, st, app.session.tool_id(), over, 1.0, true).expect("the type under the pointer is outlined");
    let line = hovered.path.bounding_box();
    assert!((line.y0 - 150.0).abs() < 0.5 && line.height() < 0.5, "the baseline only, not the box: {line:?}");
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let st = app.session.active().unwrap();
    assert!(hover(&app, st, app.session.tool_id(), over, 1.0, true).is_none(), "a selected object has no hover outline");
    assert_eq!(outlines(&app, st)[0].paths.len(), 1, "the selection outlines the baseline; the box is the bounding box's");
}

fn hover_scene() -> VectorcraftApp {
    let mut app = blank();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.select_tool("selection");
    app
}

fn hovered(app: &VectorcraftApp, still: bool) -> bool {
    hover(app, app.session.active().unwrap(), app.session.tool_id(), Point::new(150.0, 150.0), 1.0, still).is_some()
}

#[test]
fn hover_outlines_the_object_under_the_pointer_unless_something_hides_it() {
    let mut app = hover_scene();
    assert!(hovered(&app, true));
    // Snap to Grid, Pixel Preview, the preference, and a view that is still moving.
    app.ui.view.snap_to_grid = true;
    assert!(!hovered(&app, true), "Snap to Grid hides it");
    app.ui.view.snap_to_grid = false;
    app.ui.view.pixel_preview = true;
    assert!(!hovered(&app, true), "Pixel Preview hides it");
    app.ui.view.pixel_preview = false;
    app.session.prefs.object_highlighting = false;
    assert!(!hovered(&app, true), "the preference off hides it");
    app.session.prefs.object_highlighting = true;
    assert!(!hovered(&app, false), "a moving view leaves the art under the pointer alone");
    assert!(hovered(&app, true), "and it shows once the view rests");
    app.select_tool("pen");
    assert!(!hovered(&app, true), "a tool that is not a selection tool");
}

fn held(shift: bool, alt: bool, cmd: bool, space: bool) -> Mods {
    Mods { shift, alt, cmd, space, ..Mods::default() }
}

const NO_KEYS: Mods = Mods { shift: false, alt: false, cmd: false, ctrl: false, space: false };

/// The box of the hover outline at `at` with `held` keys down, as the canvas finds it: the tool whose marks show
/// (with Cmd, the selection tool a press there borrows), then that tool's outline.
fn outline_box(app: &mut VectorcraftApp, at: Point, held: Mods) -> Option<vectorcraft_geom::Rect> {
    let view = app.view_info();
    let tool = app.session.hover_tool_at(Some(at), &held, view);
    hover_target(app, tool, at, held, 1.0, true).map(|h| h.path.bounding_box())
}

/// Whether two boxes agree to within a point.
fn same_box(a: Option<vectorcraft_geom::Rect>, b: vectorcraft_geom::Rect) -> bool {
    a.is_some_and(|a| (a.x0 - b.x0).abs() < 1.0 && (a.y0 - b.y0).abs() < 1.0 && (a.x1 - b.x1).abs() < 1.0 && (a.y1 - b.y1).abs() < 1.0)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> vectorcraft_geom::Rect {
    vectorcraft_geom::Rect::new(x0, y0, x1, y1)
}

/// A rectangle filled with `color`, its id.
fn filled(app: &mut VectorcraftApp, x: f64, y: f64, w: f64, h: f64, color: &str) -> NodeId {
    let id = id_of(&app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap());
    app.run("paint.setFill", json!({"ids": [id.0], "color": color})).unwrap();
    id
}

/// With the Eyedropper, Smart Guides › Object Highlighting outlines the object under the pointer as the selection
/// tools do, and nothing over empty canvas. While Space is held a press pans the view, so no tool outlines anything.
#[test]
fn eyedropper_hover_outlines_the_object_under_the_pointer() {
    let mut app = hover_scene();
    app.select_tool("eyedropper");
    let over = Point::new(150.0, 150.0);
    assert!(same_box(outline_box(&mut app, over, NO_KEYS), rect(100.0, 100.0, 200.0, 200.0)), "the rectangle under the pointer");
    assert!(outline_box(&mut app, Point::new(300.0, 50.0), NO_KEYS).is_none(), "nothing over empty canvas");
    for tool in ["eyedropper", "selection"] {
        app.select_tool(tool);
        assert!(outline_box(&mut app, over, NO_KEYS).is_some(), "{tool}: outlined");
        assert!(outline_box(&mut app, over, held(false, false, false, true)).is_none(), "{tool}: Space held");
        assert!(outline_box(&mut app, over, NO_KEYS).is_some(), "{tool}: Space let go");
    }
}

/// The Eyedropper outlines a selected object too, since its click still acts on it. With Cmd held, a click selects
/// with the selection tool lent to it, so the outline is that tool's: none on a selected object, and the object under
/// the pointer otherwise. Hiding the object under the pointer, or arming Image Trace's Ignore Color pick (which acts on
/// no object), removes the outline without the pointer moving.
#[test]
fn eyedropper_hover_outline_follows_the_selection_cmd_and_the_document() {
    let mut app = blank();
    let id = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap());
    app.select_tool("eyedropper");
    let (over, square) = (Point::new(150.0, 150.0), rect(100.0, 100.0, 200.0, 200.0));
    assert!(app.session.active().unwrap().selection.contains(id));
    assert!(same_box(outline_box(&mut app, over, NO_KEYS), square), "a selected object");
    let cmd = held(false, false, true, false);
    assert!(outline_box(&mut app, over, cmd).is_none(), "Cmd over a selected object");
    app.run("select.none", json!({})).unwrap();
    assert!(same_box(outline_box(&mut app, over, cmd), square), "Cmd over an unselected object");
    assert!(outline_box(&mut app, over, NO_KEYS).is_some(), "Cmd let go");
    app.session.set_tool_option("imageTraceIgnoreColor", &json!({"binding": 1}));
    assert!(outline_box(&mut app, over, NO_KEYS).is_none(), "an armed Ignore Color pick");
    app.session.set_tool_option("imageTraceIgnoreColor", &serde_json::Value::Null);
    assert!(outline_box(&mut app, over, NO_KEYS).is_some(), "the pick disarmed");
    app.run("object.hide", json!({"ids": [id.0]})).unwrap();
    assert!(outline_box(&mut app, over, NO_KEYS).is_none(), "hidden art");
}

/// With a group member selected (as Group Selection leaves it), the Selection tool outlines nothing over that member,
/// where a press keeps the selection, and the group over its other member, where a press selects the group.
#[test]
fn selection_hover_outline_skips_a_selected_group_member() {
    let mut app = blank();
    let a = id_of(&app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap());
    let b = id_of(&app.run("shape.rectangle", json!({"x": 200, "y": 100, "width": 50, "height": 50})).unwrap());
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    app.select_tool("selection");
    assert!(outline_box(&mut app, Point::new(125.0, 125.0), NO_KEYS).is_none(), "the selected member");
    let other = outline_box(&mut app, Point::new(225.0, 125.0), NO_KEYS);
    assert!(same_box(other, rect(100.0, 100.0, 250.0, 150.0)), "the other member: {other:?}");
}

/// For a Shift-click the Eyedropper outlines the object whose color the click samples: the blend where one of its
/// steps is drawn, and the rectangle below it in a gap between the steps.
#[test]
fn eyedropper_shift_hover_outlines_the_object_sampled() {
    let mut app = blank();
    filled(&mut app, 20.0, 50.0, 360.0, 150.0, "#00ff00");
    let a = filled(&mut app, 100.0, 100.0, 20.0, 20.0, "#ff0000");
    let b = filled(&mut app, 200.0, 100.0, 20.0, 20.0, "#0000ff");
    app.run("object.blend.make", json!({"ids": [a.0, b.0], "steps": 1})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.select_tool("eyedropper");
    let shift = held(true, false, false, false);
    // One step, halfway: (150, 100) to (170, 120).
    let step = outline_box(&mut app, Point::new(160.0, 110.0), shift);
    assert!(same_box(step, rect(100.0, 100.0, 220.0, 120.0)), "a blend step: {step:?}");
    let gap = outline_box(&mut app, Point::new(135.0, 110.0), shift);
    assert!(same_box(gap, rect(20.0, 50.0, 380.0, 200.0)), "a gap between the steps: {gap:?}");
}

/// The Eyedropper's hover outline follows Smart Guides › Object Highlighting.
#[test]
fn eyedropper_hover_outline_follows_object_highlighting() {
    let mut app = hover_scene();
    app.select_tool("eyedropper");
    let over = Point::new(150.0, 150.0);
    app.run("prefs.set", json!({"key": "objectHighlighting", "value": false})).unwrap();
    assert!(outline_box(&mut app, over, NO_KEYS).is_none(), "the option off");
    app.run("prefs.set", json!({"key": "objectHighlighting", "value": true})).unwrap();
    assert!(outline_box(&mut app, over, NO_KEYS).is_some(), "the option on");
}

/// The Eyedropper outlines the object its click there then acts on: the front object a plain click copies attributes
/// from, a locked one included (the Eyedropper reads locked art), and with Alt the unlocked object below it that an
/// Alt-click gives the selection's attributes to.
#[test]
fn eyedropper_hover_outline_is_the_object_the_click_acts_on() {
    let mut app = blank();
    let back = filled(&mut app, 100.0, 100.0, 100.0, 100.0, "#0000ff");
    let front = filled(&mut app, 150.0, 150.0, 100.0, 100.0, "#ff0000");
    let target = filled(&mut app, 20.0, 20.0, 40.0, 40.0, "#00ff00");
    app.run("select.set", json!({"ids": [target.0]})).unwrap();
    app.select_tool("eyedropper");
    let fill = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().appearance.fill_paint();
    let click = |app: &mut VectorcraftApp, at: Point, mods: Mods| {
        let view = app.view_info();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            let ev = vectorcraft_tools::PointerEvent { mods, ..vectorcraft_tools::PointerEvent::new(kind, at.x, at.y) };
            crate::canvas::dispatch(app, &ev, view);
        }
    };
    let over = Point::new(175.0, 175.0);
    let alt = held(false, true, false, false);
    let shown = outline_box(&mut app, over, NO_KEYS);
    assert!(same_box(shown, rect(150.0, 150.0, 250.0, 250.0)), "the front rectangle: {shown:?}");
    click(&mut app, over, NO_KEYS);
    assert_eq!(fill(&app, target), fill(&app, front), "the click copied the outlined rectangle's fill");

    app.run("object.lock", json!({"ids": [front.0]})).unwrap();
    app.run("select.set", json!({"ids": [target.0]})).unwrap();
    app.run("paint.setFill", json!({"ids": [target.0], "color": "#00ff00"})).unwrap();
    let shown = outline_box(&mut app, over, NO_KEYS);
    assert!(same_box(shown, rect(150.0, 150.0, 250.0, 250.0)), "the locked front rectangle is still the source: {shown:?}");
    let shown = outline_box(&mut app, over, alt);
    assert!(same_box(shown, rect(100.0, 100.0, 200.0, 200.0)), "with Alt, the unlocked rectangle below: {shown:?}");
    click(&mut app, over, alt);
    assert_eq!(fill(&app, back), fill(&app, target), "the Alt-click gave the outlined rectangle the selection's fill");
    assert_ne!(fill(&app, front), fill(&app, target));
}

#[test]
fn a_group_of_too_many_paths_has_no_hover_outline_and_a_key_outline_of_its_bounds() {
    let mut app = blank();
    let n = OVERLAY_MAX_ANCHORS + 10;
    let points = (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
    let huge = id_of(&app.run("path.create", json!({"d": format!("M20 20 {points} Z")})).unwrap());
    app.run("select.none", json!({})).unwrap();
    app.select_tool("selection");
    let st = app.session.active().unwrap();
    assert!(hover(&app, st, app.session.tool_id(), Point::new(20.0, 20.0), 1.0, true).is_none(), "no hover outline for what is too big");
    // A key object stays while another selected object lies outside it.
    let other = id_of(&app.run("shape.rectangle", json!({"x": 300, "y": 20, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [huge.0, other.0]})).unwrap();
    app.run("select.key", json!({"id": huge.0})).unwrap();
    let st = app.session.active().unwrap();
    let key = outlines(&app, st)[0].key.as_ref().unwrap().bounding_box();
    assert_eq!(key, st.doc.node(huge).unwrap().geometric_bounds_with(&st.doc.symbol_boxes()).unwrap(), "the key outline is the bounds");
}

#[test]
fn curvature_marks_existing_paths_as_circles_and_leaves_a_drafts_points_to_the_tool() {
    let (mut app, curve) = scene();
    app.run("select.set", json!({"ids": [curve.0]})).unwrap();
    app.select_tool("curvature");
    let st = app.session.active().unwrap();
    let existing = anchors(&app, st, app.session.tool_id(), false);
    assert_eq!((existing.shown, existing.paths.len()), (AnchorsShown::Curvature, 1), "one circle per point of the existing path");
    assert!(anchors(&app, st, app.session.tool_id(), true).paths.is_empty(), "while the tool draws its own points, none are drawn twice");
    app.select_tool("directSelection");
    assert_eq!(marks(&app).shown, AnchorsShown::All);
    assert!(!marks(&app).paths.is_empty(), "other tools keep their anchors whatever `tool_points` says");
}

#[test]
fn anchor_size_scales_anchors_and_handles() {
    let options = |size: u32, style: &str| path_options(&Prefs { anchor_size: size, handle_style: style.into(), ..Default::default() });
    assert_eq!(options(3, "solid").anchor_size, Some(ANCHOR_AT_DEFAULT_SIZE));
    let large = options(7, "large");
    assert_eq!((large.anchor_size, large.handles), (Some(ANCHOR_AT_DEFAULT_SIZE + 4.0), HandleStyle::Large));
    assert_eq!(options(99, "solid").anchor_size, Some(ANCHOR_AT_DEFAULT_SIZE + 4.0), "Size stays within 1–7");
    assert_eq!(options(3, "solid").handles, HandleStyle::Solid);
}

/// The revision is of the drawn outline: the same paths and colour give one value, other paths or another colour do not.
#[test]
fn the_outline_revision_follows_what_is_drawn() {
    let line = |x: f64| {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((x, 1.0));
        path
    };
    let rev = |paths: &[BezPath], key: Option<&BezPath>, color: Color32| outline_revision(paths, key, color.to_array());
    let base = rev(&[line(1.0)], None, Color32::RED);
    assert_eq!(base, rev(&[line(1.0)], None, Color32::RED), "equal inputs");
    assert_ne!(base, rev(&[line(2.0)], None, Color32::RED), "a moved point");
    assert_ne!(base, rev(&[line(1.0)], None, Color32::BLUE), "another colour");
    assert_ne!(base, rev(&[line(1.0)], Some(&line(1.0)), Color32::RED), "a key outline");
    assert_ne!(base, rev(&[line(1.0), line(1.0)], None, Color32::RED), "another number of paths");
}

#[test]
fn an_empty_selection_has_no_marks() {
    let (mut app, _) = scene();
    assert!(!seen(&app).is_empty() && !marks(&app).paths.is_empty());
    app.run("select.none", json!({})).unwrap();
    assert!(seen(&app).is_empty() && marks(&app).paths.is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(dots(&app, st).centers.len() + dots(&app, st).origins.len(), 0);
}
