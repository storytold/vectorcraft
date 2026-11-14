//! Blend edits invalidate every pixel they change, including keys aligned to a curved spine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Affine;
use vectorcraft_render::{Changed, RenderOptions, Rendered, Renderer, view_pixels};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 480;

type Retained = (Renderer, Option<(Arc<Document>, Vec<u8>)>);

fn live(s: &Session, blend: NodeId, keys: [NodeId; 2], curved: bool) {
    let doc = &s.doc().unwrap().doc;
    let NodeKind::Blend { children, spec } = &doc.node(blend).unwrap().kind else {
        panic!("INPUT: the fixture must remain a live blend");
    };
    assert_eq!(children.iter().map(|n| n.id).collect::<Vec<_>>(), keys.to_vec());
    assert_eq!(serde_json::to_value(spec).unwrap()["spacing"], json!({"steps": 3}));
    for key in keys {
        assert_eq!(doc.parent_of(key), Some(blend), "INPUT: key identity/parent changed");
    }
    if curved {
        let bounds = spec.spine.as_ref().expect("INPUT: Replace Spine did not set a spine").bounds().unwrap();
        assert!(!spec.no_spine && bounds.height() > 40.0 && bounds.width() > 250.0, "INPUT: a real curved spine is required");
    }
}

fn fixture() -> (Session, NodeId, [NodeId; 2]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": WIDTH, "height": HEIGHT})).unwrap();
    let mut rectangle = |x: f64, color: &str| {
        let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 120.0, "width": 64.0, "height": 48.0})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setFill", &json!({"color": color})).unwrap();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        id
    };
    let keys = [rectangle(90.0, "#d52c38"), rectangle(430.0, "#25b453")];
    let blend = NodeId(
        s.execute("object.blend.make", &json!({"ids": [keys[0].0, keys[1].0], "steps": 3, "orientation": "path"})).unwrap()["id"].as_u64().unwrap(),
    );
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
    live(&s, blend, keys, false);
    (s, blend, keys)
}

fn cpu() -> Renderer {
    let mut renderer = Renderer::new();
    renderer.threads = 0;
    renderer
}

fn same_pixels(actual: &Rendered, expected: &Rendered) {
    assert_eq!((actual.width, actual.height), (WIDTH, HEIGHT));
    assert_eq!((expected.width, expected.height), (WIDTH, HEIGHT));
    assert_eq!(actual.pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    assert!(actual.pixels == expected.pixels, "complete RGBA frames differ");
}

fn damage_pixels(stage: &str, changed: Changed, before: &[u8], current: &[u8]) {
    let region = match changed {
        Changed::Everything => return,
        Changed::Nothing => None,
        Changed::Area(area) => view_pixels(area, Affine::IDENTITY, WIDTH, HEIGHT),
    };
    assert_eq!(before.len(), current.len());
    for (i, (old, new)) in before.as_chunks::<4>().0.iter().zip(current.as_chunks::<4>().0.iter()).enumerate() {
        let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
        if !region.is_some_and(|[x0, y0, x1, y1]| x >= x0 && x < x1 && y >= y0 && y < y1) {
            assert_eq!(old, new, "{stage}: pixel ({x}, {y}) changed outside {changed:?}, view pixels {region:?}");
        }
    }
}

fn frame(s: &Session, retained: &mut Retained, stage: &str) -> Rendered {
    let doc = &s.doc().unwrap().doc;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), artboards: false, ..Default::default() };
    // Ask for the edit's advertised damage before rendering the new full frame on this renderer.
    let changed = retained.1.as_ref().map(|(old, _)| retained.0.changed(old.as_ref(), doc.as_ref(), &opts));
    let actual = retained.0.render(doc.as_ref(), WIDTH, HEIGHT, Affine::IDENTITY, &opts);
    let expected = cpu().render(doc.as_ref(), WIDTH, HEIGHT, Affine::IDENTITY, &opts);
    same_pixels(&actual, &expected);
    let red = expected.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] > 150 && p[1] < 100 && p[2] < 120 && p[3] == 255).count();
    let green = expected.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] < 100 && p[1] > 120 && p[2] < 150 && p[3] == 255).count();
    assert!(red > 100 && green > 100, "INPUT: both original colored keys must really draw: red={red}, green={green}");
    if let (Some((_, previous)), Some(changed)) = (&retained.1, changed) {
        damage_pixels(stage, changed, previous, &expected.pixels);
    }
    retained.1 = Some((doc.clone(), expected.pixels));
    actual
}

fn move_and_undo(s: &mut Session, retained: &mut Retained, blend: NodeId, keys: [NodeId; 2], curved: bool, before_frame: &Rendered) {
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
    let before = serde_json::to_value(s.doc().unwrap().doc.as_ref()).unwrap();
    s.execute("object.transform", &json!({"matrix": [1.0, 0.0, 0.0, 1.0, 37.0, 29.0]})).unwrap();
    live(s, blend, keys, curved);
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
    let moved = frame(s, retained, "moved");
    assert!(moved.pixels != before_frame.pixels, "INPUT: the fresh rendered fixture must actually move");
    s.execute("edit.undo", &json!({})).unwrap();
    live(s, blend, keys, curved);
    assert_eq!(serde_json::to_value(s.doc().unwrap().doc.as_ref()).unwrap(), before);
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
    let restored = frame(s, retained, "undo-move");
    same_pixels(&restored, before_frame);
}

#[test]
fn blend_5179_straight_move_undo_matches_fresh_cpu() {
    let (mut s, blend, keys) = fixture();
    let mut retained = (cpu(), None);
    let warm = frame(&s, &mut retained, "warm");
    move_and_undo(&mut s, &mut retained, blend, keys, false, &warm);
}

#[test]
fn blend_5179_replace_spine_move_undo_matches_fresh_cpu() {
    let (mut s, blend, keys) = fixture();
    let path = NodeId(s.execute("path.create", &json!({"d": "M122 144 C166 330 402 288 462 144"})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [blend.0, path.0]})).unwrap();
    let before = serde_json::to_value(s.doc().unwrap().doc.as_ref()).unwrap();
    let nodes = s.doc().unwrap().doc.node_count();
    let mut retained = (cpu(), None);
    let warm = frame(&s, &mut retained, "before-replace");
    s.execute("object.blend.replaceSpine", &json!({})).unwrap();
    live(&s, blend, keys, true);
    assert!(s.doc().unwrap().doc.node(path).is_none(), "Replace Spine consumes the original standalone path");
    assert_eq!(s.doc().unwrap().doc.node_count(), nodes - 1);
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
    let replaced = frame(&s, &mut retained, "replaced");
    assert!(replaced.pixels != warm.pixels, "INPUT: replacing the spine must visibly change the drawn fixture");
    move_and_undo(&mut s, &mut retained, blend, keys, true, &replaced);
    s.execute("edit.undo", &json!({})).unwrap();
    live(&s, blend, keys, false);
    assert_eq!(serde_json::to_value(s.doc().unwrap().doc.as_ref()).unwrap(), before);
    let restored = frame(&s, &mut retained, "undo-replace");
    same_pixels(&restored, &warm);
}
