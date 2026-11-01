//! View Opacity Mask (M3.86): the canvas shows only the mask, as greyscale coverage, while it is
//! edited; toggling it off shows the artwork again, and leaving editing by any route ends it.

use serde_json::json;
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};

use crate::*;

fn rect(s: &mut Session, x: f64, w: f64, fill: &str) -> NodeId {
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": w, "height": 200})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": fill, "ids": [id.0]})).unwrap();
    id
}

/// The canvas pixel at (x, 100), rendered as the canvas renders the document.
fn canvas_pixel(s: &Session, x: u32) -> [u8; 4] {
    let st = s.doc().unwrap();
    let opts = RenderOptions { mask_view: st.shown_mask(), ..Default::default() };
    Renderer::new().render(&st.doc, 300, 300, Affine::IDENTITY, &opts).pixel(x, 100)
}

#[test]
fn view_opacity_mask_shows_its_luminance_and_hides_the_artwork() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let obj = rect(&mut s, 0.0, 200.0, "#ff0000");
    let other = rect(&mut s, 220.0, 60.0, "#0000ff");
    // Green mask art over the left half: luminance 0.7152.
    let m = rect(&mut s, 0.0, 100.0, "#00ff00");
    s.execute("select.set", &json!({"ids": [obj.0, m.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert_eq!(canvas_pixel(&s, 250)[..3], [0, 0, 255], "the artwork, before");
    let undo = s.doc().unwrap().history.undo.len();
    // Viewing enters mask editing (one undo step) and shows only the mask.
    let r = s.execute("transparency.viewOpacityMask", &json!({})).unwrap();
    assert_eq!(r, json!({"on": true, "id": obj.0}));
    let st = s.doc().unwrap();
    assert_eq!((st.doc.mask_edit.map(|e| e.object), st.shown_mask()), (Some(obj), Some(obj)));
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(canvas_pixel(&s, 50), [182, 182, 182, 255], "the mask art's luminance");
    assert_eq!(canvas_pixel(&s, 150), [0, 0, 0, 255], "clipping: black outside the mask art (the red object is hidden)");
    assert_eq!(canvas_pixel(&s, 250), [0, 0, 0, 255], "other art is hidden");
    // The view follows the mask's options and edits of its art live.
    s.execute("transparency.setOpacityMask", &json!({"invert": true, "clip": false})).unwrap();
    assert_eq!(canvas_pixel(&s, 50), [73, 73, 73, 255]);
    assert_eq!(canvas_pixel(&s, 250), [0, 0, 0, 255], "not clipping, inverted: white outside, inverted");
    s.execute("transparency.setOpacityMask", &json!({"invert": false, "clip": true})).unwrap();
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    assert_eq!(canvas_pixel(&s, 120), [182, 182, 182, 255]);
    // Toggling again shows the artwork, still editing the mask.
    assert_eq!(s.execute("transparency.viewOpacityMask", &json!({})).unwrap(), json!({"on": false, "id": obj.0}));
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_some() && st.shown_mask().is_none());
    assert_eq!(canvas_pixel(&s, 250)[..3], [0, 0, 255]);
    // Leaving editing ends the view; so does undoing back out of editing.
    s.execute("transparency.viewOpacityMask", &json!({"on": true})).unwrap();
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().mask_view, None);
    s.execute("transparency.viewOpacityMask", &json!({"id": obj.0})).unwrap();
    assert!(s.doc().unwrap().shown_mask().is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.mask_view.is_none());
    assert_eq!(canvas_pixel(&s, 250)[..3], [0, 0, 255]);
    // Nothing to view: an error, and the view state is untouched.
    s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
    assert!(s.execute("transparency.viewOpacityMask", &json!({})).is_err());
    assert!(s.execute("transparency.viewOpacityMask", &json!({"on": true, "id": other.0})).is_err());
    assert!(s.doc().unwrap().mask_view.is_none());
}
