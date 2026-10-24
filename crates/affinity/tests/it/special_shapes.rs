//! Affinity's heart, cog and double arrow (#3865): drawn as their outlines instead of their bounding
//! ellipse. The outlines were fitted to Affinity's own renders (the thumbnails) of the public
//! corpus files `patchy-shapes-2.af` and `afdesignload-shape_test.afdesign`; the synthetic shapes
//! here only model the parameters those files carry.

use vectorcraft_affinity::{
    Document, Limits,
    model::{Kind, Path, Point},
    synth::{self, F, Method, tag},
};

/// A document holding one shape of `class` with `parameters`, in the box (0, 0)–(100, 200).
fn read(class: &[u8; 4], parameters: Vec<(vectorcraft_affinity::stream::Tag, F)>) -> Document {
    let shape = F::Def(
        100,
        vec![tag(b"ShpN")],
        vec![(tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 100.0, 200.0])), (tag(b"Shpe"), F::Def(101, vec![tag(class)], parameters))],
    );
    let data = synth::stream(&[
        (tag(b"Defs"), F::Shared(vec![shape])),
        (
            tag(b"DocR"),
            F::Def(
                1,
                vec![tag(b"DocN")],
                vec![(
                    tag(b"Chld"),
                    F::Shared(vec![F::Def(
                        2,
                        vec![tag(b"Sprd")],
                        vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 400.0, 300.0])), (tag(b"Chld"), F::Shared(vec![F::Ref(100)]))],
                    )]),
                )],
            ),
        ),
    ]);
    let bytes = synth::container(&[("doc.dat", &data, Method::Zlib)], None);
    vectorcraft_affinity::read(&bytes, Limits::default()).unwrap()
}

fn path(document: &Document) -> &Path {
    match &document.spreads[0].nodes[0].kind {
        Kind::Shape { path, .. } => path,
        other => panic!("expected shape, got {other:?}"),
    }
}

/// Every point of a subpath: its start, then each segment's controls and end.
fn points(sp: &vectorcraft_affinity::model::SubPath) -> Vec<Point> {
    std::iter::once(sp.start).chain(sp.segments.iter().flatten().copied()).collect()
}

/// The anchors of a subpath: its start and each segment's end.
fn anchors(sp: &vectorcraft_affinity::model::SubPath) -> Vec<Point> {
    std::iter::once(sp.start).chain(sp.segments.iter().map(|s| s[2])).collect()
}

/// Twice the signed area of the polygon through a subpath's anchors: positive is clockwise on
/// screen (y down).
fn area(sp: &vectorcraft_affinity::model::SubPath) -> f64 {
    let a = anchors(sp);
    (0..a.len()).map(|i| a[i].x * a[(i + 1) % a.len()].y - a[(i + 1) % a.len()].x * a[i].y).sum()
}

fn near(p: Point, x: f64, y: f64) -> bool {
    (p.x - x).abs() < 1e-6 && (p.y - y).abs() < 1e-6
}

fn no_special_warning(d: &Document) {
    assert!(!d.warnings.iter().any(|w| w.contains("special shapes")), "{:?}", d.warnings);
}

#[test]
fn a_heart_is_drawn_as_a_heart_in_its_box() {
    let d = read(b"ShHt", vec![(tag(b"Sprd"), F::F64(0.2))]);
    no_special_warning(&d);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let p = path(&d);
    assert_eq!(p.subpaths.len(), 1);
    let sp = &p.subpaths[0];
    assert!(sp.closed);
    assert_eq!(sp.segments.len(), 6, "six curves: the notch, two lobes, two sides and the point");
    let a = anchors(sp);
    assert!(near(a[0], 50.0, 0.204 * 200.0), "the notch first: {:?}", a[0]);
    assert!(a.iter().any(|p| near(*p, 50.0, 200.0)), "the point at the bottom middle");
    assert!(a.iter().any(|p| near(*p, 0.0, 0.319 * 200.0)) && a.iter().any(|p| near(*p, 100.0, 0.319 * 200.0)), "the sides touch the box");
    // Mirror-symmetric about the box's centre line, and inside the box.
    let all = points(sp);
    for q in &all {
        assert!(all.iter().any(|m| near(*m, 100.0 - q.x, q.y)), "{q:?} has no mirror");
        assert!((0.0..=100.0).contains(&q.x) && (0.0..=200.0).contains(&q.y), "{q:?}");
    }
    assert!(area(sp) > 0.0);
    // Another spread is drawn with the fitted one, and says so.
    let other = read(b"ShHt", vec![(tag(b"Sprd"), F::F64(0.5))]);
    assert_eq!(path(&other), p);
    assert!(other.warnings.iter().any(|w| w.contains("hearts")), "{:?}", other.warnings);
}

#[test]
fn a_cog_has_its_teeth_on_the_box_ellipse_and_a_hole() {
    let d = read(
        b"ShCg",
        vec![
            (tag(b"IRad"), F::F64(0.8)),
            (tag(b"Hole"), F::F64(0.25)),
            (tag(b"TtSz"), F::F64(0.3)),
            (tag(b"NtSz"), F::F64(0.5)),
            (tag(b"Curv"), F::F64(0.0)),
            (tag(b"Teth"), F::U32(8)),
        ],
    );
    no_special_warning(&d);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let p = path(&d);
    assert_eq!(p.subpaths.len(), 2, "the teeth and the hole");
    let (outline, hole) = (&p.subpaths[0], &p.subpaths[1]);
    assert!(area(outline) > 0.0 && area(hole) < 0.0, "the hole runs the other way, so it stays open");
    // Every anchor lies on the box's ellipse (tops) or at 80 % of it (roots), the hole at 25 %.
    let r = |q: &Point| ((q.x - 50.0) / 50.0).hypot((q.y - 100.0) / 100.0);
    for q in anchors(outline) {
        assert!((r(&q) - 1.0).abs() < 1e-9 || (r(&q) - 0.8).abs() < 1e-9, "{q:?}");
    }
    assert!(anchors(hole).iter().all(|q| (r(q) - 0.25).abs() < 1e-9));
    // The first tooth is straight up, its top 30 % of the pitch wide; the gap at the root is 50 %.
    let pitch = std::f64::consts::TAU / 8.0;
    let start = outline.start;
    let top = 0.3 * pitch / 2.0;
    assert!(near(start, 50.0 - 50.0 * top.sin(), 100.0 - 100.0 * top.cos()), "{start:?}");
    let tops = anchors(outline).iter().filter(|q| (r(q) - 1.0).abs() < 1e-9).count();
    assert_eq!(tops, 16, "two anchors a tooth top");
    // Curved teeth are drawn straight, and say so; counts are clamped.
    let curved = read(b"ShCg", vec![(tag(b"Curv"), F::F64(0.4)), (tag(b"Teth"), F::U32(u32::MAX))]);
    assert!(curved.warnings.iter().any(|w| w.contains("cogs")), "{:?}", curved.warnings);
    assert!(path(&curved).subpaths[0].segments.len() <= 4 * 10_000);
}

#[test]
fn a_double_arrow_has_a_shaft_and_its_heads() {
    let arrow = |l: u16, r: u16, len: f64, extra: Vec<(vectorcraft_affinity::stream::Tag, F)>| {
        let mut p = vec![
            (tag(b"Thck"), F::F64(0.4)),
            (tag(b"LSty"), F::Enum(l, 0)),
            (tag(b"RSty"), F::Enum(r, 0)),
            (tag(b"LPr1"), F::F64(len)),
            (tag(b"LPr2"), F::F64(0.0)),
            (tag(b"RPr1"), F::F64(len)),
            (tag(b"RPr2"), F::F64(0.0)),
        ];
        p.extend(extra);
        read(b"ShDA", p)
    };
    let both = arrow(1, 1, 0.25, vec![]);
    no_special_warning(&both);
    assert!(both.warnings.is_empty(), "{:?}", both.warnings);
    let sp = &path(&both).subpaths[0];
    let a = anchors(sp);
    assert_eq!(a.len(), 10, "{a:?}");
    assert!(sp.segments.iter().all(|s| s[0] == s[1] && s[1] == s[2] || s[0] == s[2]), "straight sides");
    for (x, y) in [(0.0, 100.0), (25.0, 0.0), (25.0, 60.0), (75.0, 60.0), (75.0, 0.0), (100.0, 100.0), (75.0, 140.0), (25.0, 140.0)] {
        assert!(a.iter().any(|p| near(*p, x, y)), "({x}, {y}) in {a:?}");
    }
    assert!(area(sp) > 0.0);
    // No head on the left: a flat end.
    let one = arrow(0, 1, 0.25, vec![]);
    let a = anchors(&path(&one).subpaths[0]);
    assert_eq!(a.len(), 7, "{a:?}");
    assert!(a.iter().any(|p| near(*p, 0.0, 60.0)) && a.iter().any(|p| near(*p, 0.0, 140.0)));
    // Heads half the width each meet in the middle: a diamond (patchy-shapes-2.af's "arrow").
    let diamond = arrow(1, 1, 0.5, vec![]);
    let a = anchors(&path(&diamond).subpaths[0]);
    assert_eq!(a.len(), 4, "{a:?}");
    for (x, y) in [(0.0, 100.0), (50.0, 0.0), (100.0, 100.0), (50.0, 200.0)] {
        assert!(a.iter().any(|p| near(*p, x, y)), "({x}, {y}) in {a:?}");
    }
    // Longer heads are shortened to meet in the middle.
    assert_eq!(path(&arrow(1, 1, 0.9, vec![])), path(&diamond));
    // Other head styles and swept heads are drawn as plain heads, and say so.
    for d in [arrow(3, 1, 0.25, vec![]), arrow(1, 1, 0.25, vec![(tag(b"RPr2"), F::F64(0.3))])] {
        assert!(d.warnings.iter().any(|w| w.contains("arrows")), "{:?}", d.warnings);
        assert_eq!(anchors(&path(&d).subpaths[0]).len(), 10);
    }
}

#[test]
fn other_special_shapes_are_still_their_bounding_ellipse() {
    let d = read(b"ShCl", vec![]);
    assert!(d.warnings.iter().any(|w| w.contains("special shapes")), "{:?}", d.warnings);
    assert!(!d.warnings.iter().any(|w| w.contains("heart") || w.contains("cog") || w.contains("arrow")), "{:?}", d.warnings);
}

/// The corpus file with Affinity's special shapes: its heart, cog and double arrow are no longer
/// ellipses, and fill their boxes as Affinity's thumbnail shows them.
#[test]
fn corpus_heart_cog_and_arrow_are_drawn_as_themselves() {
    let Some(dir) = crate::corpus::pinned() else { return };
    let bytes = std::fs::read(dir.join("patchy-shapes-2.af")).unwrap();
    let d = vectorcraft_affinity::read(&bytes, Limits::default()).unwrap();
    let warned = d.warnings.iter().find(|w| w.contains("special shapes")).cloned().unwrap_or_default();
    assert!(warned.contains("(6×)"), "only the diamond, double star, crescent, cloud, segment and tear: {warned}");
    let named = |name: &str| {
        d.spreads[0]
            .nodes
            .iter()
            .find(|n| n.name == name)
            .and_then(|n| match &n.kind {
                Kind::Shape { path, .. } => Some(path.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name}"))
    };
    let heart = named("heart");
    assert_eq!(heart.subpaths[0].segments.len(), 6);
    let cog = named("cog");
    assert_eq!(cog.subpaths.len(), 2);
    assert_eq!(anchors(&cog.subpaths[0]).len(), 12 * 4, "12 teeth");
    let arrow = anchors(&named("arrow").subpaths[0]);
    assert_eq!(arrow.len(), 4, "a diamond, as the thumbnail shows: {arrow:?}");
}
