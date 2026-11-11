//! The drawing the chunk index holds: pages, layers, objects and the fills and outlines they name
//! (R19 to R23). Records are decoded only when an object refers to them.

use std::collections::HashMap;

use crate::bytes::Rd;
use crate::geometry;
use crate::layout::{self, Profile};
use crate::limits::Budget;
use crate::model::{Color, Drawing, Geometry, Guideline, Layer, LayerKind, Matrix, Object, Outline, Page, Pt, Shape};
use crate::records::{self, Loda};
use crate::riff::{Chunk, Index, push};
use crate::{Error, Warning};

/// Object types (R6).
const RECTANGLE: u32 = 1;
const ELLIPSE: u32 = 2;
const CURVE: u32 = 3;
const POLYGON: u32 = 0x14;
const GUIDELINE: u32 = 0x16;
/// Argument types (R6, R20, R21).
const ARG_OUTLINE: u32 = 10;
const ARG_FILL: u32 = 20;
const ARG_COORDS: u32 = 30;
const ARG_NAME: u32 = 1000;
const ARG_POLYGON: u32 = 11000;
const ARG_GUIDE: u32 = 16000;
const ARG_PAGE_SIZE: u32 = 19130;

/// What one read keeps while mapping.
struct Ctx<'a, 'b> {
    ix: &'b Index<'a>,
    p: Profile,
    budget: &'b mut Budget,
    warn: &'b mut Vec<Warning>,
    /// Fill and outline records by id, decoded on first use.
    fill_chunks: HashMap<u32, &'b [u8]>,
    outline_chunks: HashMap<u32, &'b [u8]>,
    fills: HashMap<u32, Option<Color>>,
    outlines: HashMap<u32, Option<Outline>>,
}

pub(crate) fn drawing(ix: &Index, budget: &mut Budget, warn: &mut Vec<Warning>) -> Result<Drawing, Error> {
    let root = ix.root().ok_or(Error::NotCdr)?;
    let version = version(ix, root);
    let p = layout::for_version(version).ok_or(Error::Version(version))?;
    let mut found = Found::default();
    collect(ix, root, &mut found);
    let page_size = found.mcfg.and_then(|c| page_size(ix.data(c), &p));
    let mut cx =
        Ctx { ix, p, budget, warn, fill_chunks: HashMap::new(), outline_chunks: HashMap::new(), fills: HashMap::new(), outlines: HashMap::new() };
    for c in &found.fills {
        if let Some(id) = Rd::new(ix.data(c)).u32() {
            cx.fill_chunks.entry(id).or_insert(ix.data(c));
        }
    }
    for c in &found.outlines {
        if let Some(id) = Rd::new(ix.data(c)).u32() {
            cx.outline_chunks.entry(id).or_insert(ix.data(c));
        }
    }
    let mut guides = found.guid.map(|c| guid_chunk(ix.data(c), &cx.p, cx.budget, cx.warn)).unwrap_or_default();
    let mut pages = Vec::new();
    let mut master = Vec::new();
    for page in &found.pages {
        let (is_master, read) = cx.page(page);
        if is_master {
            // Guidelines on any of the master page's layers show on every page.
            for mut layer in read.layers {
                guides.append(&mut layer.guides);
                if !layer.objects.is_empty() && layer.kind != LayerKind::Grid {
                    master.push(layer);
                }
            }
        } else {
            pages.push(read);
        }
    }
    // A file of only a master page keeps it as its page.
    if pages.is_empty() && !master.is_empty() {
        pages.push(Page { size: None, layers: std::mem::take(&mut master) });
    }
    Ok(Drawing { version, page_size, pages, master, guides, warnings: std::mem::take(cx.warn) })
}

/// The version: the `vrsn` chunk's when it names one from 5 to 13, else the form type's (R1, R2).
fn version(ix: &Index, root: &Chunk) -> u16 {
    let vrsn = ix.children(root).iter().find(|c| &c.id == b"vrsn").and_then(|c| Rd::new(ix.data(c)).u16());
    match vrsn {
        Some(v) if (500..1400).contains(&v) => v,
        _ => ix.form_version,
    }
}

#[derive(Default)]
struct Found<'i> {
    fills: Vec<&'i Chunk>,
    outlines: Vec<&'i Chunk>,
    mcfg: Option<&'i Chunk>,
    guid: Option<&'i Chunk>,
    pages: Vec<&'i Chunk>,
}

/// Every fill, outline and page in file order, and the first `mcfg` and `guid`; pages are not
/// looked into.
fn collect<'i>(ix: &'i Index, root: &'i Chunk, found: &mut Found<'i>) {
    let mut stack: Vec<&Chunk> = ix.children(root).iter().rev().collect();
    while let Some(c) = stack.pop() {
        match &c.id {
            b"fild" | b"fill" => found.fills.push(c),
            b"outl" => found.outlines.push(c),
            b"mcfg" => found.mcfg = found.mcfg.or(Some(c)),
            b"guid" => found.guid = found.guid.or(Some(c)),
            _ if c.is_list(b"page") => found.pages.push(c),
            _ => stack.extend(ix.children(c).iter().rev()),
        }
    }
}

/// The page size in `mcfg` (R10).
fn page_size(b: &[u8], p: &Profile) -> Option<(f64, f64)> {
    let mut r = Rd::at(b, p.mcfg_skip);
    sane_size(records::coord(&mut r, p)?, records::coord(&mut r, p)?)
}

fn sane_size(w: f64, h: f64) -> Option<(f64, f64)> {
    let (w, h) = (w.abs(), h.abs());
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// The version 7 `guid` chunk (R23): two counts, then 40-byte records laid out as argument 16000.
/// Each guideline counts toward the objects a read maps; past them the rest are left out.
fn guid_chunk(b: &[u8], p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Vec<Guideline> {
    let mut r = Rd::new(b);
    let Some(n) = r.u32() else { return Vec::new() };
    let _again = r.u32();
    let fits = r.left() / 40;
    if n as usize > fits {
        push(warn, Warning::Damaged);
    }
    let n = (n as usize).min(fits);
    let mut out = Vec::new();
    for i in 0..n {
        if !budget.take_object() {
            push(warn, Warning::TooDeep);
            break;
        }
        out.extend(b.get(8 + i * 40..8 + (i + 1) * 40).and_then(|g| guideline(g, p)));
    }
    out
}

/// A guideline record (R23): 12 bytes, the angle in millionths of a degree, then a point.
fn guideline(b: &[u8], p: &Profile) -> Option<Guideline> {
    if p.short {
        return None;
    }
    let mut r = Rd::at(b, 12);
    let angle = f64::from(r.i32()?) / 1_000_000.0;
    let at = Pt { x: records::coord(&mut r, p)?, y: records::coord(&mut r, p)? };
    Some(Guideline { at, angle })
}

/// A list's flags (R19): four bytes.
fn flags(ix: &Index, list: &Chunk) -> Option<[u8; 4]> {
    let c = ix.children(list).iter().find(|c| &c.id == b"flgs")?;
    ix.data(c).get(..4)?.try_into().ok()
}

/// What a layer's flags make it (R19). Byte 0 holds the kind; versions 7 to 13 flag the master
/// page's desktop layer 0 with bytes 1 and 2 set, and its guides layer 2.
fn layer_kind(f: [u8; 4], version_below_14: bool) -> LayerKind {
    let [b0, b1, b2, _] = f;
    if b0 & 0x10 != 0 {
        LayerKind::Grid
    } else if b0 & 0x1f == 0x0a || (version_below_14 && b0 & 0x1f == 0x02) {
        LayerKind::Guides
    } else if b0 == 0x08 || (version_below_14 && b0 == 0 && b1 == 1 && b2 == 1) {
        LayerKind::Desktop
    } else {
        LayerKind::Drawing
    }
}

/// The first chunk of one of `ids` that a page, layer or object keeps for itself: among its own
/// chunks, or in its `lgob` list and the `trfl` list there (R22).
fn own_chunk<'i>(ix: &'i Index, list: &'i Chunk, ids: &[&[u8; 4]]) -> Option<&'i Chunk> {
    let mut stack: Vec<(&Chunk, u8)> = ix.children(list).iter().rev().map(|c| (c, 0)).collect();
    while let Some((c, depth)) = stack.pop() {
        if ids.contains(&&c.id) {
            return Some(c);
        }
        if depth < 2 && (c.is_list(b"lgob") || c.is_list(b"trfl")) {
            stack.extend(ix.children(c).iter().rev().map(|k| (k, depth + 1)));
        }
    }
    None
}

/// The `loda` (or `lobj`) a page, layer or object keeps for itself.
fn own_loda<'i>(ix: &'i Index, list: &'i Chunk) -> Option<&'i Chunk> {
    own_chunk(ix, list, &[b"loda", b"lobj"])
}

/// A page's layer lists in file order: its own, and those in the lists it holds, such as the
/// `gobj` list of files of versions 7 to 13 (R22).
fn page_layers<'i>(ix: &'i Index, page: &'i Chunk) -> Vec<&'i Chunk> {
    let mut out = Vec::new();
    let mut stack: Vec<&Chunk> = ix.children(page).iter().rev().collect();
    while let Some(c) = stack.pop() {
        if c.is_list(b"layr") {
            out.push(c);
        } else if c.id == crate::riff::LIST && !c.is_list(b"lgob") {
            stack.extend(ix.children(c).iter().rev());
        }
    }
    out
}

/// One frame of the object walk: a list's chunks, the next one to look at, and the objects so far.
struct Frame<'i> {
    chunks: &'i [Chunk],
    next: usize,
    objects: Vec<Object>,
    /// Whether its objects form a group in the frame below.
    group: bool,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// A page: whether it is the master page, and its layers.
    fn page(&mut self, page: &Chunk) -> (bool, Page) {
        let ix = self.ix;
        let is_master = flags(ix, page).is_some_and(|f| f[2] != 0);
        let size = own_loda(ix, page)
            .and_then(|c| records::loda(ix.data(c), &self.p, &self.budget.limits, self.warn))
            .and_then(|l| l.arg(ARG_PAGE_SIZE).and_then(|b| self.size_arg(b)));
        let mut layers = Vec::new();
        for l in page_layers(ix, page) {
            let layer = self.layer(l);
            let empty = layer.objects.is_empty() && layer.guides.is_empty();
            // The entries that stand for the master page's layers hold nothing of the page's own.
            if empty && (is_master || layer.kind != LayerKind::Drawing) {
                continue;
            }
            layers.push(layer);
        }
        (is_master, Page { size, layers })
    }

    fn size_arg(&self, b: &[u8]) -> Option<(f64, f64)> {
        let mut r = Rd::new(b);
        sane_size(records::coord(&mut r, &self.p)?, records::coord(&mut r, &self.p)?)
    }

    fn layer(&mut self, list: &Chunk) -> Layer {
        let ix = self.ix;
        let kind = flags(ix, list).map_or(LayerKind::Drawing, |f| layer_kind(f, true));
        let name = own_loda(ix, list)
            .and_then(|c| records::loda(ix.data(c), &self.p, &self.budget.limits, self.warn))
            .and_then(|l| l.arg(ARG_NAME).map(|b| records::name(b, &self.p)))
            .unwrap_or_default();
        let mut guides = Vec::new();
        let objects = self.objects(ix.children(list), &mut guides);
        Layer { name, kind, objects, guides }
    }

    /// The objects of a layer's chunks, groups nested, front to back; guidelines go to `guides`.
    fn objects(&mut self, chunks: &'b [Chunk], guides: &mut Vec<Guideline>) -> Vec<Object> {
        let ix = self.ix;
        let mut stack = vec![Frame { chunks, next: 0, objects: Vec::new(), group: false }];
        loop {
            let Some(top) = stack.last_mut() else { return Vec::new() };
            let here = top.chunks;
            let Some(c) = here.get(top.next) else {
                let Some(done) = stack.pop() else { return Vec::new() };
                match stack.last_mut() {
                    // A group counts toward the objects a read maps; past them its objects stay,
                    // ungrouped.
                    Some(parent) if done.group && !done.objects.is_empty() => {
                        if self.budget.take_object() {
                            parent.objects.push(Object::Group(done.objects));
                        } else {
                            push(self.warn, Warning::TooDeep);
                            parent.objects.extend(done.objects);
                        }
                    }
                    Some(parent) if !done.group => parent.objects.extend(done.objects),
                    Some(_) => {}
                    None => return done.objects,
                }
                continue;
            };
            top.next += 1;
            if c.is_list(b"obj ") {
                if let Some(o) = self.object(c, guides) {
                    top.objects.push(o);
                }
            } else if c.is_list(b"grp ") {
                stack.push(Frame { chunks: ix.children(c), next: 0, objects: Vec::new(), group: true });
            } else if c.is_list(b"lnkg") {
                // Linked groups (brush strokes, blends, contours) place their objects with a
                // matrix of their own, which this reader doesn't apply: they are left out.
                push(self.warn, Warning::UnsupportedObjects);
            } else if c.id == crate::riff::LIST && !c.is_list(b"lgob") {
                // Other lists under a layer are looked through (R22).
                stack.push(Frame { chunks: ix.children(c), next: 0, objects: Vec::new(), group: false });
            }
        }
    }

    /// One `LIST obj `: a shape, or `None` (a guideline goes to `guides`).
    fn object(&mut self, list: &Chunk, guides: &mut Vec<Guideline>) -> Option<Object> {
        let ix = self.ix;
        let Some(loda) = own_loda(ix, list).and_then(|c| records::loda(ix.data(c), &self.p, &self.budget.limits, self.warn)) else {
            push(self.warn, Warning::Damaged);
            return None;
        };
        // Guidelines count toward the objects a read maps.
        if !self.budget.take_object() {
            push(self.warn, Warning::TooDeep);
            return None;
        }
        if loda.kind == GUIDELINE {
            guides.extend(loda.arg(ARG_GUIDE).and_then(|b| guideline(b, &self.p)));
            return None;
        }
        let geometry = self.geometry(&loda)?;
        // A transform that is there but can't be read is damage; the shape stays where it is.
        let matrix = match own_chunk(ix, list, &[b"trfd"]) {
            None => Matrix::IDENTITY,
            Some(c) => records::trfd(ix.data(c), &self.p).unwrap_or_else(|| {
                push(self.warn, Warning::Damaged);
                Matrix::IDENTITY
            }),
        };
        let fill = loda.arg(ARG_FILL).and_then(id_of).and_then(|id| self.fill(id));
        let outline = loda.arg(ARG_OUTLINE).and_then(id_of).and_then(|id| self.outline(id));
        Some(Object::Shape(Box::new(Shape { geometry, matrix, fill, outline })))
    }

    fn geometry(&mut self, loda: &Loda) -> Option<Geometry> {
        let coords = loda.arg(ARG_COORDS);
        // A shape without its coordinates, or a polygon without its sides, is damage like a shape
        // whose coordinates can't be read.
        let g = match loda.kind {
            RECTANGLE => coords.and_then(|c| geometry::rect(c, &self.p)),
            ELLIPSE => coords.and_then(|c| geometry::ellipse(c, &self.p)),
            CURVE => coords.and_then(|c| geometry::point_list(c, &self.p, self.budget, self.warn)).map(Geometry::Path),
            POLYGON => coords
                .and_then(|c| geometry::point_list(c, &self.p, self.budget, self.warn))
                .zip(loda.arg(ARG_POLYGON))
                .and_then(|(base, sides)| geometry::polygon(&base, sides, &self.p, self.budget, self.warn)),
            _ => {
                push(self.warn, Warning::UnsupportedObjects);
                return None;
            }
        };
        if g.is_none() {
            push(self.warn, Warning::Damaged);
        }
        g
    }

    fn fill(&mut self, id: u32) -> Option<Color> {
        if let Some(f) = self.fills.get(&id) {
            return f.clone();
        }
        // A record that is there but can't be read is damage; a missing one leaves the shape
        // without a fill.
        let read = self.fill_chunks.get(&id).and_then(|b| {
            let read = records::fill(b, &self.p, self.warn);
            if read.is_none() {
                push(self.warn, Warning::Damaged);
            }
            read.and_then(|(_, c)| c)
        });
        self.fills.insert(id, read.clone());
        read
    }

    /// The outline of record `id`. Its dashes count toward the read's dashes for every shape that
    /// uses it; past them the shape's outline is solid.
    fn outline(&mut self, id: u32) -> Option<Outline> {
        if !self.outlines.contains_key(&id) {
            let read = self.outline_chunks.get(&id).and_then(|b| {
                let read = records::outline(b, &self.p, self.warn);
                if read.is_none() {
                    push(self.warn, Warning::Damaged);
                }
                read.and_then(|(_, o)| o)
            });
            // An outline with more dashes than the read may still take is kept solid.
            let read = read.map(|mut o| {
                if o.dashes.len() > self.budget.dashes_left() {
                    o.dashes = Vec::new();
                    push(self.warn, Warning::TooDeep);
                }
                o
            });
            self.outlines.insert(id, read);
        }
        let o = self.outlines.get(&id)?.as_ref()?;
        if o.dashes.is_empty() || self.budget.take_dashes(o.dashes.len()) {
            return Some(o.clone());
        }
        push(self.warn, Warning::TooDeep);
        Some(Outline { width: o.width, color: o.color.clone(), cap: o.cap, join: o.join, dashes: Vec::new() })
    }
}

/// A fill or outline argument: the record id (R20).
fn id_of(b: &[u8]) -> Option<u32> {
    Rd::new(b).u32()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_flags_name_the_kinds() {
        assert_eq!(layer_kind([0, 0, 0, 0x98], true), LayerKind::Drawing);
        assert_eq!(layer_kind([0x0a, 0, 0, 0x98], true), LayerKind::Guides);
        assert_eq!(layer_kind([0x02, 1, 1, 0x98], true), LayerKind::Guides);
        assert_eq!(layer_kind([0x08, 0, 1, 0x98], true), LayerKind::Desktop);
        assert_eq!(layer_kind([0, 1, 1, 0x98], true), LayerKind::Desktop);
        assert_eq!(layer_kind([0x1a, 0, 1, 0x98], true), LayerKind::Grid);
        assert_eq!(layer_kind([0x12, 0, 1, 0x98], true), LayerKind::Grid);
    }
}
