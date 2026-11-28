use std::sync::Arc;

use vectorcraft_doc::{Knockout, Node, NodeId};

use super::*;

/// The objects measured to leave out what is off the page and to write links while `d` is exported
/// (on this thread).
fn measures(d: &Document) -> usize {
    crate::export::MEASURES.with(|m| m.set(0));
    export(d, &PdfOptions { created: Some(0), ..PdfOptions::default() }).expect("export");
    crate::export::MEASURES.with(std::cell::Cell::get)
}

fn square(x: f64) -> Arc<Node> {
    Arc::new(rect_node(Rect::new(x, 10.0, x + 20.0, 30.0), Color::rgb(0.2, 0.4, 0.6)))
}

/// An object in groups within a layer is measured once in an export, as an object directly in a
/// layer is.
#[test]
fn an_object_in_nested_groups_is_measured_once() {
    let mut d = doc(200.0, 200.0);
    add(&mut d, (*square(10.0)).clone());
    assert_eq!(measures(&d), 1);

    let mut d = doc(200.0, 200.0);
    let inner = Node::group(NodeId(0), vec![square(10.0), square(40.0)]);
    add(&mut d, Node::group(NodeId(0), vec![Arc::new(inner), square(70.0)]));
    assert_eq!(measures(&d), 3);
}

/// A group in PDF layers, where a layer's box is not used, is measured once as well.
#[test]
fn an_object_in_a_pdf_layer_is_measured_once() {
    let mut d = doc(200.0, 200.0);
    add(&mut d, Node::group(NodeId(0), vec![square(10.0), square(40.0)]));
    let mut opts = PdfOptions { created: Some(0), ..PdfOptions::default() };
    opts.settings.create_layers = true;
    crate::export::MEASURES.with(|m| m.set(0));
    export(&d, &opts).expect("export");
    assert_eq!(crate::export::MEASURES.with(std::cell::Cell::get), 2);
}

/// The elements of a knockout group are measured for the group's box and to choose the elements
/// on the page, and not again when each is drawn and when its mask is made.
#[test]
fn an_element_of_a_knockout_group_is_measured_twice() {
    let mut d = doc(200.0, 200.0);
    let mut group = Node::group(NodeId(0), vec![square(10.0), square(20.0)]);
    group.knockout = Knockout::On;
    add(&mut d, group);
    assert_eq!(measures(&d), 4);
}

/// The links of a tree with no URLs and no clipping group are found without measuring: the export
/// measures each object once, as the walk that leaves out what is off the page does.
#[test]
fn objects_without_a_url_are_not_measured_for_links() {
    let mut d = doc(200.0, 200.0);
    let inner = Node::group(NodeId(0), vec![square(10.0), square(40.0)]);
    add(&mut d, Node::group(NodeId(0), vec![Arc::new(inner)]));
    assert_eq!(measures(&d), 2);
}

/// Only an object with a URL that opens is measured for its link, and not the groups around it.
#[test]
fn only_an_object_with_a_url_is_measured_for_its_link() {
    let mut linked = (*square(10.0)).clone();
    linked.edit_attrs(|a| a.url = "https://example.com".into());
    let mut refused = (*square(40.0)).clone();
    refused.edit_attrs(|a| a.url = "javascript:alert(1)".into());
    let mut d = doc(200.0, 200.0);
    add(&mut d, Node::group(NodeId(0), vec![Arc::new(linked), Arc::new(refused)]));
    assert_eq!(measures(&d), 3);
}

/// A clipping group is measured for the links inside it, and what it holds is not: the group for
/// the page and for the links, then the clipping path's fill and the member when they are written.
#[test]
fn a_clipping_group_is_measured_for_the_links_inside_it() {
    let mut clip = Node::group(NodeId(0), vec![square(10.0), square(15.0)]);
    clip.set_clips(true);
    let mut d = doc(200.0, 200.0);
    add(&mut d, clip);
    assert_eq!(measures(&d), 4);
}

/// A Shaper group's first member is left out of the group's box, and the boxes found by measuring
/// the group belong to the members after it. An off-page first member is not written, and the
/// member on the page is.
#[test]
fn a_member_left_out_of_the_group_box_measures_itself() {
    let on_page = square(10.0);
    let off_page = square(5000.0);

    let mut shaped = doc(200.0, 200.0);
    let mut group = Node::group(NodeId(0), vec![off_page, on_page.clone()]);
    group.extras_mut().shaper = Some(Box::default());
    add(&mut shaped, group);

    let mut plain = doc(200.0, 200.0);
    add(&mut plain, Node::group(NodeId(0), vec![on_page]));

    assert_eq!(uncompressed(&shaped), uncompressed(&plain));
}

/// A group wholly off the page is not written, with the see-through empty group inside it.
#[test]
fn a_group_off_the_page_writes_nothing() {
    let mut empty = Node::group(NodeId(0), vec![]);
    empty.opacity = 0.5;
    let mut off = doc(200.0, 200.0);
    add(&mut off, (*square(10.0)).clone());
    add(&mut off, Node::group(NodeId(0), vec![square(5000.0), Arc::new(empty)]));

    let mut none = doc(200.0, 200.0);
    add(&mut none, (*square(10.0)).clone());

    assert_eq!(uncompressed(&off), uncompressed(&none));
}
