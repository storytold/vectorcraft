//! Print Ink Fixes (`addon.color.printFixes`, Object › Plug-ins › Color Effects › Print Ink
//! Fixes…): a CMYK document's inks made ready for print in one undo step. Colours with more total
//! ink than the limit are scaled down to it, white fills and strokes stop overprinting, and black
//! ones start to, as Edit › Edit Colors › Overprint Black sets it.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::overprint::{OverprintBlack, is_white};
use vectorcraft_doc::{ColorMode, Node, NodeKind};

use super::super::super::colorcmds::{Recolor, Scope};
use super::super::super::overprint::edit_counted;
use super::super::super::{bad, checked_ids_or_selection, number_or, switch_or, text_param};
use crate::{Result, Session};

const C: &str = "addon.color.printFixes";

/// What `p` asks for.
struct Fixes {
    /// Most total ink, 0..4 (0: off).
    ink_limit: f32,
    exclude_pure_black: bool,
    remove_white_overprint: bool,
    /// Overprint Black's least black, 0..1 (0: off).
    overprint_black: f32,
    text_only: bool,
    fill: bool,
    stroke: bool,
}

/// The percentage `p` gives for `key` in 0..=`max`, as a fraction (0 when left out).
fn percent(p: &Value, key: &str, max: f64) -> Result<f32> {
    let v = number_or(p, key, 0.0, C)?;
    if !(0.0..=max).contains(&v) {
        return Err(bad(C, format!("`{key}` must be 0..{max}")));
    }
    Ok((v / 100.0) as f32)
}

impl Fixes {
    fn of(p: &Value) -> Result<Self> {
        let f = Fixes {
            ink_limit: percent(p, "inkLimit", 400.0)?,
            exclude_pure_black: switch_or(p, "excludePureBlack", true, C)?,
            remove_white_overprint: switch_or(p, "removeWhiteOverprint", false, C)?,
            overprint_black: percent(p, "overprintBlack", 100.0)?,
            text_only: switch_or(p, "textOnly", false, C)?,
            fill: switch_or(p, "fill", true, C)?,
            stroke: switch_or(p, "stroke", true, C)?,
        };
        if f.ink_limit <= 0.0 && f.overprint_black <= 0.0 && !f.remove_white_overprint {
            return Err(bad(C, "give `inkLimit`, `removeWhiteOverprint` or `overprintBlack`"));
        }
        Ok(f)
    }

    /// `c` with its total ink scaled down to the limit: CMYK, and grey (black ink) unless
    /// pure blacks are excluded, which CMYK with black ink only is too.
    fn limit(&self, c: Color) -> Color {
        const E: f32 = 0.005;
        let max = self.ink_limit;
        match c {
            Color::Cmyk { c: cy, m, y, k } => {
                let total = cy + m + y + k;
                let pure_black = cy <= E && m <= E && y <= E;
                if total <= max || (self.exclude_pure_black && pure_black) || !total.is_finite() {
                    return c;
                }
                let s = max / total;
                Color::cmyk(cy * s, m * s, y * s, k * s)
            }
            Color::Gray { k } if k > max && !self.exclude_pure_black => Color::gray(max),
            _ => c,
        }
    }

    /// Remove white overprint from `n`'s own fills and strokes and its characters': true when
    /// that changed anything.
    fn unset_white(&self, n: &mut Node) -> bool {
        let mut changed = false;
        let mut off = |aimed: bool, white: bool, flag: &mut bool| {
            if aimed && white && *flag {
                *flag = false;
                changed = true;
            }
        };
        for it in &mut n.appearance.items {
            let aimed = if it.is_fill() { self.fill } else { self.stroke };
            let white = is_white(it.paint());
            off(aimed, white, it.overprint_mut());
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                off(self.fill, is_white(&r.style.fill), &mut r.style.overprint_fill);
                off(self.stroke, is_white(&r.style.stroke), &mut r.style.overprint_stroke);
            }
        }
        changed
    }

    /// The overprint fixes on `n` and everything inside it: (objects whose white overprint was
    /// removed, objects set to overprint black).
    fn overprint(&self, n: &mut Node, black: &OverprintBlack, spots: &[String]) -> (usize, usize) {
        let white = usize::from(self.remove_white_overprint && self.unset_white(n));
        let blacks = self.overprint_black > 0.0 && (!self.text_only || matches!(n.kind, NodeKind::Text(_)));
        let mut counts = (white, usize::from(blacks && black.apply(n, spots)));
        for c in n.children_mut().into_iter().flatten() {
            let (w, b) = self.overprint(Arc::make_mut(c), black, spots);
            counts = (counts.0 + w, counts.1 + b);
        }
        counts
    }
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let fixes = Fixes::of(p)?;
    let st = s.doc()?;
    if st.doc.color_mode != ColorMode::Cmyk {
        return Err(bad(C, "Print Ink Fixes works on CMYK documents: choose File › Document Color Mode › CMYK Color first"));
    }
    let layers = st.doc.layers.iter().map(|l| l.id).collect();
    let roots = checked_ids_or_selection(s, p, C)?;
    // As Artwork Cleanup's: the selection, or the document while nothing is selected.
    let ids = match text_param(p, "scope", C)? {
        Some("document") => layers,
        None if roots.is_empty() => layers,
        Some("selection") if roots.is_empty() => return Err(bad(C, "nothing selected")),
        Some("selection") | None => roots,
        Some(other) => return Err(bad(C, format!("`scope` must be \"selection\" or \"document\", not {other:?}"))),
    };
    let black = OverprintBlack { min_k: fixes.overprint_black, fill: fixes.fill, stroke: fixes.stroke, ..OverprintBlack::default() };
    let (mut ink, mut white, mut blacks) = (0, 0, 0);
    let changed = edit_counted(s, "Print Ink Fixes", |d| {
        if fixes.ink_limit > 0.0 {
            let spots = d.spot_names();
            // Spot inks and Registration aren't process ink: they keep their colour and link.
            let links = |c: &mut Color, link: &mut Option<String>, _: &mut f32| {
                if link.as_ref().is_some_and(|n| n == REGISTRATION || spots.contains(n)) {
                    return false;
                }
                let new = fixes.limit(*c);
                if new == *c {
                    return false;
                }
                (*c, *link) = (new, None);
                true
            };
            let scope = Scope { fill: fixes.fill, stroke: fixes.stroke, images: false, patterns: false };
            ink = Recolor::with_links(scope, &|c| fixes.limit(c), &links).run(d, &ids);
        }
        if !fixes.remove_white_overprint && fixes.overprint_black <= 0.0 {
            return Ok(ink);
        }
        let spots = d.spot_names();
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                let (w, b) = fixes.overprint(n, &black, &spots);
                (white, blacks) = (white + w, blacks + b);
            }
        }
        Ok(ink + white + blacks)
    })?;
    Ok(json!({ "changed": changed, "inkLimited": ink, "whiteOverprintRemoved": white, "blackOverprinted": blacks }))
}

#[cfg(test)]
mod tests;
