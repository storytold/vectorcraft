//! Print to a PostScript file: the sheets a print job laid out ([`PrintPage`]), each drawn by the
//! document walk the EPS writer uses, in one DSC-conforming file: a page per sheet with its paper
//! size, the ink and halftone screen of a separation, the flatness, and film negatives.

use std::fmt::Write;

use kurbo::Shape;
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, Rect};

use crate::scene::{Page, Scene};
use crate::{EpsOptions, EpsOutput, Level, comments, ps};

/// One sheet of a print job (see the PDF writer's print plan for how the spaces relate).
#[derive(Clone, Debug)]
pub struct PrintPage<'a> {
    /// The document the sheet draws: the job's, or a separation's plate.
    pub doc: &'a Document,
    /// The page size in points.
    pub size: (f64, f64),
    /// Drawing space → page space (y down from the top-left corner).
    pub view: Affine,
    /// The part of the drawing space that shows (a tile): art and marks are clipped to it.
    pub window: Option<Rect>,
    /// Document space → drawing space.
    pub place: Affine,
    /// The art drawn, in document space: what lies outside it is left out.
    pub area: Rect,
    /// Printer's marks, in drawing space.
    pub marks: Option<&'a Node>,
    /// A film negative: paper black, ink clear.
    pub negative: bool,
    /// A separation's ink: its name, screen ruling (lpi) and angle (degrees).
    pub ink: Option<(&'a str, f64, f64)>,
}

/// The settings of a whole job.
#[derive(Clone, Debug, Default)]
pub struct PrintJob {
    pub level: Level,
    /// `%%Title`.
    pub title: String,
    /// `%%CreationDate` as Unix seconds (UTC).
    pub created: Option<i64>,
    /// The flatness curves are drawn with (device pixels); `None`: the device's own.
    pub flatness: Option<f64>,
}

/// Every number of `a` finite?
fn finite(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite())
}

/// The program that draws sheet `p` (its page setup and content, without the DSC page comment),
/// and what its walks found.
fn sheet(p: &PrintPage, job: &PrintJob) -> Result<(String, Vec<Page>), String> {
    let (w, h) = p.size;
    let r = p.area;
    if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0 && [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())) {
        return Err(format!("page size {w} × {h}"));
    }
    if !finite(p.view) || !finite(p.place) {
        return Err("a page's placement is not finite".into());
    }
    let opts = |region: Rect| EpsOptions { level: job.level, region, ..EpsOptions::default() };
    let art_opts = opts(r);
    let art = Scene::new(p.doc, &art_opts).run();
    // The marks' area: the drawing space the page shows (within the tile).
    let paper = Rect::new(0.0, 0.0, w, h);
    let shown = if p.view.determinant().abs() > 1e-12 { p.view.inverse().transform_rect_bbox(paper) } else { paper };
    let shown = p.window.map_or(shown, |t| t.intersect(shown));
    let marks_opts = opts(shown);
    let marks = p.marks.map(|m| Scene::new(p.doc, &marks_opts).run_node(m));
    let mut s = String::with_capacity(art.body.len() + art.setup.len() + 1024);
    let (wi, hi) = (w.ceil() as i64, h.ceil() as i64);
    let _ = writeln!(s, "%%PageBoundingBox: 0 0 {wi} {hi}");
    if let Some((name, _, _)) = p.ink {
        // A line break would end the comment and the rest would run as PostScript.
        let _ = writeln!(s, "%%PlateColor: {}", name.replace(['\r', '\n'], " "));
    }
    s.push_str("%%BeginPageSetup\n");
    let _ = writeln!(s, "/setpagedevice where {{pop << /PageSize [{} {}] >> setpagedevice}} if", ps::num(w), ps::num(h));
    s.push_str("%%EndPageSetup\nVCdict begin\nq\n");
    if p.negative {
        // Ink becomes clear and paper black: the inverted transfer paints the paper's white black.
        let _ = writeln!(s, "{{1 exch sub}} settransfer 1 g 0 0 {} {} rectfill", ps::num(w), ps::num(h));
    }
    if let Some((_, lpi, angle)) = p.ink {
        // A round dot.
        let _ = writeln!(s, "{} {} {{dup mul exch dup mul add 1 exch sub}} setscreen", ps::num(lpi), ps::num(angle));
    }
    if let Some(f) = job.flatness {
        let _ = writeln!(s, "{} setflat", ps::num(f));
    }
    // Page space (y down) onto the paper (y up), then the drawing space.
    let _ = writeln!(s, "[1 0 0 -1 0 {}] cm", ps::num(h));
    let _ = writeln!(s, "{} cm", ps::matrix(p.view));
    if let Some(t) = p.window {
        ps::push_path(&mut s, &t.to_path(0.1));
        s.push_str("W\n");
    }
    s.push_str("q\n");
    let _ = writeln!(s, "{} cm", ps::matrix(p.place));
    ps::push_path(&mut s, &r.to_path(0.1));
    s.push_str("W\n");
    s.push_str(&art.setup);
    s.push_str(&art.body);
    s.push_str("Q\n");
    let mut found = vec![art];
    if let Some(m) = marks {
        // The marks' spot colour spaces are defined after the art is drawn: names may be reused.
        s.push_str(&m.setup);
        s.push_str(&m.body);
        found.push(m);
    }
    s.push_str("Q\nend\nshowpage\n%%PageTrailer\n");
    Ok((s, found))
}

/// Write the sheets `pages` in `order` (indices into `pages`; copies repeat them) as one
/// PostScript file of `order.len()` pages. Each sheet is drawn once and repeated for its copies.
pub fn print(pages: &[PrintPage], order: &[usize], job: &PrintJob) -> Result<EpsOutput, String> {
    if order.is_empty() {
        return Err("nothing to print".into());
    }
    let mut drawn = Vec::with_capacity(pages.len());
    let mut custom: Vec<(String, [f32; 4])> = vec![];
    let mut warnings: Vec<String> = vec![];
    for p in pages {
        let (s, found) = sheet(p, job)?;
        for page in found {
            for c in page.custom {
                if !custom.iter().any(|(n, _)| *n == c.0) {
                    custom.push(c);
                }
            }
            for w in page.warnings {
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
        }
        drawn.push(s);
    }
    let (w, h) = pages.iter().fold((0.0f64, 0.0f64), |(w, h), p| (w.max(p.size.0), h.max(p.size.1)));
    let size: usize = order.iter().filter_map(|i| drawn.get(*i)).map(String::len).sum();
    let mut s = String::with_capacity(size + 4096);
    s.push_str("%!PS-Adobe-3.0\n"); // brand-ok: the DSC version line every conforming file starts with
    comments(&mut s, &job.title, job.created, job.level, &custom);
    let _ = writeln!(s, "%%BoundingBox: 0 0 {} {}", w.ceil() as i64, h.ceil() as i64);
    let _ = writeln!(s, "%%Pages: {}\n%%PageOrder: Ascend\n%%EndComments", order.len());
    s.push_str("%%BeginProlog\n");
    s.push_str(ps::PROLOG);
    s.push_str("%%EndProlog\n%%BeginSetup\n%%EndSetup\n");
    for (n, i) in order.iter().enumerate() {
        let page = drawn.get(*i).ok_or("a page out of range")?;
        let _ = writeln!(s, "%%Page: {0} {0}", n + 1);
        s.push_str(page);
    }
    s.push_str("%%Trailer\n%%EOF\n");
    Ok(EpsOutput { bytes: s.into_bytes(), warnings })
}

#[cfg(test)]
mod tests;
