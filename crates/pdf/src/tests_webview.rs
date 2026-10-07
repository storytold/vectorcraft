//! General › Embed page thumbnails and Optimize for fast web view: a `/Thumb` image on every page,
//! and a linearised file whose hint tables say where each page's objects are (checked here as a
//! linearisation checker does), also when encrypted; with both off the export is unchanged.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use image::{DynamicImage, ImageFormat};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Artboard, Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::encrypt::{Lexer, Obj};
use crate::syntax::{Indirect, indirect};
use crate::*;

/// `n` artboards side by side, each with a rectangle; `image` places the same image on every page
/// but the first (shared objects), and on the first too when `on_first`.
fn doc(n: usize, image: bool, on_first: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    for i in 1..n {
        let x = 200.0 * i as f64;
        d.artboards.push(Artboard {
            id: i as u32 + 1,
            name: format!("A{i}"),
            rect: Rect::new(x, 0.0, x + 100.0, 100.0),
            show_center_mark: false,
            show_cross_hairs: false,
            ..Default::default()
        });
    }
    let mut png = vec![];
    DynamicImage::from(image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([(x * 30) as u8, (y * 30) as u8, 90])))
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
    d.images.insert("img".into(), ImageBlob::new("image/png", png));
    let layer = d.default_layer().unwrap();
    for i in 0..n {
        let x = 200.0 * i as f64;
        let id = d.alloc_id();
        let fill = Paint::solid(Color::rgb(0.2, 0.1 * i as f32, 0.8));
        d.insert(
            Some(layer),
            0,
            Node::path(id, shapes::rectangle(Rect::new(x + 10.0, 10.0, x + 50.0, 50.0)), Appearance::basic(fill, Paint::None, 0.0)),
        )
        .unwrap();
        if image && (i > 0 || on_first) {
            let xf = Affine::translate((x + 60.0, 60.0)) * Affine::scale(4.0);
            let img = ImageObject { key: "img".into(), width: 8, height: 8, xf, link: None, placement: Default::default() };
            let id = d.alloc_id();
            d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(img))).unwrap();
        }
    }
    d
}

fn options(settings: serde_json::Value) -> PdfOptions {
    PdfOptions { settings: serde_json::from_value(settings).unwrap(), created: Some(0), ..Default::default() }
}

/// A thumbnail of `w` × `h` pixels of one grey.
fn thumb(w: u32, h: u32, grey: u8) -> Thumbnail {
    Thumbnail { width: w, height: h, rgb: vec![grey; (w * h * 3) as usize] }
}

/// The objects of a (possibly linearised) file, from its cross-reference tables: number → offset.
fn offsets(pdf: &[u8], tables: &[usize]) -> HashMap<u32, usize> {
    let mut out = HashMap::new();
    for &at in tables {
        let mut lx = Lexer::at(pdf, at);
        assert!(matches!(lx.next(), Some((crate::encrypt::Tok::Word(b"xref"), ..))), "a table at {at}");
        let word = |lx: &mut Lexer| match lx.next() {
            Some((crate::encrypt::Tok::Word(w), ..)) => std::str::from_utf8(w).unwrap().to_string(),
            t => panic!("{t:?}"),
        };
        let (first, count): (u32, u32) = (word(&mut lx).parse().unwrap(), word(&mut lx).parse().unwrap());
        for n in first..first + count {
            let (off, _, kind) = (word(&mut lx), word(&mut lx), word(&mut lx));
            if kind == "n" {
                out.insert(n, off.parse().unwrap());
            }
        }
    }
    out
}

fn int(o: &Obj, key: &[u8]) -> usize {
    usize::try_from(o.int(key).unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(key)))).unwrap()
}

fn reference(o: &Obj, key: &[u8]) -> u32 {
    match o.get(key) {
        Some(Obj::Ref(n, _)) => *n,
        _ => panic!("{} is no reference", String::from_utf8_lossy(key)),
    }
}

/// Bits read most significant first.
struct BitReader<'a> {
    data: &'a [u8],
    at: usize,
}

impl BitReader<'_> {
    fn get(&mut self, bits: u64) -> u64 {
        (0..bits).fold(0, |v, _| {
            let bit = self.data[self.at / 8] >> (7 - self.at % 8) & 1;
            self.at += 1;
            v << 1 | u64::from(bit)
        })
    }

    fn align(&mut self) {
        self.at = self.at.div_ceil(8) * 8;
    }

    /// One item for each of `n` entries, then the padding.
    fn column(&mut self, n: usize, bits: u64) -> Vec<u64> {
        let v = (0..n).map(|_| self.get(bits)).collect();
        self.align();
        v
    }
}

/// What a linearisation check finds in `pdf`.
struct Linearised {
    pages: Vec<u32>,
    /// The objects of the first page's cross-reference table.
    first_part: HashSet<u32>,
    encrypted: bool,
}

/// Check `pdf` is linearised as ISO 32000-1 Annex F says, as a linearisation checker does: the
/// linearization dictionary, both cross-reference tables and their trailers, and (unless the file
/// is encrypted) the hint tables against where every page's objects are and what each page uses.
fn check_linearised(pdf: &[u8]) -> Linearised {
    // The linearization dictionary is the first object.
    let first_obj = crate::lab_spot::find(pdf, b" 0 obj", 0).unwrap();
    let lin_at = pdf[..first_obj].iter().rposition(|b| *b == b'\n').unwrap() + 1;
    let lin = indirect(pdf, lin_at).unwrap();
    assert_eq!(lin.value.int(b"Linearized"), Some(1));
    assert_eq!(int(&lin.value, b"L"), pdf.len(), "/L is the file's length");
    // The first page's table follows it, its trailer points at the main table (/Prev), and the
    // file's last startxref at it.
    let first_table = lin.endobj + 7;
    assert!(pdf[first_table..].starts_with(b"xref"));
    let last = crate::lab_spot::rfind(pdf, b"startxref").unwrap();
    assert_eq!(std::str::from_utf8(&pdf[last + 9..]).unwrap().split_whitespace().next().unwrap().parse::<usize>().unwrap(), first_table);
    let trailer_at = crate::lab_spot::find(pdf, b"trailer", first_table).unwrap() + 7;
    let trailer = Lexer::at(pdf, trailer_at).object(0).unwrap();
    let main = int(&trailer, b"Prev");
    let size = int(&trailer, b"Size") as u32;
    let first_part: HashSet<u32> = offsets(pdf, &[first_table]).into_keys().collect();
    let all = offsets(pdf, &[first_table, main]);
    assert_eq!(all.len() as u32, size - 1, "the tables cover every object");
    let first = lin.num;
    assert!(first_part.iter().all(|n| (first..size).contains(n)) && first_part.len() as u32 == size - first);
    // /T: the white space before the main table's first entry.
    let entries = main + format!("xref\n0 {first}\n").len();
    assert!(pdf[main..].starts_with(format!("xref\n0 {first}\n").as_bytes()), "the main table covers 0..{first}");
    assert_eq!(int(&lin.value, b"T"), entries - 1);
    let main_trailer = Lexer::at(pdf, crate::lab_spot::find(pdf, b"trailer", main).unwrap() + 7).object(0).unwrap();
    assert_eq!(int(&main_trailer, b"Size") as u32, first);

    // Every object where its table says, read as the file's objects.
    let objects: HashMap<u32, Indirect> = all.iter().map(|(n, off)| (*n, indirect(pdf, *off).unwrap())).collect();
    assert!(objects.iter().all(|(n, o)| o.num == *n));
    // Each object's span: to the next object, or to the table after it.
    let mut starts: Vec<usize> = all.values().copied().chain([first_table, main]).collect();
    starts.sort_unstable();
    let span = |n: u32| {
        let at = all[&n];
        starts[starts.partition_point(|s| *s <= at)] - at
    };
    let root = &objects[&reference(&trailer, b"Root")];
    let mut pages = vec![];
    let mut nodes = HashSet::new();
    crate::linearize::page_tree(&objects, reference(&root.value, b"Pages"), 0, &mut pages, &mut nodes).unwrap();
    assert_eq!(int(&lin.value, b"N"), pages.len());
    assert_eq!(int(&lin.value, b"O") as u32, pages[0], "/O is the first page");

    // The hint stream: /H gives where it is and how long.
    let Some(Obj::Array(h)) = lin.value.get(b"H") else { panic!("no /H") };
    let [Obj::Int { value: h_at, .. }, Obj::Int { value: h_len, .. }] = &h[..] else { panic!("bad /H") };
    let (h_at, h_len) = (*h_at as usize, *h_len as usize);
    let hint = indirect(pdf, h_at).unwrap();
    assert_eq!(h_len, span(hint.num));
    // Offsets in the hint tables leave the hint stream out.
    let adjusted = |n: u32| if all[&n] > h_at { all[&n] - h_len } else { all[&n] };
    // /E: the end of the first page (the encryption dictionary is numbered after it).
    let encrypt = trailer.get(b"Encrypt").map(|_| reference(&trailer, b"Encrypt"));
    let encrypted = encrypt.is_some();
    let first_page_objects = (pages[0]..).take_while(|n| first_part.contains(n) && Some(*n) != encrypt).count() as u32;
    let first_end = all[&(pages[0] + first_page_objects - 1)] + span(pages[0] + first_page_objects - 1);
    assert_eq!(int(&lin.value, b"E"), first_end, "/E ends the first page");
    if encrypted {
        return Linearised { pages, first_part, encrypted };
    }
    let data_at = crate::lab_spot::find(pdf, b"stream", hint.value_span.1).unwrap() + 7;
    let data = &pdf[data_at..data_at + int(&hint.value, b"Length")];
    let mut r = BitReader { data, at: 0 };

    // The page offset hint table.
    let (min_objects, first_page_at, objects_bits) = (r.get(32), r.get(32) as usize, r.get(16));
    let (min_len, len_bits) = (r.get(32), r.get(16));
    let (_content_at, content_at_bits, _content_len, content_len_bits) = (r.get(32), r.get(16), r.get(32), r.get(16));
    let (nshared_bits, id_bits, numerator_bits, _denominator) = (r.get(16), r.get(16), r.get(16), r.get(16));
    assert_eq!(first_page_at, adjusted(pages[0]), "the first page's location");
    let n = pages.len();
    let counts: Vec<u64> = r.column(n, objects_bits).iter().map(|c| c + min_objects).collect();
    let lengths: Vec<u64> = r.column(n, len_bits).iter().map(|l| l + min_len).collect();
    let nshared = r.column(n, nshared_bits);
    let ids: Vec<Vec<u64>> = nshared.iter().map(|k| (0..*k).map(|_| r.get(id_bits)).collect()).collect();
    r.align();
    r.column(nshared.iter().sum::<u64>() as usize, numerator_bits);
    r.column(n, content_at_bits);
    r.column(n, content_len_bits);
    assert_eq!(counts[0] as u32, first_page_objects, "the first page's object count");
    assert_eq!(nshared[0], 0, "the first page lists no shared objects");
    let mut at = first_page_at;
    for (i, &page) in pages.iter().enumerate() {
        // A page's objects are numbered on from its page object, and lie together from it.
        assert_eq!(adjusted(page), at, "page {i} starts where the pages before it end");
        let len: usize = (page..page + counts[i] as u32).map(span).sum();
        assert_eq!(len as u64, lengths[i], "page {i}'s length");
        at += len;
    }

    // The shared object hint table.
    let s = int(&hint.value, b"S");
    let mut r = BitReader { data, at: 8 * s };
    let (first_shared, first_shared_at, nfirst, ntotal) = (r.get(32) as u32, r.get(32) as usize, r.get(32), r.get(32));
    let (group_bits, min_group, group_len_bits) = (r.get(16), r.get(32), r.get(16));
    assert_eq!(group_bits, 0, "one object a group");
    assert_eq!(nfirst as u32, first_page_objects);
    let shared_object = |id: u64| if id < nfirst { pages[0] + id as u32 } else { first_shared + (id - nfirst) as u32 };
    if ntotal > nfirst {
        assert_eq!(first_shared_at, adjusted(first_shared), "the shared objects' location");
    }
    let groups = r.column(ntotal as usize, group_len_bits);
    for (id, g) in groups.iter().enumerate() {
        assert_eq!((g + min_group) as usize, span(shared_object(id as u64)), "shared object {id}'s length");
    }
    assert!(r.column(ntotal as usize, 1).iter().all(|s| *s == 0), "no signatures");

    // What each page uses lies in its own objects, the first part or its shared objects.
    let page_set: HashSet<u32> = pages.iter().copied().collect();
    for (i, &page) in pages.iter().enumerate() {
        let own: HashSet<u32> = (page..page + counts[i] as u32).collect();
        let listed: HashSet<u32> = ids[i].iter().map(|id| shared_object(*id)).collect();
        let (mut seen, mut queue) = (HashSet::from([page]), vec![page]);
        while let Some(o) = queue.pop() {
            assert!(
                own.contains(&o) || first_part.contains(&o) && (i == 0 || o < pages[0] || listed.contains(&o)) || listed.contains(&o),
                "page {i} uses object {o}, which isn't among its objects or listed"
            );
            let mut links = vec![];
            objects[&o].value.refs(&[b"Parent", b"Thumb"], &mut links);
            queue.extend(links.into_iter().filter(|m| !page_set.contains(m) && !nodes.contains(m) && seen.insert(*m)));
        }
        assert!(listed.iter().all(|o| seen.contains(o)), "page {i} lists only objects it uses");
    }
    Linearised { pages, first_part, encrypted }
}

#[test]
fn thumbnails_are_embedded_on_every_page() {
    let d = doc(3, false, false);
    let mut o = options(json!({"thumbnails": true}));
    o.thumbnails = vec![thumb(106, 106, 10), thumb(53, 106, 20), thumb(106, 1, 30)];
    let r = export_with_report(&d, &o).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let parsed = crate::linearize::parse(&r.bytes).unwrap();
    for (page, t) in parsed.pages.iter().zip(&o.thumbnails) {
        let thumb = &parsed.objects[&reference(&parsed.objects[page].value, b"Thumb")];
        assert_eq!((int(&thumb.value, b"Width"), int(&thumb.value, b"Height")), (t.width as usize, t.height as usize));
        assert_eq!(thumb.value.name(b"ColorSpace"), Some(&b"DeviceRGB"[..]));
        let at = crate::lab_spot::find(&r.bytes, b"stream", thumb.value_span.1).unwrap() + 7;
        let mut rgb = vec![];
        flate2::read::ZlibDecoder::new(&r.bytes[at..at + int(&thumb.value, b"Length")]).read_to_end(&mut rgb).unwrap();
        assert_eq!(rgb, t.rgb, "the pixels as given");
    }
    // The file reads back: three pages, the thumbnails aside.
    assert_eq!(import(&r.bytes).unwrap().artboards.len(), 3);
    // Asked for without the pages drawn: none, with a warning. Thumbnails too large are refused.
    let r = export_with_report(&d, &options(json!({"thumbnails": true}))).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("thumbnails")) && crate::lab_spot::find(&r.bytes, b"/Thumb", 0).is_none());
    o.thumbnails[1] = thumb(107, 20, 0);
    assert!(matches!(export(&d, &o), Err(PdfError::BadSetting(_))));
}

#[test]
fn fast_web_view_writes_a_linearised_file_whose_hints_check() {
    for (pages, image, on_first) in [(1, false, false), (2, false, false), (3, true, false), (4, true, true)] {
        let d = doc(pages, image, on_first);
        let mut o = options(json!({"fastWebView": true, "compression": {"compressText": pages % 2 == 0}}));
        let plain = export(&d, &options(json!({"compression": {"compressText": pages % 2 == 0}}))).unwrap();
        let r = export_with_report(&d, &o).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.bytes.starts_with(b"%PDF-1.7"));
        let l = check_linearised(&r.bytes);
        assert_eq!(l.pages.len(), pages);
        assert!(l.first_part.contains(&l.pages[0]) && !l.encrypted);
        // The same pages: it reads back as the plain file does.
        let (a, b) = (import(&r.bytes).unwrap(), import(&plain).unwrap());
        assert_eq!(a.artboards.len(), b.artboards.len());
        assert_eq!(format!("{:?}", a.layers), format!("{:?}", b.layers), "{pages} pages");
        // With thumbnails and the editing data too (both after the pages).
        o.settings.thumbnails = true;
        o.settings.preserve_editing = true;
        o.native = Some(br#"{"pages":1}"#.to_vec());
        o.thumbnails = (0..pages).map(|i| thumb(20, 20, i as u8)).collect();
        let r = export_with_report(&d, &o).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        check_linearised(&r.bytes);
        let editing = editing(&r.bytes).unwrap();
        assert!(editing.intact, "the editing data's seal holds");
        assert_eq!(editing.data, br#"{"pages":1}"#);
    }
}

#[test]
fn encrypted_files_stay_linearised() {
    let d = doc(3, true, true);
    for c in ["1.4", "1.5", "1.6", "1.7", "2.0"] {
        let o = options(json!({"fastWebView": true, "compatibility": c, "security": {"openPassword": "open", "plaintextMetadata": c != "1.4"}}));
        let bytes = export(&d, &o).unwrap();
        let l = check_linearised(&bytes);
        assert!(l.encrypted && l.pages.len() == 3, "{c}");
        // The encryption dictionary is in the first part (opening needs it).
        let trailer_at = crate::lab_spot::find(&bytes, b"trailer", 0).unwrap() + 7;
        let trailer = Lexer::at(&bytes, trailer_at).object(0).unwrap();
        assert!(l.first_part.contains(&reference(&trailer, b"Encrypt")), "{c}");
        assert!(matches!(import(&bytes), Err(PdfError::NeedsPassword)), "{c}");
        let opened = import_with_report(&bytes, &ImportOptions { password: Some("open".into()), ..Default::default() }).unwrap();
        assert_eq!(opened.document.artboards.len(), 3, "{c}");
    }
}

#[test]
fn with_both_off_the_file_is_unchanged() {
    let d = doc(2, true, false);
    let off = options(json!({"thumbnails": false, "fastWebView": false}));
    let plain = export(&d, &options(json!({}))).unwrap();
    assert_eq!(export(&d, &off).unwrap(), plain, "the default export");
    // Finishing leaves the written file as it is.
    let mut warnings = vec![];
    assert_eq!(crate::post::finish(plain.clone(), &off, false, &mut warnings).unwrap(), plain);
    assert!(warnings.is_empty());
    // Thumbnails given but not asked for are left out.
    let given = PdfOptions { thumbnails: vec![thumb(10, 10, 0); 2], ..off };
    assert_eq!(export(&d, &given).unwrap(), plain);
}

#[test]
fn pdf_x_files_take_thumbnails_and_stay_linearised() {
    let d = doc(3, true, false);
    for standard in [Standard::PdfX1a, Standard::PdfX3, Standard::PdfX4] {
        let mut o = options(json!({"thumbnails": true, "fastWebView": true}));
        (o.settings.standard, o.settings.compatibility) = (standard, standard.version());
        o.thumbnails = vec![thumb(30, 30, 200); 3];
        let r = export_with_report(&d, &o).unwrap();
        let header = if standard == Standard::PdfX4 { "%PDF-1.6" } else { "%PDF-1.3" };
        assert!(r.bytes.starts_with(header.as_bytes()), "{standard:?}");
        assert_eq!(check_linearised(&r.bytes).pages.len(), 3, "{standard:?}");
        // Still the standard: grey thumbnails (no RGB in PDF/X-1a), nothing it forbids.
        crate::pdfx::verify(&r.bytes, standard).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&r.bytes).matches("/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/FlateDecode").count(),
            3,
            "{standard:?}"
        );
    }
}

#[test]
fn pdf_layers_stay_with_a_linearised_file() {
    let mut d = doc(3, true, true);
    let l = d.add_layer(Some("Notes"));
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(220.0, 20.0, 260.0, 60.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(l), 0, n).unwrap();
    let r = export_with_report(&d, &options(json!({"fastWebView": true, "createLayers": true}))).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let l = check_linearised(&r.bytes);
    // The optional content groups open the document: they come first, with the catalog.
    let text = String::from_utf8_lossy(&r.bytes);
    let at = text.find("/OCGs[").unwrap() + 6;
    let ocgs: Vec<u32> = text[at..at + text[at..].find(']').unwrap()].split(" 0 R").filter_map(|n| n.trim().parse().ok()).collect();
    assert!(ocgs.len() == 2 && ocgs.iter().all(|n| l.first_part.contains(n)), "{ocgs:?}");
    assert_eq!(l.pages.len(), 3);
    let back = import(&r.bytes).unwrap();
    assert!(back.layers.len() >= 2, "the layers come back");
}
