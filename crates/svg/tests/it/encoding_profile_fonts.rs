//! SVG Options → Encoding, SVG Profile and embedded fonts: UTF-16 and ISO 8859-1 files read
//! back, entity styling parses in usvg, SVG Tiny 1.2 has no filters, masks, symbols or style
//! sheets, and embedded fonts hold only the glyphs used where their licence allows it.

use std::sync::Arc;

use serde_json::json;
use skrifa::raw::TableProvider;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Effect, Node, NodeKind, OpacityMask, Symbol, TextObject, TextRun};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_svg::{Encoding, ExportOptions, Profile, Styling, compress_bytes, editing, export_full, import, text_of};
use vectorcraft_testkit::format::base64_decode;

fn add(d: &mut Document, mut n: Node) {
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

fn style(family: &str, font_style: &str) -> CharStyle {
    CharStyle { size: 24.0, font_family: family.into(), font_style: font_style.into(), fill: Paint::solid(Color::BLACK), ..CharStyle::default() }
}

fn text(at: Point, runs: &[(&str, CharStyle)]) -> Node {
    let mut t = TextObject::point(at, "", runs[0].1.clone());
    t.runs = runs.iter().map(|(s, st)| TextRun { text: (*s).into(), style: st.clone(), inline: None }).collect();
    Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(t)))
}

/// The line of type [`doc`] holds: Latin-1 characters and others the bundled fonts have.
const LINE: &str = "Grüße — 5 €";

/// A red square with a Chinese name, a Chinese title and a line of type ([`LINE`]).
fn doc() -> Document {
    let mut d = Document::new(300.0, 200.0);
    d.title = "Grüße 中文".into();
    let red = Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::None, 0.0);
    let mut sq = Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 60.0)), red);
    sq.name = Some("方块".into());
    add(&mut d, sq);
    add(&mut d, text(Point::new(20.0, 120.0), &[(LINE, style("Source Sans 3", "Regular"))]));
    d
}

fn plain(d: &Document) -> String {
    let mut out = String::new();
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            out.push_str(&t.plain_text());
        }
    });
    out
}

fn red_fill(d: &Document) -> bool {
    let mut red = false;
    d.walk(|n| red |= n.appearance.fill_paint() == Paint::solid(Color::rgb8(255, 0, 0)));
    red
}

#[test]
fn utf16_files_start_with_a_byte_order_mark_and_read_back() {
    let o = export_full(&doc(), &ExportOptions { encoding: Encoding::Utf16, ..Default::default() }, None);
    let bytes = o.bytes();
    assert_eq!(&bytes[..2], &[0xfe, 0xff], "big-endian byte order mark");
    assert_eq!(bytes.len(), 2 + 2 * o.svg.encode_utf16().count());
    let svg = text_of(&bytes).unwrap();
    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"), "{svg}");
    assert!(svg.contains(LINE) && svg.contains("<title>Grüße 中文</title>"), "{svg}");
    let back = import(&svg).unwrap();
    assert_eq!(plain(&back), LINE);
    // Gzipped (SVGZ) too, and little-endian files from elsewhere.
    assert_eq!(text_of(&compress_bytes(&bytes)).unwrap(), svg);
    let le: Vec<u8> = [0xff, 0xfe].into_iter().chain(svg.encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(text_of(&le).unwrap(), svg);
    assert!(text_of(&[0xfe, 0xff, 0xd8]).is_err(), "odd UTF-16 is refused");
}

#[test]
fn latin1_files_write_other_characters_as_references() {
    let d = doc();
    for (minify, styling) in [(false, Styling::PresentationAttributes), (true, Styling::StyleEntities)] {
        let o = export_full(
            &d,
            &ExportOptions { encoding: Encoding::Latin1, minify, styling, preserve_editing: true, ..Default::default() },
            Some(b"native"),
        );
        let bytes = o.bytes();
        // The declaration names the encoding even minified; ü is one byte, € and 中 references.
        assert!(bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>"), "{}", o.svg);
        assert!(bytes.windows(4).any(|w| w == b"Gr\xfc\xdf"), "{}", o.svg);
        assert!(o.svg.contains("&#x2014; 5 &#x20AC;") && o.svg.contains("&#x4E2D;&#x6587;"), "{}", o.svg);
        assert!(o.svg.chars().all(|c| u32::from(c) < 0x100), "{}", o.svg);
        let svg = text_of(&bytes).unwrap();
        assert_eq!(svg, o.svg);
        let back = import(&svg).unwrap();
        assert_eq!(plain(&back), LINE, "{minify}");
        assert!(red_fill(&back));
        // usvg reads the entities (and their references) too.
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        assert!(tree.root().has_children());
        // The editing data's hash covers the file as written.
        assert!(editing(&svg).unwrap().intact, "{minify}");
    }
}

#[test]
fn style_entities_parse_in_usvg() {
    let o = export_full(&doc(), &ExportOptions { styling: Styling::StyleEntities, ..Default::default() }, None);
    assert!(o.svg.contains("<!ENTITY st1 ") && o.svg.contains("style=\"&st1;\""), "{}", o.svg);
    let tree = usvg::Tree::from_str(&o.svg, &usvg::Options::default()).unwrap();
    let mut red = false;
    fn walk(g: &usvg::Group, red: &mut bool) {
        for n in g.children() {
            match n {
                usvg::Node::Path(p) => {
                    *red |= matches!(p.fill().map(|f| f.paint()), Some(usvg::Paint::Color(c)) if (c.red, c.green, c.blue) == (255, 0, 0))
                }
                usvg::Node::Group(g) => walk(g, red),
                _ => {}
            }
        }
    }
    walk(tree.root(), &mut red);
    assert!(red, "the entity's fill reached usvg: {}", o.svg);
}

#[test]
fn tiny_has_no_filters_masks_symbols_or_style_sheets() {
    let mut d = Document::new(300.0, 200.0);
    let fill = |r, g, b| Appearance::basic(Paint::solid(Color::rgb8(r, g, b)), Paint::None, 0.0);
    // A shadowed square, a masked one in Multiply, a symbol instance and an outside stroke.
    let mut a = Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 60.0)), fill(255, 0, 0));
    a.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: json!({}), visible: true });
    add(&mut d, a);
    let mut b = Node::path(vectorcraft_doc::NodeId(0), shapes::rectangle(Rect::new(80.0, 10.0, 130.0, 60.0)), fill(0, 0, 255));
    b.blend = BlendMode::Multiply;
    b.extras_mut().mask =
        Some(Box::new(OpacityMask::new(Node::path(d.alloc_id(), shapes::ellipse(Rect::new(80.0, 10.0, 130.0, 60.0)), fill(255, 255, 255)), true)));
    add(&mut d, b);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(-5.0, -5.0, 5.0, 5.0)), fill(0, 128, 0));
    d.symbols.push(Symbol { name: "Dot".into(), art: Arc::new(art) });
    add(&mut d, Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((200.0, 30.0)) }));
    let mut c = Node::path(
        vectorcraft_doc::NodeId(0),
        shapes::rectangle(Rect::new(150.0, 100.0, 250.0, 150.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 6.0),
    );
    if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) = c.appearance.items.get_mut(1) {
        s.align = vectorcraft_doc::StrokeAlign::Outside;
    }
    add(&mut d, c);
    let full = export_full(&d, &ExportOptions { styling: Styling::InternalCss, ..Default::default() }, None).svg;
    assert!(full.contains("<filter") && full.contains("<mask") && full.contains("<symbol") && full.contains("<style>"), "{full}");
    let o =
        export_full(&d, &ExportOptions { styling: Styling::InternalCss, profile: Profile::Tiny12, embed_fonts: true, ..Default::default() }, None);
    let svg = &o.svg;
    assert!(svg.contains("version=\"1.2\" baseProfile=\"tiny\""), "{svg}");
    for gone in ["<filter", "<mask", "<symbol", "<style", "style=", "class=", "mix-blend-mode"] {
        assert!(!svg.contains(gone), "{gone}: {svg}");
    }
    assert!(svg.contains("clip-rule=\"evenodd\""), "the outside stroke is clipped outside the shape: {svg}");
    for w in ["filters", "masks", "style sheets"] {
        assert!(o.warnings.iter().any(|x| x.contains(w)), "{w}: {:?}", o.warnings);
    }
    let back = import(svg).unwrap();
    assert!(red_fill(&back));
    assert!(back.symbols.is_empty(), "the instance came back as art");
}

/// The fonts a file embeds: (font-family, weight, the font file).
fn embedded(svg: &str) -> Vec<(String, u16, Vec<u8>)> {
    let css = svg.replace("&quot;", "\"");
    css.split("@font-face{")
        .skip(1)
        .map(|r| {
            let field = |k: &str| r.split(&format!("{k}:")).nth(1).unwrap().split(';').next().unwrap().to_string();
            let data = r.split("base64,").nth(1).unwrap().split(')').next().unwrap();
            (field("font-family"), field("font-weight").parse().unwrap(), base64_decode(data).unwrap())
        })
        .collect()
}

#[test]
fn embedded_fonts_hold_only_the_glyphs_used() {
    let mut d = Document::new(300.0, 200.0);
    add(&mut d, text(Point::new(20.0, 60.0), &[("Hello ", style("Source Sans 3", "Regular")), ("Bold", style("Source Sans 3", "Semibold"))]));
    let svg = export_full(&d, &ExportOptions { embed_fonts: true, ..Default::default() }, None).svg;
    let fonts = embedded(&svg);
    assert_eq!(fonts.iter().map(|f| (f.0.as_str(), f.1)).collect::<Vec<_>>(), [("\"Source Sans 3\"", 400), ("\"Source Sans 3\"", 600)], "{svg}");
    assert!(svg.contains("font-weight=\"600\"") && svg.contains("format(&quot;truetype&quot;)"), "{svg}");
    for (_, w, data) in &fonts {
        let f = skrifa::FontRef::new(data).unwrap();
        let cmap = f.cmap().unwrap();
        let want: &[char] = if *w == 400 { &['H', 'e', 'l', 'o', ' '] } else { &['B', 'o', 'l', 'd'] };
        assert!(want.iter().all(|c| cmap.map_codepoint(*c).is_some()), "{w}");
        assert!(cmap.map_codepoint('x').is_none(), "{w}: unused characters are left out");
        // .notdef and the characters' own glyphs.
        let unique = want.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert_eq!(f.maxp().unwrap().num_glyphs() as usize, unique + 1, "{w}");
    }
    // The weights read back as the styles.
    let back = import(&svg).unwrap();
    let mut styles = vec![];
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            styles.extend(t.runs.iter().map(|r| r.style.font_style.to_ascii_lowercase()));
        }
    });
    assert_eq!(styles, ["regular", "semibold"]);
    // Without the option, or with outlined text, nothing is embedded.
    assert!(!export_full(&d, &ExportOptions::default(), None).svg.contains("@font-face"));
    assert!(!export_full(&d, &ExportOptions { embed_fonts: true, outline_text: true, ..Default::default() }, None).svg.contains("@font-face"));
}

/// A named instance of a variable font embeds as a static font of that instance: the variable
/// file would show its default instance in browsers (#296).
#[test]
fn variable_font_instances_embed_as_themselves() {
    use vectorcraft_text::test_fonts::{VARIABLE_FAMILY, variable_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(variable_font().unwrap());
    let mut d = Document::new(300.0, 200.0);
    add(&mut d, text(Point::new(20.0, 60.0), &[("ll", style(VARIABLE_FAMILY, "Regular")), ("ll", style(VARIABLE_FAMILY, "Bold"))]));
    let o = export_full(&d, &ExportOptions { embed_fonts: true, ..Default::default() }, None);
    let fonts = embedded(&o.svg);
    assert_eq!(fonts.iter().map(|f| f.1).collect::<Vec<_>>(), [400, 700], "{:?}", o.warnings);
    let mut advances = vec![];
    for ((_, w, data), style) in fonts.iter().zip(["Regular", "Bold"]) {
        let f = skrifa::FontRef::new(data).unwrap();
        assert!(f.fvar().is_err() && f.gvar().is_err(), "{w}: a static font");
        let gid = f.cmap().unwrap().map_codepoint('l').unwrap();
        let adv = f.hmtx().unwrap().advance(gid).unwrap();
        let face = db.face(VARIABLE_FAMILY, style).unwrap();
        assert_eq!(f64::from(adv), face.advance(face.glyph_for('l')).round(), "{style}");
        advances.push(adv);
    }
    assert!(advances[1] > advances[0], "{advances:?}");
}

/// Variable Font settings are written as `font-variation-settings`, with the `wght` setting as the
/// weight and the `wdth` setting as the width; the font embeds as a static font at the settings,
/// and the settings read back. Faces that one weight, style and width describe and that share
/// characters are warned about.
#[test]
fn variable_font_settings_are_written_embedded_and_read_back() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY as FAMILY, two_axis_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(two_axis_font().unwrap());
    let set = [("wdth".to_string(), 90.0), ("wght".to_string(), 650.0)].into_iter().collect::<std::collections::BTreeMap<_, _>>();
    let varied = CharStyle { font_axes: set.clone(), ..style(FAMILY, "Regular") };
    let mut d = Document::new(300.0, 200.0);
    add(&mut d, text(Point::new(20.0, 60.0), &[("ll", style(FAMILY, "Regular")), ("ll", varied.clone())]));
    let o = export_full(&d, &ExportOptions { embed_fonts: true, ..Default::default() }, None);
    assert!(o.svg.contains("font-variation-settings:&quot;wdth&quot; 90, &quot;wght&quot; 650"), "{}", o.svg);
    assert!(o.svg.contains("font-weight=\"650\"") && o.svg.contains("font-stretch=\"90%\""), "{}", o.svg);
    let fonts = embedded(&o.svg);
    assert_eq!(fonts.iter().map(|f| f.1).collect::<Vec<_>>(), [400, 650], "{:?}", o.warnings);
    assert!(o.svg.contains("font-style:normal;font-stretch:90%;src:"), "{}", o.svg);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    // The Condensed instance and Regular: one description, the same characters.
    let mut same = Document::new(300.0, 200.0);
    add(&mut same, text(Point::new(20.0, 60.0), &[("ll", style(FAMILY, "Regular")), ("ll", style(FAMILY, "Condensed"))]));
    let warnings = export_full(&same, &ExportOptions { embed_fonts: true, ..Default::default() }, None).warnings;
    assert!(warnings.iter().any(|w| w.contains(FAMILY) && w.contains("share characters")), "{warnings:?}");
    let face = db.style_face(&varied).unwrap();
    let f = skrifa::FontRef::new(&fonts[1].2).unwrap();
    assert!(f.fvar().is_err() && f.gvar().is_err(), "a static font");
    let adv = f.hmtx().unwrap().advance(f.cmap().unwrap().map_codepoint('l').unwrap()).unwrap();
    assert_eq!(f64::from(adv), face.advance(face.glyph_for('l')).round());
    let regular = db.face(FAMILY, "Regular").unwrap();
    assert!(f64::from(adv) > regular.advance(regular.glyph_for('l')) + 5.0, "the instance's own glyphs");
    // Read back: the settings, on the run that has them.
    let back = import(&o.svg).unwrap();
    let mut axes = vec![];
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            axes.extend(t.runs.iter().map(|r| r.style.font_axes.clone()));
        }
    });
    assert_eq!(axes, [Default::default(), set]);
}

/// The bundled Source Sans 3 Regular renamed `family` (13 characters, as long as the original
/// name) with OS/2 `fsType` set to `fs_type`.
fn licensed_font(family: &str, fs_type: u16) -> Vec<u8> {
    let mut data = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").to_vec();
    let table = |data: &[u8], tag: &[u8; 4]| {
        let f = skrifa::FontRef::new(data).unwrap();
        let r = f.table_directory.table_records().iter().find(|r| r.tag().to_be_bytes() == *tag).unwrap();
        (r.offset() as usize, r.length() as usize)
    };
    let (os2, _) = table(&data, b"OS/2");
    data[os2 + 8..os2 + 10].copy_from_slice(&fs_type.to_be_bytes());
    let (name, len) = table(&data, b"name");
    assert_eq!(family.len(), "Source Sans 3".len());
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
        let mut i = name;
        while let Some(at) = data[i..name + len].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

#[test]
fn font_licences_are_respected() {
    let db = vectorcraft_text::FontDb::global();
    db.add_font(licensed_font("Sealed Sans 3", 0x0002));
    db.add_font(licensed_font("Wholed Sans 3", 0x0100));
    assert!(db.has_family("Sealed Sans 3") && db.has_family("Wholed Sans 3"));
    let mut d = Document::new(300.0, 200.0);
    add(&mut d, text(Point::new(20.0, 60.0), &[("Restricted ", style("Sealed Sans 3", "Regular")), ("whole", style("Wholed Sans 3", "Regular"))]));
    let o = export_full(&d, &ExportOptions { embed_fonts: true, ..Default::default() }, None);
    let fonts = embedded(&o.svg);
    assert_eq!(fonts.len(), 1, "{:?}", o.warnings);
    assert_eq!(fonts[0].0, "\"Wholed Sans 3\"");
    let size = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf").len();
    assert!(fonts[0].2.len() + 64 >= size, "embedded whole: {} of {size} bytes", fonts[0].2.len());
    assert!(o.warnings.iter().any(|w| w.contains("Sealed Sans 3") && w.contains("doesn't allow embedding")), "{:?}", o.warnings);
    assert!(o.warnings.iter().any(|w| w.contains("Wholed Sans 3") && w.contains("whole")), "{:?}", o.warnings);
}

proptest::proptest! {
    /// Reading any bytes as SVG text never panics: byte order marks with odd or unpaired UTF-16,
    /// declarations naming encodings, junk.
    #[test]
    fn any_bytes_decode_or_fail_cleanly(
        head in proptest::sample::select(vec![&b""[..], &[0xfe, 0xff][..], &[0xff, 0xfe][..], &[0xef, 0xbb, 0xbf][..], &b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>"[..], &b"<?xml encoding='latin1"[..]]),
        body in proptest::collection::vec(proptest::num::u8::ANY, 0..64),
    ) {
        let bytes: Vec<u8> = head.iter().chain(&body).copied().collect();
        let _ = text_of(&bytes);
        let _ = text_of(&compress_bytes(&bytes));
    }
}
