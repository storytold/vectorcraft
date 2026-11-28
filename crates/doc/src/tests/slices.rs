//! Slice layout: object slices follow their objects, clipping, numbering and auto slices.

use proptest::prelude::*;
use vectorcraft_geom::{Point, Rect, shapes};

use crate::slices::auto_slices;
use crate::{Appearance, Document, Node, Slice, SliceOptions, SliceSource};

fn rect(d: &mut Document, r: Rect) -> crate::NodeId {
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::path(id, shapes::rectangle(r), Appearance::basic(crate::color::Paint::None, crate::color::Paint::None, 0.0)))
        .unwrap();
    id
}

fn user(d: &mut Document, r: Rect) -> crate::NodeId {
    let id = d.alloc_id();
    d.slices.push(Slice { id, rect: r, options: SliceOptions::default() });
    id
}

/// How many of `rects` hold point `p` (strictly inside).
fn holders(rects: &[Rect], p: Point) -> usize {
    rects.iter().filter(|r| r.x0 < p.x && p.x < r.x1 && r.y0 < p.y && p.y < r.y1).count()
}

#[test]
fn an_object_slice_follows_its_object_and_numbers_run_by_rows() {
    let mut d = Document::new(300.0, 200.0);
    let a = rect(&mut d, Rect::new(100.0, 50.0, 200.0, 150.0));
    assert!(d.slice_layout().is_empty(), "no slices, no auto slices");
    d.node_mut(a).unwrap().extras_mut().slice = Some(Box::default());
    let lay = d.slice_layout();
    let own = lay.iter().find(|s| s.id == Some(a)).unwrap();
    assert_eq!((own.source, own.rect), (SliceSource::Object, Rect::new(100.0, 50.0, 200.0, 150.0)));
    // Three rows: the band above, the object's row (left, object, right) and the band below.
    assert_eq!(lay.len(), 5);
    assert_eq!(lay.iter().map(|s| s.number).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(own.number, 3);
    assert_eq!(d.slice_name(own), "Untitled-1_03");
    // Moving the object moves its slice.
    d.node_mut(a).unwrap().transform(vectorcraft_geom::Affine::translate((-50.0, 0.0)), crate::Scaling::default());
    assert_eq!(d.slice_layout().iter().find(|s| s.id == Some(a)).unwrap().rect, Rect::new(50.0, 50.0, 150.0, 150.0));
    // Hidden objects have no slice.
    d.node_mut(a).unwrap().visible = false;
    assert!(d.slice_layout().is_empty());
}

#[test]
fn clip_to_artboard_clips_slices_and_off_covers_the_art() {
    let mut d = Document::new(100.0, 100.0);
    let s = user(&mut d, Rect::new(50.0, 50.0, 150.0, 150.0));
    let own = |d: &Document| d.slice_layout().into_iter().find(|a| a.id == Some(s)).unwrap().rect;
    assert_eq!(own(&d), Rect::new(50.0, 50.0, 100.0, 100.0));
    let lay = d.slice_layout();
    let autos: Vec<Rect> = lay.iter().filter(|a| a.source == SliceSource::Auto).map(|a| a.rect).collect();
    assert_eq!(autos.iter().map(|r| r.area()).sum::<f64>(), 100.0 * 100.0 - 50.0 * 50.0);
    d.slices_clip_to_artboard = false;
    assert_eq!(own(&d), Rect::new(50.0, 50.0, 150.0, 150.0));
    // The region is now the slice itself (no art): nothing left for auto slices.
    assert_eq!(d.slice_layout().len(), 1);
    rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0));
    let total: f64 = d.slice_layout().iter().map(|a| a.rect.area()).sum();
    assert_eq!(total, 150.0 * 150.0);
}

#[test]
fn slices_survive_a_native_round_trip_and_old_files_load() {
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 20.0, 20.0));
    d.node_mut(a).unwrap().extras_mut().slice =
        Some(Box::new(SliceOptions { kind: crate::SliceKind::NoImage, text: "<b>hi</b>".into(), ..Default::default() }));
    let s = user(&mut d, Rect::new(30.0, 30.0, 60.0, 60.0));
    d.slice_mut(s).unwrap().options.url = "https://example.com".into();
    d.slices_clip_to_artboard = false;
    let json = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back.slices, d.slices);
    assert_eq!(back.node(a).unwrap().extras().slice, d.node(a).unwrap().extras().slice);
    assert!(!back.slices_clip_to_artboard);
    // A document without slices writes none of the new keys (older readers see what they knew).
    let plain = serde_json::to_string(&Document::new(10.0, 10.0)).unwrap();
    assert!(!plain.contains("slice"), "{plain}");
    let old: Document = serde_json::from_str(&plain).unwrap();
    assert!(old.slices.is_empty() && old.slices_clip_to_artboard);
}

#[test]
fn auto_slices_fill_around_a_slice_in_rows() {
    let autos = auto_slices(Rect::new(0.0, 0.0, 90.0, 90.0), &[Rect::new(30.0, 30.0, 60.0, 60.0)]);
    let mut got: Vec<(f64, f64, f64, f64)> = autos.iter().map(|r| (r.x0, r.y0, r.x1, r.y1)).collect();
    got.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    assert_eq!(got, [(0.0, 0.0, 90.0, 30.0), (0.0, 30.0, 30.0, 60.0), (60.0, 30.0, 90.0, 60.0), (0.0, 60.0, 90.0, 90.0)]);
    // Spans that continue down one column merge.
    let autos = auto_slices(Rect::new(0.0, 0.0, 90.0, 90.0), &[Rect::new(30.0, 0.0, 60.0, 40.0), Rect::new(30.0, 40.0, 60.0, 90.0)]);
    assert_eq!(autos.len(), 2, "{autos:?}");
}

fn arb_rect() -> impl Strategy<Value = Rect> {
    (-20.0..120.0f64, -20.0..120.0f64, 1.0..80.0f64, 1.0..80.0f64).prop_map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h))
}

proptest! {
    /// Auto slices never overlap each other or the slices, and with the slices they cover the
    /// whole region.
    #[test]
    fn auto_slices_cover_without_overlap(cut in prop::collection::vec(arb_rect(), 0..8)) {
        let region = Rect::new(0.0, 0.0, 100.0, 100.0);
        let autos = auto_slices(region, &cut);
        for r in &autos {
            prop_assert!(r.x0 >= region.x0 && r.y0 >= region.y0 && r.x1 <= region.x1 && r.y1 <= region.y1, "{r:?} leaves the region");
        }
        // Sample off every edge (edges are at integers plus random fractions; the samples sit at
        // odd 1/8ths, which a random float edge practically never hits).
        for i in 0..50 {
            for j in 0..50 {
                let p = Point::new(i as f64 * 2.0 + 0.125 * 7.0, j as f64 * 2.0 + 0.125 * 3.0);
                let in_cut = holders(&cut, p) > 0;
                let in_auto = holders(&autos, p);
                prop_assert!(in_auto <= 1, "{p:?} in {in_auto} auto slices");
                prop_assert!(in_cut != (in_auto == 1), "{p:?}: in a slice {in_cut}, in an auto slice {in_auto}");
            }
        }
    }
}
