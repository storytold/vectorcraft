//! Illustrator files (Illustrator 3–8 and their EPS): the groups they write (`u` … `U`) come in
//! as groups, nested as they were, between and inside clipping groups.

use std::sync::Arc;

use vectorcraft_doc::clipnest::MAX_NEST;
use vectorcraft_doc::{LayerColor, LineCap, Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::import::import;

/// A minimal file in Illustrator's format: its header comments, a prolog of our own defining the
/// operators `body` uses (`u` and `U` as `ugroup` and `Ugroup`), and `body`.
fn ai(header: &str, ugroup: &str, body: &str) -> Vec<u8> {
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n{header}%%BoundingBox: 0 0 100 100\n%%EndComments\n%%BeginProlog\n\
         /m {{moveto}} def /L {{lineto}} def /f {{closepath fill}} def /S {{stroke}} def /g {{setgray}} def\n\
         /u {{{ugroup}}} def /U {{{}}} def\n%%EndProlog\n{body}\nshowpage\n%%EOF\n",
        if ugroup.contains("gsave") { "grestore" } else { "" }
    )
    .into_bytes()
}

/// A legacy Illustrator format header: the reader takes `u` … `U` as groups only in files whose
/// header says they are in that format (`%AI…` comments or the creator, which the format's
/// published specification documents; see the `import` module docs).
const AI8: &str = "%%Creator: Adobe Illustrator(R) 8.0\n%AI5_FileFormat 4.0\n";

/// A triangle filled at `x`.
fn tri(x: u32) -> String {
    format!("{x} {x} m {} {x} L {} {} L f", x + 5, x + 5, x + 5)
}

/// The layer's objects as a tree: `p` a path, `c` a clip path, `g[…]` a group, `k[…]` a clipping
/// group.
fn tree(nodes: &[Arc<Node>]) -> String {
    let one = |n: &Arc<Node>| match &n.kind {
        NodeKind::Group { children, clip: false } => format!("g[{}]", tree(children)),
        NodeKind::Group { children, clip: true } => format!("k[{}]", tree(children)),
        NodeKind::Path { clipping: true, .. } => "c".into(),
        NodeKind::Path { .. } => "p".into(),
        k => format!("{k:?}"),
    };
    nodes.iter().map(one).collect::<Vec<_>>().join(" ")
}

fn read(header: &str, ugroup: &str, body: &str) -> (String, Vec<String>) {
    let r = import(&ai(header, ugroup, body)).unwrap();
    (tree(r.document.layers[0].children().unwrap()), r.warnings)
}

#[test]
fn groups_and_nested_groups_come_in_as_groups() {
    let body = format!("u {} u {} {} U U {}", tri(10), tri(30), tri(50), tri(70));
    assert_eq!(read(AI8, "", &body), ("g[p g[p p]] p".into(), vec![]));
    // Illustrator 3's header names it only as the creator; a prolog whose groups save and restore
    // the graphics state groups the same.
    assert_eq!(read("%%Creator:Adobe Illustrator(TM) 3.2\n", "gsave", &body).0, "g[p g[p p]] p");
    // Groups of one object, and groups next to each other.
    assert_eq!(read(AI8, "", &format!("u {} U u {} U", tri(10), tri(30))).0, "g[p] g[p]");
}

#[test]
fn groups_nest_with_clipping_groups() {
    // A clipping group inside a group, a group inside a clipping group.
    let clip = "gsave 0 0 m 50 0 L 50 50 L 0 50 L closepath clip newpath";
    let body = format!("u {} {clip} u {} {} U grestore {} U", tri(10), tri(20), tri(30), tri(40));
    assert_eq!(read(AI8, "", &body).0, "g[p k[c g[p p]] p]");
    // A fill and a stroke of one path in a group stay one object.
    let body = "u 10 10 m 20 10 L 20 20 L closepath gsave 0 g fill grestore S U";
    let r = import(&ai(AI8, "", body)).unwrap();
    let layer = r.document.layers[0].children().unwrap();
    assert_eq!(tree(layer), "g[p]");
    assert_eq!(layer[0].children().unwrap()[0].appearance.items.len(), 2);
}

#[test]
fn other_postscript_keeps_its_own_u() {
    // A program that isn't Illustrator's may name its own procedures `u` and `U`.
    let body = format!("u {} {} U", tri(10), tri(30));
    assert_eq!(read("%%Creator: some app\n", "", &body).0, "p p");
}

/// A `.ai` in PostScript form as other apps write it (#1027): its header and prolog name the
/// procsets that define its operators (by the DSC resource comments) without including them, so
/// its program can't be run. Its layers come from the program itself: names as they are, the
/// legacy format's ten-operand `Lb`, flatness `i`, the bounding box as the page.
const NAMED_PROLOG: &str = "%!PS-Adobe-3.0\n%%Creator: a CAD app\n%%BoundingBox: 0 0 200 100\n\
    %%DocumentNeededResources: procset Adobe_packedarray 2.0 0\n%%+ procset Adobe_IllustratorA_AI3 1.0 0\n\
    %AI3_TemplateBox: 288 384 288 384\n%%EndComments\n%%BeginProlog\n%%IncludeResource: procset Adobe_packedarray 2.0 0\n\
    Adobe_packedarray /initialize get exec\n%%IncludeResource: procset Adobe_IllustratorA_AI3 1.0 0\n%%EndProlog\n\
    %%BeginSetup\nAdobe_IllustratorA_AI3 /initialize get exec\n%%EndSetup\n\
    %AI5_BeginLayer\n1 1 1 1 0 0 -1 0 0 0 Lb\n(SECTION::Cut) Ln\n0 A\n0 R\n0 0 0 1 K\n0 i 1 J 1 j 0.8 w 4 M []0 d\n0 D\n\
    10 10 m\n90 10 L\n90 90 L\n10 90 L\n10 10 L\nS\nLB\n%AI5_EndLayer--\n\
    %AI5_BeginLayer\n1 1 1 1 0 0 -1 191 63 255 Lb\n(SECTION::Hatch) Ln\n0 A\n0 R\n0 0 0 1 K\n0.1 w\n110 10 m\n190 90 L\nS\nLB\n%AI5_EndLayer--\n\
    %%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\nAdobe_IllustratorA_AI3 /terminate get exec\n%%EOF\n";

#[test]
fn a_postscript_ai_that_names_its_prolog_without_including_it_opens_with_its_layers() {
    let r = import(NAMED_PROLOG.as_bytes()).unwrap();
    assert!(r.warnings.is_empty() && !r.preview, "{:?}", r.warnings);
    let d = &r.document;
    let names: Vec<_> = d.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["SECTION::Cut", "SECTION::Hatch"]);
    let colors: Vec<_> = d.layers.iter().map(|l| if let NodeKind::Layer { color, .. } = l.kind { Some(color) } else { None }).collect();
    assert_eq!(colors, [Some(LayerColor::Custom([0, 0, 0])), Some(LayerColor::Custom([191, 63, 255]))]);
    assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 100.0), "the page is the bounding box");
    let square = &d.layers[0].children().unwrap()[0];
    assert_eq!(square.geometric_bounds(), Some(Rect::new(10.0, 10.0, 90.0, 90.0)));
    let stroke = square.appearance.stroke().unwrap();
    assert_eq!((stroke.width, stroke.cap), (0.8, LineCap::Round));
    assert_eq!(tree(d.layers[1].children().unwrap()), "p");
}

/// A `.ai` in PostScript form whose layers can't be read either says why, after what stopped its
/// program.
#[test]
fn a_postscript_ai_whose_layers_cant_be_read_says_why() {
    let damaged = NAMED_PROLOG.replace("0 D\n", "0 D\n1 2 frobnicate\n");
    let e = import(damaged.as_bytes()).unwrap_err();
    assert!(e.contains("`Adobe_packedarray`") && e.contains("its layers couldn't be read either") && e.contains("`frobnicate`"), "{e}");
}

/// A `.ai` in PostScript form (read through its layers) whose text carries hyphenation and
/// tab-ruler attributes: `XL` (hyphenation language), `Xb`/`XB` (a tab definition) and `Th`
/// (hyphen control). These keep no geometry, so the reader drops them rather than failing the
/// file the way it did before — the symptom seen on Illustrator 6/8 EPS files whose
/// `Adobe_level2_AI5` program can't be run.
#[test]
fn text_with_hyphenation_and_tab_attributes_opens_instead_of_failing() {
    let text = "0 To\n1 0 0 1 10 50 0 Tp\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 9 Tf\n\
        %_ 0 XL\n36 0 Xb\nXB\n0 1 2 2 99 Th\n0 Ta\n(hi) Tx\nTO\n";
    // The operators sit on a visible layer (so they'd be reported as unknown, not quietly skipped
    // on a hidden one), beside the square so the layer is never empty.
    let src = NAMED_PROLOG.replace("S\nLB\n%AI5_EndLayer--\n%AI5_BeginLayer", &format!("S\n{text}LB\n%AI5_EndLayer--\n%AI5_BeginLayer"));
    let r = import(src.as_bytes()).unwrap();
    assert!(!r.preview, "read through the layers, not a fallback preview");
    assert!(!r.warnings.iter().any(|w| w.contains("doesn't know") || w.contains("couldn't be read")), "{:?}", r.warnings);
}

/// An Illustrator 3 file of paths alone, as image editors export their paths (storytold/photocraft#6036):
/// no prolog and no layers, the paths unpainted (`n` closes, `N` doesn't), a compound path of two.
const PATHS_ONLY: &str = "%!PS-Adobe-2.0 EPSF-1.2\n%%Creator: an image editor\n%%BoundingBox: 0 0 200 100\n\
    %AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n\
    %%Note: Outline\n10 10 m\n100 10 L\n50 80 60 90 10 10 C\nn\n\
    *u\n20 20 m\n30 20 L\nN\n40 40 m\n50 50 L\n60 40 L\nn\n*U\n%%PageTrailer\n%%Trailer\n%%EOF\n";

#[test]
fn an_illustrator_3_file_without_layers_opens_its_paths_on_one_layer() {
    let r = import(PATHS_ONLY.as_bytes()).unwrap();
    assert!(r.warnings.is_empty() && !r.preview, "{:?}", r.warnings);
    let d = &r.document;
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 100.0));
    let art = d.layers[0].children().unwrap();
    assert_eq!(art.len(), 2);
    // Art space has y up: (10, 10) is 10 above the artboard's bottom.
    let NodeKind::Path { path, .. } = &art[0].kind else { panic!("{:?}", art[0].kind) };
    let ends: Vec<_> = path.subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect();
    assert_eq!(ends, [(10.0, 90.0), (100.0, 90.0)]);
    assert!(path.subpaths[0].closed);
    assert!(art[0].appearance.items.is_empty(), "unpainted");
    let NodeKind::Compound { children, .. } = &art[1].kind else { panic!("{:?}", art[1].kind) };
    let closed: Vec<_> = children.iter().map(|c| if let NodeKind::Path { path, .. } = &c.kind { path.is_closed() } else { false }).collect();
    assert_eq!(closed, [false, true]);
    assert_eq!(art[1].geometric_bounds(), Some(Rect::new(20.0, 50.0, 60.0, 80.0)));
}

/// The same program in a file that doesn't say it is in Illustrator's format is PostScript whose
/// operators the reader doesn't know: it is still refused.
#[test]
fn the_same_program_in_other_postscript_is_still_refused() {
    let other = PATHS_ONLY.replace("%AI3_Cropmarks: 0 0 200 100\n", "");
    let e = import(other.as_bytes()).unwrap_err();
    assert!(e.contains("it uses `m`") && !e.contains("layers"), "{e}");
}

/// The editing text of a newer file saved as PostScript (no prolog at all, a non-printing setup):
/// it opens with its sublayers.
#[test]
fn newer_editing_text_as_postscript_opens_with_its_sublayers() {
    let layer =
        |name: &str, art: &str| format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 -1 79 128 255 0 50 0 Lb\n({name}) Ln\n0 A\n0 Xw\n{art}LB\n%AI5_EndLayer--\n");
    let ground = layer("Ground", &(layer("Lawn", "0 0 0 0.3 k\n10 20 m\n70 20 L\n70 80 L\n10 80 L\nf\n") + &layer("Paving", "")));
    let text = format!(
        "%!PS-Adobe-3.0\n%%Creator: a script\n%%BoundingBox: 0 0 250 100\n%AI5_FileFormat 14.0\n%AI3_Cropmarks: 0 0 250 100\n%%EndComments\n\
         %%BeginProlog\n%%EndProlog\n%%BeginSetup\n%AI5_Begin_NonPrinting\nNp\n%AI5_End_NonPrinting--\n%%EndSetup\n{ground}%%PageTrailer\n%%Trailer\n%%EOF\n"
    );
    let r = import(text.as_bytes()).unwrap();
    let d = &r.document;
    assert_eq!(d.layers.len(), 1, "{:?}", r.warnings);
    let subs: Vec<_> = d.layers[0].children().unwrap().iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(subs, ["Lawn", "Paving"]);
}

/// A file in the legacy format whose prolog is included (here one of our own) runs as its page, and
/// comes in through the layers its program has when they look like the page.
#[test]
fn a_legacy_file_with_its_prolog_comes_in_through_its_layers() {
    let prolog = "/Lb {10 {pop} repeat} def /Ln {pop} def /LB {} def /A {pop} def /k {setcmykcolor} def";
    let body = format!("%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n(Back) Ln\n0 A\n0 0 0 1 k\n{} LB\n%AI5_EndLayer--\n", tri(10));
    let file = String::from_utf8(ai(AI8, "", &body)).unwrap().replace("%%EndProlog", &format!("{prolog}\n%%EndProlog"));
    let r = import(file.as_bytes()).unwrap();
    let names: Vec<_> = r.document.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Back"], "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // Without the editing data, it is its page.
    let page = crate::import::import_with(file.as_bytes(), false).unwrap();
    assert_eq!(page.document.layers[0].name.as_deref(), Some("Layer 1"));
}

#[test]
fn unbalanced_and_runaway_groups_are_read_safely() {
    // Ends without a beginning end nothing; groups left open hold the rest.
    assert_eq!(read(AI8, "", &format!("U U {} u {} u {}", tri(10), tri(20), tri(30))).0, "p g[p g[p]]");
    // Groups nest only so deep: the deeper ones' art goes into the innermost group read, and the
    // groups around them close where they should.
    let n = MAX_NEST + 50;
    let body = format!("{} {} {} {}", "u ".repeat(n), tri(10), "U ".repeat(n), tri(30));
    let (t, warnings) = read(AI8, "", &body);
    assert_eq!(t, format!("{}p{} p", "g[".repeat(MAX_NEST), "]".repeat(MAX_NEST)));
    assert!(warnings.iter().any(|w| w.contains("nested more than")), "{warnings:?}");
}
