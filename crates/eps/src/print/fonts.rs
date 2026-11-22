//! The fonts a print job shows its type in: Type 3 fonts drawn from the faces' own outlines, sent
//! with the pages that use them or once for the whole job, or the printer's own copy of each font,
//! asked for by its PostScript name.
//!
//! A face's glyphs are shown through fonts of at most 256 glyphs each: glyph `gid` is code
//! `gid % 256` of the face's font number `gid / 256`, so every page and the job's setup name each
//! glyph the same way.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::sync::Arc;

use craft_ps::write::{PathOps, push_path};
use kurbo::{BezPath, Shape};
use vectorcraft_text::FontFace;
use vectorcraft_text::embed::Embedding;

use crate::ps;

/// How a print job writes its type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrintFonts {
    /// As glyph outlines in the art: no fonts.
    #[default]
    Outlines,
    /// In the printer's copy of each font, asked for by its PostScript name: no font program is
    /// sent, and a printer without the font substitutes another.
    Resident,
    /// In fonts holding the glyphs each page shows, sent with that page.
    Subset,
    /// In fonts holding every glyph of each face the job shows, sent once before the first page.
    Complete,
}

/// Most bytes the font definitions of one page, or of a job's setup, may take.
pub(crate) const MAX_FONTS: usize = 64 << 20;

/// The glyphs a walk showed through fonts: the job's index of each face → its glyph ids.
pub(crate) type UsedGlyphs = BTreeMap<usize, BTreeSet<u32>>;

/// One face the job shows type in.
struct JobFace {
    face: Arc<FontFace>,
    /// Its type is shown through fonts (false: drawn as outlines, for its license).
    shown: bool,
}

/// The fonts of a whole job, shared by the walks of its sheets.
pub(crate) struct FontJob {
    pub mode: PrintFonts,
    /// Most bytes the font definitions of one page, or of the job's setup, may take
    /// ([`MAX_FONTS`]).
    pub max: usize,
    /// The faces in the order the job first shows them.
    faces: Vec<JobFace>,
    /// Face id ([`FontFace::id`]) → its index in `faces`.
    by_id: HashMap<u32, usize>,
    /// Glyph names in the faces' `post` or `CFF` tables, read once ([`PrintFonts::Resident`]).
    names: HashMap<(usize, u32), Option<String>>,
    /// What prints otherwise than the mode asks.
    pub warnings: Vec<String>,
    /// Notes on the mode: the fonts the printer must have.
    pub notes: Vec<String>,
}

/// The name a job's font number `chunk` of face `face` is defined and shown by.
pub(crate) fn font_name(face: usize, chunk: u32) -> String {
    format!("VCft{face}.{chunk}")
}

/// The face's name as the warnings give it.
fn display_name(face: &FontFace) -> String {
    format!("{} {}", face.family, face.style)
}

/// Is `name` a glyph name a PostScript name literal can carry as it is?
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 127 && name.bytes().all(|b| b.is_ascii_graphic() && !b"[](){}<>/%".contains(&b))
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

impl FontJob {
    pub fn new(mode: PrintFonts) -> Self {
        Self { mode, max: MAX_FONTS, faces: vec![], by_id: HashMap::new(), names: HashMap::new(), warnings: vec![], notes: vec![] }
    }

    /// The job's index of the face of glyph `gid` of face `id`, and the face's units per em, when
    /// the glyph is shown through a font; `None` when it is drawn as an outline. `elements` is the
    /// number of path elements of the glyph's outline in the layout: an outline the layout changed
    /// from the face's own is drawn as it is.
    pub fn glyph(&mut self, id: u32, gid: u32, elements: usize) -> Option<(usize, f64)> {
        let i = self.face(id)?;
        let face = self.faces.get(i)?.face.clone();
        if vectorcraft_text::FontDb::global().outline(&face, gid).elements().len() != elements {
            return None;
        }
        if self.mode == PrintFonts::Resident && self.glyph_name(i, gid).is_none() {
            push_unique(
                &mut self.warnings,
                format!("some glyphs of the font “{}” have no names to ask the printer's copy for: they print as outlines", display_name(&face)),
            );
            return None;
        }
        Some((i, face.units_per_em()))
    }

    /// The job's index of face `id` when its type is shown through fonts.
    fn face(&mut self, id: u32) -> Option<usize> {
        if let Some(i) = self.by_id.get(&id) {
            return self.faces.get(*i).filter(|f| f.shown).map(|_| *i);
        }
        let face = vectorcraft_text::FontDb::global().face_by_id(id)?;
        let name = display_name(&face);
        let shown = match (self.mode, face.embedding()) {
            (PrintFonts::Outlines, _) => false,
            (PrintFonts::Resident, _) => {
                push_unique(&mut self.notes, format!("Fonts: None: the printer needs the font “{}” ({})", face.postscript_name(), name));
                true
            }
            (PrintFonts::Subset | PrintFonts::Complete, Embedding::Forbidden) => {
                push_unique(
                    &mut self.warnings,
                    format!("the license of the font “{name}” doesn't allow sending it to the printer: its type prints as outlines"),
                );
                false
            }
            (PrintFonts::Subset, Embedding::Whole) => {
                push_unique(
                    &mut self.warnings,
                    format!("the font “{name}” may only be sent whole, and Fonts is Subset: its type prints as outlines (Complete sends it)"),
                );
                false
            }
            (PrintFonts::Subset | PrintFonts::Complete, _) => face.units_per_em() > 0.0,
        };
        let i = self.faces.len();
        self.faces.push(JobFace { face, shown });
        self.by_id.insert(id, i);
        shown.then_some(i)
    }

    /// The name of glyph `gid` of the job's face `i` that a PostScript name literal can carry.
    fn glyph_name(&mut self, i: usize, gid: u32) -> Option<String> {
        if let Some(n) = self.names.get(&(i, gid)) {
            return n.clone();
        }
        let name = self.faces.get(i).and_then(|f| f.face.glyph_name(gid)).filter(|n| plain_name(n) && n != ".notdef");
        self.names.insert((i, gid), name.clone());
        name
    }

    /// Every glyph of every face the job shows through fonts ([`PrintFonts::Complete`]).
    pub fn every_glyph(&self, used: &UsedGlyphs) -> UsedGlyphs {
        used.keys().filter_map(|i| Some((*i, (0..self.faces.get(*i)?.face.glyph_count()).collect()))).collect()
    }

    /// The fonts `used` names, as DSC resource names: the printer's fonts with
    /// [`PrintFonts::Resident`], else the fonts the job defines.
    pub fn resources(&self, used: &UsedGlyphs) -> Vec<String> {
        let mut out = vec![];
        for (i, gids) in used {
            match self.faces.get(*i) {
                Some(f) if self.mode == PrintFonts::Resident => push_unique(&mut out, f.face.postscript_name()),
                Some(_) => {
                    for chunk in gids.iter().map(|g| g >> 8).collect::<BTreeSet<_>>() {
                        push_unique(&mut out, font_name(*i, chunk));
                    }
                }
                None => {}
            }
        }
        out
    }

    /// Append the definitions of the fonts that show `used` to `s`: Type 3 fonts holding those
    /// glyphs, or, with [`PrintFonts::Resident`], the printer's fonts with an encoding naming them.
    /// An error, before `s` holds any of them, when they would take more than [`Self::max`] bytes.
    pub fn define(&mut self, used: &UsedGlyphs, s: &mut String) -> Result<(), String> {
        let mut out = String::new();
        self.define_into(used, &mut out)?;
        s.push_str(&out);
        Ok(())
    }

    fn define_into(&mut self, used: &UsedGlyphs, s: &mut String) -> Result<(), String> {
        for (i, gids) in used {
            let Some(face) = self.faces.get(*i).map(|f| f.face.clone()) else { continue };
            let mut chunks: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for g in gids {
                chunks.entry(g >> 8).or_default().push(*g);
            }
            let ps_name = face.postscript_name();
            if self.mode == PrintFonts::Resident {
                let _ = writeln!(s, "%%IncludeResource: font {ps_name}");
            }
            for (chunk, gids) in chunks {
                let name = font_name(*i, chunk);
                if self.mode == PrintFonts::Resident {
                    // The printer's font, with an encoding that gives each glyph its code.
                    let _ = writeln!(s, "/{ps_name} findfont dup length dict begin\n{{1 index /FID ne {{def}} {{pop pop}} ifelse}} forall");
                    s.push_str("/Encoding 256 array def 0 1 255 {Encoding exch /.notdef put} for\n");
                    for g in gids {
                        if let Some(n) = self.glyph_name(*i, g) {
                            let _ = writeln!(s, "Encoding {} /{n} put", g & 0xff);
                        }
                    }
                    let _ = writeln!(s, "currentdict end /{name} exch definefont pop");
                } else if !type3(s, &name, &face, &gids, self.max.saturating_sub(s.len())) {
                    return Err(too_large(&face, self.max));
                }
                if s.len() > self.max {
                    return Err(too_large(&face, self.max));
                }
            }
        }
        Ok(())
    }
}

fn too_large(face: &FontFace, max: usize) -> String {
    let size = if max >= 1 << 20 { format!("{} MB", max >> 20) } else { format!("{} KB", max >> 10) };
    format!("the fonts to send take more than {size} (the font “{}” has {} glyphs): choose Fonts: Subset", display_name(face), face.glyph_count())
}

/// Append Type 3 font `name` holding glyphs `gids` of `face` (one font number: codes `gid % 256`)
/// to `s`, drawn from the face's outlines in its font units, y up. False, with nothing added to
/// `s`, when its glyph procedures take more than `room` bytes.
fn type3(s: &mut String, name: &str, face: &FontFace, gids: &[u32], room: usize) -> bool {
    let db = vectorcraft_text::FontDb::global();
    let upem = face.units_per_em();
    let flip = kurbo::Affine::FLIP_Y;
    let mut procs = String::new();
    let mut bbox: Option<kurbo::Rect> = None;
    for g in gids {
        let outline: BezPath = flip * (*db.outline(face, *g)).clone();
        let b = if outline.elements().is_empty() { kurbo::Rect::ZERO } else { outline.bounding_box() };
        if !outline.elements().is_empty() {
            bbox = Some(bbox.map_or(b, |r| r.union(b)));
        }
        let _ = write!(procs, "/g{g} {{{} 0 ", ps::num(face.advance(*g)));
        ps::push_nums(&mut procs, &[b.x0, b.y0, b.x1, b.y1]);
        procs.push_str("setcachedevice\n");
        if !outline.elements().is_empty() {
            push_path(&mut procs, &outline, PathOps::STANDARD);
            procs.push_str("fill\n");
        }
        procs.push_str("} bind def\n");
        if procs.len() > room {
            return false;
        }
    }
    let b = bbox.unwrap_or(kurbo::Rect::ZERO);
    let _ = writeln!(s, "%%BeginResource: font {name}");
    let _ = writeln!(s, "/{name} 10 dict begin\n/FontType 3 def");
    s.push_str("/FontMatrix [");
    ps::push_num_places(s, 1.0 / upem, 9);
    s.push_str(" 0 0 ");
    ps::push_num_places(s, 1.0 / upem, 9);
    s.push_str(" 0 0] def\n/FontBBox [");
    ps::push_nums(s, &[b.x0, b.y0, b.x1, b.y1]);
    s.push_str("] def\n/Encoding 256 array def 0 1 255 {Encoding exch /.notdef put} for\n");
    for g in gids {
        let _ = writeln!(s, "Encoding {} /g{g} put", g & 0xff);
    }
    let _ = writeln!(s, "/CharProcs {} dict def\nCharProcs begin\n/.notdef {{0 0 0 0 0 0 setcachedevice}} bind def", gids.len() + 1);
    s.push_str(&procs);
    s.push_str("end\n/BuildGlyph {exch /CharProcs get exch 2 copy known not {pop /.notdef} if get exec} bind def\n");
    s.push_str("/BuildChar {1 index /Encoding get exch get 1 index /BuildGlyph get exec} bind def\n");
    s.push_str("currentdict end definefont pop\n%%EndResource\n");
    true
}
