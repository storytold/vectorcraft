//! Gradient mesh points picked with one of the tools that pick them (the Mesh tool, Direct
//! Selection, the Lasso, the Polygonal Lasso) stay picked when another of them is chosen, or lent
//! for a Cmd-click, so a fill color then recolors just them.

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_doc::NodeKind;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

/// A session with a selected 1 x 1 gradient mesh (four white points) at (10, 10), 50 pt square.
fn mesh_session() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let m = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.mesh.create", &json!({"ids": [m.0], "rows": 1, "cols": 1})).unwrap();
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    (s, m)
}

fn colors(s: &Session, m: NodeId) -> Vec<Color> {
    match &s.doc().unwrap().doc.node(m).unwrap().kind {
        NodeKind::Mesh(g) => g.points.iter().map(|p| p.color).collect(),
        k => panic!("not a mesh: {k:?}"),
    }
}

/// Click mesh point `i` of `m` with the active tool.
fn press(s: &mut Session, m: NodeId, i: usize, mods: Mods) {
    let q = match &s.doc().unwrap().doc.node(m).unwrap().kind {
        NodeKind::Mesh(g) => g.points[i].p,
        k => panic!("not a mesh: {k:?}"),
    };
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, q.x, q.y).with_mods(mods), ViewInfo::default()).unwrap();
    }
}

/// A point picked with the Mesh tool stays picked through Direct Selection, the Lasso and the
/// Polygonal Lasso and back to the Mesh tool: each fill color recolors just that point.
#[test]
fn mesh_point_picks_carry_between_the_mesh_point_tools() {
    let (mut s, m) = mesh_session();
    let v = ViewInfo::default();
    let white = colors(&s, m)[0];
    s.select_tool("mesh", v).unwrap();
    press(&mut s, m, 0, Mods::default());
    for (tool, hex, c) in [
        ("directSelection", "#0000ff", Color::rgb8(0, 0, 255)),
        ("lasso", "#00ff00", Color::rgb8(0, 255, 0)),
        ("polygonalLasso", "#ff0000", Color::rgb8(255, 0, 0)),
        ("directSelection", "#ffff00", Color::rgb8(255, 255, 0)),
        ("mesh", "#00ffff", Color::rgb8(0, 255, 255)),
    ] {
        s.select_tool(tool, v).unwrap();
        s.execute("paint.setFill", &json!({"color": hex})).unwrap();
        assert_eq!(colors(&s, m), [c, white, white, white], "{tool} recolors only the picked point");
    }
}

/// A point picked with Direct Selection stays picked for the Mesh tool.
#[test]
fn direct_selection_picks_carry_to_the_mesh_tool() {
    let (mut s, m) = mesh_session();
    let v = ViewInfo::default();
    let white = colors(&s, m)[0];
    s.select_tool("directSelection", v).unwrap();
    press(&mut s, m, 2, Mods::default());
    s.select_tool("mesh", v).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colors(&s, m), [white, white, Color::rgb8(0, 0, 255), white]);
}

/// The Selection tool picks no mesh points: after it, the Mesh tool starts with none picked, and a
/// fill color leaves every point as it was.
#[test]
fn a_tool_that_picks_no_mesh_points_drops_the_picks() {
    let (mut s, m) = mesh_session();
    let v = ViewInfo::default();
    let before = colors(&s, m);
    s.select_tool("mesh", v).unwrap();
    press(&mut s, m, 0, Mods::default());
    s.select_tool("selection", v).unwrap();
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    s.select_tool("mesh", v).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colors(&s, m), before, "no point is picked");
}

/// A plain Lasso click away from the art deselects it and unpicks the points the Mesh tool picked,
/// so the Mesh tool chosen next has none picked.
#[test]
fn a_lasso_click_that_deselects_unpicks_the_mesh_points() {
    let (mut s, m) = mesh_session();
    let v = ViewInfo::default();
    let before = colors(&s, m);
    s.select_tool("mesh", v).unwrap();
    press(&mut s, m, 0, Mods::default());
    s.select_tool("lasso", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 500.0, 500.0), v).unwrap();
    }
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the click deselects the mesh");
    s.execute("select.set", &json!({"ids": [m.0]})).unwrap();
    s.select_tool("mesh", v).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colors(&s, m), before, "no point is picked");
}

/// Direct Selection lent to the Mesh tool for a Cmd-click starts from the Mesh tool's picks, so a
/// Shift-click adds to them, and gives back what it left picked.
#[test]
fn the_lent_direct_selection_carries_the_mesh_tools_picks() {
    let (mut s, m) = mesh_session();
    let v = ViewInfo::default();
    let white = colors(&s, m)[0];
    // Cmd lends the selection tool chosen last.
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("mesh", v).unwrap();
    press(&mut s, m, 0, Mods::default());
    press(&mut s, m, 3, Mods { cmd: true, shift: true, ..Default::default() });
    assert_eq!(s.tool_id(), "mesh");
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let b = Color::rgb8(0, 0, 255);
    assert_eq!(colors(&s, m), [b, white, white, b], "both points stay picked");
}
