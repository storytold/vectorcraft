//! A warp grid kept before type's measure is registered is built again once it is, even when the
//! registered inputs number is 0. One test in its own process, so nothing has registered the
//! measure before it runs.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_doc::live::{EnvelopeOptions, set_type_ink, warp_grid};
use vectorcraft_doc::{CharStyle, EnvelopeKind, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Point, Rect};

/// A measure that gives every type the same box, well away from its laid-out box.
fn fixed_ink(_: &TextObject, _: Affine, _: Option<(vectorcraft_geom::Homography, Affine)>) -> Option<Rect> {
    Some(Rect::new(0.0, 0.0, 400.0, 300.0))
}

fn no_inputs() -> u64 {
    0
}

/// An arc warp around `t`, its content in a new `Arc`.
fn warp_of(t: &TextObject) -> Node {
    let content = Arc::new(Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone()))));
    let kind = EnvelopeKind::Warp { style: "arc".into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true, version: 1 };
    Node::new(
        NodeId(2),
        NodeKind::Envelope {
            content: vec![content],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: EnvelopeOptions::NEW,
            frame: Box::new(Affine::IDENTITY),
        },
    )
}

#[test]
fn a_warp_grid_kept_before_the_type_measure_is_built_again() {
    let t = TextObject::point(Point::new(100.0, 100.0), "Type", CharStyle::default());
    let n = warp_of(&t);
    let before = warp_grid(&n);
    set_type_ink(fixed_ink, no_inputs);
    let fresh = warp_grid(&warp_of(&t));
    assert!(fresh.is_some());
    assert_ne!(fresh, before, "the registered measure moves the grid");
    assert_eq!(warp_grid(&n), fresh, "the grid kept from before the registration is built again");
}
