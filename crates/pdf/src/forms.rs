//! What the PDF writer has no operators for — optional content (PDF layers), overprint and
//! invisible text — drawn as form XObjects of their own (without a transparency group, so they
//! draw as if in place), each starting with a mark: an empty clip whose first point says what the
//! form is. Marked forms also stay apart (the writer shares identical forms). [`finish`] completes
//! them in the written file: a layer's form is drawn as the layer's optional content (`/OC …
//! BDC`), an overprinting form sets a graphics state that overprints first, and the text of an
//! invisible form is drawn in rendering mode 3 (neither filled nor stroked).

use std::collections::HashMap;
use std::io::Read;

use craft_pdf::write::backend as krilla;
use krilla::geom::PathBuilder;
use krilla::graphic::Graphic;
use krilla::surface::Surface;
use std::sync::Arc;

use vectorcraft_doc::{Document, Node, NodeKind};

use crate::encrypt::{Lexer, Obj, StreamSpans, Tok, int, stream_spans};
use crate::output::{deflate, text_string};
use crate::patch::{Patch, Xref};
use crate::{PdfError, PdfSettings, Standard};

/// What a marked form is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// The art of layer `i` of [`pdf_layers`].
    Layer(usize),
    /// A fill or stroke that overprints.
    Overprint,
    /// Text that draws nothing (rendering mode 3): characters with neither fill nor stroke.
    Invisible,
}

/// A layer written as a PDF layer (an optional content group).
pub(crate) struct PdfLayer<'a> {
    pub node: &'a Node,
    /// Its parent layer's index in [`pdf_layers`] (none for a top-level layer).
    pub parent: Option<usize>,
}

/// The layers written as PDF layers: the top-level layers and their sublayers, but template layers
/// (and what they hold), each before its sublayers, in paint order (bottom first).
pub(crate) fn pdf_layers(doc: &Document) -> Vec<PdfLayer<'_>> {
    fn add<'a>(nodes: &'a [Arc<Node>], parent: Option<usize>, out: &mut Vec<PdfLayer<'a>>) {
        for n in nodes {
            if let NodeKind::Layer { template: false, children, .. } = &n.kind {
                let i = out.len();
                out.push(PdfLayer { node: n, parent });
                add(children, Some(i), out);
            }
        }
    }
    let mut out = vec![];
    add(&doc.layers, None, &mut out);
    out
}

/// The x coordinate of a layer mark's first point (its y is the layer index).
const LAYER_X: i32 = -32000;
/// The x coordinate of an overprint mark's first point.
const OVERPRINT_X: i32 = -32001;
/// The x coordinate of an invisible text mark's first point.
const INVISIBLE_X: i32 = -32002;

impl Mark {
    fn at(self) -> (f32, f32) {
        match self {
            // Exact in `f32` up to 2^24 layers.
            Self::Layer(i) => (LAYER_X as f32, i as f32),
            Self::Overprint => (OVERPRINT_X as f32, 0.0),
            Self::Invisible => (INVISIBLE_X as f32, 0.0),
        }
    }
}

/// Draw `draw` as a form of its own, marked `mark`.
pub(crate) fn form(s: &mut Surface, mark: Mark, draw: impl FnOnce(&mut Surface)) {
    let mut sb = s.stream_builder();
    let mut fs = sb.surface();
    let (x, y) = mark.at();
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + 1.0, y);
    pb.line_to(x + 1.0, y + 1.0);
    pb.close();
    // A form's drawing space is its own: the mark's points are written as they are.
    if let Some(p) = pb.finish() {
        fs.push_clip_path(&p, &krilla::paint::FillRule::NonZero);
        fs.pop();
    }
    draw(&mut fs);
    fs.finish();
    let stream = sb.finish();
    s.draw_graphic(Graphic::new(stream, false));
}

// ---------- finishing the written file ----------

/// The resource name of the overprinting graphics state.
const OVERPRINT_GS: &str = "VCop";
/// The prefix of the resource names of the layers' optional content groups (then the index).
const LAYER_OC: &str = "VCoc";
/// The most bytes of a form's content read to find its mark.
const HEAD: u64 = 64;

fn bad(why: &str) -> PdfError {
    PdfError::Write(format!("can't write PDF layers, overprint or invisible text: {why}"))
}

/// An object of the written file with a dictionary, and its stream's [`StreamSpans`].
struct Object<'a> {
    dict: Obj<'a>,
    stream: Option<StreamSpans>,
}

impl Object<'_> {
    /// The stream's data, decoded (Flate or none), at most `limit` bytes.
    fn data(&self, pdf: &[u8], limit: u64) -> Option<Vec<u8>> {
        let (_, (start, end)) = self.stream?;
        let data = pdf.get(start..end)?;
        let mut out = vec![];
        match self.dict.get(b"Filter") {
            None => out.extend_from_slice(data.get(..usize::try_from(limit).unwrap_or(usize::MAX).min(data.len()))?),
            Some(Obj::Name(b"FlateDecode")) => {
                flate2::read::ZlibDecoder::new(data).take(limit).read_to_end(&mut out).ok()?;
            }
            Some(_) => return None,
        }
        Some(out)
    }

    /// Replace the stream's data with `content` (compressed as before).
    fn rewrite(&self, patch: &mut Patch, content: &[u8]) -> Result<(), PdfError> {
        let ((len_start, len_end), (start, end)) = self.stream.ok_or_else(|| bad("a content stream isn't a stream"))?;
        let data = if self.dict.get(b"Filter").is_some() { deflate(content).map_err(|e| bad(&e.to_string()))? } else { content.to_vec() };
        patch.replace(len_start, len_end, data.len().to_string().into_bytes());
        patch.replace(start, end, data);
        Ok(())
    }

    /// The mark of a marked form.
    fn mark(&self, pdf: &[u8]) -> Option<Mark> {
        if self.dict.name(b"Subtype") != Some(b"Form") {
            return None;
        }
        let head = self.data(pdf, HEAD)?;
        let mut lx = Lexer::at(&head, 0);
        let mut word = || match lx.next() {
            Some((Tok::Word(w), ..)) => Some(w),
            _ => None,
        };
        let (Some(b"q"), Some(x), Some(y), Some(b"m")) = (word(), word().and_then(int), word().and_then(int), word()) else { return None };
        match x {
            _ if x == i64::from(LAYER_X) => usize::try_from(y).ok().map(Mark::Layer),
            _ if x == i64::from(OVERPRINT_X) && y == 0 => Some(Mark::Overprint),
            _ if x == i64::from(INVISIBLE_X) && y == 0 => Some(Mark::Invisible),
            _ => None,
        }
    }
}

/// The objects of the written file that have a dictionary, by number.
fn objects<'a>(pdf: &'a [u8], xref: &Xref) -> HashMap<u32, Object<'a>> {
    (1..xref.count())
        .filter_map(|n| {
            let n = u32::try_from(n).ok()?;
            let mut lx = Lexer::at(pdf, xref.offset(pdf, n)?);
            (lx.header()?.0 == n).then_some(())?;
            let dict = lx.object(0)?;
            let Obj::Dict { .. } = dict else { return None };
            let stream = stream_spans(pdf, &mut lx, &dict).ok().flatten();
            Some((n, Object { dict, stream }))
        })
        .collect()
}

/// The resource dictionary of the content of `o` (a page or a form).
fn resources<'o>(objects: &'o HashMap<u32, Object<'_>>, o: &'o Object<'_>) -> Option<&'o Obj<'o>> {
    match o.dict.get(b"Resources")? {
        Obj::Ref(n, _) => objects.get(n).map(|r| &r.dict),
        d => Some(d),
    }
}

/// Add `entries` (`/VCoc0 12 0 R`) to the `category` (`Properties`, `ExtGState`) of resource
/// dictionary `res`.
fn add_resources(patch: &mut Patch, res: &Obj<'_>, category: &str, entries: &str) -> Result<(), PdfError> {
    match (res.get(category.as_bytes()), res) {
        (Some(Obj::Dict { start, .. }), _) => patch.replace(start + 2, start + 2, entries.as_bytes().to_vec()),
        (None, Obj::Dict { start, .. }) => patch.replace(start + 2, start + 2, format!("/{category}<<{entries}>>").into_bytes()),
        _ => return Err(bad("unexpected resources")),
    }
    Ok(())
}

/// `content` with each `Do` of a layer's form (`forms`: its resource name → the layer) marked as
/// that layer's optional content, and the layers marked; `None` when there is none.
fn mark_layers(content: &[u8], forms: &HashMap<&[u8], usize>) -> Option<(Vec<u8>, Vec<usize>)> {
    let mut lx = Lexer::at(content, 0);
    let mut name = None;
    let mut spans = vec![];
    while let Some((tok, start, end)) = lx.next() {
        name = match tok {
            Tok::Name(n) => forms.get(n).map(|layer| (start, *layer)),
            Tok::Word(b"Do") => {
                spans.extend(name.map(|(name, layer)| (name, end, layer)));
                None
            }
            // Inline image data isn't PDF syntax (the writer writes none).
            Tok::Word(b"ID") => break,
            _ => None,
        };
    }
    if spans.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(content.len() + spans.len() * 24);
    let mut at = 0;
    for (start, end, layer) in &spans {
        out.extend_from_slice(content.get(at..*start)?);
        out.extend_from_slice(format!("/OC /{LAYER_OC}{layer} BDC ").as_bytes());
        out.extend_from_slice(content.get(*start..*end)?);
        out.extend_from_slice(b" EMC");
        at = *end;
    }
    out.extend_from_slice(content.get(at..)?);
    let mut layers: Vec<usize> = spans.iter().map(|s| s.2).collect();
    layers.sort_unstable();
    layers.dedup();
    Some((out, layers))
}

/// `content` (an invisible form's) with its text drawn in rendering mode 3: the mode set first,
/// and every mode the writer sets (it sets one for each run of glyphs) made 3. `None` when the
/// content can't be read to its end.
fn invisible_text(content: &[u8]) -> Option<Vec<u8>> {
    let mut lx = Lexer::at(content, 0);
    let mut operand = None;
    let mut modes = vec![];
    let mut read = 0;
    while let Some((tok, start, end)) = lx.next() {
        read = end;
        operand = match tok {
            Tok::Word(b"Tr") => {
                modes.extend(operand);
                None
            }
            Tok::Word(w) if int(w).is_some() => Some((start, end)),
            // Inline image data isn't PDF syntax (the writer writes none).
            Tok::Word(b"ID") => return None,
            _ => None,
        };
    }
    // The lexer also stops at a string it can't read: a mode after it would be missed.
    if !content.get(read..)?.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let mut out = Vec::with_capacity(content.len() + 8);
    out.extend_from_slice(b"3 Tr\n");
    let mut at = 0;
    for (start, end) in modes {
        out.extend_from_slice(content.get(at..start)?);
        out.push(b'3');
        at = end;
    }
    out.extend_from_slice(content.get(at..)?);
    Some(out)
}

/// Finish the marked forms of `pdf` (as [`crate::export`] writes `doc` with `set`): with
/// `layers`, each layer and sublayer (but template layers) becomes an optional content group —
/// named as the layer, off when it is hidden, not printed (`/PrintState /OFF`) when its Print
/// option is off, locked when it is locked — and the uses of its forms its optional content;
/// with `overprint`, the overprinting forms get a graphics state that overprints (`/OP`, `/op`,
/// `/OPM 1`; `/OPM 0` in PDF/A, which allows no other with ICC-based CMYK colours); with
/// `invisible`, the text of the invisible forms is drawn in rendering mode 3.
pub(crate) fn finish(pdf: Vec<u8>, doc: &Document, set: &PdfSettings, layers: bool, overprint: bool, invisible: bool) -> Result<Vec<u8>, PdfError> {
    if !layers && !overprint && !invisible {
        return Ok(pdf);
    }
    let xref = Xref::read(&pdf).ok_or_else(|| bad("the written PDF has no cross-reference table"))?;
    let objects = objects(&pdf, &xref);
    let marks: HashMap<u32, Mark> = objects.iter().filter_map(|(n, o)| Some((*n, o.mark(&pdf)?))).collect();
    let mut patch = Patch::new(&xref);
    if overprint {
        let opm = u8::from(set.standard != Standard::PdfA2b);
        let gs = patch.add_object(format!("<</Type/ExtGState/OP true/op true/OPM {opm}>>").into_bytes());
        for (n, _) in marks.iter().filter(|(_, m)| **m == Mark::Overprint) {
            let Some(o) = objects.get(n) else { continue };
            let (Some(content), Some(res)) = (o.data(&pdf, u64::MAX), resources(&objects, o)) else {
                return Err(bad("an overprinting form can't be read"));
            };
            let mut new = format!("/{OVERPRINT_GS} gs\n").into_bytes();
            new.extend_from_slice(&content);
            o.rewrite(&mut patch, &new)?;
            add_resources(&mut patch, res, "ExtGState", &format!("/{OVERPRINT_GS} {gs} 0 R"))?;
        }
    }
    if invisible {
        for (n, _) in marks.iter().filter(|(_, m)| **m == Mark::Invisible) {
            let Some(o) = objects.get(n) else { continue };
            let content = o.data(&pdf, u64::MAX).ok_or_else(|| bad("an invisible text form can't be read"))?;
            o.rewrite(&mut patch, &invisible_text(&content).ok_or_else(|| bad("an invisible text form can't be read"))?)?;
        }
    }
    if layers {
        write_layers(&pdf, &xref, &objects, &marks, &mut patch, doc, set)?;
        for (n, _) in marks.iter().filter(|(_, m)| matches!(m, Mark::Layer(_))) {
            if let Some(o) = objects.get(n) {
                unbox(&pdf, o, &mut patch)?;
            }
        }
    }
    let pdf = patch.apply(&pdf, &xref)?;
    // The file still is what its standard says.
    crate::pdfx::verify(&pdf, set.standard)?;
    Ok(pdf)
}

/// The optional content groups of `doc`'s layers and sublayers ([`pdf_layers`]), the uses of their
/// forms marked as their content, and the catalog's `/OCProperties`, listing sublayers under their
/// layers.
fn write_layers(
    pdf: &[u8],
    xref: &Xref,
    objects: &HashMap<u32, Object<'_>>,
    marks: &HashMap<u32, Mark>,
    patch: &mut Patch,
    doc: &Document,
    set: &PdfSettings,
) -> Result<(), PdfError> {
    // One group per layer, in paint order (bottom first).
    let layers = pdf_layers(doc);
    let mut groups: HashMap<usize, u32> = HashMap::new();
    let (mut all, mut off, mut locked) = (vec![], vec![], vec![]);
    // Each layer's sublayers, and the top-level layers, in paint order.
    let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (i, PdfLayer { node: l, parent }) in layers.iter().enumerate() {
        let siblings = children.entry(*parent).or_default();
        siblings.push(i);
        let name = l.name.clone().unwrap_or_else(|| format!("Layer {}", siblings.len()));
        let printable = matches!(l.kind, NodeKind::Layer { printable: true, .. });
        let on = |b: bool| if b { "ON" } else { "OFF" };
        let g = patch.add_object(
            format!("<</Type/OCG/Name{}/Usage<</View<</ViewState/{}>>/Print<</PrintState/{}>>>>>>", text_string(&name), on(l.visible), on(printable))
                .into_bytes(),
        );
        groups.insert(i, g);
        all.push(g);
        if !l.visible {
            off.push(g);
        }
        if l.locked {
            locked.push(g);
        }
    }
    // Every content stream drawing layers' forms: the pages' and the forms'.
    for o in objects.values() {
        let content = match o.dict.get(b"Contents") {
            Some(Obj::Ref(c, _)) if o.dict.name(b"Type") == Some(b"Page") => objects.get(c),
            _ if o.dict.name(b"Subtype") == Some(b"Form") => Some(o),
            _ => None,
        };
        let (Some(content), Some(res)) = (content, resources(objects, o)) else { continue };
        let Some(Obj::Dict { entries, .. }) = res.get(b"XObject") else { continue };
        let forms: HashMap<&[u8], usize> = entries
            .iter()
            .filter_map(|(name, x)| {
                let Obj::Ref(n, _) = x else { return None };
                match marks.get(n) {
                    Some(Mark::Layer(i)) if groups.contains_key(i) => Some((*name, *i)),
                    _ => None,
                }
            })
            .collect();
        if forms.is_empty() {
            continue;
        }
        let data = content.data(pdf, u64::MAX).ok_or_else(|| bad("a content stream can't be read"))?;
        let Some((new, marked)) = mark_layers(&data, &forms) else { continue };
        content.rewrite(patch, &new)?;
        let entries: String = marked.iter().filter_map(|i| Some(format!("/{LAYER_OC}{i} {} 0 R", groups.get(i)?))).collect();
        add_resources(patch, res, "Properties", &entries)?;
    }
    let refs = |v: &[u32]| v.iter().map(|g| format!("{g} 0 R")).collect::<Vec<_>>().join(" ");
    let mut config = format!("/Name(Layers)/Order[{}]", order(&children, &groups, None));
    if !off.is_empty() {
        config.push_str(&format!("/OFF[{}]", refs(&off)));
    }
    if !locked.is_empty() {
        config.push_str(&format!("/Locked[{}]", refs(&locked)));
    }
    // Viewers apply the print states when printing (the standards allow no automatic states).
    if set.standard == Standard::None {
        config.push_str(&format!("/AS[<</Event/Print/OCGs[{}]/Category[/Print]>>]", refs(&all)));
    }
    let root = xref.trailer_ref(pdf, b"/Root").and_then(|n| objects.get(&n));
    let Some(Object { dict: Obj::Dict { start, .. }, .. }) = root else { return Err(bad("the written PDF has no catalog")) };
    patch.replace(start + 2, start + 2, format!("/OCProperties<</OCGs[{}]/D<<{config}>>>>", refs(&all)).into_bytes());
    Ok(())
}

/// A form's box as large as a page can be: the box of a layer's form is only the bounds of its
/// art, and a reader that makes a clipping group of a form's box (as importers do) would gather a
/// layer's art and its sublayers' in one group, which no layer can hold. A box round the whole
/// page clips nothing.
const UNBOXED: &[u8] = b"[-32767 -32767 32767 32767]";

/// Give the layer's form `o` the [`UNBOXED`] box.
fn unbox(pdf: &[u8], o: &Object<'_>, patch: &mut Patch) -> Result<(), PdfError> {
    let Obj::Dict { start, end, .. } = o.dict else { return Err(bad("a layer's form has no dictionary")) };
    let dict = pdf.get(start..end).ok_or_else(|| bad("a layer's form can't be read"))?;
    // The writer writes the box as an array of numbers.
    let at = dict.windows(5).position(|w| w == b"/BBox").ok_or_else(|| bad("a layer's form has no box"))? + 5;
    let open = at + dict.get(at..).and_then(|d| d.iter().position(|&c| c == b'[')).ok_or_else(|| bad("a layer's form box isn't an array"))?;
    let close = open + dict.get(open..).and_then(|d| d.iter().position(|&c| c == b']')).ok_or_else(|| bad("a layer's form box isn't an array"))? + 1;
    patch.replace(start + open, start + close, UNBOXED.to_vec());
    Ok(())
}

/// The `/Order` entries of `parent`'s sublayers (the top-level layers for none), the top one first
/// as Layers panels list them, each followed by an array of its own sublayers when it has some.
fn order(children: &HashMap<Option<usize>, Vec<usize>>, groups: &HashMap<usize, u32>, parent: Option<usize>) -> String {
    let mut out = vec![];
    for &i in children.get(&parent).into_iter().flatten().rev() {
        let Some(g) = groups.get(&i) else { continue };
        out.push(format!("{g} 0 R"));
        if children.contains_key(&Some(i)) {
            out.push(format!("[{}]", order(children, groups, Some(i))));
        }
    }
    out.join(" ")
}
