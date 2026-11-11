//! `.cdr` drawings: Open, Place and New from Template, by extension and by content, mapped to
//! artboards, layers, guides, live shapes, solid paint and strokes.

use super::*;
use serde_json::json;
use vectorcraft_cdr::synth::{self, V, chunk, list};
use vectorcraft_doc::{LiveShape, Node, NodeKind};

fn walk(nodes: &[std::sync::Arc<Node>], out: &mut Vec<Node>) {
    for n in nodes {
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => walk(children, out),
            NodeKind::RulerGuide { .. } => {}
            _ => out.push((**n).clone()),
        }
    }
}

fn paths(doc: &vectorcraft_doc::Document) -> Vec<Node> {
    let mut out = Vec::new();
    walk(&doc.layers, &mut out);
    out
}

#[test]
fn every_version_opens_as_editable_art() {
    // Versions 14 on are ZIP packages.
    for version in [500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400, 1500, 1600, 2000, 2700] {
        let mut s = Session::new();
        let r = open_bytes(&mut s, "drawing.cdr", &synth::sample_file(version, version >= 700), None).unwrap();
        assert_eq!(r["format"], "cdr", "{version}");
        let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
        assert!(w.iter().any(|w| w.contains("text, bitmaps")), "{version}: {w:?}");
        let doc = &s.doc().unwrap().doc;
        assert_eq!(doc.artboards.len(), 1);
        assert_eq!((doc.artboards[0].rect.width().round(), doc.artboards[0].rect.height().round()), (612.0, 792.0));
        // The page's layer "Art", above the master page's desktop layer.
        let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
        assert_eq!(names.last().map(String::as_str), Some("Art"), "{version}: {names:?}");
        let p = paths(doc);
        // Five shapes on the page and one on the desktop.
        assert_eq!(p.len(), 6, "{version}");
        let rect = p.iter().find(
            |n| matches!(&n.kind, NodeKind::Path { live: Some(l), .. } if matches!(**l, LiveShape::Rectangle { w, .. } if (w - 100.0).abs() < 0.1)),
        );
        let rect = rect.unwrap_or_else(|| panic!("{version}: no live rectangle"));
        let NodeKind::Path { live: Some(live), .. } = &rect.kind else { unreachable_in_test() };
        let LiveShape::Rectangle { radii, .. } = **live else { unreachable_in_test() };
        assert!(radii.iter().all(|r| (r - 5.0).abs() < 0.1), "{version}: {radii:?}");
        assert!(p.iter().any(|n| matches!(&n.kind, NodeKind::Path { live: Some(l), .. } if matches!(**l, LiveShape::Ellipse { .. }))));
        // The rectangle is filled red and stroked 2 pt with round caps.
        let a = &rect.appearance;
        assert_eq!(a.items.len(), 2, "{version}");
        // The master page's vertical guideline at 36 pt right of the page centre, a ruler guide
        // on the top layer.
        if version >= 600 {
            let guides = doc.ruler_guides();
            let top = doc.layers.last().map(|l| l.id);
            assert!(
                guides.iter().any(|g| g.vertical && (g.pos - (306.0 + 36.0)).abs() < 0.05 && g.artboard == Some(1) && g.layer == top),
                "{version}: {guides:?}"
            );
        }
    }
}

fn unreachable_in_test() -> ! {
    panic!("the match above already checked the shape")
}

#[test]
fn objects_keep_the_file_order_front_to_back() {
    let v = V(1100);
    let rect =
        |w: f64| synth::object(v, 1, &[(30, [v.coord(w), v.coord(10.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    // The first object listed is the front-most.
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![rect(30.0), rect(20.0)])]);
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), page]);
    let (doc, _) = super::cdr::import(&file).unwrap();
    let p = paths(&doc);
    let widths: Vec<f64> = p
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Path { path, .. } => path.bounds(),
            _ => None,
        })
        .map(|b| b.width().round())
        .collect();
    assert_eq!(widths, vec![20.0, 30.0], "bottom first");
}

#[test]
fn spot_colors_become_spot_swatches_and_cmyk_drawings_open_in_cmyk() {
    let v = V(1100);
    let spot = synth::color(25, [7, 0, 40, 0]);
    let cmyk = synth::color(2, [0, 100, 0, 0]);
    let doc = list(b"doc ", vec![list(b"filt", vec![chunk(b"fild", v.fill(1, &spot)), chunk(b"fild", v.fill(2, &cmyk))])]);
    let obj = |fill: u32| {
        synth::object(
            v,
            1,
            &[
                (20, fill.to_le_bytes().to_vec()),
                (30, [v.coord(10.0), v.coord(10.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat()),
            ],
            None,
        )
    };
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![obj(1), obj(2), obj(2)])]);
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), doc, page]);
    let (doc, warnings) = super::cdr::import(&file).unwrap();
    let s = doc.swatches.iter().find(|s| s.name == "Spot 0:7").unwrap();
    assert!(s.spot && s.global);
    assert!(warnings.iter().any(|w| w.contains("library colors")), "{warnings:?}");
    let filled_with_spot = paths(&doc).iter().any(|n| {
        n.appearance.items.iter().any(|i| matches!(i, vectorcraft_doc::appearance::AppearanceItem::Fill(f) if matches!(&f.paint, vectorcraft_color::Paint::Solid { swatch: Some(name), tint, .. } if name == "Spot 0:7" && (*tint - 0.4).abs() < 1e-6)))
    });
    assert!(filled_with_spot);
    assert_eq!(doc.color_mode, vectorcraft_doc::ColorMode::Cmyk);
}

#[test]
fn detection_by_content_place_and_template() {
    let bytes = synth::sample(1100, true);
    let mut s = Session::new();
    // A renamed drawing is found by its content.
    assert_eq!(open_bytes(&mut s, "renamed.bin", &bytes, None).unwrap()["format"], "cdr");
    assert_eq!(detect("drawing.cdt", &bytes).map(|f| f.id), Some("cdt"));
    assert_eq!(detect("drawing.png", &bytes).map(|f| f.id), Some("cdr"));
    // So is a package of version 14 or later, and other ZIP archives are not taken for one.
    for version in [1400, 2700] {
        let package = synth::sample_file(version, true);
        assert_eq!(open_bytes(&mut s, "renamed.zip", &package, None).unwrap()["format"], "cdr", "{version}");
        assert_eq!(detect("drawing.cdt", &package).map(|f| f.id), Some("cdt"));
    }
    let other = synth::zip(&[synth::Member { name: "content/other.dat".into(), data: vec![1, 2, 3], deflate: false }], b"");
    assert_eq!(detect("archive.zip", &other).map(|f| f.id), None);
    let r = open_bytes(&mut s, "letter.cdt", &bytes, Some("/t/letter.cdt".into())).unwrap();
    assert_eq!(r["format"], "cdt");
    assert!(s.doc().unwrap().path.is_none(), "a template opens untitled");
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    s.execute("file.place", &json!({"name":"art.cdr", "dataBase64":vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert!(s.doc().unwrap().doc.layers.iter().any(|l| !l.children().unwrap().is_empty()));
    // Packages place too.
    for version in [1400, 2700] {
        s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
        let package = synth::sample_file(version, true);
        s.execute("file.place", &json!({"name":"art.cdr", "dataBase64":vectorcraft_format::base64_encode(&package)})).unwrap();
        assert!(s.doc().unwrap().doc.layers.iter().any(|l| !l.children().unwrap().is_empty()), "{version}");
    }
    for ext in ["cdr", "cdt"] {
        let f = format(ext).unwrap();
        assert!(f.read && !f.write);
        assert!(OPEN_EXTS.contains(&ext) && PLACE_EXTS.contains(&ext) && UNASSOCIATED_EXTS.contains(&ext), "{ext}");
        assert!(!SAVE_FORMATS.contains(&ext));
    }
}

#[test]
fn layers_of_one_name_on_a_page_stay_apart_and_pages_share_them() {
    let v = V(1100);
    let rect = || synth::object(v, 1, &[(30, [v.coord(5.0), v.coord(5.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let layer = |name: &str| synth::layer(v, [0, 0, 0, 0x98], name, vec![rect()]);
    // Front to back: A, B, A on each of two pages.
    let page = || synth::page(false, vec![layer("A"), layer("B"), layer("A")]);
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), page(), page()]);
    let (doc, _) = super::cdr::import(&file).unwrap();
    let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["A", "B", "A"]);
    assert!(doc.layers.iter().all(|l| l.children().unwrap().len() == 2), "one object from each page");
}

/// Pages that stack layers of the same names in another order keep their own order: the later
/// page gets a new layer rather than art under a layer it stacks above. A layer a later page adds
/// below the shared ones goes below them.
#[test]
fn each_page_keeps_its_own_layer_order() {
    let v = V(1100);
    let rect = || synth::object(v, 1, &[(30, [v.coord(5.0), v.coord(5.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let page = |names: &[&str]| synth::page(false, names.iter().map(|n| synth::layer(v, [0, 0, 0, 0x98], n, vec![rect()])).collect());
    // Front to back on each page.
    for (second, want, counts) in [(&["A", "B"][..], vec!["A", "B", "A"], vec![1, 2, 1]), (&["B", "A", "C"][..], vec!["C", "A", "B"], vec![1, 2, 2])]
    {
        let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), page(&["B", "A"]), page(second)]);
        let (doc, _) = super::cdr::import(&file).unwrap();
        let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
        assert_eq!(names, want, "bottom to top, second page {second:?}");
        let got: Vec<usize> = doc.layers.iter().map(|l| l.children().unwrap().len()).collect();
        assert_eq!(got, counts, "second page {second:?}");
    }
}

/// Pages that alternate between two orders of the same layers reuse the layers made for each
/// order instead of adding layers page after page.
#[test]
fn pages_in_alternating_layer_orders_reuse_their_layers() {
    let v = V(1100);
    let rect = || synth::object(v, 1, &[(30, [v.coord(5.0), v.coord(5.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let page = |names: &[&str]| synth::page(false, names.iter().map(|n| synth::layer(v, [0, 0, 0, 0x98], n, vec![rect()])).collect());
    // Front to back on each page.
    let mut kids = vec![chunk(b"vrsn", 1100u16.to_le_bytes().to_vec())];
    kids.extend((0..6).map(|i| if i % 2 == 0 { page(&["B", "A"]) } else { page(&["A", "B"]) }));
    let (doc, _) = super::cdr::import(&synth::riff(1100, &kids)).unwrap();
    let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["A", "B", "A"], "bottom to top");
    let got: Vec<usize> = doc.layers.iter().map(|l| l.children().unwrap().len()).collect();
    assert_eq!(got, [3, 6, 3]);
}

#[test]
fn pages_and_the_guides_copied_onto_them_are_bounded() {
    let v = V(1100);
    let guide = |x: f64| {
        let mut g = vec![0; 12];
        g.extend(90_000_000i32.to_le_bytes());
        g.extend([v.coord(x), v.coord(0.0), v.coord(x), v.coord(10.0), v.coord(x), v.coord(0.0)].concat());
        synth::object(v, 0x16, &[(16000, g)], None)
    };
    let guides: Vec<_> = (0..200).map(|i| guide(f64::from(i))).collect();
    let mut kids = vec![chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), synth::page(true, vec![synth::layer(v, [0x0a, 0, 1, 0x98], "G", guides)])];
    kids.extend((0..1100).map(|_| synth::page(false, vec![])));
    let (doc, warnings) = super::cdr::import(&synth::riff(1100, &kids)).unwrap();
    assert_eq!(doc.artboards.len(), 1000);
    let guides = doc.ruler_guides().len();
    assert!(guides <= 100_000, "{guides}");
    assert!(warnings.iter().any(|w| w.contains("past the import limits")), "{warnings:?}");
}

/// Layers are held to 10,000, the master page's and the pages' together; at the limit, guide
/// paths of angled guidelines go on the top layer.
#[test]
fn layers_are_bounded() {
    let v = V(1100);
    let rect = || synth::object(v, 1, &[(30, [v.coord(5.0), v.coord(5.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let mut angled = vec![0; 12];
    angled.extend(45_000_000i32.to_le_bytes());
    angled.extend([v.coord(0.0), v.coord(0.0)].concat());
    let angled = synth::object(v, 0x16, &[(16000, angled)], None);
    let page_layers =
        (0..10_050).map(|i| synth::layer(v, [0, 0, 0, 0x98], &format!("L{i}"), if i == 0 { vec![angled.clone()] } else { vec![] })).collect();
    let master_layers = (0..10_050).map(|i| synth::layer(v, [0, 0, 1, 0x98], &format!("M{i}"), vec![rect()])).collect();
    for (page, guide_paths) in [(synth::page(false, page_layers), 1), (synth::page(true, master_layers), 0)] {
        let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), page, synth::page(false, vec![])]);
        let (doc, warnings) = super::cdr::import(&file).unwrap();
        assert_eq!(doc.layers.len(), 10_000);
        assert!(warnings.iter().any(|w| w.contains("past the import limits")), "{warnings:?}");
        let guides = paths(&doc).iter().filter(|n| matches!(n.kind, NodeKind::Path { guide: true, .. })).count();
        assert_eq!(guides, guide_paths);
    }
}

/// A page longer than the longest artboard opens at that length, and the warnings say so.
#[test]
fn pages_past_the_longest_artboard_are_cut_with_a_warning() {
    let v = V(1100);
    for (w, cut) in [(200_000.0, true), (100_000.0, false)] {
        let doc = list(b"doc ", vec![synth::mcfg(v, w, 792.0)]);
        let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), doc, synth::page(false, vec![])]);
        let (doc, warnings) = super::cdr::import(&file).unwrap();
        assert_eq!(doc.artboards[0].rect.width().round(), if cut { 163_830.0 } else { w }, "{w}");
        assert_eq!(warnings.iter().any(|x| x.contains("longer than 163,830 pt")), cut, "{w}: {warnings:?}");
    }
}

#[test]
fn fills_and_outlines_keep_their_colors_widths_caps_joins_and_dashes() {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::appearance::{AppearanceItem, LineCap, LineJoin};
    let v = V(1300);
    let red = synth::color(5, [0, 0, 255, 0]);
    let black = synth::color(2, [0, 0, 0, 100]);
    let pen = synth::Pen { line_type: 0, width: 2.0, cap: 1, join: 2 };
    let args = vec![
        (20, 1u32.to_le_bytes().to_vec()),
        (10, 2u32.to_le_bytes().to_vec()),
        (30, v.points(&[(0.0, 0.0, 0x08), (10.0, 0.0, 0x40), (10.0, 10.0, 0x48)])),
    ];
    let styles = list(
        b"doc ",
        vec![list(b"filt", vec![chunk(b"fild", v.fill(1, &red))]), list(b"otlt", vec![chunk(b"outl", v.outline(2, pen, &black, &[3, 1]))])],
    );
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 3, &args, None)])]);
    let (doc, _) = super::cdr::import(&synth::riff(1300, &[chunk(b"vrsn", 1300u16.to_le_bytes().to_vec()), styles, page])).unwrap();
    let p = paths(&doc);
    let [AppearanceItem::Fill(fill), AppearanceItem::Stroke(stroke)] = p[0].appearance.items.as_slice() else {
        panic!("{:?}", p[0].appearance.items)
    };
    assert_eq!(fill.paint, Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(stroke.paint, Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    assert!((stroke.width - 2.0).abs() < 0.01, "{}", stroke.width);
    assert_eq!((stroke.cap, stroke.join), (LineCap::Round, LineJoin::Bevel));
    // Dashes are stored in multiples of the width.
    let dash = stroke.dash.as_ref().unwrap();
    assert!(dash.pattern.len() == 2 && (dash.pattern[0] - 6.0).abs() < 0.01 && (dash.pattern[1] - 2.0).abs() < 0.01, "{:?}", dash.pattern);
}

#[test]
fn other_versions_and_damaged_drawings_say_why() {
    let mut s = Session::new();
    let e = open_bytes(&mut s, "old.cdr", &synth::riff(400, &[]), None).unwrap_err().to_string();
    assert!(e.contains("CDR 4 drawings can't be opened"), "{e}");
    let e = open_bytes(&mut s, "new.cdr", &synth::riff(2800, &[]), None).unwrap_err().to_string();
    assert!(e.contains("CDR 28 drawings can't be opened: VectorCraft reads versions 5 to 27"), "{e}");
    // A package cut short, and one whose drawing is encrypted (flag bit 0 of its directory entry).
    let package = synth::sample_file(1400, false);
    let e = open_bytes(&mut s, "cut.cdr", &package[..package.len() / 2], None).unwrap_err().to_string();
    assert!(e.contains("can't be read (damaged package directory)"), "{e}");
    let mut encrypted = package.clone();
    let dir = package.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    encrypted[dir + 8] |= 1;
    let e = open_bytes(&mut s, "locked.cdr", &encrypted, None).unwrap_err().to_string();
    assert!(e.contains("can't be read (encrypted packages are not read)"), "{e}");
    let mut big = synth::riff(1100, &[]);
    big[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    // A declared size past the end keeps what the file holds.
    assert!(open_bytes(&mut s, "cut.cdr", &big, None).is_ok());
}
