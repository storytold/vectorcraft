//! Type → Find Font: the fonts a document uses (flagging missing ones), replacing one font with
//! another everywhere or in the selection, and selecting the text that uses a font. Also the fonts
//! available (the bundled and installed ones) and rescanning the installed fonts.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::Rect;

use super::select::on_artboard;
use super::typecmd::refresh_bounds;
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, index, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.fonts",
            "Fonts in Document",
            [],
            None,
            "{selectionOnly?} (the type in the art of the symbols in scope included: all of the document's, or those the selected instances show) → [{family, style, runs, objects, missing, status: exact|substitute|missing, resolved: {family, style}, missingGlyphs, version?: the installed version the type names (when it names one)}] sorted by name. A font is found by any of its names (ヒラギノ角ゴシック = Hiragino Sans) among the loaded and installed fonts; substitute: the family is there but not the style (resolved names the stand-in); missing: the family is unknown (the fallback family stands in); missingGlyphs: characters of its text the resolved font lacks (drawn by a fallback font). For a composite font, status is the worst among the parts its characters are drawn in, and missingGlyphs counts the characters their part's font lacks",
            has_doc,
            fonts
        ),
        cmd!(
            "text.replaceFont",
            "Replace Font",
            [],
            None,
            "{from: {family, style?}, to: {family (any of its names; stored as the font's own family name, a composite font's as its own name), style?}, selectionOnly?} (style omitted: every style of the family / keep the closest style); the type in the art of the symbols in scope changes too (all of the document's, or those the selected instances show, so every instance of them follows); text that is hidden or locked, in a hidden or locked group or layer (or group of a symbol's art), or on a locked artboard keeps its font; when no run changes (text already set in the new font, or none to change), the document and its undo history stay as they are → {runs: the runs changed, skipped?: the text objects left in the font, warning?: text (some text was left in its font, or the style set isn't available: its type is drawn in the family's closest style)}",
            has_doc,
            replace
        ),
        cmd!(
            "select.font",
            "Select Text by Font",
            [],
            None,
            "{family, style?, scope?: \"document\" (default) | {artboards: [index, …] (0-based)} | {layers: [id, …] (layers or sublayers)}} select the text objects using the font, and the instances of the symbols whose art uses it, that are not hidden or locked, in a hidden or locked group or layer, or on a locked artboard; with scope, only those on one of the artboards (as All on Active Artboard takes them: their bounds overlap it or their center is on it) or in one of the layers → {count}",
            has_doc,
            select_font
        ),
        cmd!(
            query "text.fontList",
            "Font List",
            [],
            None,
            "{family?, style?} → {families: [names]} sorted: the bundled fonts, fonts added and the fonts installed on the system (none on the web), each the family's English (canonical) name for `text.setStyle` and documents (without the system's hidden families, whose names start with a dot; they still resolve by name). Preferences › Type › Show Font Names in English only changes the UI menu labels, not this list; with family: {family, styles: [names] (upright styles by weight, then italics; an error when the family isn't available), axes: [{tag, name, min, default, max, value, hidden}] (a variable font's variation axes, the Character panel's Variable Font sliders, with `value` the setting of `style`, else of the first style; empty for a static font; `hidden`: the font asks for the axis not to be shown)}",
            always,
            font_list
        ),
        cmd!(
            "text.rescanFonts",
            "Refresh Font List",
            [],
            None,
            "{} scan the system font folders again, for fonts installed or removed since the app started; text in a font that became available redraws in it, and text in a family that was removed redraws in the fallback font (no fonts are installed on the web) → {families, faces} (families listed, installed faces found)",
            always,
            rescan
        ),
    ]
}

/// Text objects in scope: the selection's (and their descendants') or the whole document's.
fn scope(s: &Session, selection_only: bool, selected: Option<&[NodeId]>) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut v = vec![];
    let mut seen = HashSet::new();
    let mut add = |n: &vectorcraft_doc::Node| {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Text(_)) && seen.insert(c.id) {
                v.push(c.id);
            }
        })
    };
    if selection_only {
        for id in selected.unwrap_or(&st.selection.objects) {
            if let Some(n) = st.doc.node(*id) {
                add(n);
            }
        }
    } else {
        for l in &st.doc.layers {
            add(l);
        }
    }
    Ok(v)
}

/// Whether `name` is the symbol being edited, whose art is on the editing layer.
fn being_edited(d: &Document, name: &str) -> bool {
    d.symbol_edit.as_ref().is_some_and(|e| e.symbol == name)
}

/// The symbols that the selected objects of `s` show: those of the instances among them and their
/// descendants, and of the instances in those symbols' art that are not hidden or locked, nor in a
/// hidden or locked group of the art. Returned as (all of them, those shown by instances the
/// Selection tool can reach: not hidden or locked, as `Document::editable` says).
fn selected_symbols(s: &Session, selected: Option<&[NodeId]>) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    fn open_instances(n: &Node, out: &mut BTreeSet<String>) {
        if !n.visible || n.locked {
            return;
        }
        if let NodeKind::SymbolInstance { symbol, .. } = &n.kind {
            out.insert(symbol.clone());
        }
        for c in n.children().map(Vec::as_slice).unwrap_or_default() {
            open_instances(c, out);
        }
    }
    let expand = |names: &mut BTreeSet<String>, d: &Document| loop {
        let mut more = BTreeSet::new();
        for sym in d.symbols.iter().filter(|sym| names.contains(&sym.name)) {
            open_instances(&sym.art, &mut more);
        }
        let before = names.len();
        names.extend(more);
        if names.len() == before {
            break;
        }
    };
    let st = s.doc()?;
    let (mut all, mut open) = (BTreeSet::new(), BTreeSet::new());
    let editable = st.doc.editable();
    for id in selected.unwrap_or(&st.selection.objects) {
        if let Some(n) = st.doc.node(*id) {
            n.walk(&mut |c| {
                if let NodeKind::SymbolInstance { symbol, .. } = &c.kind {
                    all.insert(symbol.clone());
                    if editable(c.id) {
                        open.insert(symbol.clone());
                    }
                }
            });
        }
    }
    drop(editable);
    expand(&mut all, &st.doc);
    expand(&mut open, &st.doc);
    Ok((all, open))
}

/// The text objects in the art of `d`'s symbols (with `only`, of those it names), each with
/// whether it is open: neither it nor a group around it in the art is hidden or locked. The symbol
/// being edited is left out: its art is on the editing layer.
pub(super) fn symbol_texts<'a>(d: &'a Document, only: Option<&BTreeSet<String>>) -> Vec<(&'a TextObject, bool)> {
    fn visit<'a>(n: &'a Node, open: bool, out: &mut Vec<(&'a TextObject, bool)>) {
        let open = open && n.visible && !n.locked;
        if let NodeKind::Text(t) = &n.kind {
            out.push((&**t, open));
        }
        for c in n.children().map(Vec::as_slice).unwrap_or_default() {
            visit(c, open, out);
        }
    }
    let mut out = vec![];
    for s in d.symbols.iter().filter(|s| !being_edited(d, &s.name) && only.is_none_or(|o| o.contains(&s.name))) {
        visit(&s.art, true, &mut out);
    }
    out
}

/// Calls `f` on each text object in the art of `d`'s symbols (with `only`, of those it names) for
/// which `wants(text, open)` holds (open as in [`symbol_texts`]), the symbol being edited left out.
/// Art without such text is left as it is.
pub(super) fn edit_symbol_texts(
    d: &mut Document,
    only: Option<&BTreeSet<String>>,
    wants: &impl Fn(&TextObject, bool) -> bool,
    f: &mut impl FnMut(&mut TextObject),
) {
    fn wanted(n: &Node, open: bool, wants: &impl Fn(&TextObject, bool) -> bool) -> bool {
        let open = open && n.visible && !n.locked;
        matches!(&n.kind, NodeKind::Text(t) if wants(t, open)) || n.children().is_some_and(|ch| ch.iter().any(|c| wanted(c, open, wants)))
    }
    fn visit(n: &mut Node, open: bool, wants: &impl Fn(&TextObject, bool) -> bool, f: &mut impl FnMut(&mut TextObject)) {
        let open = open && n.visible && !n.locked;
        if let NodeKind::Text(t) = &mut n.kind
            && wants(t, open)
        {
            f(t);
        }
        for c in n.children_mut().into_iter().flatten() {
            if wanted(c, open, wants) {
                visit(Arc::make_mut(c), open, wants, f);
            }
        }
    }
    let edited = d.symbol_edit.as_ref().map(|e| e.symbol.clone());
    for s in &mut d.symbols {
        if edited.as_ref() != Some(&s.name) && only.is_none_or(|o| o.contains(&s.name)) && wanted(&s.art, true, wants) {
            visit(Arc::make_mut(&mut s.art), true, wants, f);
        }
    }
}

/// A font as type names it: family, style and, when it names one, the installed version.
pub(super) type UsedFont = (String, String, Option<String>);

/// How a font is named in reports: family and style, then the version the type names, if any.
pub(super) fn font_label((family, style, version): &UsedFont) -> String {
    match version {
        Some(v) => format!("{family} {style} ({v})"),
        None => format!("{family} {style}"),
    }
}

/// The fonts (family, style, version) run `r` is drawn in: its own, or, set in a composite font,
/// the fonts of the parts its characters fall in ([`vectorcraft_text::FontDb::composite_fonts_for`]).
pub(super) fn run_fonts(r: &TextRun) -> Vec<UsedFont> {
    let parts = if r.inline.is_some() { None } else { vectorcraft_text::FontDb::global().composite_fonts_for(&r.style.font_family, &r.text) };
    match parts {
        Some(parts) => parts.into_iter().map(|(family, style)| (family, style, None)).collect(),
        None => vec![font_of(&r.style)],
    }
}

/// The fonts (family, style, version) the type in the layers and symbols of `d` is drawn in, a
/// composite font's parts in place of its name ([`run_fonts`]).
pub(super) fn used_fonts(d: &Document) -> BTreeSet<UsedFont> {
    art_fonts(d, &[])
}

/// The fonts (family, style, version) of the type in the layers and symbols of `d` and in `art`
/// (art read from a file, its resources in `d`).
pub(super) fn art_fonts(d: &Document, art: &[Node]) -> BTreeSet<UsedFont> {
    fonts_of(d, art, false)
}

/// [`art_fonts`]; `drawn`: only those the type is drawn in, not the fonts of runs drawn in a font
/// their file carried ([`in_file_fonts`]).
fn fonts_of(d: &Document, art: &[Node], drawn: bool) -> BTreeSet<UsedFont> {
    let mut fonts = BTreeSet::new();
    let mut add = |n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            let file = if drawn { in_file_fonts(t) } else { vec![] };
            fonts.extend(t.runs.iter().filter(|r| !file.contains(&(r.style.font_family.as_str(), r.style.font_style.as_str()))).flat_map(run_fonts));
        }
    };
    d.walk(&mut add);
    for s in &d.symbols {
        s.art.walk(&mut add);
    }
    for n in art {
        n.walk(&mut add);
    }
    fonts
}

/// The type of `d` that prints on pages showing `areas` (document space): the fonts (family, style,
/// version) it is set in, and the ids of the area type and type on a path among the objects of the
/// document whose text doesn't fit (not those in the art of symbols, opacity masks, patterns,
/// brushes or placed documents).
/// The type that prints is visible type on `layers` (the layers that print) but not on a template
/// layer (of area type and type on a path that don't fit, the runs that their layout draws),
/// where what it paints ([`vectorcraft_render::painted_bounds`], its effects applied) meets an area
/// inside the clipping paths around it, the type in the opacity masks and patterns of the objects
/// that print and in the pieces their brushes draw, and the type of the symbols that their
/// instances and inline graphics show.
/// The objects of a blend, envelope, repeat or compound path, or of a layer or group with effects,
/// fills or strokes of its own, are taken with it when what it paints meets an area, and so is the
/// art of a placed document (`d` as [`Document::with_placed_art`] gives it, for that art's
/// resources).
pub(super) fn printed_type(d: &Document, layers: &[Arc<Node>], areas: &[Rect]) -> (BTreeSet<UsedFont>, BTreeSet<NodeId>) {
    let mut walk = Printed { d, own: true, fonts: BTreeSet::new(), overflow: BTreeSet::new(), symbols: BTreeSet::new(), patterns: BTreeSet::new() };
    for l in layers {
        walk.node(l, Some(areas));
    }
    (walk.fonts, walk.overflow)
}

/// The warning of an export that draws `layers` of `d` over `areas` (document space; `None`: the
/// whole of them) about the area type and type on a path that it draws and whose text doesn't fit,
/// found as [`printed_type`] finds them for Print: visible objects of the document that aren't on
/// a template layer.
pub(super) fn export_overflow_warning(d: &Document, layers: &[Arc<Node>], areas: Option<&[Rect]>) -> Option<String> {
    let mut walk = Printed { d, own: true, fonts: BTreeSet::new(), overflow: BTreeSet::new(), symbols: BTreeSet::new(), patterns: BTreeSet::new() };
    for l in layers {
        walk.node(l, areas);
    }
    let overflow: Vec<NodeId> = walk.overflow.into_iter().collect();
    super::typecmd::export_overflow_warning(&overflow)
}

/// The walk of [`printed_type`].
struct Printed<'a> {
    d: &'a Document,
    /// Whether the walk is among the objects of the document, not in art drawn for one.
    own: bool,
    fonts: BTreeSet<UsedFont>,
    /// The objects of the document whose text doesn't fit.
    overflow: BTreeSet<NodeId>,
    /// The symbols and patterns whose art was walked.
    symbols: BTreeSet<String>,
    patterns: BTreeSet<String>,
}

impl Printed<'_> {
    /// Adds the fonts of `n` and its descendants that print, and the ids of those that don't fit
    /// while [`Printed::own`] is set: those whose bounds meet one of `areas`, or with `None` (in the
    /// art of a symbol, opacity mask, pattern or placed document, in brush pieces, or in a live
    /// object) all of them but those outside the clipping path of a clip group.
    fn node(&mut self, n: &Node, areas: Option<&[Rect]>) {
        if !n.visible || matches!(n.kind, NodeKind::Layer { template: true, .. }) {
            return;
        }
        // A layer or group with effects, fills or strokes of its own prints where they paint (moved,
        // shadowed…), drawn from its members' shapes.
        let moved = vectorcraft_render::effects::has_container_appearance(n);
        let open = !moved && matches!(n.kind, NodeKind::Layer { clip: false, .. } | NodeKind::Group { clip: false, .. });
        if let Some(a) = areas
            && !open
            && !self.meets(n, a)
        {
            return;
        }
        // The art that its opacity mask, pattern paints and brushed strokes draw.
        let mask = n.extras().mask.as_deref().filter(|m| !m.disabled);
        let (runs, overflows) = match &n.kind {
            NodeKind::Text(t) => drawn_runs(t),
            _ => (vec![], false),
        };
        let items: Vec<&AppearanceItem> =
            n.appearance.items.iter().filter(|i| i.visible() && !matches!(i, AppearanceItem::Stroke(s) if s.width <= 0.0)).collect();
        let patterns: Vec<&str> = items
            .iter()
            .map(|i| i.paint())
            .chain(runs.iter().flat_map(|r| [Some(&r.style.fill), (r.style.stroke_width > 0.0).then_some(&r.style.stroke)]).flatten())
            .filter_map(|p| match p {
                Paint::Pattern { pattern, .. } => Some(pattern.as_str()),
                _ => None,
            })
            .collect();
        let brushed = vectorcraft_brush::has_brush(n);
        if (mask.is_some() || !patterns.is_empty() || brushed) && areas.is_none_or(|a| !open || self.meets(n, a)) {
            if let Some(m) = mask {
                self.art(&m.art);
            }
            let d = self.d;
            for name in patterns {
                if self.patterns.insert(name.to_string())
                    && let Some(p) = d.pattern(name)
                {
                    for art in &p.art {
                        self.art(art);
                    }
                }
            }
            // The pieces its brushes draw along the path (art and pattern brushes leave type out).
            if brushed {
                for piece in vectorcraft_brush::node_pieces(d, n).unwrap_or_default() {
                    self.art(&piece);
                }
            }
        }
        match &n.kind {
            NodeKind::Text(t) => {
                if overflows && self.own {
                    self.overflow.insert(n.id);
                }
                let file = in_file_fonts(t);
                for r in runs {
                    // Drawn in a font its file carried: printed as the file drew it.
                    if !file.contains(&(r.style.font_family.as_str(), r.style.font_style.as_str())) {
                        self.fonts.extend(run_fonts(r));
                    }
                    // Inline graphics draw their symbols' art.
                    if let Some(art) = &r.inline {
                        self.symbol(&art.symbol);
                    }
                }
            }
            NodeKind::SymbolInstance { symbol, .. } => self.symbol(symbol),
            NodeKind::PlacedDocument(p) => {
                for art in p.art(n) {
                    self.art(&art);
                }
            }
            NodeKind::Layer { children, clip, .. } | NodeKind::Group { children, clip } if !moved => {
                // Each object prints where it lies, inside the clipping path of a clip group.
                let clipped = match (clip, areas) {
                    (false, _) => None,
                    (true, Some(a)) => Some(self.clip(n, a)),
                    (true, None) => vectorcraft_render::painted_bounds(self.d, n).map(|c| vec![c]),
                };
                for c in children {
                    self.node(c, clipped.as_deref().or(areas));
                }
            }
            _ => {
                for c in n.children().map(Vec::as_slice).unwrap_or_default() {
                    self.node(c, None);
                }
            }
        }
    }

    /// Adds the fonts of symbol `name`'s art, the first time it is shown.
    fn symbol(&mut self, name: &str) {
        let d = self.d;
        if self.symbols.insert(name.to_string())
            && let Some(s) = d.symbols.iter().find(|s| s.name == name)
        {
            self.art(&s.art);
        }
    }

    /// Adds the fonts of `art`, drawn for an object of the document but not one of its objects (the
    /// art of a symbol, opacity mask, pattern or placed document, or a brush piece), with
    /// [`Printed::node`]'s `None`.
    fn art(&mut self, art: &Node) {
        let own = std::mem::replace(&mut self.own, false);
        self.node(art, None);
        self.own = own;
    }

    /// Whether what `n` paints, its effects applied, meets one of `areas`.
    fn meets(&self, n: &Node, areas: &[Rect]) -> bool {
        vectorcraft_render::painted_bounds(self.d, n).is_some_and(|b| areas.iter().any(|r| b.intersect(*r).area() > 0.0 || r.contains_rect(b)))
    }

    /// The parts of `areas` inside the bounds of clip group `n` (its clipping path's).
    fn clip(&self, n: &Node, areas: &[Rect]) -> Vec<Rect> {
        let Some(c) = vectorcraft_render::painted_bounds(self.d, n) else { return areas.to_vec() };
        areas.iter().filter(|r| c.intersect(**r).area() > 0.0 || r.contains_rect(c)).map(|r| r.intersect(c)).collect()
    }
}

/// The runs of `t` that print, and whether its text doesn't fit: all of them, or of area type and
/// type on a path that don't fit, the runs that their layout draws glyphs or inline graphics of.
fn drawn_runs(t: &vectorcraft_doc::TextObject) -> (Vec<&TextRun>, bool) {
    let drawn: Option<BTreeSet<usize>> = (!matches!(t.kind, TextKind::Point))
        .then(|| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t))
        .filter(|lay| lay.overflow)
        .map(|lay| lay.glyphs.iter().map(|g| g.run).chain(lay.inlines.iter().map(|i| i.run)).collect());
    let runs = t.runs.iter().enumerate().filter(|(i, _)| drawn.as_ref().is_none_or(|d| d.contains(i))).map(|(_, r)| r).collect();
    (runs, drawn.is_some())
}

/// What an export that draws type (as outlines, embedded fonts or pixels) says when `d` uses font
/// families that aren't available (their type is drawn in the fallback font, not the font it
/// names) or styles that its family doesn't have (the closest style is drawn). Type drawn in a
/// font its file carried isn't substituted.
pub(super) fn substitution_warning(d: &Document) -> Option<String> {
    substitution_warning_of(fonts_of(d, &[], true))
}

/// The families and styles of the runs of `t` drawn in a font its file carried
/// ([`vectorcraft_text::FontDb::file_faces`]): drawn as the file drew them, not in a substitute,
/// though the font they name isn't available.
fn in_file_fonts(t: &vectorcraft_doc::TextObject) -> Vec<(&str, &str)> {
    let faces = vectorcraft_text::FontDb::global().file_faces(t);
    t.runs.iter().zip(faces).filter(|(_, f)| f.is_some()).map(|(r, _)| (r.style.font_family.as_str(), r.style.font_style.as_str())).collect()
}

/// [`substitution_warning`] for type set in `fonts`.
pub(super) fn substitution_warning_of(fonts: BTreeSet<UsedFont>) -> Option<String> {
    substitutions(fonts, ("were written", "was drawn"))
}

/// The font (family, style, version) of character style `st`.
pub(super) fn font_of(st: &vectorcraft_doc::CharStyle) -> UsedFont {
    (st.font_family.clone(), st.font_style.clone(), st.font_version.clone())
}

/// What `text.create`, `text.setStyle`, `text.setRangeStyle` and `text.replaceFont` say when the
/// fonts they set type in (`fonts`) don't draw it: the family isn't available (the fallback font
/// is drawn) or doesn't have that style (its closest style is drawn). `None` when all draw it.
pub(super) fn set_font_warning(fonts: BTreeSet<UsedFont>) -> Option<String> {
    substitutions(fonts, ("are drawn", "is drawn"))
}

/// The warning for type set in `fonts`, with the verbs `written` and `drawn` (an export's are
/// `were written` and `was drawn`).
fn substitutions(fonts: BTreeSet<UsedFont>, (written, drawn): (&str, &str)) -> Option<String> {
    let db = vectorcraft_text::FontDb::global();
    let mut missing: Vec<String> = Vec::new();
    let mut styles: BTreeSet<String> = BTreeSet::new();
    for (family, style, _) in fonts {
        match db.resolve(&family, &style) {
            None | Some((_, vectorcraft_text::FontMatch::Missing)) => missing.push(family),
            Some((face, vectorcraft_text::FontMatch::Style)) => {
                styles.insert(format!("{family} {style} {drawn} in {} {}", face.family, face.style));
            }
            Some(_) => {}
        }
    }
    // Sorted by family: its styles and versions are neighbours.
    missing.dedup();
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!(
            "fonts that aren't available {written} in the fallback font, {}: {} (install them, or replace them with Find Font)",
            vectorcraft_text::FALLBACK_FAMILY,
            missing.join(", ")
        ));
    }
    if !styles.is_empty() {
        parts.push(format!(
            "font styles that aren't available {written} in the closest style: {} (pick a style the family has)",
            styles.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn text(d: &Document, id: NodeId) -> Option<&vectorcraft_doc::TextObject> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

fn fonts(s: &mut Session, p: &Value) -> Result<Value> {
    if bool_or(p, "selectionOnly", false) {
        let selected = s.doc()?.selection.objects.clone();
        font_inventory(s, Some(&selected))
    } else {
        font_inventory(s, None)
    }
}

impl Session {
    /// List the fonts in the document, or in the given objects and their symbols.
    pub fn font_inventory(&self, selected: Option<&[NodeId]>) -> Result<Value> {
        font_inventory(self, selected)
    }

    /// List text currently unavailable to a Replace Font request in its original selection.
    pub fn skipped_font_ids(&self, params: &Value, selected: Option<&[NodeId]>) -> Result<Vec<NodeId>> {
        let (family, style) = font_param(params, "from", "text.replaceFont")?;
        let selection_only = bool_or(params, "selectionOnly", false);
        let selected_symbols = selection_only.then(|| selected_symbols(self, selected)).transpose()?;
        let d = &self.doc()?.doc;
        let other: Vec<NodeId> = {
            let editable = d.editable();
            scope(self, selection_only, selected)?.into_iter().filter(|id| !editable(*id)).collect()
        };
        let uses = |t: &TextObject| {
            t.runs.iter().any(|r| {
                r.style.font_family.eq_ignore_ascii_case(&family) && style.as_ref().is_none_or(|name| r.style.font_style.eq_ignore_ascii_case(name))
            })
        };
        let mut ids = other.into_iter().filter(|id| text(d, *id).is_some_and(uses)).collect::<Vec<_>>();
        fn visit(n: &Node, open: bool, closed: bool, uses: &impl Fn(&TextObject) -> bool, ids: &mut Vec<NodeId>) {
            let open = open && n.visible && !n.locked;
            if let NodeKind::Text(t) = &n.kind
                && (!open || closed)
                && uses(t)
            {
                ids.push(n.id);
            }
            for child in n.children().map(Vec::as_slice).unwrap_or_default() {
                visit(child, open, closed, uses, ids);
            }
        }
        for symbol in d
            .symbols
            .iter()
            .filter(|symbol| !being_edited(d, &symbol.name) && selected_symbols.as_ref().is_none_or(|(all, _)| all.contains(&symbol.name)))
        {
            let closed = selected_symbols.as_ref().is_some_and(|(_, open)| !open.contains(&symbol.name));
            visit(&symbol.art, true, closed, &uses, &mut ids);
        }
        Ok(ids)
    }
}

fn font_inventory(s: &Session, selected: Option<&[NodeId]>) -> Result<Value> {
    let ids = scope(s, selected.is_some(), selected)?;
    let d = &s.doc()?.doc;
    let db = vectorcraft_text::FontDb::global();
    // The type in the art of the symbols in scope too: all of them, or those the selection shows.
    let symbols = if selected.is_some() { Some(selected_symbols(s, selected)?.0) } else { None };
    let mut texts: Vec<&TextObject> = ids.iter().filter_map(|id| text(d, *id)).collect();
    texts.extend(symbol_texts(d, symbols.as_ref()).into_iter().map(|(t, _)| t));
    // Each font's runs, objects and the text of its runs.
    let mut found: BTreeMap<UsedFont, (usize, usize, Vec<&str>)> = BTreeMap::new();
    for t in texts {
        let mut seen = BTreeSet::new();
        for r in &t.runs {
            let font = (r.style.font_family.clone(), r.style.font_style.clone(), r.style.font_version.clone());
            let e = found.entry(font.clone()).or_default();
            e.0 += 1;
            e.2.push(&r.text);
            if seen.insert(font) {
                e.1 += 1;
            }
        }
    }
    let list: Vec<Value> = found
        .into_iter()
        .map(|((family, style, version), (runs, objects, texts))| {
            // The face the type is set in: the version it names when that one is installed.
            let mut resolved = db.resolve(&family, &style).map(|(f, m)| (db.face_version(&family, &style, version.as_deref()).unwrap_or(f), m));
            let (status, lacking) = match db.composite_coverage(&family, &texts) {
                // A composite font: each character is drawn by the font of the part it falls in, and
                // the row names the face of the first part that isn't there as named.
                Some(c) => {
                    if c.status != vectorcraft_text::FontMatch::Exact {
                        resolved = c.face.map(|f| (f, c.status));
                    }
                    (c.status, c.lacking)
                }
                None => {
                    let status = resolved.as_ref().map(|(_, m)| *m).unwrap_or(vectorcraft_text::FontMatch::Missing);
                    let mut lacking: Vec<char> = texts.concat().chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect();
                    lacking.sort_unstable();
                    lacking.dedup();
                    lacking.retain(|c| !resolved.as_ref().is_some_and(|(f, _)| f.covers(*c)));
                    (status, lacking)
                }
            };
            let mut row = json!({
                "family": family, "style": style, "runs": runs, "objects": objects,
                "missing": status == vectorcraft_text::FontMatch::Missing,
                "status": status.as_str(),
                "resolved": resolved.as_ref().map(|(f, _)| json!({ "family": f.family, "style": f.style })),
                "missingGlyphs": lacking.len(),
            });
            if let Some(v) = version {
                row["version"] = json!(v);
            }
            row
        })
        .collect();
    Ok(json!(list))
}

fn font_param(p: &Value, k: &str, c: &str) -> Result<(String, Option<String>)> {
    let v = p.get(k).ok_or_else(|| bad(c, format!("missing `{k}`")))?;
    let family =
        v.get("family").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).ok_or_else(|| bad(c, format!("`{k}.family` is required")))?;
    Ok((family.to_string(), v.get("style").and_then(Value::as_str).map(str::to_string)))
}

fn skipped_font_texts(
    d: &Document,
    other: &[NodeId],
    symbols: &[(&TextObject, bool)],
    closed: &BTreeSet<String>,
    uses: &impl Fn(&TextObject) -> bool,
) -> usize {
    let mut skipped = other.iter().filter(|id| text(d, **id).is_some_and(uses)).count();
    skipped += symbols.iter().filter(|(t, open)| !open && uses(t)).count();
    if !closed.is_empty() {
        skipped += symbol_texts(d, Some(closed)).into_iter().filter(|(t, _)| uses(t)).count();
    }
    skipped
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.replaceFont";
    let (ff, fs) = font_param(p, "from", C)?;
    let (tf, ts) = font_param(p, "to", C)?;
    let db = vectorcraft_text::FontDb::global();
    // Any name of the family (ヒラギノ角ゴシック) is stored as its own name (Hiragino Sans), and a
    // composite font as its own name.
    let composite = db.composite_font(&tf).is_some();
    let Some(tf) = db.stored_family(&tf, ts.as_deref().unwrap_or("Regular")) else {
        return Err(bad(C, format!("font family `{tf}` is not available")));
    };
    let matches = |st: &vectorcraft_doc::CharStyle| {
        st.font_family.eq_ignore_ascii_case(&ff) && fs.as_ref().is_none_or(|x| st.font_style.eq_ignore_ascii_case(x))
    };
    // The style a matching run is set in after the change, or `None` when the run doesn't match or
    // is already set in that family and style with no version or Variable Font settings.
    let new_style = |st: &vectorcraft_doc::CharStyle| {
        if !matches(st) {
            return None;
        }
        // An explicit style, else the closest the new family has to the old one (a composite font's
        // parts name their own styles: the run keeps its style).
        let closest = || if composite { None } else { db.face(&tf, &st.font_style).map(|f| f.style.clone()) };
        let style = ts.clone().or_else(closest).unwrap_or_else(|| st.font_style.clone());
        (st.font_family != tf || st.font_style != style || st.font_version.is_some() || !st.font_axes.is_empty()).then_some(style)
    };
    // Only text the Selection tool can reach changes: not hidden or locked, in a hidden or locked
    // group or layer, or on a locked artboard. The text left in the font is counted. The type in
    // the art of the symbols in scope (all of them, or those the selection shows) changes too,
    // unless it or a group around it in the art is hidden or locked.
    let selection_only = bool_or(p, "selectionOnly", false);
    // With selectionOnly: the symbols the selection shows; those shown only by hidden or locked
    // instances keep their type, counted in `skipped`.
    let (symbols, closed) = if selection_only {
        let (all, open) = selected_symbols(s, None)?;
        let closed: BTreeSet<String> = all.difference(&open).cloned().collect();
        (Some(open), closed)
    } else {
        (None, BTreeSet::new())
    };
    let uses = |t: &TextObject| t.runs.iter().any(|r| matches(&r.style));
    // Whether a run of `t` changes.
    let changes = |t: &TextObject| t.runs.iter().any(|r| new_style(&r.style).is_some());
    let (ids, skipped, in_symbols) = {
        let d = &s.doc()?.doc;
        let editable = d.editable();
        let (ids, other): (Vec<NodeId>, Vec<NodeId>) = scope(s, selection_only, None)?.into_iter().partition(|id| editable(*id));
        let texts = symbol_texts(d, symbols.as_ref());
        let skipped = skipped_font_texts(d, &other, &texts, &closed, &uses);
        // The text that changes: when none does, the document and its undo history stay as they are.
        let ids: Vec<NodeId> = ids.into_iter().filter(|id| text(d, *id).is_some_and(changes)).collect();
        let in_symbols = texts.iter().any(|(t, open)| *open && changes(t));
        (ids, skipped, in_symbols)
    };
    // The fonts the replaced runs are set in, for the warning.
    let mut set = BTreeSet::new();
    let n = if ids.is_empty() && !in_symbols {
        0
    } else {
        s.edit("Replace Font", |d, _| {
            let mut n = 0;
            let mut replace_in = |t: &mut TextObject| {
                let mut changed = false;
                for r in &mut t.runs {
                    if let Some(style) = new_style(&r.style) {
                        r.style.font_family = tf.clone();
                        r.style.font_style = style;
                        // Another font: none of the old one's versions or Variable Font settings.
                        r.style.font_version = None;
                        r.style.font_axes.clear();
                        set.extend(run_fonts(r));
                        changed = true;
                        n += 1;
                    }
                }
                if changed {
                    vectorcraft_text::edit::normalize(&mut t.runs);
                    refresh_bounds(t);
                }
            };
            for id in &ids {
                if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                    replace_in(t);
                }
            }
            edit_symbol_texts(d, symbols.as_ref(), &|t, open| open && changes(t), &mut replace_in);
            Ok(n)
        })?
    };
    let mut out = json!({ "runs": n });
    let mut warnings = Vec::new();
    if skipped > 0 {
        out["skipped"] = json!(skipped);
        warnings.push(REPLACE_SKIPPED.to_string());
    }
    warnings.extend(set_font_warning(set));
    if !warnings.is_empty() {
        out["warning"] = json!(warnings.join("; "));
    }
    Ok(out)
}

/// What Replace Font says when it left text in the font that it can't reach.
const REPLACE_SKIPPED: &str = "some text is hidden or locked, so its font wasn't replaced";

fn select_font(s: &mut Session, p: &Value) -> Result<Value> {
    let family = str_param(p, "family").ok_or_else(|| bad("select.font", "missing `family`"))?.to_string();
    let style = str_param(p, "style").map(str::to_string);
    let ids = scope(s, false, None)?;
    let d = &s.doc()?.doc;
    let within = FontScope::read(d, p)?;
    // Only text the Selection tool can reach: not hidden or locked, in a hidden or locked group
    // or layer, or on a locked artboard.
    let editable = d.editable();
    let uses = |t: &TextObject| {
        t.runs
            .iter()
            .any(|r| r.style.font_family.eq_ignore_ascii_case(&family) && style.as_ref().is_none_or(|x| r.style.font_style.eq_ignore_ascii_case(x)))
    };
    let mut hits: Vec<NodeId> = ids.into_iter().filter(|id| editable(*id) && text(d, *id).is_some_and(uses)).collect();
    // The symbols whose art uses the font in type that is not hidden or locked, nor in a hidden or
    // locked group of the art, also through the instances of other symbols it holds.
    fn reaches(n: &Node, uses: &impl Fn(&TextObject) -> bool, symbols: &BTreeSet<&str>) -> bool {
        n.visible
            && !n.locked
            && match &n.kind {
                NodeKind::Text(t) => uses(t),
                NodeKind::SymbolInstance { symbol, .. } => symbols.contains(symbol.as_str()),
                _ => n.children().is_some_and(|ch| ch.iter().any(|c| reaches(c, uses, symbols))),
            }
    }
    let mut symbols: BTreeSet<&str> = BTreeSet::new();
    loop {
        let found: Vec<&str> = d
            .symbols
            .iter()
            .filter(|sym| !symbols.contains(sym.name.as_str()) && reaches(&sym.art, &uses, &symbols))
            .map(|sym| sym.name.as_str())
            .collect();
        if found.is_empty() {
            break;
        }
        symbols.extend(found);
    }
    // Their instances the Selection tool can reach.
    if !symbols.is_empty() {
        d.walk(&mut |n: &Node| {
            if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
                && symbols.contains(symbol.as_str())
                && editable(n.id)
            {
                hits.push(n.id);
            }
        });
    }
    drop(editable);
    let symbols = d.symbol_boxes();
    hits.retain(|id| within.holds(d, &symbols, *id));
    drop(symbols);
    let n = hits.len();
    s.select(|_, sel| sel.set(hits))?;
    Ok(json!({ "count": n }))
}

/// Where Select Text by Font looks: its `scope`.
enum FontScope {
    Document,
    /// The artboards' rectangles.
    Artboards(Vec<Rect>),
    /// The layers and sublayers, with what they hold.
    Layers(BTreeSet<NodeId>),
}

impl FontScope {
    fn read(d: &Document, p: &Value) -> Result<Self> {
        const C: &str = "select.font";
        let expected = || bad(C, "`scope` must be \"document\", {artboards: [index, …]} or {layers: [id, …]}");
        let scope = match p.get("scope") {
            None | Some(Value::Null) => return Ok(Self::Document),
            Some(Value::String(v)) if v == "document" => return Ok(Self::Document),
            Some(Value::Object(m)) if m.len() == 1 => m,
            Some(_) => return Err(expected()),
        };
        let list = |key: &str| -> Result<Option<&Vec<Value>>> {
            match scope.get(key) {
                None => Ok(None),
                Some(Value::Array(a)) if !a.is_empty() => Ok(Some(a)),
                Some(_) => Err(bad(C, format!("`scope.{key}` must be a non-empty array"))),
            }
        };
        if let Some(a) = list("artboards")? {
            let rect = |v: &Value| v.as_u64().and_then(|i| d.artboards.get(index(i))).map(|b| b.rect);
            return a.iter().map(|v| rect(v).ok_or_else(|| bad(C, format!("no such artboard {v}")))).collect::<Result<_>>().map(Self::Artboards);
        }
        if let Some(a) = list("layers")? {
            let layer = |v: &Value| v.as_u64().map(NodeId).filter(|id| d.node(*id).is_some_and(Node::is_layer));
            return a
                .iter()
                .map(|v| layer(v).ok_or_else(|| bad(C, format!("{v} names no layer or sublayer"))))
                .collect::<Result<_>>()
                .map(Self::Layers);
        }
        Err(expected())
    }

    /// Is object `id` in the scope.
    fn holds(&self, d: &Document, symbols: &vectorcraft_doc::SymbolBoxes, id: NodeId) -> bool {
        match self {
            Self::Document => true,
            Self::Artboards(rects) => d.node(id).is_some_and(|n| rects.iter().any(|r| on_artboard(n, symbols, *r))),
            Self::Layers(layers) => d.ancestry(id).is_some_and(|a| a.iter().any(|i| layers.contains(i))),
        }
    }
}

fn font_list(_: &mut Session, p: &Value) -> Result<Value> {
    let db = vectorcraft_text::FontDb::global();
    match str_param(p, "family") {
        Some(family) => {
            if !db.has_family(family) {
                return Err(bad("text.fontList", format!("font family `{family}` is not available")));
            }
            // The name as the font spells it.
            let name = db.family_list().iter().find(|f| f.eq_ignore_ascii_case(family)).cloned().unwrap_or_else(|| family.to_string());
            let styles = db.styles(family);
            let style = str_param(p, "style").map(str::to_string).or_else(|| styles.first().cloned()).unwrap_or_else(|| "Regular".into());
            let axes: Vec<Value> = db
                .face(family, &style)
                .map(|face| {
                    let axes = face.axes();
                    axes.iter()
                        .map(|a| {
                            json!({
                                "tag": String::from_utf8_lossy(&a.tag), "name": a.name, "min": a.min, "default": a.default, "max": a.max,
                                "value": face.axis_value(a.tag).unwrap_or(a.default), "hidden": a.hidden,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({ "family": name, "styles": styles, "axes": axes }))
        }
        None => Ok(json!({ "families": *db.menu_family_list() })),
    }
}

pub(crate) fn rescan(s: &mut Session, _: &Value) -> Result<Value> {
    let db = vectorcraft_text::FontDb::global();
    #[cfg(not(target_arch = "wasm32"))]
    let faces = db.load_system_fonts();
    #[cfg(target_arch = "wasm32")]
    let faces = 0;
    // The canvas redraws type in the fonts available now (renderers lay it out again when the
    // fonts change), and the bounds follow; that isn't an edit to save.
    for d in &mut s.docs {
        let mut texts = vec![];
        d.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                texts.push(n.id);
            }
        });
        if !texts.is_empty() {
            d.change_unsaved(|doc| {
                let doc = std::sync::Arc::make_mut(doc);
                for id in texts {
                    if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
                        refresh_bounds(t);
                    }
                }
            });
        }
        d.revision += 1;
    }
    Ok(json!({ "families": db.menu_family_list().len(), "faces": faces }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type › Options › Additional Fonts Folder (#683) adds its folder to the system font scan,
    /// trimmed; clearing it takes the folder out again.
    #[test]
    fn the_additional_fonts_folder_joins_the_font_scan() {
        let mut s = Session::new();
        let dir = std::env::temp_dir().join("vectorcraft-fonts-folder-pref");
        s.execute("prefs.set", &json!({"key": "fontsFolder", "value": format!("  {}  ", dir.display())})).unwrap();
        assert_eq!(vectorcraft_text::user_font_dirs(), std::slice::from_ref(&dir));
        if !cfg!(target_arch = "wasm32") {
            assert!(vectorcraft_text::system_font_dirs().contains(&dir));
        }
        s.execute("prefs.set", &json!({"key": "fontsFolder", "value": ""})).unwrap();
        assert!(vectorcraft_text::user_font_dirs().is_empty() && !vectorcraft_text::system_font_dirs().contains(&dir));
    }

    #[test]
    fn list_flag_missing_replace_and_select() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let a = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap();
        let b = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "Gone", "font": "No Such Font Family"})).unwrap()["id"].as_u64().unwrap();
        let list = s.execute("text.fonts", &json!({})).unwrap();
        let missing: Vec<&str> = list.as_array().unwrap().iter().filter(|f| f["missing"] == true).map(|f| f["family"].as_str().unwrap()).collect();
        assert_eq!(missing, ["No Such Font Family"]);
        assert_eq!(s.execute("select.font", &json!({"family": "No Such Font Family"})).unwrap()["count"], 1);
        assert_eq!(s.doc().unwrap().selection.objects, [NodeId(b)]);
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "No Such Font Family"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r["runs"], 1);
        let fams: Vec<String> =
            s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap().to_string()).collect();
        assert_eq!(fams, ["Source Sans 3", "Source Serif 4"]);
        assert!(s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Nope"}})).is_err());
        let _ = a;
    }

    /// Select Text by Font leaves out hidden and locked text, and text inside a hidden or locked
    /// group or layer. Text on a visible, unlocked template layer is selected.
    #[test]
    fn select_font_skips_hidden_and_locked_text() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let text = |s: &mut Session, y: i32, font: &str| {
            s.execute("text.create", &json!({"x": 10, "y": y, "text": "Serif", "font": font})).unwrap()["id"].as_u64().unwrap()
        };
        let mut ids = vec![];
        for y in [20, 40, 60, 80, 100] {
            ids.push(text(&mut s, y, "Source Serif 4"));
        }
        // Inter is used only by hidden or locked text.
        let only_hidden = text(&mut s, 120, "Inter");
        for (id, group, cmd) in [
            (ids[1], false, "object.hide"),
            (ids[2], false, "object.lock"),
            (ids[3], true, "object.hide"),
            (ids[4], true, "object.lock"),
            (only_hidden, false, "object.hide"),
        ] {
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            if group {
                s.execute("object.group", &json!({})).unwrap();
            }
            s.execute(cmd, &json!({})).unwrap();
        }
        let mut layer_text = |props: &[Value]| {
            let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
            let id = text(&mut s, 140, "Source Serif 4");
            for p in props {
                let mut p = p.clone();
                p["id"] = json!(layer);
                s.execute("layer.setProps", &p).unwrap();
            }
            id
        };
        layer_text(&[json!({"visible": false})]);
        layer_text(&[json!({"locked": true})]);
        // Template on locks the layer; unlocking it makes its text selectable again.
        let template = layer_text(&[json!({"template": true}), json!({"locked": false})]);
        assert_eq!(s.execute("select.font", &json!({"family": "Source Serif 4"})).unwrap()["count"], 2);
        assert_eq!(s.doc().unwrap().selection.objects, [NodeId(ids[0]), NodeId(template)]);
        assert_eq!(s.execute("select.font", &json!({"family": "Inter"})).unwrap()["count"], 0);
        assert!(s.doc().unwrap().selection.objects.is_empty());
    }

    /// Select Text by Font with an artboard scope takes the text on the artboards, as All on
    /// Active Artboard does: overlapping one, or centered on it, a straddling one included.
    #[test]
    fn select_font_in_artboards() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("artboard.new", &json!({"x": 300, "y": 0, "width": 200, "height": 200})).unwrap();
        s.execute("artboard.new", &json!({"x": 600, "y": 0, "width": 200, "height": 200})).unwrap();
        let text = |s: &mut Session, x: i32| {
            NodeId(s.execute("text.create", &json!({"x": x, "y": 50, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap())
        };
        let first = text(&mut s, 10);
        let straddling = text(&mut s, 180);
        let second = text(&mut s, 350);
        let third = text(&mut s, 650);
        let _off = text(&mut s, 1000);
        let select = |s: &mut Session, scope: Value| {
            s.execute("select.font", &json!({"family": "Source Serif 4", "scope": scope})).unwrap();
            s.doc().unwrap().selection.objects.clone()
        };
        assert_eq!(select(&mut s, json!({"artboards": [0]})), [first, straddling]);
        assert_eq!(select(&mut s, json!({"artboards": [1, 2]})), [second, third]);
        assert_eq!(select(&mut s, json!("document")).len(), 5);
        // The same objects All on Active Artboard takes.
        s.execute("select.allOnArtboard", &json!({"artboard": 0})).unwrap();
        assert_eq!(s.doc().unwrap().selection.objects, [first, straddling]);
        // Hidden text stays out.
        s.execute("select.set", &json!({"ids": [third.0]})).unwrap();
        s.execute("object.hide", &json!({})).unwrap();
        assert_eq!(select(&mut s, json!({"artboards": [1, 2]})), [second]);
        for bad in [json!({"artboards": [3]}), json!({"artboards": []}), json!({"artboards": ["x"]}), json!({"pages": [0]}), json!("all"), json!(7)] {
            assert!(s.execute("select.font", &json!({"family": "Source Serif 4", "scope": bad})).is_err(), "{bad}");
        }
    }

    /// Select Text by Font with a layer scope takes the text in the layers, their sublayers
    /// included, and still leaves out hidden and locked text.
    #[test]
    fn select_font_in_layers() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let text = |s: &mut Session, y: i32| {
            NodeId(s.execute("text.create", &json!({"x": 10, "y": y, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap())
        };
        let first_layer = s.doc().unwrap().doc.layers[0].id;
        let a = text(&mut s, 20);
        let second_layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
        let b = text(&mut s, 40);
        let sub = s.execute("layer.newSublayer", &json!({"parent": second_layer})).unwrap()["id"].as_u64().unwrap();
        s.execute("layer.setCurrent", &json!({"id": sub})).unwrap();
        let in_sub = text(&mut s, 60);
        let locked = text(&mut s, 80);
        s.execute("select.set", &json!({"ids": [locked.0]})).unwrap();
        s.execute("object.lock", &json!({})).unwrap();
        let select = |s: &mut Session, layers: Value| {
            s.execute("select.font", &json!({"family": "Source Serif 4", "scope": {"layers": layers}})).unwrap();
            let mut v = s.doc().unwrap().selection.objects.clone();
            v.sort();
            v
        };
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.ancestry(in_sub).unwrap()[1], NodeId(sub), "the text is in the sublayer");
        assert_eq!(select(&mut s, json!([first_layer.0])), [a]);
        assert_eq!(select(&mut s, json!([second_layer])), [b, in_sub]);
        assert_eq!(select(&mut s, json!([sub])), [in_sub]);
        assert_eq!(select(&mut s, json!([first_layer.0, sub])), [a, in_sub]);
        // Not a layer: an error, the selection as it was.
        assert!(s.execute("select.font", &json!({"family": "Source Serif 4", "scope": {"layers": [a.0]}})).is_err());
        assert!(s.execute("select.font", &json!({"family": "Source Serif 4", "scope": {"layers": [u64::MAX]}})).is_err());
        assert_eq!(s.doc().unwrap().selection.objects.len(), 2);
    }

    /// A symbol instance whose art uses the font is selected when the instance is in scope.
    #[test]
    fn select_font_scope_takes_symbol_instances_in_it() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("artboard.new", &json!({"x": 300, "y": 0, "width": 200, "height": 200})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].clone();
        s.execute("symbol.new", &json!({"name": "Mark", "ids": [id]})).unwrap();
        let instance = s.doc().unwrap().selection.objects[0];
        let select = |s: &mut Session, i: u64| {
            s.execute("select.font", &json!({"family": "Source Serif 4", "scope": {"artboards": [i]}})).unwrap()["count"].clone()
        };
        assert_eq!(select(&mut s, 0), 1);
        assert_eq!(s.doc().unwrap().selection.objects, [instance]);
        assert_eq!(select(&mut s, 1), 0);
    }

    /// Replace Font leaves hidden and locked text in its font, and text inside a hidden or locked
    /// group or layer, and says so. Text on a visible, unlocked template layer is changed.
    #[test]
    fn replace_font_skips_hidden_and_locked_text() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let text = |s: &mut Session, y: i32| {
            s.execute("text.create", &json!({"x": 10, "y": y, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap()
        };
        let mut ids = vec![];
        for y in [20, 40, 60, 80, 100] {
            ids.push(text(&mut s, y));
        }
        for (id, group, cmd) in
            [(ids[1], false, "object.hide"), (ids[2], false, "object.lock"), (ids[3], true, "object.hide"), (ids[4], true, "object.lock")]
        {
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            if group {
                s.execute("object.group", &json!({})).unwrap();
            }
            s.execute(cmd, &json!({})).unwrap();
        }
        let mut layer_text = |props: &[Value]| {
            let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
            let id = text(&mut s, 140);
            for p in props {
                let mut p = p.clone();
                p["id"] = json!(layer);
                s.execute("layer.setProps", &p).unwrap();
            }
            id
        };
        ids.push(layer_text(&[json!({"visible": false})]));
        ids.push(layer_text(&[json!({"locked": true})]));
        // Template on locks the layer; unlocking it makes its text editable again.
        let template = layer_text(&[json!({"template": true}), json!({"locked": false})]);
        let family = |s: &Session, id: u64| text_family(s, id);
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r["runs"], 2, "{r}");
        assert_eq!(r["skipped"], 6, "{r}");
        assert_eq!(r["warning"], REPLACE_SKIPPED, "{r}");
        assert_eq!(family(&s, ids[0]), "Source Sans 3");
        assert_eq!(family(&s, template), "Source Sans 3");
        for id in &ids[1..] {
            assert_eq!(family(&s, *id), "Source Serif 4", "object {id}");
        }
        // The font stays in the document's list while the skipped text uses it.
        let fams: Vec<String> =
            s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap().to_string()).collect();
        assert_eq!(fams, ["Source Sans 3", "Source Serif 4"]);
        // Only skipped text left: nothing changes, and the warning says why.
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!((r["runs"].clone(), r["skipped"].clone(), r["warning"].clone()), (json!(0), json!(6), json!(REPLACE_SKIPPED)), "{r}");
        // A plain replacement carries no warning.
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Inter"}})).unwrap();
        assert_eq!(r, json!({"runs": 2}));
    }

    /// Replace Font adds no undo step when no run changes: only locked text uses the font, no text
    /// uses it, or the text is already set in the new font and style.
    #[test]
    fn replace_font_that_changes_nothing_adds_no_undo_step() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let serif = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].as_u64().unwrap();
        let sans = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "Sans", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [serif]})).unwrap();
        s.execute("object.lock", &json!({})).unwrap();
        let steps = |s: &Session| s.doc().unwrap().history.undo.len();
        let before = steps(&s);
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r, json!({"runs": 0, "skipped": 1, "warning": REPLACE_SKIPPED}));
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Nothing Uses This"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r, json!({"runs": 0}));
        let style = text(&s.doc().unwrap().doc, NodeId(sans)).unwrap().runs[0].style.font_style.clone();
        let r =
            s.execute("text.replaceFont", &json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Source Sans 3", "style": style}})).unwrap();
        assert_eq!(r, json!({"runs": 0}));
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!(r, json!({"runs": 0}));
        assert_eq!(steps(&s), before);
        // A change is still one step.
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Inter"}})).unwrap();
        assert_eq!(r, json!({"runs": 1}));
        assert_eq!(steps(&s), before + 1);
        assert_eq!(text_family(&s, sans), "Inter");
    }

    /// Change and Change All add no undo step when the type in a symbol's art already has the new
    /// font, and a change to that type is still one step.
    #[test]
    fn replace_font_in_symbols_that_changes_nothing_adds_no_undo_step() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Sans", "font": "Source Sans 3"})).unwrap()["id"].clone();
        s.execute("symbol.new", &json!({"name": "Mark", "ids": [id]})).unwrap();
        let mut on_layers = 0;
        s.doc().unwrap().doc.walk(|n| on_layers += usize::from(matches!(n.kind, NodeKind::Text(_))));
        assert_eq!(on_layers, 0, "the type is only in the symbol");
        let family = |s: &Session| symbol_texts(&s.doc().unwrap().doc, None)[0].0.runs[0].style.font_family.clone();
        let style = symbol_texts(&s.doc().unwrap().doc, None)[0].0.runs[0].style.font_style.clone();
        let steps = |s: &Session| s.doc().unwrap().history.undo.len();
        let before = steps(&s);
        // New Symbol selects the instance: Change (the selection) and Change All.
        for selection_only in [true, false] {
            for to in [json!({"family": "Source Sans 3"}), json!({"family": "Source Sans 3", "style": style})] {
                let p = json!({"from": {"family": "Source Sans 3"}, "to": to, "selectionOnly": selection_only});
                assert_eq!(s.execute("text.replaceFont", &p).unwrap(), json!({"runs": 0}), "{p}");
            }
        }
        assert_eq!(steps(&s), before);
        // A change is still one step, with the instance selected and without.
        let r =
            s.execute("text.replaceFont", &json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Inter"}, "selectionOnly": true})).unwrap();
        assert_eq!(r, json!({"runs": 1}));
        assert_eq!(steps(&s), before + 1);
        assert_eq!(family(&s), "Inter");
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Inter"}, "to": {"family": "Source Serif 4"}})).unwrap();
        assert_eq!(r, json!({"runs": 1}));
        assert_eq!(steps(&s), before + 2);
        assert_eq!(family(&s), "Source Serif 4");
    }

    /// The fonts of the symbols' art: Find Font lists them, and Change All replaces them, leaving
    /// hidden or locked type in the art as it is.
    #[test]
    fn find_font_reaches_the_type_in_symbols() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let mut ids = vec![];
        for y in [20, 40, 60] {
            ids.push(s.execute("text.create", &json!({"x": 10, "y": y, "text": "Serif", "font": "Source Serif 4"})).unwrap()["id"].clone());
        }
        s.execute("symbol.new", &json!({"name": "Mark", "ids": ids})).unwrap();
        s.edit("Hide and lock", |d, _| {
            let art = Arc::make_mut(&mut d.symbols[0].art);
            let members = art.children_mut().unwrap();
            Arc::make_mut(&mut members[1]).visible = false;
            Arc::make_mut(&mut members[2]).locked = true;
            Ok(())
        })
        .unwrap();
        let mut on_layers = 0;
        s.doc().unwrap().doc.walk(|n| on_layers += usize::from(matches!(n.kind, NodeKind::Text(_))));
        assert_eq!(on_layers, 0, "the type is only in the symbol");
        let fonts = s.execute("text.fonts", &json!({})).unwrap();
        assert_eq!(
            (fonts[0]["family"].as_str(), fonts[0]["runs"].as_u64(), fonts[0]["objects"].as_u64()),
            (Some("Source Serif 4"), Some(3), Some(3)),
            "{fonts}"
        );
        // New Symbol selects the instance, which shows the symbol's type.
        assert_eq!(s.execute("text.fonts", &json!({"selectionOnly": true})).unwrap(), fonts);
        s.execute("select.set", &json!({"ids": []})).unwrap();
        assert_eq!(s.execute("text.fonts", &json!({"selectionOnly": true})).unwrap(), json!([]));
        // Find selects the symbol's instance, also when it is inside another symbol's instance.
        let mut instances = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
                instances.push(n.id);
            }
        });
        assert_eq!(instances.len(), 1);
        assert_eq!(s.execute("select.font", &json!({"family": "Source Serif 4"})).unwrap()["count"], 1);
        assert_eq!(s.doc().unwrap().selection.objects, instances);
        s.execute("symbol.new", &json!({"name": "Outer", "ids": [instances[0].0]})).unwrap();
        assert_eq!(s.execute("select.font", &json!({"family": "Source Serif 4"})).unwrap()["count"], 1);
        assert_ne!(s.doc().unwrap().selection.objects, instances);
        assert_eq!(s.execute("select.font", &json!({"family": "Inter"})).unwrap()["count"], 0);
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Source Sans 3"}})).unwrap();
        assert_eq!((r["runs"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(2)), "{r}");
        let family = |s: &Session| symbol_texts(&s.doc().unwrap().doc, None)[0].0.runs[0].style.font_family.clone();
        let families: Vec<String> = symbol_texts(&s.doc().unwrap().doc, None).iter().map(|(t, _)| t.runs[0].style.font_family.clone()).collect();
        assert_eq!(families, ["Source Sans 3", "Source Serif 4", "Source Serif 4"]);
        // Find leaves out instances whose symbol uses the font only in hidden or locked type.
        assert_eq!(s.execute("select.font", &json!({"family": "Source Serif 4"})).unwrap()["count"], 0);
        // Only the selection: with nothing selected, the symbols' art stays as it is.
        let sans_to_inter = json!({"from": {"family": "Source Sans 3"}, "to": {"family": "Inter"}, "selectionOnly": true});
        assert_eq!(s.execute("text.replaceFont", &sans_to_inter).unwrap(), json!({"runs": 0}));
        assert_eq!(family(&s), "Source Sans 3");
        // With the instance that Find selects, the selection's fonts and Change reach its symbols' type.
        assert_eq!(s.execute("select.font", &json!({"family": "Source Sans 3"})).unwrap()["count"], 1);
        let fonts = s.execute("text.fonts", &json!({"selectionOnly": true})).unwrap();
        let rows: Vec<(&str, u64)> = fonts.as_array().unwrap().iter().map(|f| (f["family"].as_str().unwrap(), f["runs"].as_u64().unwrap())).collect();
        assert_eq!(rows, [("Source Sans 3", 1), ("Source Serif 4", 2)]);
        assert_eq!(s.execute("text.replaceFont", &sans_to_inter).unwrap()["runs"].as_u64(), Some(1));
        assert_eq!(family(&s), "Inter");
        // While the symbol is edited, its type on the editing layer is counted once.
        s.execute("symbol.edit", &json!({"name": "Mark"})).unwrap();
        let fonts = s.execute("text.fonts", &json!({})).unwrap();
        let serif = fonts.as_array().unwrap().iter().find(|f| f["family"] == "Source Serif 4").unwrap();
        assert_eq!((serif["runs"].as_u64(), serif["objects"].as_u64()), (Some(2), Some(2)), "{fonts}");
        let r = s.execute("text.replaceFont", &json!({"from": {"family": "Inter"}, "to": {"family": "Source Serif 4"}})).unwrap();
        assert_eq!(r["runs"].as_u64(), Some(1), "{r}");
        s.execute("symbol.exitEditing", &json!({})).unwrap();
        // A selected locked instance: its symbols' fonts are listed, and Change leaves them, counted.
        let mut outer = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Outer") {
                outer.push(n.id.0);
            }
        });
        assert_eq!(outer.len(), 1);
        s.execute("select.set", &json!({"ids": outer})).unwrap();
        s.execute("object.lock", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": outer})).unwrap();
        assert_eq!(s.execute("text.fonts", &json!({"selectionOnly": true})).unwrap()[0]["runs"], 3);
        let serif_to_sans = json!({"from": {"family": "Source Serif 4"}, "to": {"family": "Source Sans 3"}, "selectionOnly": true});
        let r = s.execute("text.replaceFont", &serif_to_sans).unwrap();
        assert_eq!((r["runs"].as_u64(), r["skipped"].as_u64()), (Some(0), Some(3)), "{r}");
        // A hidden instance in a symbol's art does not bring its symbol into the selection's scope.
        s.execute("object.unlockAll", &json!({})).unwrap();
        s.edit("Hide the inner instance", |d, _| {
            let i = d.symbols.iter().position(|sym| sym.name == "Outer").unwrap();
            let art = Arc::make_mut(&mut d.symbols[i].art);
            match art.children_mut() {
                Some(members) => Arc::make_mut(&mut members[0]).visible = false,
                None => art.visible = false,
            }
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": outer})).unwrap();
        assert_eq!(s.execute("text.fonts", &json!({"selectionOnly": true})).unwrap(), json!([]));
        assert_eq!(s.execute("text.replaceFont", &serif_to_sans).unwrap(), json!({"runs": 0}));
    }

    /// The font family of text object `id`'s first run.
    fn text_family(s: &Session, id: u64) -> String {
        text(&s.doc().unwrap().doc, NodeId(id)).unwrap().runs[0].style.font_family.clone()
    }
}

#[cfg(test)]
mod open_tests {
    use super::*;

    #[test]
    fn list_says_how_each_font_resolved() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        // "Black Wide" is a style no Source Sans 3 has, bundled or installed, so it is always a
        // substitute (the installed family can hold a real Black).
        for (text, font, style) in
            [("Exact", "Source Sans 3", "Semibold"), ("Closest", "Source Sans 3", "Black Wide"), ("Gone", "No Such Font Family", "Regular")]
        {
            s.execute("text.create", &json!({"x": 10, "y": 20, "text": text, "font": font, "style": style})).unwrap();
        }
        let list = s.execute("text.fonts", &json!({})).unwrap();
        let rows: Vec<(String, String, String, String)> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["style"].as_str().unwrap().into(),
                    f["status"].as_str().unwrap().into(),
                    f["resolved"]["family"].as_str().unwrap().into(),
                    f["resolved"]["style"].as_str().unwrap().into(),
                )
            })
            .collect();
        let row = |style: &str| rows.iter().find(|r| r.0 == style).cloned().unwrap();
        assert_eq!(row("Semibold").1, "exact");
        assert_eq!(row("Black Wide").1, "substitute", "{rows:?}");
        assert_eq!(row("Black Wide").2, "Source Sans 3");
        assert_eq!((row("Regular").1.as_str(), row("Regular").2.as_str()), ("missing", vectorcraft_text::FALLBACK_FAMILY));
        s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Ab 雅楽", "font": "Inter"})).unwrap();
        let inter = s.execute("text.fonts", &json!({})).unwrap().as_array().unwrap().iter().find(|f| f["family"] == "Inter").cloned().unwrap();
        assert_eq!(inter["missingGlyphs"], 2, "雅 and 楽 come from a fallback font: {inter}");
    }

    #[test]
    fn export_warns_when_a_style_is_drawn_in_another_style() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi", "font": "Source Sans 3", "style": "Black Wide"})).unwrap();
        let out = s.execute("document.export", &json!({"format": "pdf"})).unwrap();
        let warnings = out["warnings"].to_string();
        assert!(warnings.contains("Source Sans 3 Black Wide was drawn in Source Sans 3"), "{out}");
    }

    #[test]
    fn setting_type_in_a_font_it_cant_draw_warns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let warning = |out: &Value| out["warning"].as_str().map(str::to_string);
        let made = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi", "font": "Source Sans 3", "style": "Black Wide"})).unwrap();
        assert!(warning(&made).is_some_and(|w| w.contains("Source Sans 3 Black Wide is drawn in Source Sans 3")), "{made}");
        let gone = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "Gone", "font": "No Such Font Family"})).unwrap();
        let w = warning(&gone).unwrap_or_default();
        assert!(w.contains("No Such Font Family") && w.contains(vectorcraft_text::FALLBACK_FAMILY), "{gone}");
        let plain = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Fine", "font": "Source Sans 3", "style": "Semibold"})).unwrap();
        assert_eq!(warning(&plain), None, "{plain}");
        let id = plain["id"].as_u64().unwrap();
        let set = s.execute("text.setStyle", &json!({"ids": [id], "style": "Black Wide"})).unwrap();
        assert!(warning(&set).is_some_and(|w| w.contains("Source Sans 3 Black Wide is drawn in Source Sans 3")), "{set}");
        let back = s.execute("text.setStyle", &json!({"ids": [id], "style": "Bold"})).unwrap();
        assert_eq!(warning(&back), None, "{back}");
        // Only the font the command set: the other objects' substitutes aren't its to report.
        let size = s.execute("text.setStyle", &json!({"ids": [id], "size": 30})).unwrap();
        assert_eq!(warning(&size), None, "{size}");
        // Only the characters the command styles: with "Fi" in Black Wide, styling "ne" doesn't warn.
        let first = s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 2, "style": "Black Wide"})).unwrap();
        assert!(warning(&first).is_some_and(|w| w.contains("Source Sans 3 Black Wide is drawn in Source Sans 3")), "{first}");
        let rest = s.execute("text.setRangeStyle", &json!({"id": id, "start": 2, "end": 4, "font": "Source Sans 3"})).unwrap();
        assert_eq!(warning(&rest), None, "{rest}");
        let range = s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 2, "font": "No Such Font Family"})).unwrap();
        assert!(warning(&range).is_some_and(|w| w.contains("No Such Font Family")), "{range}");
        let replaced = s
            .execute(
                "text.replaceFont",
                &json!({"from": {"family": "No Such Font Family"}, "to": {"family": "Source Sans 3", "style": "Black Wide"}}),
            )
            .unwrap();
        assert!(warning(&replaced).is_some_and(|w| w.contains("Source Sans 3 Black Wide is drawn in Source Sans 3")), "{replaced}");
        // Empty type: the font set is the one the next characters typed get.
        let empty = s.execute("text.create", &json!({"x": 10, "y": 140, "text": ""})).unwrap()["id"].as_u64().unwrap();
        let set = s.execute("text.setStyle", &json!({"ids": [empty], "style": "Black Wide"})).unwrap();
        assert!(warning(&set).is_some_and(|w| w.contains("Black Wide")), "{set}");
        let range = s.execute("text.setRangeStyle", &json!({"id": empty, "font": "No Such Font Family"})).unwrap();
        assert!(warning(&range).is_some_and(|w| w.contains("No Such Font Family")), "{range}");
    }

    #[test]
    fn opened_documents_have_exact_text_bounds() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "WIDE TRACKED", "size": 40})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.setStyle", &json!({"tracking": 400})).unwrap();
        let want = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        // Save and reopen: the cache isn't in the file, yet the bounds must match.
        let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
        let reopened = vectorcraft_format::load(&bytes).unwrap();
        let mut t = Session::new();
        t.add_document(reopened, None);
        let got = t.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
        assert!((got.width() - want.width()).abs() < 1e-6, "{got:?} vs {want:?}");
    }
}

#[cfg(test)]
mod dirty_tests {
    use super::*;

    #[test]
    fn selection_never_marks_modified_and_undo_to_saved_is_clean() {
        let mut s = Session::new();
        s.add_document(Document::new(100.0, 100.0), None);
        let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.doc_mut().unwrap().mark_saved();
        s.execute("select.none", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(!s.doc().unwrap().is_dirty(), "selecting is not an edit");
        s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
        assert!(s.doc().unwrap().is_dirty());
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!s.doc().unwrap().is_dirty(), "back to the saved state");
    }
}
