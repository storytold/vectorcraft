//! Custom arrowheads (Stroke panel menu › Edit Arrowheads…). The arrowheads file is a native
//! document kept next to the preferences ([`Session::attach_arrowheads_file`]). Each of its symbols
//! is one arrowhead, listed by the symbol's name after the built-in heads in the Stroke panel's
//! arrowhead menus. A head points right (+x): the symbol's registration point is its tip, and the
//! art's height (its extent across the line) is drawn as wide as the built-in heads, four times
//! the head's weight. A head covers what its art paints: the visible fills, and the visible
//! strokes outlined, of the paths, compound paths and groups in it, and its gradient meshes, with
//! their geometry effects applied and clip groups clipped. A head is one solid shape in the stroke's paint: art at 0 %
//! opacity is left out, and other opacities, blend modes and opacity masks do not change the
//! shape. Saving the file reloads the heads. A stroke that uses a custom head carries a copy of its art
//! ([`vectorcraft_doc::CustomArrowhead`]), so its document draws it anywhere.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CustomArrowhead, Document, Node, NodeId, NodeKind, Symbol, SymbolRegistration};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, SubPath};
use vectorcraft_pathops::BoolOp;

use super::{CommandSpec, Result, Session, always, bad, cmd};

/// The arrowheads file's name, in the folder of the preferences.
pub const FILE_NAME: &str = "Arrowheads.vectorcraft";
/// The largest arrowheads file read, and the most its document or its images may unpack to when
/// compressed.
pub const MAX_FILE_BYTES: u64 = 64 << 20;
/// The most heads read from the file; symbols past it are skipped.
pub const MAX_HEADS: usize = 256;
/// The most anchors a symbol's art, and the shapes it paints, may have; a symbol past it is skipped.
pub const MAX_ART_ANCHORS: usize = 16 * CustomArrowhead::MAX_ANCHORS;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stroke.arrowheads.list",
            "Arrowheads",
            [],
            None,
            "{} → {arrowheads: [{name (stroke.set `startArrow`/`endArrow` as {custom: name}), reach, halfWidth (in head weights)}], file: the arrowheads file's path or null (no file here, as on the web), skipped: [{name, reason}] symbols of the file that are not heads, error: why the file could not be read, or null} the custom arrowheads, listed after the built-in ones in the Stroke panel's arrowhead menus",
            always,
            list
        ),
        cmd!(
            "stroke.arrowheads.edit",
            "Edit Arrowheads",
            [],
            None,
            "{} open the arrowheads file (a native document next to the preferences; created first with one example head when it is missing). Each symbol in it is one custom arrowhead named after the symbol: it points right, its registration point is the tip, and its height is drawn as wide as the built-in heads; its visible fills and outlined strokes make the head. Saving the file reloads the heads → {path, created: whether the file was made now}",
            has_file,
            edit
        ),
        cmd!(
            "stroke.arrowheads.reload",
            "Reload Arrowheads",
            [],
            None,
            "{} read the arrowheads file again (a save of it from VectorCraft does this already) → {count, skipped: [{name, reason}]}",
            has_file,
            reload
        ),
    ]
}

/// The custom arrowheads read from the arrowheads file.
#[derive(Default)]
pub struct Arrowheads {
    path: Option<String>,
    heads: Vec<Arc<CustomArrowhead>>,
    skipped: Vec<(String, String)>,
    error: Option<String>,
}

impl Arrowheads {
    /// The arrowheads file's path; `None` without one (the web, runs without preferences).
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
    /// The heads, in the order of the file's symbols.
    pub fn heads(&self) -> &[Arc<CustomArrowhead>] {
        &self.heads
    }
    /// Head `name`.
    pub fn get(&self, name: &str) -> Option<Arc<CustomArrowhead>> {
        self.heads.iter().find(|h| h.name == name).cloned()
    }
    fn set(&mut self, read: Read) {
        (self.heads, self.skipped) = (read.heads.into_iter().map(Arc::new).collect(), read.skipped);
        self.error = None;
    }
}

/// The heads a document holds, and the symbols that are not heads with the reason.
#[derive(Debug, Default)]
pub struct Read {
    pub heads: Vec<CustomArrowhead>,
    pub skipped: Vec<(String, String)>,
}

/// The heads of document `doc`: one per symbol, up to [`MAX_HEADS`] (see the module docs).
pub fn heads_of(doc: &Document) -> Read {
    let mut d = doc.clone();
    let brushes = vectorcraft_brush::library(&d);
    let mut out = Read::default();
    for (i, sym) in doc.symbols.iter().enumerate() {
        if i >= MAX_HEADS {
            out.skipped.push((sym.name.clone(), format!("only the first {MAX_HEADS} symbols are read")));
            continue;
        }
        if out.heads.iter().any(|h| h.name == sym.name) {
            out.skipped.push((sym.name.clone(), "another symbol has this name".into()));
            continue;
        }
        match head(&mut d, &brushes, sym) {
            Ok(h) => out.heads.push(h),
            Err(why) => out.skipped.push((sym.name.clone(), why.into())),
        }
    }
    out
}

/// Symbol `sym` of `d` as a head.
fn head(d: &mut Document, brushes: &[vectorcraft_brush::Brush], sym: &Symbol) -> std::result::Result<CustomArrowhead, &'static str> {
    const TOO_DETAILED: &str = "its art is too long or too detailed for a head";
    const UNITED: &str = "its shapes could not be combined into one";
    if art_anchors(&sym.art) > MAX_ART_ANCHORS {
        return Err(TOO_DETAILED);
    }
    let art = baked_art(d, sym);
    let mut shapes = Shapes::default();
    painted(d, brushes, &art, &mut shapes);
    if shapes.too_detailed {
        return Err(TOO_DETAILED);
    }
    if shapes.failed {
        return Err(UNITED);
    }
    let area = shapes.united().ok_or(UNITED)?;
    let b = area.bounds().filter(|b| b.height() > 1e-9 && b.is_finite()).ok_or("its art paints nothing")?;
    let tip = b.center() + d.symbol_registration(&sym.name).offset_from_center(b.width(), b.height());
    // Head units: x back from the tip, y across to the right of +x, 4 units to the art's height.
    let k = 4.0 / b.height();
    let local = Affine::new([-k, 0.0, 0.0, k, k * tip.x, -k * tip.y]);
    let h = CustomArrowhead::new(sym.name.clone(), area.transformed(local));
    if h.path.subpaths.is_empty() {
        return Err(TOO_DETAILED);
    }
    Ok(h)
}

/// The anchors of the paths in `n` and of the pieces of its gradient meshes ([`mesh_anchors`]), its
/// members' included.
fn art_anchors(n: &Node) -> usize {
    let own = match &n.kind {
        NodeKind::Mesh(m) => mesh_anchors(m),
        _ => n.path_data().map_or(0, PathData::anchor_count),
    };
    n.children().map_or(own, |c| c.iter().fold(own, |sum, c| sum.saturating_add(art_anchors(c))))
}

/// The art of symbol `sym` of `d` with every geometry effect in it applied (its objects', fills',
/// strokes' and groups'), as the canvas draws it: the art baked as for export, on a copy of `d`.
fn baked_art(d: &Document, sym: &Symbol) -> Node {
    // A plain new layer, so the file's own layers' effects stay out of the art.
    let mut tmp = d.clone();
    tmp.layers = Document::new(1.0, 1.0).layers;
    let Some(children) = tmp.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) else { return (*sym.art).clone() };
    *children = vec![sym.art.clone()];
    let baked = vectorcraft_render::effects::bake_document(&tmp);
    baked.as_ref().and_then(|b| b.layers.first()?.children()?.first().map(|c| (**c).clone())).unwrap_or_else(|| (*sym.art).clone())
}

/// The shapes a symbol's art paints, up to [`MAX_ART_ANCHORS`] anchors in all.
#[derive(Default)]
struct Shapes {
    list: Vec<(PathData, FillRule)>,
    anchors: usize,
    /// More anchors were asked for than fit; the shapes past the limit were not made.
    too_detailed: bool,
    /// A clip group's art could not be combined or clipped, so a shape it paints is missing.
    failed: bool,
}

impl Shapes {
    /// Whether `anchors` more fit; when they do not, the shapes are too detailed.
    fn room(&mut self, anchors: usize) -> bool {
        self.too_detailed |= self.anchors.saturating_add(anchors) > MAX_ART_ANCHORS;
        !self.too_detailed
    }
    fn push(&mut self, shape: (PathData, FillRule)) {
        if self.room(shape.0.anchor_count()) {
            self.anchors += shape.0.anchor_count();
            self.list.push(shape);
        }
    }
    /// The shapes united, or `None` when one of them could not be (the union would miss it).
    fn united(&self) -> Option<PathData> {
        let refs: Vec<(&PathData, FillRule)> = self.list.iter().map(|(p, r)| (p, *r)).collect();
        vectorcraft_pathops::try_unite_all(&refs).ok()
    }
    /// The shapes of `inside` clipped to `outline` (filled under `rule`), added as one.
    fn push_clipped(&mut self, inside: Shapes, outline: &PathData, rule: FillRule) {
        if inside.too_detailed || !self.room(inside.anchors) {
            self.too_detailed = true;
            return;
        }
        self.failed |= inside.failed;
        let clip = |art: PathData| {
            vectorcraft_pathops::try_boolean(&art, FillRule::NonZero, outline, rule, BoolOp::Intersect, vectorcraft_pathops::DEFAULT_PRECISION).ok()
        };
        let Some(clipped) = inside.united().and_then(clip) else {
            self.failed = true;
            return;
        };
        if !clipped.subpaths.is_empty() {
            self.push((clipped, FillRule::NonZero));
        }
    }
}

/// The shapes the shown paths and compound paths in `n` (baked art) paint: their filled areas and
/// their strokes' outlines, its groups' members included (baking has already made a group's own
/// fills and strokes art among its members), and the areas its gradient meshes cover. A clip
/// group's clipped art is clipped to
/// its clipping path, whose own fill and stroke paint unclipped, as on the canvas. Hidden or fully
/// transparent art is left out.
fn painted(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &Node, out: &mut Shapes) {
    if !n.visible || n.opacity <= 0.0 || out.too_detailed {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => {
            fills(n, out);
            // The outline holds a copy of the path for each fill too, which adds nothing new.
            if let Some(o) = super::pathops::outline_strokes(d, brushes, n) {
                fills_in(&o, out);
            }
        }
        NodeKind::Group { children, clip: true } => {
            let Some((clip, rest)) = children.split_first() else { return };
            painted(d, brushes, clip, out);
            // Nothing to clip by hides the clipped art, as on the canvas.
            let Some((outline, rule)) = vectorcraft_render::effects::clip_outline(clip) else { return };
            let mut inside = Shapes { anchors: out.anchors, ..Default::default() };
            rest.iter().for_each(|c| painted(d, brushes, c, &mut inside));
            inside.anchors -= out.anchors;
            out.push_clipped(inside, &PathData::from_bezpath(&outline), rule);
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().for_each(|c| painted(d, brushes, c, out)),
        NodeKind::Mesh(_) => mesh_area(n, out),
        _ => {}
    }
}

/// The areas the shown paths and gradient meshes in `n` (an outlined stroke) fill, clipped by its
/// clip groups, whose clipping paths' own fills paint unclipped. A stroke with a gradient along or
/// across it is outlined as gradient meshes clipped to the stroke's outline.
fn fills_in(n: &Node, out: &mut Shapes) {
    if !n.visible || n.opacity <= 0.0 || out.too_detailed {
        return;
    }
    match &n.kind {
        NodeKind::Mesh(_) => mesh_area(n, out),
        NodeKind::Group { children, clip: true } => {
            let Some((clip, rest)) = children.split_first() else { return };
            fills_in(clip, out);
            let Some((outline, rule)) = vectorcraft_render::effects::clip_outline(clip) else { return };
            let mut inside = Shapes { anchors: out.anchors, ..Default::default() };
            rest.iter().for_each(|c| fills_in(c, &mut inside));
            inside.anchors -= out.anchors;
            out.push_clipped(inside, &PathData::from_bezpath(&outline), rule);
        }
        _ => match n.children() {
            Some(children) => children.iter().for_each(|c| fills_in(c, out)),
            None => fills(n, out),
        },
    }
}

/// The anchors [`mesh_area`] makes at most of mesh `m`: its outline's, one a patch along each side,
/// when no point of it is fully transparent, else four by four quads of four anchors a patch.
fn mesh_anchors(m: &vectorcraft_doc::GradientMesh) -> usize {
    let (rows, cols) = (m.rows as usize, m.cols as usize);
    if opaque(m) { rows.saturating_add(cols).saturating_mul(2) } else { rows.saturating_mul(cols).saturating_mul(64) }
}

fn opaque(m: &vectorcraft_doc::GradientMesh) -> bool {
    m.points.iter().all(|p| p.opacity > 0.0)
}

/// The area gradient mesh `n` paints: its outline when no point of it is fully transparent, else
/// the pieces of it that are not fully transparent (four by four quads a patch). A mesh whose
/// pieces do not fit in `out` makes the shapes too detailed.
fn mesh_area(n: &Node, out: &mut Shapes) {
    let NodeKind::Mesh(m) = &n.kind else { return };
    if !out.room(mesh_anchors(m)) {
        return;
    }
    if opaque(m) {
        let outline = m.outline();
        if !outline.subpaths.is_empty() {
            out.push((outline, FillRule::NonZero));
        }
        return;
    }
    for q in m.quads(4) {
        if q.opacity > 0.0 || q.corners.iter().any(|(_, o)| *o > 0.0) {
            out.push((PathData::single(SubPath::polyline(&q.pts, true)), FillRule::NonZero));
        }
    }
}

/// The area path `n` fills, when a shown fill paints it.
fn fills(n: &Node, out: &mut Shapes) {
    let paints = |i: &AppearanceItem| matches!(i, AppearanceItem::Fill(f) if f.visible && f.opacity > 0.0 && !f.paint.is_none());
    if n.appearance.items.iter().any(paints)
        && let Some(shape) = super::pathops::node_path(n)
    {
        out.push(shape);
    }
}

/// The document a missing arrowheads file starts as: one example head, a solid triangle.
pub fn starter() -> Document {
    let mut doc = Document::new(612.0, 792.0);
    doc.title = "Arrowheads".into();
    let name = "Example Arrowhead";
    let tri = SubPath::polyline(&[Point::new(-10.0, -8.0), Point::new(10.0, 0.0), Point::new(-10.0, 8.0)], true);
    let art = Node::path(NodeId(0), PathData::single(tri), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0));
    let art = doc.reid(&art);
    doc.symbols.push(Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
    // The tip is the right edge's middle.
    doc.set_symbol_registration(name, SymbolRegistration(5));
    let id = doc.alloc_id();
    let instance = Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::translate((306.0, 396.0)) });
    let layer = doc.layers.first().map(|l| l.id);
    let _ = doc.insert(layer, 0, instance);
    doc
}

impl Session {
    /// Keep the custom arrowheads in the file at `path` and read the heads it holds (none while it
    /// is missing; an unreadable file is reported by `stroke.arrowheads.list`).
    pub fn attach_arrowheads_file(&mut self, path: String) {
        self.arrowheads.path = Some(path);
        self.reload_arrowheads();
    }

    /// Read the arrowheads file again.
    fn reload_arrowheads(&mut self) {
        let Some(path) = self.arrowheads.path.clone() else { return };
        match read(&path) {
            Ok(Some(doc)) => self.arrowheads.set(heads_of(&doc)),
            Ok(None) => self.arrowheads.set(Read::default()),
            Err(e) => {
                self.arrowheads.set(Read::default());
                self.arrowheads.error = Some(e);
            }
        }
    }

    /// A document was saved to `path`: when that is the arrowheads file, its heads are read back
    /// from what was written.
    pub(crate) fn arrowheads_saved(&mut self, path: &str) {
        if self.arrowheads.path.as_deref().is_some_and(|a| same_file(a, path)) {
            self.reload_arrowheads();
        }
    }
}

fn same_file(a: &str, b: &str) -> bool {
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| std::path::PathBuf::from(p));
    std::path::Path::new(a) == std::path::Path::new(b) || canon(a) == canon(b)
}

/// The document in the arrowheads file at `path`; `None` when there is no file.
fn read(path: &str) -> std::result::Result<Option<Document>, String> {
    let size = match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{path}: {e}")),
        Ok(m) if !m.is_file() => return Err(format!("{path}: not a file")),
        Ok(m) => m.len(),
    };
    if size > MAX_FILE_BYTES {
        return Err(format!("{path}: larger than {} MB", MAX_FILE_BYTES >> 20));
    }
    let bytes = crate::file_access::unconfined(|| super::fileio::read_file(path)).map_err(|e| e.to_string())?;
    // A compressed file's document, and its images, may unpack to no more than an uncompressed
    // file may hold.
    let parse = || vectorcraft_format::load_file_capped(&bytes, MAX_FILE_BYTES).map(|f| f.doc).map_err(|e| format!("{path}: {e}"));
    crate::guard::catch_panic(parse).map_err(|_| format!("{path}: could not be read"))?.map(Some)
}

fn has_file(s: &Session) -> std::result::Result<(), String> {
    if s.arrowheads.path.is_some() { Ok(()) } else { Err("no arrowheads file here".into()) }
}

fn skipped(s: &Session) -> Vec<Value> {
    s.arrowheads.skipped.iter().map(|(name, reason)| json!({"name": name, "reason": reason})).collect()
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let heads: Vec<Value> = s.arrowheads.heads.iter().map(|h| json!({"name": h.name, "reach": h.reach(), "halfWidth": h.half_width()})).collect();
    Ok(json!({"arrowheads": heads, "file": s.arrowheads.path, "skipped": skipped(s), "error": s.arrowheads.error}))
}

fn reload(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "stroke.arrowheads.reload";
    s.reload_arrowheads();
    if let Some(e) = &s.arrowheads.error {
        return Err(bad(C, e.clone()));
    }
    Ok(json!({"count": s.arrowheads.heads.len(), "skipped": skipped(s)}))
}

fn edit(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "stroke.arrowheads.edit";
    let path = s.arrowheads.path.clone().ok_or_else(|| bad(C, "no arrowheads file here"))?;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| bad(C, format!("{}: {e}", dir.display())))?;
    }
    // Made only when no file is there, also when another run makes one at the same moment.
    let bytes = vectorcraft_format::save_file(&starter());
    let created = match crate::file_access::unconfined(|| super::fileio::write_new_file(&path, &bytes)) {
        Ok(()) => true,
        Err(_) if std::path::Path::new(&path).exists() => false,
        Err(e) => return Err(bad(C, e.to_string())),
    };
    if created {
        s.reload_arrowheads();
    }
    // Already open: that document comes to the front.
    let open = s.documents().iter().position(|st| st.path.as_deref().is_some_and(|p| same_file(p, &path)));
    match open {
        Some(i) => {
            s.set_active(i);
        }
        None => {
            crate::file_access::unconfined(|| s.execute("document.open", &json!({ "path": path })))?;
        }
    }
    Ok(json!({"path": path, "created": created}))
}
