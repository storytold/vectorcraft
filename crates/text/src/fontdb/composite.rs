//! Composite fonts (Type › Composite Fonts…) in the font database: each is listed first in the
//! font menus and resolves to its base (Kanji) font, and layout sets each character of type in one
//! in the font of the character's class ([`FontDb::composite_split`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use unicode_segmentation::UnicodeSegmentation;
use vectorcraft_doc::{CharClass, CompositeFont, TextObject, TextRun};

use super::face::FALLBACK_FAMILY;
use super::{Arc, FontDb, FontFace, FontMatch};

/// A face [`FontDb::resolve`] found and how it matched.
type Resolved = Option<(Arc<FontFace>, FontMatch)>;

thread_local! {
    /// The composite fonts the guards of this thread lend ([`FontDb::lend_composite_fonts`]), the
    /// latest loan last.
    static LENT: RefCell<Vec<Loan>> = const { RefCell::new(Vec::new()) };
}

/// The number of the next loan.
static NEXT_LOAN: AtomicU64 = AtomicU64::new(1);

/// The composite fonts one guard lends, on the thread that made it.
struct Loan {
    /// The lending database's [`FontDb::id`].
    db: u64,
    /// Tells this loan from the others.
    number: u64,
    /// The fonts, by lowercased name.
    fonts: HashMap<String, Arc<CompositeFont>>,
    /// The database's generation when they were lent, when each was then the font the database
    /// holds by its name.
    same_at: Option<u64>,
    /// A number for the definitions lent: loans of the same definitions have the same one.
    print: u64,
}

/// Feed composite font `f`'s name and parts to `h`.
fn hash_font(f: &CompositeFont, h: &mut DefaultHasher) {
    f.name.hash(h);
    let p = &f.parts;
    for part in [&p.kanji, &p.kana, &p.punctuation, &p.symbols, &p.roman, &p.numerals] {
        (&part.font, &part.style, part.scale_at_center).hash(h);
        for v in [part.size, part.baseline, part.h_scale, part.v_scale] {
            v.to_bits().hash(h);
        }
    }
}

/// How a composite font draws some text ([`FontDb::composite_coverage`]).
#[derive(Clone, Debug)]
pub struct CompositeCoverage {
    /// The worst match among the parts that draw it.
    pub status: FontMatch,
    /// The face of the first part with that match.
    pub face: Option<Arc<FontFace>>,
    /// The characters whose part's face lacks them.
    pub lacking: Vec<char>,
}

impl FontDb {
    /// Make `font` available by its name, in place of a composite font of that name (any case).
    /// The font menus and anything else that lists fonts change only when the definition does.
    /// Whether it did. A font of that name a guard lends ([`FontDb::lend_composite_fonts`]) stays
    /// in use on the guard's thread.
    pub fn set_composite_font(&self, font: &CompositeFont) -> bool {
        let key = font.name.to_lowercase();
        {
            let mut map = self.composites.write().unwrap_or_else(|e| e.into_inner());
            if map.get(&key).is_some_and(|f| **f == *font) {
                return false;
            }
            map.insert(key, Arc::new(font.clone()));
        }
        self.changed();
        true
    }

    /// Forget the composite font named `name` (any case). Whether there was one.
    pub fn remove_composite_font(&self, name: &str) -> bool {
        let removed = self.composites.write().unwrap_or_else(|e| e.into_inner()).remove(&name.to_lowercase()).is_some();
        if removed {
            self.changed();
        }
        removed
    }

    /// The composite font named `name` (any case): the one the latest guard of this thread that
    /// lends that name lends ([`FontDb::lend_composite_fonts`]), else the one the database holds.
    pub fn composite_font(&self, name: &str) -> Option<Arc<CompositeFont>> {
        if self.lends() {
            let key = name.to_lowercase();
            if let Some(f) = self.lent_font(&key) {
                return Some(f);
            }
            return self.composites.read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        }
        let map = self.composites.read().unwrap_or_else(|e| e.into_inner());
        if map.is_empty() {
            return None;
        }
        map.get(&name.to_lowercase()).cloned()
    }

    /// Whether a guard of this thread lends composite fonts of this database.
    fn lends(&self) -> bool {
        LENT.try_with(|l| l.borrow().iter().any(|loan| loan.db == self.id)).unwrap_or(false)
    }

    /// The composite font named `key` (lowercased) that the latest guard of this thread lending that
    /// name lends.
    fn lent_font(&self, key: &str) -> Option<Arc<CompositeFont>> {
        LENT.try_with(|l| l.borrow().iter().rev().filter(|loan| loan.db == self.id).find_map(|loan| loan.fonts.get(key).cloned())).ok().flatten()
    }

    /// [`FontDb::generation`] on this thread, from the database's own generation `g`: `g` while each
    /// font the guards of this thread lend is the one the database held by its name when it was lent
    /// and the database has not changed since, else a number made from `g` and the definitions
    /// lent, so what was laid out in one document's definitions is not taken for another's.
    pub(super) fn lent_generation(&self, g: u64) -> u64 {
        LENT.try_with(|l| {
            let l = l.borrow();
            let loans = l.iter().filter(|loan| loan.db == self.id);
            if loans.clone().all(|loan| loan.same_at == Some(g)) {
                return g;
            }
            let mut h = DefaultHasher::new();
            g.hash(&mut h);
            for loan in loans {
                loan.print.hash(&mut h);
            }
            // The database's generations count up from 0 and never reach the top bit.
            h.finish() | 1 << 63
        })
        .unwrap_or(g)
    }

    /// The face composite font `name` sets character `c` in (its class's part's), and whether
    /// that part's font is missing. `None` when `name` is no composite font.
    pub fn composite_face(&self, name: &str, c: char) -> Option<(bool, u32)> {
        self.composite_resolve(name, c).map(|(face, m)| (m == FontMatch::Missing, face.id()))
    }

    /// [`FontDb::composite_resolve`] for the character at byte `at` of run text `text`: the part of
    /// the grapheme cluster that holds it, by the cluster's first character, as layout sets it.
    pub fn composite_resolve_at(&self, name: &str, text: &str, at: usize) -> Option<(Arc<FontFace>, FontMatch)> {
        let first = text.grapheme_indices(true).take_while(|(i, _)| *i <= at).last().and_then(|(_, g)| g.chars().next())?;
        self.composite_resolve(name, first)
    }

    /// The face composite font `name` sets character `c` in (its class's part's), and how that
    /// part's font matched ([`FontDb::resolve`]). `None` when `name` is no composite font, or no
    /// font at all is loaded.
    pub fn composite_resolve(&self, name: &str, c: char) -> Option<(Arc<FontFace>, FontMatch)> {
        let f = self.composite_font(name)?;
        let (family, style) = self.part_font(&f, CharClass::of(c));
        self.resolve(&family, &style)
    }

    /// The fonts (family, style) composite font `name` sets `text` in: those of the parts its
    /// grapheme clusters fall in (each by its first character, as layout splits them), each once,
    /// in the order first used; the Kanji part's for text without characters. `None` when `name` is
    /// no composite font.
    pub fn composite_fonts_for(&self, name: &str, text: &str) -> Option<Vec<(String, String)>> {
        let f = self.composite_font(name)?;
        let mut classes: Vec<CharClass> = Vec::new();
        for g in text.graphemes(true) {
            let c = g.chars().next().map_or(CharClass::Kanji, CharClass::of);
            if !classes.contains(&c) {
                classes.push(c);
            }
        }
        if classes.is_empty() {
            classes.push(CharClass::Kanji);
        }
        let mut fonts: Vec<(String, String)> = Vec::with_capacity(classes.len());
        for c in classes {
            let font = self.part_font(&f, c);
            if !fonts.contains(&font) {
                fonts.push(font);
            }
        }
        Some(fonts)
    }

    /// How composite font `name` draws the runs of text `texts` (each split into grapheme clusters
    /// on its own, as layout splits runs): the worst match among the parts their grapheme clusters
    /// fall in (missing, then another style, then exact), the face of the first part with that
    /// match, and the characters (neither whitespace nor control characters, sorted, each once)
    /// that their part's face lacks. `None` when `name` is no composite font.
    pub fn composite_coverage(&self, name: &str, texts: &[&str]) -> Option<CompositeCoverage> {
        let f = self.composite_font(name)?;
        let rank = |m: FontMatch| match m {
            FontMatch::Exact => 0,
            FontMatch::Style => 1,
            FontMatch::Missing => 2,
        };
        let mut faces: Vec<(CharClass, Resolved)> = Vec::new();
        let mut face_of = |class: CharClass| {
            if let Some((_, found)) = faces.iter().find(|(c, _)| *c == class) {
                return found.clone();
            }
            let (family, style) = self.part_font(&f, class);
            let found = self.resolve(&family, &style);
            faces.push((class, found.clone()));
            found
        };
        let mut worst: Option<(FontMatch, Option<Arc<FontFace>>)> = None;
        let mut lacking: Vec<char> = Vec::new();
        let mut graphemes: Vec<&str> = texts.iter().flat_map(|t| t.graphemes(true)).collect();
        if graphemes.is_empty() {
            graphemes.push("");
        }
        for g in graphemes {
            let class = g.chars().next().map_or(CharClass::Kanji, CharClass::of);
            let found = face_of(class);
            let m = found.as_ref().map_or(FontMatch::Missing, |(_, m)| *m);
            if worst.as_ref().is_none_or(|(w, _)| rank(m) > rank(*w)) {
                worst = Some((m, found.as_ref().map(|(face, _)| face.clone())));
            }
            lacking.extend(g.chars().filter(|c| !c.is_whitespace() && !c.is_control() && !found.as_ref().is_some_and(|(face, _)| face.covers(*c))));
        }
        lacking.sort_unstable();
        lacking.dedup();
        let (status, face) = worst.unwrap_or((FontMatch::Exact, None));
        Some(CompositeCoverage { status, face, lacking })
    }

    /// The family type in `family` is stored under when it is set in that font: a composite font's
    /// own name, else the family of the face it resolves to (any name of a family, ヒラギノ角ゴシック,
    /// is stored as its own, Hiragino Sans). `None` when the family isn't available.
    pub fn stored_family(&self, family: &str, style: &str) -> Option<String> {
        if let Some(f) = self.composite_font(family) {
            return Some(f.name.clone());
        }
        match self.resolve(family, style) {
            Some((face, m)) if m != FontMatch::Missing => Some(face.family.clone()),
            _ => None,
        }
    }

    /// Make each of the composite fonts `fonts` (a document's) available on this thread while the
    /// returned guard lives, in place of a composite font of the same name that the database holds
    /// or that an earlier guard of this thread lends. An app's session registers the composite fonts
    /// of its front document; with the guard, a document exported without a session, one that is not
    /// in front, or one placed in another document draws its type in its own, also while another
    /// thread draws another document that defines the same name differently, or the session
    /// registers another document's. Of fonts that share a name (in any case), the first is lent.
    /// Other threads, and the fonts the database holds and lists, are left as they are. The guard
    /// stays on its thread and ends its loan when it drops, also when the thread unwinds.
    pub fn lend_composite_fonts(&self, fonts: &[CompositeFont]) -> LentCompositeFonts<'_> {
        // A font a document can't use (its name an installed family's, or a value out of range)
        // is not lent: an app leaves it out when it opens the document.
        let usable: Vec<&CompositeFont> = fonts.iter().filter(|f| f.check().is_ok() && !self.has_installed_family(&f.name)).collect();
        if usable.is_empty() {
            return LentCompositeFonts { number: None, thread: PhantomData };
        }
        let mut lent: HashMap<String, Arc<CompositeFont>> = HashMap::with_capacity(usable.len());
        let mut h = DefaultHasher::new();
        let mut same = true;
        let generation = {
            let map = self.composites.read().unwrap_or_else(|e| e.into_inner());
            for f in usable {
                let key = f.name.to_lowercase();
                if lent.contains_key(&key) {
                    continue;
                }
                hash_font(f, &mut h);
                let font = match map.get(&key) {
                    Some(have) if **have == *f => have.clone(),
                    _ => {
                        same = false;
                        Arc::new(f.clone())
                    }
                };
                lent.insert(key, font);
            }
            self.generation.load(Ordering::Relaxed)
        };
        let number = NEXT_LOAN.fetch_add(1, Ordering::Relaxed);
        let loan = Loan { db: self.id, number, fonts: lent, same_at: same.then_some(generation), print: h.finish() };
        let pushed = LENT.try_with(|l| l.borrow_mut().push(loan)).is_ok();
        LentCompositeFonts { number: pushed.then_some(number), thread: PhantomData }
    }

    /// The family and style composite font `f` sets characters of `class` in: its part's, or the
    /// fallback font's when that part names a composite font.
    fn part_font(&self, f: &CompositeFont, class: CharClass) -> (String, String) {
        let p = f.parts.get(class);
        if self.composite_font(&p.font).is_some() { (FALLBACK_FAMILY.to_string(), "Regular".to_string()) } else { (p.font.clone(), p.style.clone()) }
    }

    /// The names of the composite fonts the database holds (not those guards lend), sorted.
    pub fn composite_font_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.composites.read().unwrap_or_else(|e| e.into_inner()).values().map(|f| f.name.clone()).collect();
        v.sort_by_key(|n| n.to_lowercase());
        v
    }

    /// The family and style that stand for composite font `name` where one font is needed (its
    /// metrics, whether it is missing): its Kanji part's, or the fallback font's when that part
    /// names a composite font too. `None` when `name` is no composite font.
    pub(super) fn composite_base(&self, name: &str) -> Option<(String, String)> {
        let f = self.composite_font(name)?;
        Some(self.part_font(&f, CharClass::Kanji))
    }

    /// `t` with each run set in a composite font split into runs of one [`CharClass`] each, set in
    /// that class's font: its size and baseline shift from the part's size and baseline (in
    /// percent of the run's size), its scales multiplied by the part's, and Kana scaled at the
    /// center raised so the em box's center stays where it was. Each grapheme cluster goes with
    /// its first character. Also the index of the run of `t` each run came from. `None` when no run
    /// is set in a composite font.
    pub fn composite_split(&self, t: &TextObject) -> Option<(TextObject, Vec<usize>)> {
        if !self.lends() && self.composites.read().unwrap_or_else(|e| e.into_inner()).is_empty() {
            return None;
        }
        let fonts: Vec<Option<Arc<CompositeFont>>> =
            t.runs.iter().map(|r| if r.inline.is_some() { None } else { self.composite_font(&r.style.font_family) }).collect();
        if fonts.iter().all(Option::is_none) {
            return None;
        }
        let mut runs = Vec::with_capacity(t.runs.len());
        let mut from = Vec::with_capacity(t.runs.len());
        for (i, (r, font)) in t.runs.iter().zip(&fonts).enumerate() {
            let Some(font) = font else {
                runs.push(r.clone());
                from.push(i);
                continue;
            };
            let mut start = 0;
            let mut class = None;
            for (at, g) in r.text.grapheme_indices(true) {
                let c = g.chars().next().map_or(CharClass::Kanji, CharClass::of);
                if let Some(k) = class
                    && k != c
                {
                    runs.push(self.composite_run(r.text.get(start..at).unwrap_or_default(), r, font, k));
                    from.push(i);
                    start = at;
                }
                class = Some(c);
            }
            runs.push(self.composite_run(r.text.get(start..).unwrap_or_default(), r, font, class.unwrap_or(CharClass::Kanji)));
            from.push(i);
        }
        Some((TextObject { runs, ..t.clone() }, from))
    }

    /// Characters `text` of run `r` (set in composite font `font`), all of class `class`, as a run
    /// set in that class's part.
    fn composite_run(&self, text: &str, r: &TextRun, font: &CompositeFont, class: CharClass) -> TextRun {
        let p = font.parts.get(class);
        let mut st = r.style.clone();
        let base = st.size;
        let (family, style) = self.part_font(font, class);
        st.font_family = family;
        st.font_style = style;
        st.font_version = None;
        st.font_axes.clear();
        st.size = base * p.size / 100.0;
        st.baseline_shift += base * p.baseline / 100.0;
        st.h_scale *= p.h_scale / 100.0;
        st.v_scale *= p.v_scale / 100.0;
        if class == CharClass::Kana && p.scale_at_center && p.v_scale != 100.0 {
            let centre = self.style_face(&st).map_or(0.38, |f| f.ideographic_centre());
            st.baseline_shift += st.size * centre * (1.0 - p.v_scale / 100.0);
        }
        TextRun { text: text.to_string(), style: st, inline: None }
    }
}

/// The composite fonts [`FontDb::lend_composite_fonts`] made available on its thread, until it
/// drops. It is neither `Send` nor `Sync`: the loan belongs to the thread that made it.
#[must_use = "the fonts are given back when the guard drops"]
pub struct LentCompositeFonts<'a> {
    /// The loan's number, when it lends any font.
    number: Option<u64>,
    thread: PhantomData<(&'a FontDb, *const ())>,
}

impl Drop for LentCompositeFonts<'_> {
    fn drop(&mut self) {
        let Some(number) = self.number else { return };
        // Fails only while the thread's own storage is being destroyed, which drops the loans.
        let _ = LENT.try_with(|l| {
            let mut l = l.borrow_mut();
            if let Some(at) = l.iter().rposition(|loan| loan.number == number) {
                l.remove(at);
            }
        });
    }
}
