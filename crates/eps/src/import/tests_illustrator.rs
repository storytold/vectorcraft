//! Illustrator files (Illustrator 3–8 and their EPS): the groups they write (`u` … `U`) come in
//! as groups, nested as they were, between and inside clipping groups.

use std::sync::Arc;

use vectorcraft_doc::clipnest::MAX_NEST;
use vectorcraft_doc::{Node, NodeKind};

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

/// A file of the legacy format that names Illustrator's procsets (as Rhino's export does) instead of
/// defining its operators comes in through the layers reader, with its layer names, though its
/// header has no `%AI…` comments.
#[test]
fn a_file_that_names_illustrators_procsets_comes_in_with_its_layers() {
    let layer = |name: &str, body: &str| {
        let style = "0 A\n0 R\n0 0 0 1 K\n0 i 1 J 1 j 0.8 w 4 M []0 d\n0 D";
        format!("%AI5_BeginLayer\n1 1 1 1 0 0 -1 0 0 0 Lb\n({name}) Ln\n{style}\n{body}\nS\nLB\n%AI5_EndLayer--\n")
    };
    let text = format!(
        "%!PS-Adobe-3.0\n%%Creator: Rhinoceros\n%%BoundingBox: 0 0 200 100\n\
         %%DocumentNeededResources: procset Adobe_packedarray 2.0 0\n%%+ procset Adobe_IllustratorA_AI3 1.0 0\n%%EndComments\n\
         %%BeginProlog\n%%IncludeResource: procset Adobe_packedarray 2.0 0\nAdobe_packedarray /initialize get exec\n%%EndProlog\n\
         %%BeginSetup\n%AI5_BeginNonPrinting\nNp\n%AI5_EndNonPrinting--\nAdobe_cmykcolor /initialize get exec\n%%EndSetup\n{}{}\
         %%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\nAdobe_packedarray /terminate get exec\n%%EOF\n",
        layer("SECTION::Cut", "10 10 m\n90 10 L\n90 90 L"),
        layer("SECTION::Hatch", "110 10 m\n190 90 L"),
    );
    let r = import(text.as_bytes()).unwrap();
    let names: Vec<_> = r.document.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["SECTION::Cut", "SECTION::Hatch"]);
    assert!(!r.preview);
}
