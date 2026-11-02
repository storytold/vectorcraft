use super::{
    Affine, Arc, BezPath, ClipPath, Context, Device, Glyph, GlyphDrawMode, HashMap, Image, Page, PathDrawMode, PathEl, Rect, Shape, SoftMask,
};

/// How many paths the interpreter draws on `page` (each fill and each stroke), leaving out those
/// of soft masks, pattern cells and type 3 glyphs, which [`Builder`] reads nested. `None` when the
/// interpreter fails.
pub(super) fn drawn_paths<'a>(page: &'a Page<'a>, mut ctx: Context<'a>) -> Option<usize> {
    struct Count(usize);
    impl<'a> Device<'a> for Count {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
        fn set_blend_mode(&mut self, _: craft_pdf::interpret::BlendMode) {}
        // As `Builder::draw_path` takes them: each fill and each stroke.
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &craft_pdf::interpret::Paint<'a>, _: &PathDrawMode) {
            self.0 += 1;
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: craft_pdf::interpret::BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'a>, _: Affine, _: Affine, _: &craft_pdf::interpret::Paint<'a>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let mut count = Count(0);
    craft_pdf::interpret_page(page, &mut ctx, &mut count).ok()?;
    Some(count.0)
}

/// The characters of a CID-keyed CFF font embedded without a ToUnicode map (as macOS writes
/// Hiragino): each glyph's CID from the font's charset, then Adobe's CID → Unicode table of its
/// character collection (Japan1, GB1, CNS1, Korea1).
pub(crate) struct CidText {
    /// CID of each glyph.
    cids: Vec<u16>,
    table: craft_pdf::interpret::hayro_cmap::CMap,
}

impl CidText {
    pub(crate) fn of(data: &[u8]) -> Option<Self> {
        // An embedded CFF of a few CJK glyphs is kilobytes; whole fonts are some megabytes.
        if data.len() > 64 << 20 {
            return None;
        }
        use craft_pdf::interpret::hayro_cmap::{CMap, CMapName, load_embedded};
        use skrifa::raw::ps::cff::{CffFontRef, dict, v1::Cff};
        use skrifa::raw::{FontData, FontRead};
        let cff = Cff::read(FontData::new(data)).ok()?;
        let top = cff.top_dicts().get(0)?;
        let ordering = dict::entries(top, None).filter_map(Result::ok).find_map(|e| match e {
            dict::Entry::Ros { ordering, .. } => Some(ordering),
            _ => None,
        })?;
        let name = match cff.string(ordering)? {
            b"Japan1" => CMapName::AdobeJapan1Ucs2,
            b"GB1" => CMapName::AdobeGb1Ucs2,
            b"CNS1" => CMapName::AdobeCns1Ucs2,
            b"Korea1" => CMapName::AdobeKorea1Ucs2,
            _ => return None,
        };
        let table = CMap::parse(load_embedded(name)?, load_embedded)?;
        let charset = CffFontRef::new(data, 0, None).ok()?.charset()?;
        let cids = (0..charset.num_glyphs().min(65_536)).map(|g| charset.string_id(skrifa::GlyphId::new(g)).map_or(0, |s| s.to_u16())).collect();
        Some(Self { cids, table })
    }

    pub(super) fn unicode(&self, glyph: u32) -> Option<craft_pdf::interpret::hayro_cmap::BfString> {
        let cid = *self.cids.get(glyph as usize)?;
        self.table.lookup_bf_string(u32::from(cid))
    }
}

/// A ToUnicode value that names no character: U+FFFD alone (#708).
pub(super) fn replacement(u: &craft_pdf::interpret::hayro_cmap::BfString) -> bool {
    use craft_pdf::interpret::hayro_cmap::BfString;
    match u {
        BfString::Char(c) => *c == char::REPLACEMENT_CHARACTER,
        BfString::String(s) => !s.is_empty() && s.chars().all(|c| c == char::REPLACEMENT_CHARACTER),
    }
}

/// The character of each glyph of an embedded TrueType or OpenType font program, from its own
/// `cmap` (the first character mapped to each glyph); `None` for a bare CFF or a font without one.
pub(crate) fn font_chars(data: &[u8]) -> Option<HashMap<u32, char>> {
    use skrifa::MetadataProvider;
    // As for CidText: whole fonts are some megabytes.
    if data.len() > 64 << 20 {
        return None;
    }
    let font = skrifa::FontRef::new(data).ok()?;
    let mut chars = HashMap::new();
    for (c, g) in font.charmap().mappings() {
        if let Some(c) = char::from_u32(c).filter(|c| !c.is_control()) {
            chars.entry(g.to_u32()).or_insert(c);
        }
    }
    (!chars.is_empty()).then_some(chars)
}

/// The ideographs of the Kangxi radicals U+2F00–U+2FD5 (their NFKC forms): the CID → Unicode
/// tables of some PDFs map 龍 to the radical ⿓, which looks the same but isn't the character.
const KANGXI: &str = "一丨丶丿乙亅二亠人儿入八冂冖冫几凵刀力勹匕匚匸十卜卩厂厶又口囗土士夂夊夕大女子宀寸小尢尸屮山巛工己巾干幺广廴廾弋弓彐彡彳心戈戶手支攴文斗斤方无日曰月木欠止歹殳毋比毛氏气水火爪父爻爿片牙牛犬玄玉瓜瓦甘生用田疋疒癶白皮皿目矛矢石示禸禾穴立竹米糸缶网羊羽老而耒耳聿肉臣自至臼舌舛舟艮色艸虍虫血行衣襾見角言谷豆豕豸貝赤走足身車辛辰辵邑酉釆里金長門阜隶隹雨靑非面革韋韭音頁風飛食首香馬骨高髟鬥鬯鬲鬼魚鳥鹵鹿麥麻黃黍黑黹黽鼎鼓鼠鼻齊齒龍龜龠";

/// How far glyph `o` (the PDF's, for `text`) is from the installed `face`'s glyph for it: the
/// largest difference between their boxes, in thousandths of an em. `None` when they are clearly
/// different glyphs (or the face lacks the character); `Some(0)` when there is nothing to compare
/// (several characters, a vertical form, no ink in either).
fn glyph_deviation(face: &vectorcraft_text::FontFace, text: &str, o: &craft_pdf::interpret::font::OutlineGlyph, vertical: bool) -> Option<f64> {
    let Some(c) = comparable(text, vertical) else { return Some(0.0) };
    let gid = face.glyph_for(c);
    if gid == 0 {
        return None;
    }
    let embedded = o.outline();
    let installed = vectorcraft_text::FontDb::global().outline(face, gid);
    match (embedded.elements().is_empty(), installed.elements().is_empty()) {
        (true, true) => Some(0.0),
        (false, false) => {
            let e = embedded.bounding_box();
            let k = 1000.0 / face.units_per_em();
            // Installed outlines are y-down; the PDF's glyph space is y-up.
            let i = installed.bounding_box();
            let i = Rect::new(i.x0 * k, -i.y1 * k, i.x1 * k, -i.y0 * k);
            let off = [(e.x0, i.x0), (e.x1, i.x1), (e.y0, i.y0), (e.y1, i.y1)].iter().fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
            (off <= 100.0).then_some(off)
        }
        _ => None,
    }
}

/// The face of `face`'s family that draws glyph `o` (for `text`): `face` itself, or the style whose
/// glyph is closest (a variable font's named instances, or a family whose PostScript names don't
/// tell its styles apart, draw a character differently in each style). `None`: no style of the
/// family has that glyph, so the font isn't the installed one.
pub(super) fn matching_face(
    face: &Arc<vectorcraft_text::FontFace>,
    text: &str,
    o: &craft_pdf::interpret::font::OutlineGlyph,
    vertical: bool,
) -> Option<Arc<vectorcraft_text::FontFace>> {
    let own = glyph_deviation(face, text, o, vertical);
    if own == Some(0.0) {
        return Some(face.clone());
    }
    let db = vectorcraft_text::FontDb::global();
    // Other installed versions of its family and style (a font library keeps old ones), then the
    // family's other styles.
    let versions = db.versions(face).into_iter().skip(1).take(16);
    let styles =
        db.styles(&face.family).into_iter().filter(|s| !s.eq_ignore_ascii_case(&face.style)).take(64).filter_map(|s| db.face(&face.family, &s));
    let others = versions.chain(styles);
    let mut best = own.map(|d| (d, face.clone()));
    for f in others {
        if let Some(d) = glyph_deviation(&f, text, o, vertical)
            && best.as_ref().is_none_or(|(b, _)| d < *b)
        {
            best = Some((d, f));
        }
    }
    best.map(|(_, f)| f)
}

/// The characters a glyph no mapping names is looked for among (#811): those single-byte fonts
/// encode (ASCII, Latin-1, Latin Extended-A, general punctuation, the euro and trade mark signs,
/// the fi and fl ligatures).
const IDENTIFIABLE: [std::ops::RangeInclusive<u32>; 6] =
    [0x21..=0x7E, 0xA1..=0x17F, 0x2010..=0x2044, 0x20AC..=0x20AC, 0x2122..=0x2122, 0xFB01..=0xFB02];

/// The character whose glyph in installed `face` draws `embedded` (a glyph outline in thousandths
/// of an em, y up): the first of [`IDENTIFIABLE`] with the same outline, point for point. A subset
/// font embedded without a character map still carries the font's own outlines, so a glyph whose
/// subset code names nothing is found this way. `None` when no glyph matches (another version of
/// the font, or a glyph that is no character), and for empty glyphs, which every space draws.
pub(crate) fn identify_glyph(face: &vectorcraft_text::FontFace, embedded: &BezPath) -> Option<char> {
    if embedded.elements().is_empty() {
        return None;
    }
    let k = 1000.0 / face.units_per_em();
    // Installed outlines are y-down in font units.
    let to_pdf = Affine::new([k, 0.0, 0.0, -k, 0.0, 0.0]);
    let db = vectorcraft_text::FontDb::global();
    IDENTIFIABLE.iter().cloned().flatten().filter_map(char::from_u32).find(|c| {
        let gid = face.glyph_for(*c);
        gid != 0 && same_shape(embedded, &(to_pdf * db.outline(face, gid).as_ref().clone()))
    })
}

/// Do outlines `a` and `b` draw the same segments, every point within 2/1000 em (closing
/// commands aside)?
fn same_shape(a: &BezPath, b: &BezPath) -> bool {
    let drawn = |p: &BezPath| p.elements().iter().filter(|e| !matches!(e, PathEl::ClosePath)).copied().collect::<Vec<_>>();
    let (a, b) = (drawn(a), drawn(b));
    let near = |p: kurbo::Point, q: kurbo::Point| (p - q).hypot() <= 2.0;
    a.len() == b.len()
        && a.iter().zip(&b).all(|(x, y)| match (x, y) {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => near(*p, *q),
            (PathEl::QuadTo(p1, p2), PathEl::QuadTo(q1, q2)) => near(*p1, *q1) && near(*p2, *q2),
            (PathEl::CurveTo(p1, p2, p3), PathEl::CurveTo(q1, q2, q3)) => near(*p1, *q1) && near(*p2, *q2) && near(*p3, *q3),
            _ => false,
        })
}

/// The character of glyph `text` when its outline can be compared with an installed font's: a
/// single character, and not CJK punctuation set vertically (its vertical form differs).
pub(super) fn comparable(text: &str, vertical: bool) -> Option<char> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    if vertical && matches!(c as u32, 0x2014 | 0x2015 | 0x2025 | 0x2026 | 0x3000..=0x303F | 0x30FC | 0xFE30..=0xFE4F | 0xFF00..=0xFF65) {
        return None;
    }
    Some(c)
}

pub(crate) fn unify_radical(c: char) -> char {
    let i = (c as u32).wrapping_sub(0x2F00);
    if i < 214 { KANGXI.chars().nth(i as usize).unwrap_or(c) } else { c }
}

/// The font a PDF font draws with ([`Builder::font_name`]).
#[derive(Clone)]
pub(super) struct FontInfo {
    pub(super) family: String,
    pub(super) style: String,
    /// The installed face of its PostScript name.
    pub(super) face: Option<Arc<vectorcraft_text::FontFace>>,
}
