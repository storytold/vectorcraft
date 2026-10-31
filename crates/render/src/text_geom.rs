use super::{Affine, Arc, BezPath, Node, Rect, Shape, TextObject, lru};

/// The whole text's glyph outlines as one path, and which of them each run draws.
pub(crate) struct TextGeom {
    pub(crate) all: BezPath,
    /// Spans of `all`'s elements drawn by a run, in order: (run, start, end). Outlines of runs
    /// the text doesn't have are in `all` only.
    spans: Vec<(usize, usize, usize)>,
    pub(crate) bounds: Rect,
    /// The boxes of characters whose font is missing (drawn in a substitute), and of glyphs the
    /// font lacks (drawn from a fallback font): Highlight Substituted Fonts and Highlight
    /// Substituted Glyphs.
    pub(crate) substituted_fonts: BezPath,
    pub(crate) substituted_glyphs: BezPath,
    /// Glyphs OpenType may have substituted (a ligature, or not the face's cmap glyph for their
    /// character), each with its box: what Preferences › Highlight Alternate Glyphs weighs
    /// ([`TextGeom::alternate_glyphs`]).
    pub(crate) alternate_candidates: Vec<AlternateCandidate>,
    /// The boxes of the candidates that are alternates, measured the first time the preference
    /// shows them.
    alternate_glyphs: std::sync::OnceLock<BezPath>,
    /// Inline graphics placed by the layout.
    pub(crate) inlines: Vec<vectorcraft_text::InlineGlyph>,
    /// Colour bitmap glyphs: (run, image, its pixels → text space), in text order.
    pub(crate) images: Vec<(usize, Arc<vectorcraft_text::GlyphImage>, Affine)>,
    /// COLR colour glyphs: (run, their layers placed in text space), in text order.
    pub(crate) colours: Vec<(usize, Arc<Node>)>,
}

impl TextGeom {
    /// The outlines run `i` draws: the glyphs of `all` it has, in their order. Kept once rather
    /// than per run as well, which held every outline twice.
    pub(crate) fn run(&self, i: usize) -> std::borrow::Cow<'_, BezPath> {
        let els = self.all.elements();
        if let [(r, 0, end)] = self.spans[..]
            && r == i
            && end == els.len()
        {
            return std::borrow::Cow::Borrowed(&self.all);
        }
        let mut p = BezPath::new();
        for &(_, start, end) in self.spans.iter().filter(|s| s.0 == i) {
            p.extend(els.get(start..end).unwrap_or_default().iter().copied());
        }
        std::borrow::Cow::Owned(p)
    }

    /// The boxes of the glyphs of `t` (the text this geometry is of) that optional OpenType features
    /// chose (Preferences › Highlight Alternate Glyphs): candidates the text's required shaping
    /// alone doesn't give ([`required_shaping`]). Arabic letters joining, Indic conjuncts and the
    /// lam-alef ligature are what the script needs, not alternates (the lam-alef ligature also when
    /// a font files it under `liga`: [`is_lam_alef`]). Measured once.
    pub(crate) fn alternate_glyphs(&self, t: &TextObject) -> &BezPath {
        self.alternate_glyphs.get_or_init(|| {
            let mut out = BezPath::new();
            if self.alternate_candidates.is_empty() {
                return out;
            }
            let required = required_shaping(t);
            for c in &self.alternate_candidates {
                if !required.get(&(c.byte, c.len)).is_some_and(|glyphs| glyphs.contains(&(c.font_id, c.gid))) {
                    out.extend(c.cell.iter());
                }
            }
            out
        })
    }

    /// Approximate heap bytes, for the cache's budget.
    pub(crate) fn bytes(&self) -> usize {
        size_of::<Self>()
            + [&self.all, &self.substituted_fonts, &self.substituted_glyphs].into_iter().map(lru::path_bytes).sum::<usize>()
            + self.alternate_candidates.iter().map(|c| size_of::<AlternateCandidate>() + lru::path_bytes(&c.cell)).sum::<usize>()
            + self.alternate_glyphs.get().map_or(0, lru::path_bytes)
            + self.spans.len() * size_of::<(usize, usize, usize)>()
            + self.inlines.len() * size_of::<vectorcraft_text::InlineGlyph>()
            + self.images.len() * size_of::<(usize, Arc<vectorcraft_text::GlyphImage>, Affine)>()
            + self.colours.iter().map(|(_, art)| lru::node_bytes(art)).sum::<usize>()
    }
}

/// OpenType-substituted form: a ligature (several source characters → one glyph) or a glyph whose
/// id is not the face's cmap mapping for its first character (contextual / stylistic alternate).
pub(crate) fn is_opentype_alternate(nominal_gid: u32, shaped_gid: u32, cluster_chars: usize) -> bool {
    shaped_gid != 0 && (cluster_chars > 1 || shaped_gid != nominal_gid)
}

/// Whether `cluster` is a lam followed by an alef (plain, with madda, or with hamza above or below),
/// with any Arabic marks between or after them: the lam-alef ligature, which the Unicode
/// Standard's Arabic chapter treats as obligatory, not a stylistic choice. Fonts made before
/// `rlig` was common put it in `liga`, which [`required_shaping`] turns off, so it is required
/// by its characters whatever feature the font files it under.
pub(crate) fn is_lam_alef(cluster: &str) -> bool {
    let is_mark = |c: &char| matches!(c, '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06DC}' | '\u{06DF}'..='\u{06E4}' | '\u{06E7}' | '\u{06E8}' | '\u{06EA}'..='\u{06ED}');
    let mut letters = cluster.chars().filter(|c| !is_mark(c));
    matches!((letters.next(), letters.next(), letters.next()), (Some('\u{0644}'), Some('\u{0622}' | '\u{0623}' | '\u{0625}' | '\u{0627}'), None))
}

/// A glyph [`is_opentype_alternate`] marks, in its cluster of the text (`byte`, `len`), with its
/// box.
#[derive(Debug)]
pub(crate) struct AlternateCandidate {
    byte: usize,
    len: usize,
    font_id: u32,
    gid: u32,
    cell: BezPath,
}

/// The glyphs `t` is shaped to by required shaping alone, by cluster (`byte`, `len`) → `(font,
/// glyph)`: laid out again with every optional feature off (standard and contextual ligatures and
/// alternates, and the features its characters turn on: stylistic sets, swashes, discretionary
/// ligatures, figure styles…). What is left is what the script needs: `ccmp`, `locl`, `rlig`,
/// Arabic's `init`, `medi`, `fina` and `isol`, the Indic and other complex scripts' required sets.
fn required_shaping(t: &TextObject) -> std::collections::HashMap<(usize, usize), Vec<(u32, u32)>> {
    let mut plain = t.clone();
    for run in &mut plain.runs {
        run.style.features = vec!["-liga".into(), "-calt".into()];
    }
    let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &plain);
    let mut glyphs: std::collections::HashMap<(usize, usize), Vec<(u32, u32)>> = std::collections::HashMap::new();
    for g in &layout.glyphs {
        glyphs.entry((g.byte, g.len)).or_default().push((g.font_id, g.gid));
    }
    glyphs
}

pub(crate) fn text_geom(t: &TextObject) -> TextGeom {
    text_geom_snapped(t, None)
}

/// Glyph geometry of `t`; `snap` (text space → device pixels) puts the glyphs on whole pixels.
pub(crate) fn text_geom_snapped(t: &TextObject, snap: Option<Affine>) -> TextGeom {
    let db = vectorcraft_text::FontDb::global();
    let mut layout = vectorcraft_text::layout(db, t);
    if let Some(xf) = snap {
        layout.snap_to_pixels(xf);
    }
    let mut all = BezPath::new();
    let mut spans: Vec<(usize, usize, usize)> = vec![];
    // Append `outline` to `all`, drawn by `run` when the text has that run.
    let mut push = |all: &mut BezPath, run: usize, outline: &BezPath| {
        let start = all.elements().len();
        all.extend(outline.iter());
        let end = all.elements().len();
        if run >= t.runs.len() || start == end {
            return;
        }
        match spans.last_mut() {
            Some((r, _, e)) if *r == run && *e == start => *e = end,
            _ => spans.push((run, start, end)),
        }
    };
    // Per run: is its family missing, and the face its glyphs should come from.
    let faces: Vec<(bool, Option<u32>)> = t
        .runs
        .iter()
        .map(|r| match db.resolve(&r.style.font_family, &r.style.font_style) {
            // The version the type names, when it's installed (see `FontDb::face_version`), at its
            // Variable Font settings (see `FontDb::style_face`).
            Some((f, m)) => {
                let face = db.style_face(&r.style).unwrap_or(f);
                (m == vectorcraft_text::FontMatch::Missing, Some(face.id()))
            }
            None => (true, None),
        })
        .collect();
    let (mut substituted_fonts, mut substituted_glyphs, mut alternate_candidates) = (BezPath::new(), BezPath::new(), vec![]);
    let plain = t.plain_text();
    let (mut images, mut colours) = (vec![], vec![]);
    for g in &layout.glyphs {
        push(&mut all, g.run, &g.outline);
        if let Some((image, xf)) = g.placed_image() {
            images.push((g.run, image.clone(), xf));
        }
        if let Some(art) = g.placed_colour(Affine::IDENTITY) {
            colours.push((g.run, Arc::new(art)));
        }
        let Some(&(missing, face)) = faces.get(g.run) else { continue };
        let Some(line) = layout.lines.get(g.line) else { continue };
        let mut cell = Rect::new(g.origin.x, g.origin.y - line.ascent, g.origin.x + g.advance, g.origin.y + line.descent).to_path(0.1);
        cell.apply_affine(Affine::rotate_about(g.angle, g.origin));
        if missing {
            substituted_fonts.extend(cell.iter());
            continue;
        }
        if face.is_some_and(|f| f != g.font_id) {
            substituted_glyphs.extend(cell.iter());
            continue;
        }
        // Same face: OpenType may have swapped the glyph (liga, calt, …).
        let Some(cluster) = plain.get(g.byte..g.byte.saturating_add(g.len)) else { continue };
        let cluster_chars = cluster.chars().count();
        let Some(first) = cluster.chars().next() else { continue };
        let nominal = db.face_by_id(g.font_id).map(|f| f.glyph_for(first)).unwrap_or(0);
        if is_opentype_alternate(nominal, g.gid, cluster_chars) && !is_lam_alef(cluster) {
            alternate_candidates.push(AlternateCandidate { byte: g.byte, len: g.len, font_id: g.font_id, gid: g.gid, cell });
        }
    }
    // Ruby (furigana) glyphs: painted with their base text's run.
    for g in &layout.ruby {
        push(&mut all, g.run, &g.outline);
    }
    // Underline and strikethrough bars, painted as their run's type is (#847).
    for (run, bar) in vectorcraft_text::decorations(&layout, db, t) {
        push(&mut all, run, &bar);
    }
    TextGeom {
        all,
        spans,
        bounds: layout.bounds,
        substituted_fonts,
        substituted_glyphs,
        alternate_candidates,
        alternate_glyphs: std::sync::OnceLock::new(),
        inlines: layout.inlines,
        images,
        colours,
    }
}
