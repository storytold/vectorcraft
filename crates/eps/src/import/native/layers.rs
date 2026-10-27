use super::build::{Page, build};
use super::slots::LAYERS_UNREAD;
use super::*;

/// The EPS `visible` (read as its page) read through its editing copy instead (its private data, or
/// its program in the legacy format, see [`editing_copy`]): its layers (the
/// hidden ones too), groups, compound paths, clipping groups and artboards, with the text and the
/// strokes around it of the page. When the file has no editing copy, `visible` as it was; when it
/// has one that can't be used, `visible` with a warning that says why.
pub(in super::super) fn layered(ps: &[u8], visible: Imported, trust: bool) -> Imported {
    let Some(data) = editing_copy(ps) else { return visible };
    let result = data.and_then(|data| build(&data, Some(&visible.document), Page::Art, false, trust, 1.0));
    match result {
        Ok((document, mut warnings)) => {
            for w in &visible.warnings {
                if !warnings.contains(w) {
                    warnings.push(w.clone());
                }
            }
            Imported { document, warnings, preview: false }
        }
        Err(why) => {
            let mut v = visible;
            v.warnings.push(format!("{LAYERS_UNREAD} ({why}): it comes in as its page, in one layer"));
            v
        }
    }
}

/// The `.ai` `visible` (read as its PDF part), or a PDF saved with the editing data, read through
/// its editing copy (`private`, the joined `AIPrivateData` or `AIPDFPrivateData` streams) instead,
/// as [`layered`] reads an EPS: with the layers and objects of the file, its artboards (with the
/// PDF part's bleed), and the art that lies outside them, which its PDF part doesn't have.
/// `warnings` are the notes of reading the PDF part; the notes returned are those, then the layers'
/// notes, or a note saying why the layers weren't read when the editing data can't be decompressed
/// or its layers can't be read. The flag returned is `true` when the document is the one read from
/// the layers; else the document is `visible`, its pages moved by `place_pages` when the editing
/// data was decompressed but its layers can't be read. `outlined`: the PDF part's type was read as
/// outlines (a file whose type shows then comes in as its PDF part).
/// `trust`: layers that draw differently from the PDF part are used all the same, with a warning.
/// `unit`: the size in points of a unit of the PDF part's user space, its first page's `UserUnit`,
/// by which its art came in scaled. Editing data whose artboard matches the page only in those
/// units comes in `unit` times as large, and so does the artboard layout `place_pages` takes from
/// it.
pub fn layered_ai(private: &[u8], visible: Document, warnings: Vec<String>, outlined: bool, trust: bool, unit: f64) -> (Document, Vec<String>, bool) {
    let unread = |mut all: Vec<String>, why: &str| {
        all.push(format!("{LAYERS_UNREAD} from its editing data ({why}): they come from its PDF part"));
        all
    };
    let data = match ai::decode_private(private) {
        Ok(data) => data,
        Err(why) => return (visible, unread(warnings, &why), false),
    };
    if !has_layers(&data) {
        return (visible, warnings, false);
    }
    // The PDF part, its pages placed and named as the editing data's artboards are, when the
    // layers can't be used (`why`).
    let from_pdf_part = |mut visible: Document, warnings: Vec<String>, why: &str| {
        place_pages(&ai::artboards(&data), &mut visible, unit);
        let mut all = unread(warnings, why);
        all.extend(ai::variables(&data).note());
        (visible, all, false)
    };
    match build(&data, Some(&visible), Page::Artboard, outlined, trust, unit) {
        // An image too large for the editing data's reader would be missing from the layers, while
        // the PDF part has it: the file comes in as its PDF part instead (#1479).
        Ok((_, notes)) if notes.iter().any(|n| n == ai::IMAGE_TOO_LARGE) => {
            from_pdf_part(visible, warnings, "an image is too large to read from them")
        }
        Ok((mut done, notes)) => {
            done.setup.bleed = visible.setup.bleed;
            let mut all = warnings;
            all.extend(notes.into_iter().filter(|n| !all.contains(n)).collect::<Vec<_>>());
            (done, all, true)
        }
        Err(why) => from_pdf_part(visible, warnings, &why),
    }
}

/// The pages of `doc` (a `.ai` read as its PDF part: a page per artboard, in a row) named as
/// `boards` (the artboards of its editing data, in art space) name them, and moved, with the art on
/// each, to where they are, so the artboards keep their names and layout when the layers can't be
/// read (#1068). Left as they are unless there are as many artboards as pages, each the size of its
/// page in points, or, when `unit` is above 1, each the size of its page in units of `unit` points:
/// then the layout is drawn `unit` times as large.
fn place_pages(boards: &[(String, Rect)], doc: &mut Document, unit: f64) {
    if boards.is_empty() || boards.len() != doc.artboards.len() {
        return;
    }
    let differ = |k: f64| {
        boards
            .iter()
            .zip(&doc.artboards)
            .any(|((_, b), a)| (b.width() - a.rect.width() / k).abs() > 0.5 || (b.height() - a.rect.height() / k).abs() > 0.5)
    };
    let k = if !differ(1.0) {
        1.0
    } else if unit.is_finite() && unit > 1.0 && !differ(unit) {
        unit
    } else {
        return;
    };
    for ((name, _), a) in boards.iter().zip(&mut doc.artboards) {
        a.name.clone_from(name);
    }
    let [(_, first), _, ..] = boards else { return };
    // Art space (y up) → the document, the first artboard's top left at the origin, as `read` has it,
    // `k` points a unit.
    let to_doc = Affine::new([k, 0.0, 0.0, -k, -first.x0 * k, first.y1 * k]);
    let pages: Vec<Rect> = doc.artboards.iter().map(|a| a.rect).collect();
    let moves: Vec<Vec2> = boards.iter().zip(&pages).map(|((_, b), page)| to_doc.transform_rect_bbox(*b).origin() - page.origin()).collect();
    for layer in &mut doc.layers {
        place_art(Arc::make_mut(layer), &pages, &moves);
    }
    for (a, v) in doc.artboards.iter_mut().zip(&moves) {
        a.rect = a.rect + *v;
    }
}

/// The objects in `layer` (and its sublayers) moved by the move of the page they are on.
fn place_art(layer: &mut Node, pages: &[Rect], moves: &[Vec2]) {
    let Some(children) = layer.children_mut() else { return };
    for child in children {
        if matches!(child.kind, NodeKind::Layer { .. }) {
            place_art(Arc::make_mut(child), pages, moves);
            continue;
        }
        let Some(r) = child.visual_bounds() else { continue };
        // The page it is on: the one with most of it (pages keep only the art that reaches into
        // them), else the nearest.
        let fit = |p: &Rect| (p.intersect(r).area().max(0.0), -(p.center() - r.center()).hypot());
        let Some((i, _)) = pages.iter().map(fit).enumerate().max_by(|(_, a), (_, b)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))) else { continue };
        if let Some(v) = moves.get(i).filter(|v| **v != Vec2::ZERO) {
            Arc::make_mut(child).transform(Affine::translate(*v), false);
        }
    }
}

/// A `.ai` saved without its PDF part (its pages show only a placeholder) from its editing copy
/// alone (`private`, the joined `AIPrivateData` streams) → the document and the import's notes.
pub fn ai_alone(private: &[u8]) -> Result<(Document, Vec<String>), String> {
    let data = ai::decode_private(private)?;
    build(&data, None, Page::Artboard, false, false, 1.0)
}

/// Does the editing data (or a program in the legacy format) have layers?
fn has_layers(data: &[u8]) -> bool {
    data.windows(15).any(|w| w == b"%AI5_BeginLayer")
}

/// The editing copy of the art in the PostScript `ps`: an EPS's private data, else the program
/// itself when it has layers (the legacy format). `None` without one.
fn editing_copy(ps: &[u8]) -> Option<Result<Cow<'_, [u8]>, String>> {
    match ai::eps_data(ps) {
        Some(data) => Some(data.map(Cow::Owned)),
        None => has_layers(ps).then_some(Ok(Cow::Borrowed(ps))),
    }
}

/// A file in the legacy format whose page can't be read (a `.ai` in PostScript form that names its
/// prolog without including it) from its layers alone (`ps`: its program, which has them) → the
/// document and the import's notes. A file whose header says it is in the format (`illustrator`)
/// but whose program has no layers (Illustrator 3 had none; image editors still export their
/// paths so, storytold/photocraft#6036) is read with its art on one layer. `None` when the program
/// has no layers and isn't in the format: other PostScript isn't read as the format's operators.
pub(in super::super) fn program_alone(ps: &[u8], illustrator: bool) -> Option<Result<(Document, Vec<String>), String>> {
    if ai::has_eps_data(ps) {
        return None;
    }
    if has_layers(ps) {
        return Some(build(ps, None, Page::Art, false, false, 1.0));
    }
    let program = one_layer(ps).filter(|_| illustrator)?;
    Some(build(&program, None, Page::Art, false, false, 1.0))
}

/// `ps` with its art (from its `%%EndSetup` to its `%%PageTrailer`, else its `%%Trailer`, else its
/// end) on a layer that shows, prints and isn't locked, as the version 5 format writes one. `None`
/// without a `%%EndSetup`: where the art starts isn't known.
fn one_layer(ps: &[u8]) -> Option<Vec<u8>> {
    let setup = line_with(ps, 0, b"%%EndSetup")?;
    let art = line_with(ps, setup + 1, b"").unwrap_or(ps.len());
    let end = line_with(ps, art, b"%%PageTrailer").or_else(|| line_with(ps, art, b"%%Trailer")).unwrap_or(ps.len());
    let mut out = Vec::with_capacity(ps.len() + 80);
    out.extend_from_slice(ps.get(..art)?);
    out.extend_from_slice(b"\n%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n(Layer 1) Ln\n");
    out.extend_from_slice(ps.get(art..end)?);
    out.extend_from_slice(b"\nLB\n%AI5_EndLayer--\n");
    out.extend_from_slice(ps.get(end..)?);
    Some(out)
}

/// Where the first line of `ps` from `from` on that starts with `key` starts.
fn line_with(ps: &[u8], from: usize, key: &[u8]) -> Option<usize> {
    (from..ps.len()).find(|&i| (i == 0 || matches!(ps.get(i - 1), Some(b'\n' | b'\r'))) && ps.get(i..).is_some_and(|rest| rest.starts_with(key)))
}

/// `path` with each open subpath whose last anchor lies on its first (within a thousandth of a
/// point) closed there: the object's own path of type round a closed path is written open, ending
/// on its start, and type on an open path stops at its end, so it had no room to run past it.
pub(super) fn closed_where_ends_meet(mut path: PathData) -> PathData {
    for sp in &mut path.subpaths {
        let (Some(first), Some(last)) = (sp.anchors.first(), sp.anchors.last()) else { continue };
        if sp.closed || sp.anchors.len() < 3 || first.p.distance(last.p) >= 1e-3 {
            continue;
        }
        if let Some(l) = sp.anchors.pop()
            && let Some(a0) = sp.anchors.first_mut()
        {
            a0.h_in = l.h_in;
        }
        sp.closed = true;
    }
    path
}

#[cfg(test)]
mod closed_ends_tests {
    use super::*;

    /// A path drawn round to its start (the last anchor on the first) is closed there, its first
    /// anchor taking the last one's incoming handle; one whose ends are apart stays open.
    #[test]
    fn a_path_ending_on_its_start_is_closed() {
        let mut bp = kurbo::BezPath::new();
        bp.move_to((100.0, 0.0));
        bp.curve_to((100.0, 55.0), (55.0, 100.0), (0.0, 100.0));
        bp.curve_to((-55.0, 100.0), (-100.0, 55.0), (-100.0, 0.0));
        bp.curve_to((-100.0, -55.0), (-55.0, -100.0), (0.0, -100.0));
        bp.curve_to((55.0, -100.0), (100.0, -55.0), (100.0, 0.0001));
        let closed = closed_where_ends_meet(PathData::from_bezpath(&bp));
        let sp = &closed.subpaths[0];
        assert!(sp.closed && sp.anchors.len() == 4, "{sp:?}");
        assert_eq!(sp.anchors[0].h_in, Point::new(100.0, -55.0));
        let mut open = kurbo::BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((50.0, 0.0));
        open.line_to((50.0, 50.0));
        assert!(!closed_where_ends_meet(PathData::from_bezpath(&open)).subpaths[0].closed);
    }
}
