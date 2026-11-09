//! CID-keyed fonts (PLRM 3rd ed., §5.11): CMaps built with the operators of the `CIDInit`
//! procedure set (and the predefined `Identity-H` and `Identity-V`), `composefont`, Type 0
//! fonts with `FMapType` 9 that map a show string's codes through their CMap to CIDs, and the
//! glyphs of the CIDFonts they select: Type 0 CIDFonts (Type 1 charstrings in the binary data
//! `StartData` reads, or in a `GlyphDirectory`: PLRM 3rd ed., §5.11.3, and Adobe Technical Note
//! #5014, "Adobe CMap and CIDFont Files Specification") and Type 2 CIDFonts (TrueType glyphs in
//! `sfnts`, picked through `CIDMap`). CIDs carry no characters: the glyphs are drawn from the
//! font's outlines, as a composite font's are.

use std::rc::Rc;

use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cff::index::Index;
use skrifa::raw::ps::cs::{CharstringContext, CharstringKind, CommandSink, NopFilterSink, TransformSink};
use skrifa::raw::ps::error::Error as CsError;
use skrifa::raw::types::{Fixed, GlyphId};
use vectorcraft_geom::BezPath;

use super::interp::{Interp, matrix_obj};
use super::obj::{Dict, DictRef, Key, Obj, Op, PsError, Res, Shared, ps_err};

/// Most codespace ranges, character and notdef mappings a CMap may hold.
const MAX_RANGES: usize = 1 << 16;
/// Most bytes of glyph data a Type 0 CIDFont may carry.
const MAX_GLYPH_DATA: usize = 64 << 20;
/// Most CMaps whose mappings are kept read between shows.
const MAX_CMAPS: usize = 8;
/// Longest code a codespace range may give (PLRM 3rd ed., §5.11.5: the bytes of an integer).
const MAX_CODE: usize = 4;

/// The CMap entries the CIDInit operators fill: codespace ranges (`[lo hi …]`, strings), and
/// character and notdef mappings to CIDs (`[lo hi cid font …]`, the codes as strings).
const CODESPACE: &str = "CodeSpaceRange";
const CIDS: &str = "CIDRange";
const NOTDEFS: &str = "NotDefRange";

/// The mappings of the CMaps shown with so far, by their character mapping, codespace and
/// notdef mapping arrays, the most recent last.
pub(super) type CMaps = Vec<([Option<Shared<Obj>>; 3], Rc<CMap>)>;

/// A CMap's mappings, read from its dictionary.
pub(super) struct CMap {
    /// Codespace ranges: each byte's bounds.
    space: Vec<Vec<(u8, u8)>>,
    /// Character mappings (code length, first code, last code, first CID, font), latest first.
    cids: Vec<(usize, u32, u32, u32, usize)>,
    /// Notdef mappings, latest first.
    notdefs: Vec<(usize, u32, u32, u32, usize)>,
}

/// A character of an `FMapType` 9 font: its code's descendant font and CID, and the notdef
/// mapping's CID for when that font has no glyph for it.
pub(super) struct CidChar {
    pub font: usize,
    pub cid: u32,
    pub notdef: Option<u32>,
}

impl CMap {
    /// The mappings of CMap dictionary `d`.
    fn read(d: &Dict) -> Self {
        let items = |k: &str| d.get(&Key::name(k)).and_then(|a| a.items().map(|i| i.to_vec())).unwrap_or_default();
        let bytes = |o: &Obj| match o {
            Obj::Str(s) => Some(s.to_vec()),
            _ => None,
        };
        let space = items(CODESPACE)
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[lo, hi]| {
                let (lo, hi) = (bytes(lo)?, bytes(hi)?);
                (!lo.is_empty() && lo.len() == hi.len() && lo.len() <= MAX_CODE).then(|| lo.into_iter().zip(hi).collect())
            })
            .collect();
        let maps = |k: &str| -> Vec<(usize, u32, u32, u32, usize)> {
            items(k)
                .as_chunks::<4>()
                .0
                .iter()
                .rev()
                .filter_map(|[lo, hi, cid, font]| {
                    let (lo, hi) = (bytes(lo)?, bytes(hi)?);
                    let cid = cid.as_num().filter(|c| (0.0..=f64::from(u32::MAX)).contains(c))? as u32;
                    let font = font.as_num().filter(|f| (0.0..=255.0).contains(f))? as usize;
                    (!lo.is_empty() && lo.len() == hi.len() && lo.len() <= MAX_CODE).then(|| (lo.len(), code(&lo), code(&hi), cid, font))
                })
                .collect()
        };
        Self { space, cids: maps(CIDS), notdefs: maps(NOTDEFS) }
    }

    /// The characters of show string `s` (PLRM 3rd ed., §5.11.5 "CMap Mapping" and "Handling
    /// Undefined Characters"), and how many ranges and mappings were compared to find them. It
    /// stops once that passes `limit` (the characters found so far, and the work done).
    pub fn decode(&self, s: &[u8], limit: u64) -> (Vec<CidChar>, u64) {
        let mut out = Vec::with_capacity(s.len() / 2);
        let mut work = 0u64;
        let mut i = 0;
        while let Some(rest) = s.get(i..).filter(|r| !r.is_empty() && work <= limit) {
            let matched = (1..=MAX_CODE.min(rest.len())).find(|&n| {
                work += self.space.len() as u64;
                self.space.iter().any(|r| r.len() == n && fits(r, rest))
            });
            let n = match matched {
                Some(n) => n,
                // An invalid code: as many bytes as the best partly matching range's codes.
                None => self.partial(rest),
            };
            let n = n.clamp(1, rest.len());
            let c = code(rest.get(..n).unwrap_or_default());
            let find = |maps: &[(usize, u32, u32, u32, usize)], work: &mut u64| {
                let at = maps.iter().position(|m| m.0 == n && (m.1..=m.2).contains(&c));
                *work += at.map_or(maps.len(), |k| k + 1) as u64;
                at.and_then(|k| maps.get(k)).map(|m| (m.4, m.3.saturating_add(c - m.1), m.3))
            };
            let notdef = find(&self.notdefs, &mut work).map(|(_, _, base)| base);
            out.push(match (matched.is_some(), find(&self.cids, &mut work)) {
                (true, Some((font, cid, _))) => CidChar { font, cid, notdef },
                // No character mapping: the notdef mapping's CID, else font 0's CID 0.
                _ => match find(&self.notdefs, &mut work) {
                    Some((font, _, base)) if matched.is_some() => CidChar { font, cid: base, notdef: None },
                    _ => CidChar { font: 0, cid: 0, notdef: None },
                },
            });
            i += n;
        }
        (out, work)
    }

    /// How many bytes an invalid code at the start of `rest` takes: the length of the codes of
    /// the range matching the most of its first bytes (the shortest of those), or of the
    /// shortest range when none matches its first byte.
    fn partial(&self, rest: &[u8]) -> usize {
        let prefix = |r: &[(u8, u8)]| r.iter().zip(rest).take_while(|((lo, hi), b)| (lo..=hi).contains(b)).count();
        let best = self.space.iter().map(|r| prefix(r)).max().unwrap_or(0);
        let pick = self.space.iter().filter(|r| best == 0 || prefix(r) == best).map(Vec::len).min();
        pick.unwrap_or(1)
    }
}

/// Does code `bytes` (as long as `range`, at least) start in codespace range `range`?
fn fits(range: &[(u8, u8)], bytes: &[u8]) -> bool {
    range.len() <= bytes.len() && range.iter().zip(bytes).all(|((lo, hi), b)| (lo..=hi).contains(&b))
}

/// The number a code's bytes make, the first byte highest.
fn code(bytes: &[u8]) -> u32 {
    bytes.iter().take(MAX_CODE).fold(0u32, |c, b| c << 8 | u32::from(*b))
}

/// The number `n` bytes at `at` of `data` make, the first byte highest.
fn number(data: &[u8], at: usize, n: usize) -> Option<u64> {
    let bytes = data.get(at..at.checked_add(n)?)?;
    (n <= 8).then(|| bytes.iter().fold(0u64, |v, b| v << 8 | u64::from(*b)))
}

/// A predefined Identity CMap: two-byte codes, each the CID of its own value; vertical for
/// `Identity-V`.
pub(super) fn identity(name: &str, vertical: bool) -> Obj {
    let full = || Obj::string(vec![0xff, 0xff]);
    let zero = || Obj::string(vec![0, 0]);
    let mut d = Dict::new();
    d.insert(Key::name("CMapName"), Obj::name(name));
    d.insert(Key::name("CMapType"), Obj::Int(1));
    d.insert(Key::name("WMode"), Obj::Int(i64::from(vertical)));
    d.insert(Key::name(CODESPACE), Obj::array(vec![zero(), full()]));
    d.insert(Key::name(CIDS), Obj::array(vec![zero(), full(), Obj::Int(0), Obj::Int(0)]));
    Obj::dict(d)
}

impl Interp<'_> {
    /// `composefont` and the operators of the `CIDInit` and `FontSetInit` procedure sets.
    pub fn cid_op(&mut self, op: Op) -> Res {
        use Op::*;
        match op {
            // A CMap's mappings take font 0 until its own `usefont` (Adobe Technical Note #5014).
            BeginCMap => self.cmap_font = 0,
            EndCMap | EndRearrangedFont => {}
            BeginCodeSpaceRange | BeginBfChar | BeginBfRange | BeginCidChar | BeginCidRange | BeginNotDefChar | BeginNotDefRange => {
                self.pop_int()?;
                self.push(Obj::Mark)?;
            }
            EndCodeSpaceRange => {
                let items = self.pop_mark_items()?;
                if items.len() % 2 != 0 || !items.iter().all(|o| matches!(o, Obj::Str(_))) {
                    return ps_err("typecheck", "endcodespacerange");
                }
                self.cmap_append(CODESPACE, items)?;
            }
            EndCidChar | EndCidRange | EndNotDefChar | EndNotDefRange => {
                let items = self.pop_mark_items()?;
                let per = if matches!(op, EndCidChar | EndNotDefChar) { 2 } else { 3 };
                if items.len() % per != 0 {
                    return ps_err("rangecheck", op.name());
                }
                let mut out = Vec::with_capacity(items.len() / per * 4);
                for m in items.chunks_exact(per) {
                    let (lo, hi, cid) = match m {
                        [lo, cid] => (lo, lo, cid),
                        [lo, hi, cid] => (lo, hi, cid),
                        _ => return ps_err("rangecheck", op.name()),
                    };
                    if !(matches!(lo, Obj::Str(_)) && matches!(hi, Obj::Str(_)) && cid.as_num().is_some()) {
                        return ps_err("typecheck", op.name());
                    }
                    out.extend([lo.clone(), hi.clone(), cid.clone(), Obj::Int(self.cmap_font)]);
                }
                self.cmap_append(if matches!(op, EndCidChar | EndCidRange) { CIDS } else { NOTDEFS }, out)?;
            }
            // Mappings to character codes or names in base fonts aren't read: their codes take
            // font 0's CID 0, as an unmapped code does.
            EndBfChar | EndBfRange => {
                self.pop_mark_items()?;
            }
            UseFont => {
                let n = self.pop_int()?;
                self.cmap_font = n.clamp(0, 255);
            }
            UseCMap => {
                let name = self.pop()?;
                let parent = self.find_cmap(&name)?;
                let d = self.dicts.last().cloned().ok_or(PsError::Ps("dictstackunderflow", "usecmap".into()))?;
                for k in [CODESPACE, CIDS, NOTDEFS] {
                    // Both arrays are copied into a new one: charged by its size first, as
                    // `cmap_append` charges its copy, so `usecmap` of a large CMap in a loop runs
                    // out of the budgets instead of the memory.
                    let len = |c: &DictRef| c.borrow().get(&Key::name(k)).and_then(|a| a.items().map(|i| i.len())).unwrap_or(0);
                    let n = len(&parent).saturating_add(len(&d));
                    if n > MAX_RANGES * 4 {
                        return Err(PsError::Limit("a CMap has too many mappings"));
                    }
                    self.alloc(n.saturating_mul(std::mem::size_of::<Obj>()))?;
                    self.spend_bytes(n.saturating_mul(std::mem::size_of::<Obj>()))?;
                    let theirs = parent.borrow().get(&Key::name(k)).and_then(|a| a.items().map(|i| i.to_vec())).unwrap_or_default();
                    let ours = d.borrow().get(&Key::name(k)).and_then(|a| a.items().map(|i| i.to_vec())).unwrap_or_default();
                    // The parent's mappings first: the CMap's own come later and win.
                    let all: Vec<Obj> = theirs.into_iter().chain(ours).collect();
                    if all.len() > MAX_RANGES * 4 {
                        return Err(PsError::Limit("a CMap has too many mappings"));
                    }
                    self.insert(&d, Key::name(k), Some(Obj::array(all)))?;
                }
                let w = parent.borrow().get(&Key::name("WMode")).cloned();
                if let Some(w) = w
                    && !d.borrow().contains_key(&Key::name("WMode"))
                {
                    self.insert(&d, Key::name("WMode"), Some(w))?;
                }
            }
            BeginRearrangedFont => {
                self.pop()?;
                self.pop()?;
            }
            BeginUseMatrix => {
                self.pop_int()?;
            }
            EndUseMatrix => {
                self.pop()?;
            }
            StartData => self.start_data()?,
            ComposeFont => self.compose_font()?,
            _ => return ps_err("undefined", op.name()),
        }
        Ok(())
    }

    /// The operands down to the mark that a `begin…` operator pushed.
    fn pop_mark_items(&mut self) -> Res<Vec<Obj>> {
        let at = self.stack.iter().rposition(|o| matches!(o, Obj::Mark)).ok_or(PsError::Ps("unmatchedmark", String::new()))?;
        self.spend((self.stack.len() - at) as u64)?;
        let items = self.stack.split_off(at + 1);
        self.stack.pop();
        Ok(items)
    }

    /// `items` added to the CMap being defined (the current dictionary) under `key`.
    fn cmap_append(&mut self, key: &str, items: Vec<Obj>) -> Res {
        let d = self.dicts.last().cloned().ok_or(PsError::Ps("dictstackunderflow", key.into()))?;
        let mut all = d.borrow().get(&Key::name(key)).and_then(|a| a.items().map(|i| i.to_vec())).unwrap_or_default();
        if all.len().saturating_add(items.len()) > MAX_RANGES * 4 {
            return Err(PsError::Limit("a CMap has too many mappings"));
        }
        self.alloc(items.len() * std::mem::size_of::<Obj>())?;
        // The mappings so far are copied too: a CMap of many small blocks pays for each copy.
        self.spend_bytes(all.len().saturating_mul(std::mem::size_of::<Obj>()))?;
        all.extend(items);
        self.insert(&d, Key::name(key), Some(Obj::array(all)))
    }

    /// CMap `name` (a name, or a CMap dictionary itself).
    pub(super) fn find_cmap(&mut self, name: &Obj) -> Res<DictRef> {
        if let Obj::Dict(d) = name {
            return Ok(d.clone());
        }
        let key = name.key().ok_or(PsError::Ps("typecheck", "CMap".into()))?;
        match self.instances(&Obj::name("CMap"))?.borrow().get(&key) {
            Some(Obj::Dict(d)) => Ok(d.clone()),
            _ => Err(PsError::Ps("undefinedresource", name.text().map(|t| t.to_string()).unwrap_or_default())),
        }
    }

    /// CIDFont `name` (a name, or a dictionary itself): one the program defined, in the CIDFont
    /// category or as a font, else a CIDFont of that name without glyphs (its glyphs show
    /// nothing).
    fn find_cid_font(&mut self, name: &Obj) -> Res<DictRef> {
        if let Obj::Dict(d) = name {
            return Ok(d.clone());
        }
        let key = name.key().ok_or(PsError::Ps("typecheck", "composefont".into()))?;
        if let Some(Obj::Dict(d)) = self.instances(&Obj::name("CIDFont"))?.borrow().get(&key) {
            return Ok(d.clone());
        }
        if let Some(Obj::Dict(d)) = self.fonts.borrow().get(&key) {
            return Ok(d.clone());
        }
        let mut d = Dict::new();
        d.insert(Key::name("CIDFontName"), key.obj());
        d.insert(Key::name("CIDFontType"), Obj::Int(0));
        d.insert(Key::name("FontType"), Obj::Int(9));
        d.insert(Key::name("FontMatrix"), matrix_obj(vectorcraft_geom::Affine::scale(0.001)));
        Ok(DictRef::new(d))
    }

    /// `key cmap [fonts] composefont font`: a Type 0 font with `FMapType` 9 over `cmap` and the
    /// CIDFonts (or fonts) named, defined as a font (PLRM 3rd ed., `composefont` in chapter 8).
    fn compose_font(&mut self) -> Res {
        let fonts = self.pop()?;
        let cmap = self.pop()?;
        let key = self.pop()?.key().ok_or(PsError::Ps("typecheck", "composefont".into()))?;
        let names = fonts.items().map(|i| i.to_vec()).ok_or(PsError::Ps("typecheck", "composefont".into()))?;
        let cmap = self.find_cmap(&cmap)?;
        let mut deps = Vec::with_capacity(names.len());
        for n in &names {
            deps.push(Obj::Dict(self.find_cid_font(n)?));
        }
        let mut d = Dict::new();
        d.insert(Key::name("FontName"), key.obj());
        d.insert(Key::name("FontType"), Obj::Int(0));
        d.insert(Key::name("FMapType"), Obj::Int(9));
        d.insert(Key::name("FontMatrix"), matrix_obj(vectorcraft_geom::Affine::IDENTITY));
        d.insert(Key::name("Encoding"), Obj::array((0..deps.len() as i64).map(Obj::Int).collect()));
        d.insert(Key::name("FDepVector"), Obj::array(deps));
        if let Some(w) = cmap.borrow().get(&Key::name("WMode")).cloned() {
            d.insert(Key::name("WMode"), w);
        }
        d.insert(Key::name("CMap"), Obj::Dict(cmap));
        let font = DictRef::new(d);
        self.fonts.borrow_mut().insert(key, Obj::Dict(font.clone()));
        self.push(Obj::Dict(font))
    }

    /// `StartData`: `(Binary) n StartData` in a Type 0 CIDFont file reads its binary data section
    /// (`n` bytes after the space that ends the name, or `n` bytes in hexadecimal for `(Hex)`)
    /// into the CIDFont's `GlyphData`, defines the CIDFont (the current dictionary) as a resource
    /// and ends it and the CIDInit dictionary under it (Adobe Technical Note #5014, §4.3). A
    /// FontSet's `/name n StartData` (Compact Font Format data: PLRM 3rd ed., §5.8.1) defines its
    /// CID-keyed fonts as CIDFonts (see [`Self::font_set`]); its other fonts are set as type by
    /// name.
    fn start_data(&mut self) -> Res {
        // Glyph data is often larger than the most other operators take (`pop_count`).
        let n = self.pop_int()?;
        let kind = self.pop()?;
        let f = self.running.last().cloned().unwrap_or_else(|| Rc::new(std::cell::RefCell::new(super::data::Stream::Current)));
        if n < 0 {
            return ps_err("rangecheck", "StartData");
        }
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        if n > MAX_GLYPH_DATA {
            return Err(PsError::Limit("a font's data is too large"));
        }
        self.alloc(n)?;
        self.spend((n / 256) as u64)?;
        let hex = match &kind {
            Obj::Str(s) => s.to_vec() == b"Hex",
            _ => false,
        };
        let data = if hex {
            let d = self.read_hex(&f, n)?;
            // The data can end with the `>` of a hexadecimal string.
            self.skip_hex_end(&f)?;
            d
        } else {
            self.read(&f, n)?
        };
        if !matches!(kind, Obj::Str(_)) {
            return self.font_set(data);
        }
        let d = self.dicts.last().cloned().ok_or(PsError::Ps("dictstackunderflow", "StartData".into()))?;
        self.insert(&d, Key::name("GlyphData"), Some(Obj::string(data)))?;
        // A Type 0 CIDFont without a matrix of its own: the 1000-unit glyph space of Type 1
        // fonts, and each FDArray matrix scaled to it (PLRM 3rd ed., §5.11.3).
        // The changes go through the journal, so `restore` undoes them as it does the program's.
        if !d.borrow().contains_key(&Key::name("FontMatrix")) {
            self.insert(&d, Key::name("FontMatrix"), Some(matrix_obj(vectorcraft_geom::Affine::scale(0.001))))?;
            let fds = d.borrow().get(&Key::name("FDArray")).and_then(|a| a.items().map(|i| i.to_vec())).unwrap_or_default();
            for fd in fds {
                if let Obj::Dict(fd) = fd {
                    let m = fd.borrow().get(&Key::name("FontMatrix")).and_then(|o| o.items().and_then(|i| super::interp::matrix_of(&i.borrow())));
                    if let Some(m) = m {
                        self.insert(&fd, Key::name("FontMatrix"), Some(matrix_obj(vectorcraft_geom::Affine::scale(1000.0) * m)))?;
                    }
                }
            }
        }
        let name = d.borrow().get(&Key::name("CIDFontName")).and_then(Obj::key);
        if let Some(k) = name {
            self.instances(&Obj::name("CIDFont"))?.borrow_mut().insert(k, Obj::Dict(d.clone()));
        }
        if self.dicts.len() > 3 && self.dicts.last().is_some_and(|t| t.same(&d)) {
            self.dicts.pop();
            let cid_init = self.instances(&Obj::name("ProcSet"))?.borrow().get(&Key::name("CIDInit")).cloned();
            if let (Some(Obj::Dict(p)), Some(top)) = (cid_init, self.dicts.last())
                && top.same(&p)
                && self.dicts.len() > 3
            {
                self.dicts.pop();
            }
        }
        Ok(())
    }

    /// The CID-keyed fonts of FontSet data `data` (Compact Font Format: Adobe Technical Note
    /// #5176, a top DICT with `ROS`), each defined as a Type 0 CIDFont of its name in the CFF's
    /// Name INDEX, in glyph space of its units an em, whose glyphs are its Type 2 charstrings
    /// picked by the CIDs its charset gives them.
    fn font_set(&mut self, data: Vec<u8>) -> Res {
        let data: Rc<[u8]> = data.into();
        for index in 0..MAX_FONT_SET {
            let Ok(font) = CffFontRef::new_cff(&data, index, None) else { break };
            let Some(name) = font.metadata().and_then(|m| m.name().map(str::to_string)) else { continue };
            if !font.is_cid() || self.cff.len() >= MAX_CFF {
                continue;
            }
            let upem = f64::from(font.upem());
            let upem = if upem.is_finite() && upem >= 1.0 { upem } else { 1000.0 };
            let mut d = Dict::new();
            d.insert(Key::name("CIDFontName"), Obj::name(&name));
            d.insert(Key::name("CIDFontType"), Obj::Int(0));
            d.insert(Key::name("FontType"), Obj::Int(9));
            d.insert(Key::name("FontMatrix"), matrix_obj(vectorcraft_geom::Affine::scale(1.0 / upem)));
            let d = DictRef::new(d);
            self.instances(&Obj::name("CIDFont"))?.borrow_mut().insert(Key::name(&name), Obj::Dict(d.clone()));
            self.cff.push((d, Rc::new(Cff::new(data.clone(), index))));
        }
        Ok(())
    }

    /// The mappings of CMap dictionary `d` (read once while it stays the same).
    pub(super) fn cmap(&mut self, d: &DictRef) -> Res<Rc<CMap>> {
        let array = |k: &str| d.borrow().get(&Key::name(k)).and_then(|a| a.items().cloned());
        let key = [array(CIDS), array(CODESPACE), array(NOTDEFS)];
        let same = |a: &Option<Shared<Obj>>, b: &Option<Shared<Obj>>| match (a, b) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if let Some((_, m)) = self.cmaps.iter().find(|(k, _)| k.iter().zip(&key).all(|(a, b)| same(a, b))) {
            return Ok(m.clone());
        }
        let m = Rc::new(CMap::read(&d.borrow()));
        self.spend((m.cids.len() + m.space.len() + m.notdefs.len()) as u64)?;
        if self.cmaps.len() >= MAX_CMAPS {
            self.cmaps.remove(0);
        }
        self.cmaps.push((key, m.clone()));
        Ok(m)
    }

    /// Glyph `cid` of CIDFont `font`: its outline in the CIDFont's glyph space and its width
    /// there, or `None` when the font has no glyph for it (or isn't a CIDFont with glyphs).
    pub(super) fn cid_glyph(&mut self, font: &DictRef, cid: u32) -> Res<Option<(BezPath, f64)>> {
        let (kind, count) = {
            let f = font.borrow();
            let num = |k: &str| f.get(&Key::name(k)).and_then(Obj::as_num);
            (num("CIDFontType"), num("CIDCount"))
        };
        if count.is_some_and(|c| f64::from(cid) >= c) {
            return Ok(None);
        }
        let glyph = match kind {
            Some(2.0) => self.cid_type2(font, cid)?,
            Some(0.0) => self.cid_type0(font, cid)?,
            _ => None,
        };
        // Widths from the font's `Metrics` override the glyph's own (PLRM 3rd ed., §5.9.2).
        Ok(glyph.map(|(outline, w)| {
            let m = font.borrow().get(&Key::name("Metrics")).cloned();
            let w = match m {
                Some(Obj::Dict(m)) => match m.borrow().get(&Key::Int(i64::from(cid))) {
                    Some(o) => match o.as_num() {
                        Some(v) => v,
                        None => o
                            .items()
                            .and_then(|i| match i.len() {
                                2 => i.get(1).and_then(|v| v.as_num()),
                                4 => i.get(2).and_then(|v| v.as_num()),
                                _ => None,
                            })
                            .unwrap_or(w),
                    },
                    None => w,
                },
                _ => w,
            };
            (outline, if w.is_finite() { w } else { 0.0 })
        }))
    }

    /// Glyph `cid` of a Type 2 CIDFont: the TrueType glyph its `CIDMap` gives it (a string, or
    /// strings, of `GDBytes`-byte glyph indices by CID; a dictionary; or an integer added to the
    /// CID: PLRM 3rd ed., §5.11.3), its width in ems.
    fn cid_type2(&mut self, font: &DictRef, cid: u32) -> Res<Option<(BezPath, f64)>> {
        let Some(data) = self.type42_data(font)? else { return Ok(None) };
        let gid = {
            let f = font.borrow();
            let gd = f.get(&Key::name("GDBytes")).and_then(Obj::as_num).filter(|n| (1.0..=4.0).contains(n)).map_or(2, |n| n as usize);
            match f.get(&Key::name("CIDMap")) {
                None => Some(u64::from(cid)),
                Some(Obj::Int(off)) => u64::try_from(i64::from(cid).saturating_add(*off)).ok(),
                Some(Obj::Dict(d)) => d.borrow().get(&Key::Int(i64::from(cid))).and_then(Obj::as_num).filter(|g| *g >= 0.0).map(|g| g as u64),
                Some(Obj::Str(s)) => (cid as usize).checked_mul(gd).and_then(|at| number(&s.borrow(), at, gd)),
                Some(Obj::Array { items, .. }) => {
                    // The strings one after another, each a whole number of entries.
                    let Some(mut at) = (cid as usize).checked_mul(gd) else { return Ok(None) };
                    let mut found = None;
                    for s in items.borrow().iter() {
                        let Obj::Str(s) = s else { continue };
                        let s = s.borrow();
                        let len = s.len() - s.len() % gd;
                        if at < len {
                            found = number(&s, at, gd);
                            break;
                        }
                        at -= len;
                    }
                    found
                }
                Some(_) => None,
            }
        };
        let Some(gid) = gid.and_then(|g| u32::try_from(g).ok()).filter(|g| *g <= u32::from(u16::MAX)) else { return Ok(None) };
        if gid == 0 && cid != 0 {
            return Ok(None);
        }
        Ok(super::type42::glyph(&data.data, gid))
    }

    /// Glyph `cid` of a Type 0 CIDFont: the Type 1 charstring its `GlyphDirectory` entry or
    /// its binary data section's offset table gives it, run with the subroutines of its
    /// FDArray dictionary's `Private` dictionary, in the FDArray dictionary's glyph space under
    /// the CIDFont's; its width in the CIDFont's glyph space.
    fn cid_type0(&mut self, font: &DictRef, cid: u32) -> Res<Option<(BezPath, f64)>> {
        if let Some(cff) = self.cff.iter().find(|(k, _)| k.same(font)).map(|(_, c)| c.clone()) {
            self.spend(64)?;
            let read = cff.gids_read().is_some();
            let glyph = cff.glyph(cid);
            self.spend(cff.take_work())?;
            // Reading the charset visits each of its glyphs.
            if !read {
                self.spend(cff.gids_read().unwrap_or(0) as u64)?;
            }
            return Ok(glyph);
        }
        let f = font.borrow();
        let num = |k: &str, default: f64| f.get(&Key::name(k)).and_then(Obj::as_num).unwrap_or(default);
        let fd_bytes = num("FDBytes", 0.0);
        let gd_bytes = num("GDBytes", 0.0);
        if !(0.0..=4.0).contains(&fd_bytes) || !(0.0..=4.0).contains(&gd_bytes) {
            return Ok(None);
        }
        let (fd_bytes, gd_bytes) = (fd_bytes as usize, gd_bytes as usize);
        let data: Option<Shared<u8>> = match f.get(&Key::name("GlyphData")) {
            Some(Obj::Str(s)) => Some(s.clone()),
            _ => None,
        };
        // The FDArray index and the charstring.
        let found: Option<(usize, Vec<u8>)> = match f.get(&Key::name("GlyphDirectory")) {
            Some(dir @ (Obj::Array { .. } | Obj::Dict(_))) => {
                let entry = match dir {
                    Obj::Dict(d) => d.borrow().get(&Key::Int(i64::from(cid))).cloned(),
                    _ => dir.items().and_then(|i| i.get(cid as usize)),
                };
                match entry {
                    Some(Obj::Str(s)) => {
                        let s = s.to_vec();
                        let fd = number(&s, 0, fd_bytes).unwrap_or(0) as usize;
                        Some((fd, s.get(fd_bytes..).unwrap_or_default().to_vec()))
                    }
                    _ => None,
                }
            }
            _ => data.as_ref().and_then(|data| {
                let data = data.borrow();
                let map = num("CIDMapOffset", 0.0);
                if !(0.0..=data.len() as f64).contains(&map) {
                    return None;
                }
                let at = (map as usize).checked_add((cid as usize).checked_mul(fd_bytes + gd_bytes)?)?;
                let fd = if fd_bytes == 0 { 0 } else { number(&data, at, fd_bytes)? as usize };
                let start = number(&data, at + fd_bytes, gd_bytes)? as usize;
                let end = number(&data, at + 2 * fd_bytes + gd_bytes, gd_bytes)? as usize;
                Some((fd, data.get(start..end)?.to_vec()))
            }),
        };
        let Some((fd, charstring)) = found.filter(|(_, c)| !c.is_empty()) else { return Ok(None) };
        let Some(Obj::Dict(fd)) = f.get(&Key::name("FDArray")).and_then(|a| a.items().and_then(|i| i.get(fd))) else { return Ok(None) };
        drop(f);
        let fd = fd.borrow();
        let fdm = fd
            .get(&Key::name("FontMatrix"))
            .and_then(|o| o.items().and_then(|i| super::interp::matrix_of(&i.borrow())))
            .unwrap_or(vectorcraft_geom::Affine::IDENTITY);
        let Some(Obj::Dict(private)) = fd.get(&Key::name("Private")).cloned() else { return Ok(None) };
        drop(fd);
        let len_iv = private.borrow().get(&Key::name("lenIV")).and_then(Obj::as_num).unwrap_or(4.0);
        let subrs = self.cid_subrs(&private, data.as_ref(), len_iv)?;
        self.spend((charstring.len() as u64).saturating_add(64))?;
        let charstring = decrypt(&charstring, len_iv);
        let (glyph, work) = type1_glyph(&charstring, &subrs);
        // The subroutines it ran, which the evaluator limits for each glyph, not for the file.
        self.spend(work)?;
        let Some((outline, width)) = glyph else { return Ok(None) };
        let mut outline = outline;
        outline.apply_affine(fdm);
        let w = fdm * vectorcraft_geom::Point::new(width, 0.0) - fdm * vectorcraft_geom::Point::ZERO;
        Ok(Some((outline, w.x)))
    }

    /// The subroutines of a Type 0 CIDFont's FDArray `Private` dictionary `private`, decrypted
    /// with `len_iv`: its `Subrs` array, else the subroutine table of the CIDFont's binary data
    /// `data` (Adobe Technical Note #5014). Read once while they stay the same, and counted
    /// against the budgets before they are copied.
    fn cid_subrs(&mut self, private: &DictRef, data: Option<&Shared<u8>>, len_iv: f64) -> Res<Rc<Vec<Vec<u8>>>> {
        let same_data = |d: &Option<Shared<u8>>| match (d, data) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if let Some(i) = self.cid_subrs.iter().position(|(p, d, _)| p.same(private) && same_data(d)) {
            let hit = self.cid_subrs.remove(i);
            let subrs = hit.2.clone();
            self.cid_subrs.push(hit);
            return Ok(subrs);
        }
        let p = private.borrow();
        // Each subroutine's bytes: a string of `Subrs`, or a range of the binary data.
        let strings: Option<Vec<Option<Shared<u8>>>> = match p.get(&Key::name("Subrs")) {
            Some(Obj::Array { items, .. }) => Some(
                items
                    .borrow()
                    .iter()
                    .map(|s| match s {
                        Obj::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        };
        let mut ranges: Vec<Option<(usize, usize)>> = Vec::new();
        if strings.is_none() {
            let num = |k: &str| p.get(&Key::name(k)).and_then(Obj::as_num).filter(|n| *n >= 0.0);
            if let (Some(map), Some(sd), Some(count), Some(data)) = (num("SubrMapOffset"), num("SDBytes"), num("SubrCount"), data) {
                let data = data.borrow();
                let (map, sd, count) = (map as usize, (sd as usize).min(4), (count as usize).min(MAX_RANGES));
                for i in 0..count {
                    let at = map.saturating_add(i.saturating_mul(sd));
                    let range = number(&data, at, sd).zip(number(&data, at.saturating_add(sd), sd));
                    ranges.push(range.map(|(a, b)| (a as usize, b as usize)).filter(|(a, b)| a <= b && *b <= data.len()));
                }
            }
        }
        drop(p);
        let total = match &strings {
            Some(strings) => strings.iter().flatten().fold(0usize, |t, s| t.saturating_add(s.len())),
            None => ranges.iter().flatten().fold(0usize, |t, (a, b)| t.saturating_add(b - a)),
        };
        if total > MAX_GLYPH_DATA {
            return Err(PsError::Limit("a font's subroutines are too large"));
        }
        self.alloc(total)?;
        self.spend_bytes(total)?;
        self.spend((strings.as_ref().map_or(ranges.len(), Vec::len) / 16) as u64)?;
        let subrs: Vec<Vec<u8>> = match (&strings, data) {
            (Some(strings), _) => strings.iter().map(|s| s.as_ref().map(|s| decrypt(&s.borrow(), len_iv)).unwrap_or_default()).collect(),
            (None, Some(data)) => {
                let data = data.borrow();
                ranges.iter().map(|r| r.and_then(|(a, b)| data.get(a..b)).map(|s| decrypt(s, len_iv)).unwrap_or_default()).collect()
            }
            (None, None) => vec![],
        };
        let subrs = Rc::new(subrs);
        if self.cid_subrs.len() >= MAX_SUBR_SETS {
            self.cid_subrs.remove(0);
        }
        self.cid_subrs.push((private.clone(), data.cloned(), subrs.clone()));
        Ok(subrs)
    }
}

/// Most fonts read from one FontSet.
const MAX_FONT_SET: u32 = 64;
/// Most CIDFonts read from FontSets.
const MAX_CFF: usize = 256;

/// A CID-keyed font in Compact Font Format data: the data and the font's index in it, and the
/// glyph its charset gives each CID (read when first asked).
pub(super) struct Cff {
    data: Rc<[u8]>,
    index: u32,
    gids: std::cell::OnceCell<std::collections::HashMap<u16, GlyphId>>,
    /// The charstring bytes glyphs entered, not yet charged.
    work: std::cell::Cell<u64>,
}

impl Cff {
    pub(super) fn new(data: Rc<[u8]>, index: u32) -> Self {
        Self { data, index, gids: std::cell::OnceCell::new(), work: std::cell::Cell::new(0) }
    }

    /// Whether the CID-to-glyph map has been read, and the glyphs it holds.
    fn gids_read(&self) -> Option<usize> {
        self.gids.get().map(std::collections::HashMap::len)
    }

    /// Glyph `cid`: the glyph the charset gives that CID, drawn from its Type 2 charstring with
    /// the subroutines and widths of its FDSelect subfont, in font units; `None` when the font
    /// has no glyph for it.
    pub(super) fn glyph(&self, cid: u32) -> Option<(BezPath, f64)> {
        let font = CffFontRef::new_cff(&self.data, self.index, None).ok()?;
        // The charset read once into a map, instead of searched for each glyph.
        let gids = self.gids.get_or_init(|| font.charset().map(|c| c.iter().map(|(g, sid)| (sid.to_u16(), g)).collect()).unwrap_or_default());
        let gid = *gids.get(&u16::try_from(cid).ok()?)?;
        if gid.to_u32() == 0 && cid != 0 {
            return None;
        }
        let sub = font.subfont(font.subfont_index(gid)?, &[]).ok()?;
        // Run as `CffFontRef::draw` does, through the counting context, so the subroutines a
        // glyph enters (up to a million operations each time) are paid for.
        let subrs = match sub.subrs_offset() {
            0 => Index::Empty,
            at => Index::new(self.data.get(at as usize..)?, false).ok()?,
        };
        let charstrings = font.charstrings().clone();
        let data = charstrings.get(gid.to_u32() as usize)?;
        let ctx = (&self.data[..], &charstrings, font.global_subrs(), &subrs);
        let mut pen = super::type42::Pen(BezPath::new(), 1.0);
        let transform = font.transform(&sub, None);
        let mut filter = NopFilterSink::new(&mut pen);
        let mut sink = TransformSink::new(&mut filter, transform);
        let (w, work) = super::type1::evaluate(&ctx, data, &mut sink);
        self.work.set(self.work.get().saturating_add(work));
        let w = w.ok()?.map(|w| w + sub.nominal_width()).or(sub.default_width());
        let w = w.map_or(0.0, |w| f64::from(transform.transform_h_metric(w).to_f32().max(0.0)));
        Some((pen.0, if w.is_finite() { w } else { 0.0 }))
    }

    /// The charstring bytes entered since the last call (see [`super::type1::evaluate`]).
    pub(super) fn take_work(&self) -> u64 {
        self.work.take()
    }
}

/// Most FDArray dictionaries whose subroutines are kept decrypted between glyphs.
const MAX_SUBR_SETS: usize = 32;

/// The decrypted subroutines of FDArray `Private` dictionaries, by the dictionary and the
/// CIDFont's binary data (see [`Interp::cid_subrs`]), the most recently used last.
pub(super) type CidSubrs = Vec<(DictRef, Option<Shared<u8>>, Rc<Vec<Vec<u8>>>)>;

/// Type 1 charstring `data` decrypted (the charstring key 4330, the first `len_iv` bytes
/// dropped: Adobe Type 1 Font Format, §7.2); as it is when `len_iv` is negative.
pub(super) fn decrypt(data: &[u8], len_iv: f64) -> Vec<u8> {
    if len_iv < 0.0 {
        return data.to_vec();
    }
    let mut r: u16 = 4330;
    let plain = data.iter().map(|&c| {
        let p = c ^ (r >> 8) as u8;
        r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
        p
    });
    plain.skip(len_iv.min(64.0) as usize).collect()
}

/// The outline (glyph space) and width of decrypted Type 1 charstring `data`, with
/// subroutines `subrs`, or `None` when it can't be run; and the bytes of the charstrings it
/// entered (see [`super::type1::evaluate`]).
pub(super) fn type1_glyph(data: &[u8], subrs: &[Vec<u8>]) -> (Option<(BezPath, f64)>, u64) {
    let ctx = Type1 { subrs };
    let mut sink = Sink(BezPath::new());
    let (w, work) = super::type1::evaluate(&ctx, data, &mut sink);
    (w.ok().map(|w| (sink.0, w.map_or(0.0, |w| f64::from(w.to_f32())))), work)
}

/// What a Type 1 charstring runs with: its font's subroutines (no `seac` accents: a CIDFont's
/// glyphs have no names to build them from).
struct Type1<'a> {
    subrs: &'a [Vec<u8>],
}

impl CharstringContext for Type1<'_> {
    fn kind(&self) -> CharstringKind {
        CharstringKind::Type1
    }
    fn seac_components(&self, base: i32, _accent: i32) -> Result<[&[u8]; 2], CsError> {
        Err(CsError::InvalidSeacCode(base))
    }
    fn global_subr(&self, _index: i32) -> Result<&[u8], CsError> {
        Err(CsError::MissingSubroutines)
    }
    fn subr(&self, index: i32) -> Result<&[u8], CsError> {
        usize::try_from(index).ok().and_then(|i| self.subrs.get(i)).map(Vec::as_slice).ok_or(CsError::MissingSubroutines)
    }
}

/// A charstring's path, in glyph space.
struct Sink(BezPath);

impl CommandSink for Sink {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.move_to((f64::from(x.to_f32()), f64::from(y.to_f32())));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.line_to((f64::from(x.to_f32()), f64::from(y.to_f32())));
    }
    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        let p = |a: Fixed, b: Fixed| (f64::from(a.to_f32()), f64::from(b.to_f32()));
        self.0.curve_to(p(cx0, cy0), p(cx1, cy1), p(x, y));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}
