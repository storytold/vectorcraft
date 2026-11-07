//! `vector-effect="non-scaling-stroke"` on import: the stroke keeps its screen width and dashes
//! whatever the transforms above it, on shapes and type; a warning where it can't be kept.

use vectorcraft_doc::{Document, Node, NodeKind, TextObject};
use vectorcraft_svg::import_with_report;

/// The stroke width and dash pattern of the object named `name`.
fn stroke(d: &Document, name: &str) -> (f64, Option<Vec<f64>>) {
    let mut found = None;
    d.walk(|n: &Node| {
        if n.name.as_deref() == Some(name) {
            let s = n.appearance.stroke().expect("stroked");
            found = Some((s.width, s.dash.as_ref().map(|d| d.pattern.clone())));
        }
    });
    found.unwrap_or_else(|| panic!("no object '{name}'"))
}

fn text(d: &Document) -> TextObject {
    let mut found = None;
    d.walk(|n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            found = Some((**t).clone());
        }
    });
    found.expect("a text")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_non_scaling_stroke_keeps_its_width_under_a_scale() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 200 200">
      <g transform="scale(3)">
        <path id="red" d="M10 15 H55" stroke="#e33" stroke-width="4"/>
        <path id="green" d="M10 35 H55" stroke="#3a3" stroke-width="4" vector-effect="non-scaling-stroke"/>
      </g>
    </svg>"##;
    let (d, warnings) = import_with_report(svg).unwrap();
    assert!(close(stroke(&d, "red").0, 12.0));
    assert!(close(stroke(&d, "green").0, 4.0), "{:?}", stroke(&d, "green"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn widths_and_dashes_are_in_screen_pixels_past_the_view_box() {
    // The viewBox makes a user unit 2 px; a rule asks for the non-scaling stroke.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 100 50">
      <style>.thin { vector-effect: non-scaling-stroke }</style>
      <rect id="box" class="thin" x="10" y="10" width="50" height="20" fill="none" stroke="#000" stroke-width="2" stroke-dasharray="6 3"/>
      <circle id="ring" cx="80" cy="25" r="10" fill="none" stroke="#000" stroke-width="2"/>
    </svg>"##;
    let (d, _) = import_with_report(svg).unwrap();
    let (w, dash) = stroke(&d, "box");
    assert!(close(w, 2.0) && dash == Some(vec![6.0, 3.0]), "{w} {dash:?}");
    assert!(close(stroke(&d, "ring").0, 4.0));

    // In a root sized in millimetres a CSS pixel is 0.75 pt.
    let mm = svg.replace(r#"width="200" height="100""#, r#"width="200mm" height="100mm""#);
    let (d, _) = import_with_report(&mm).unwrap();
    assert!(close(stroke(&d, "box").0, 1.5), "a 2 px stroke is 1.5 pt: {:?}", stroke(&d, "box"));
}

#[test]
fn a_reused_non_scaling_stroke_warns() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200">
      <defs><path id="tick" d="M0 0 H20" stroke="#000" stroke-width="2" vector-effect="non-scaling-stroke"/></defs>
      <use xlink:href="#tick" transform="scale(4)"/>
    </svg>"##;
    let (_, warnings) = import_with_report(svg).unwrap();
    assert!(warnings.iter().any(|w| w.contains("non-scaling stroke on 'tick'")), "{warnings:?}");
}

#[test]
fn type_keeps_its_non_scaling_stroke_width() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">
      <text x="10" y="40" transform="scale(2)" font-size="20" stroke="#000" stroke-width="3" stroke-dasharray="4 2"
            vector-effect="non-scaling-stroke">Ab</text>
    </svg>"##;
    let (d, warnings) = import_with_report(svg).unwrap();
    let t = text(&d);
    let st = &t.runs[0].style;
    // The scale became the type's size: the stroke and its dashes keep their width on screen.
    assert!(close(st.size, 40.0) && close(t.xf.determinant(), 1.0), "{} {:?}", st.size, t.xf);
    assert!(close(st.stroke_width * t.xf.determinant().sqrt(), 3.0), "{} {:?}", st.stroke_width, t.xf);
    assert_eq!(st.stroke_dash.as_ref().map(|d| d.pattern.clone()), Some(vec![4.0, 2.0]));
    assert!(warnings.is_empty(), "{warnings:?}");

    let (_, warnings) = import_with_report(&svg.replace("scale(2)", "scale(2 1)")).unwrap();
    assert!(warnings.iter().any(|w| w.contains("non-scaling stroke on a text approximated")), "{warnings:?}");
}
