//! File → Place: another file's art into the active document as one undo step that never touches
//! the clipboard.
//!
//! - A raster image becomes one image at 100% of its physical size: its pixels at the resolution
//!   the file declares ([`fileio::ppi`]; 72 ppi when it declares none), linked to its file with
//!   `link` ([`super::links`]).
//! - An SVG becomes one group of its art; so does a DXF drawing, read with the DXF options
//!   (`dxf`, [`fileio::dxfimport`]).
//! - An EMF or WMF picture becomes one group, clipped to its frame (`crop: "crop"`) or bounded by
//!   its art (`crop: "bounding"`).
//! - An EPS file (or a PostScript .ai) becomes one group, clipped to its bounding box or bounded by
//!   its art: the document an EPS file of ours carries, else what its PostScript draws, else its
//!   preview ([`fileio::load`]).
//! - A PDF/.ai/.ait page or a native document's artboard becomes one group, clipped to the page
//!   (`crop: "crop"`; a PDF page's `art`, `trim`, `bleed` or `media` box too) or bounded by its art
//!   (`crop: "bounding"`).
//! - A text file becomes area type, decoded and cleaned up with the Text Import Options (`text`,
//!   [`text`]).
//!
//! The art lands centred on `at`, fitted into `rect`, or (Replace) where the replaced object was,
//! with its transform; Template puts it on a new template layer. The images, symbols, patterns and
//! swatches it uses join the document ([`adopt`]). `file.place.queue` loads the place cursor (the
//! `place` tool) with several files, which emits one `file.place` per click or drag.

mod adopt;
mod text;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, Document, ImageObject, LinkInfo, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_pdf::CropTo;

use super::fileio::{self, Format, RasterImage};
use super::*;
use crate::{EngineError, MAX_COORD};

const PLACE: &str = "file.place";
const QUEUE: &str = "file.place.queue";
/// The most files one `file.place.queue` loads.
const MAX_QUEUE: usize = 100;
/// The longest thumbnail side `thumbnail` may ask for (px).
const MAX_THUMBNAIL: f64 = 512.0;
/// A text file's natural box (`file.place.info`, the place cursor): a letter page less 1 in margins.
const TEXT_FRAME: Rect = Rect::new(0.0, 0.0, 468.0, 648.0);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.place",
            "Place…",
            ["File"],
            Some("Cmd+Shift+P"),
            "{path | name+dataBase64, link?: true (a raster image keeps its file's path; other files are embedded), text?: {characterSet?: \"unicode\" (UTF-8, or UTF-16 with a byte-order mark; other bytes as the platform's 8-bit set) | \"ansi\" (the platform's 8-bit set), platform?: \"windows\" (Windows-1252) | \"mac\" (Mac Roman), removeLineReturns?: false (each block of lines becomes one paragraph; blank lines end paragraphs), removeParagraphReturns?: false (drop blank lines), replaceSpaces?: n (runs of n ≥ 2 spaces become a tab)} (a .txt file, placed as area type filling rect, the replaced object's bounds, or else the artboard less a 36 pt margin), template?: false (onto a new locked template layer below the current layer), replace?: false (swap the one selected object, keeping its stacking place and transform; no at/rect), at?: [x, y] centre (default: the first artboard's centre), rect?: [x, y, width, height] fit inside, aspect kept (wins over at), page?: 1 (PDF/.ai page, or a native document's artboard), crop?: \"crop\" (clipped to that page or artboard, default) | \"bounding\" (the art's bounds) | \"art\" | \"trim\" | \"bleed\" | \"media\" (a PDF page's boxes), password? (an encrypted PDF), dxf?: {…the DXF options of document.open} (a .dxf drawing: fit fills the artboard under at, else the first; fitted or uncentred art lands with its drawing's artboard on that artboard, else it is centred on at)} → {ids, name, format, linked, width, height, warnings}. Raster images come in at 100% of their physical size (the file's ppi, else 72); SVG, DXF, PDF/.ai, EMF/WMF (clipped to the picture's frame), EPS (clipped to its bounding box; as document.open reads it) and native documents as one group, with the images, symbols, patterns and swatches they use. One undo step; selects what it placed (unless on a template layer); never touches the clipboard",
            has_doc,
            place
        ),
        cmd!(
            query "file.place.info",
            "Placed File Info",
            [],
            None,
            "{path | name+dataBase64, page?, crop?, password?, thumbnail?: px} what file.place would place, without placing it → {name, format, width, height (pt at 100%), pixelWidth?, pixelHeight?, ppi?: [x, y], colorMode?: RGB|Grayscale|CMYK (raster images), warnings, thumbnailBase64?: PNG of at most `thumbnail` (≤ 512) px on its longer side}",
            always,
            info
        ),
        cmd!(
            query "image.info",
            "Image Info",
            [],
            None,
            "{id?} an image object (default: the one selected) → {id, name, linked, link, colorMode: RGB|Grayscale|CMYK, pixelWidth, pixelHeight, ppi: [x, y] at its placed size, width, height}",
            has_doc,
            image_info
        ),
        cmd!(
            query "file.place.queue",
            "Load Place Cursor",
            [],
            None,
            "{paths?: [path…], files?: [{name, dataBase64}…], link?, template?, page?, crop?, text?, dxf?, thumbnail?: px} load the place cursor (the `place` tool) with up to 100 files (as file.place reads them): a click places the current file at 100% with its top-left corner there, a drag places it at the dragged size (aspect kept), ←/→ and ↑/↓ cycle the files, Esc discards the current one; each placement is a file.place, and the previous tool returns after the last → {count, files: [{name, format, width, height, thumbnailBase64?}], skipped: [{name, error}]}",
            has_doc,
            queue
        ),
    ]
}

// ---------- reading the file ----------

/// A file's art at its natural size, before it joins a document.
enum Art {
    Image(RasterImage),
    /// A text file's text, placed as area type filling the natural box.
    Text(String),
    Vector {
        /// The loaded document: the source of the art's resources.
        src: Box<Document>,
        nodes: Vec<Node>,
        /// Crop: the page or artboard the art is clipped to.
        clip: Option<Rect>,
    },
}

struct Loaded {
    /// The file name (the placed object's name).
    name: String,
    format: &'static Format,
    art: Art,
    /// The art's natural box (what `at` centres and `rect` fits).
    natural: Rect,
    warnings: Vec<String>,
    /// The link to a raster image's file (for Link), when read from a path.
    link: Option<LinkInfo>,
    /// The artboard a DXF drawing fitted to an artboard, or not centred, was laid out on: it lands
    /// on the target artboard the same way.
    board: Option<Rect>,
}

/// Points per pixel of a raster image at the resolution it declares (72 ppi when none).
pub(crate) fn pt_per_px(ppi: Option<(f64, f64)>) -> (f64, f64) {
    let (x, y) = ppi.unwrap_or((72.0, 72.0));
    (72.0 / x, 72.0 / y)
}

/// The file `p` names, read into placeable art; a DXF drawing's Fit to Artboard fills `board`
/// (unless the params name their own box).
fn load(p: &Value, cmd: &str, board: Option<Rect>) -> Result<Loaded> {
    let src = fileio::source(p, cmd)?;
    let name = fileio::file_name(src.name);
    if fileio::TEXT_EXTS.contains(&fileio::extension(src.name).as_str()) {
        let text = text::import(&src.bytes, text::TextOptions::parse(p, cmd)?, cmd)?;
        if text.trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(Loaded { name, format: &text::FORMAT, art: Art::Text(text), natural: TEXT_FRAME, warnings: vec![], link: None, board: None });
    }
    let format = fileio::detect(src.name, &src.bytes)
        .ok_or_else(|| bad(cmd, format!("can't place `{name}`: not a format Vector W3K2 reads (see document.formats)")))?;
    let page = match p.get("page") {
        None => 1,
        Some(v) => v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))? as usize,
    };
    let mut opts = fileio::LoadOptions::from_params(cmd, p)?;
    if let Some(b) = board
        && p.get("dxf").and_then(|d| d.get("fitTo")).is_none()
    {
        opts.dxf.fit_to = (b.width(), b.height());
    }
    // PostScript .ai files are read like EPS.
    let pdf = matches!(format.id, "pdf" | "ai" | "ait") && !vectorcraft_pdf::is_postscript(&src.bytes);
    if !pdf && !matches!(opts.crop, CropTo::Crop | CropTo::Bounding) {
        return Err(bad(
            cmd,
            format!("crop `{}`: only PDF pages have art, trim, bleed and media boxes; use \"crop\" or \"bounding\"", opts.crop.id()),
        ));
    }
    let crop = opts.crop != CropTo::Bounding;
    if format.raster {
        let img = fileio::raster_image(&src.bytes)?;
        let (sx, sy) = pt_per_px(img.ppi);
        let natural = Rect::new(0.0, 0.0, img.width as f64 * sx, img.height as f64 * sy);
        let link = src.path.map(|path| super::links::link_info(path, &src.bytes));
        return Ok(Loaded { name, format, art: Art::Image(img), natural, warnings: vec![], link, board: None });
    }
    let anchored = format.id == "dxf" && (opts.dxf.fit || !opts.dxf.center);
    let (mut doc, warnings) = match format.id {
        // Only the page placed is read, its artboard the box asked for (Bounding Box: the art's
        // bounds, below).
        "pdf" | "ai" | "ait" if pdf => {
            let o = fileio::LoadOptions { crop: if crop { opts.crop } else { CropTo::Crop }, ..opts };
            fileio::page_document(&src.bytes, page - 1, &o).map_err(|e| bad(cmd, format!("{name}: {e}")))?
        }
        _ => {
            // Of the open options, only a DXF drawing's apply (the colour mode stays the file's).
            let o = fileio::LoadOptions { dxf: opts.dxf, ..Default::default() };
            let mut l = fileio::load_with(src.name, &src.bytes, &o)?;
            // Linked images (a native document's) show their files, found from its folder.
            super::links::resolve(&mut l.doc, src.path, false);
            (l.doc, l.warnings)
        }
    };
    doc.drop_edit_modes();
    let board = doc.artboards.first().map(|a| a.rect).filter(|_| anchored);
    let layers = std::mem::take(&mut doc.layers);
    let (nodes, clip) = if matches!(format.id, "svg" | "svgz" | "dxf") {
        (art_of(layers.iter().filter(|l| placeable(l)).flat_map(|l| l.children().into_iter().flatten())), None)
    } else {
        // The page placed is a PDF import's only one.
        let index = if pdf { 0 } else { page - 1 };
        let n = doc.artboards.len();
        let board = doc.artboards.get(index).map(|a| a.rect).ok_or_else(|| {
            let what = if matches!(format.id, "vectorcraft" | "template") { "artboard" } else { "page" };
            bad(cmd, format!("page {page}: `{name}` has {n} {what}(s)"))
        })?;
        // A PDF import has one layer per page; a native document's (or an EPS file's) art is
        // whatever lies on the artboard.
        let pages: Vec<&Arc<Node>> = if pdf { layers.get(index).into_iter().collect() } else { layers.iter().collect() };
        let on_board = |c: &&Arc<Node>| c.visual_bounds().is_some_and(|b| b.intersect(board).area() > 0.0 || board.contains(b.origin()));
        let nodes = art_of(pages.into_iter().filter(|l| placeable(l)).flat_map(|l| l.children().into_iter().flatten()).filter(on_board));
        (nodes, crop.then_some(board))
    };
    let natural = clip.or_else(|| nodes.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.visual_bounds())));
    let natural = natural.filter(|r| r.width() > 0.0 || r.height() > 0.0).ok_or_else(|| bad(cmd, format!("`{name}` has no art to place")))?;
    Ok(Loaded { name, format, art: Art::Vector { src: Box::new(doc), nodes, clip }, natural, warnings, link: None, board })
}

/// A visible, non-template layer.
fn placeable(l: &Node) -> bool {
    l.visible && !matches!(l.kind, NodeKind::Layer { template: true, .. })
}

/// The placeable art among `nodes`: guides left out, visible sublayers as groups.
fn art_of<'a>(nodes: impl Iterator<Item = &'a Arc<Node>>) -> Vec<Node> {
    nodes
        .filter_map(|c| match &c.kind {
            NodeKind::Path { guide: true, .. } => None,
            NodeKind::Layer { children, .. } => placeable(c).then(|| {
                let mut g = (**c).clone();
                g.kind = NodeKind::Group { children: art_of(children.iter()).into_iter().map(Arc::new).collect(), clip: false };
                g
            }),
            _ => Some((**c).clone()),
        })
        .collect()
}

/// The art as one object of `d` (its resources added), at its natural size and position.
fn build(d: &mut Document, l: Loaded, link: Option<LinkInfo>) -> Node {
    let mut node = match l.art {
        Art::Image(img) => {
            let (sx, sy) = pt_per_px(img.ppi);
            super::links::store_image(d, &img.key, img.blob, link.is_some());
            let im = ImageObject {
                key: img.key,
                width: img.width,
                height: img.height,
                xf: Affine::scale_non_uniform(sx, sy),
                link,
                placement: Default::default(),
            };
            Node::new(d.alloc_id(), NodeKind::Image(im))
        }
        Art::Text(t) => Node::new(d.alloc_id(), NodeKind::Text(Box::new(text::area_text(t, l.natural)))),
        Art::Vector { src, mut nodes, clip } => {
            adopt::adopt(d, &src, &mut nodes);
            let mut children: Vec<Arc<Node>> = nodes.iter().map(|n| Arc::new(d.reid(n))).collect();
            if let Some(r) = clip {
                let mut c = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::None, Paint::None, 0.0));
                if let NodeKind::Path { clipping, .. } = &mut c.kind {
                    *clipping = true;
                }
                children.insert(0, Arc::new(c));
            }
            Node::new(d.alloc_id(), NodeKind::Group { children, clip: clip.is_some() })
        }
    };
    node.name = Some(l.name);
    node
}

/// `node` moved and scaled by `m` as a whole: strokes, effects and pattern tiles go along.
fn transformed(mut node: Node, m: Affine) -> Node {
    node.transform(m, Scaling { strokes: true, effects: Some(vectorcraft_render::effects::scale_effect), ..Default::default() });
    adopt::each_mut(&mut node, &mut |n| vectorcraft_doc::pattern::transform_pattern_paints(n, m));
    node
}

// ---------- file.place ----------

/// `[x, y, width, height]` with a positive size.
fn rect_param(p: &Value) -> Result<Option<Rect>> {
    let Some(v) = p.get("rect") else { return Ok(None) };
    let n: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    match n[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Ok(Some(Rect::new(x, y, x + w, y + h))),
        _ => Err(bad(PLACE, "rect must be [x, y, width, height] with a positive width and height")),
    }
}

/// The transform that puts new art with the natural box `natural` where `old` is: an image's scale
/// and rotation relative to its 100% size (its resolution's) and its centre are kept (with
/// `image_xf`); other objects keep their centre.
fn replace_xf(d: &Document, old: &Node, natural: Rect, image_xf: bool) -> Affine {
    if image_xf && let NodeKind::Image(im) = &old.kind {
        let (sx, sy) = pt_per_px(d.images.get(&im.key).and_then(|b| fileio::ppi::resolution(&b.bytes)));
        let full = Affine::scale_non_uniform(sx, sy);
        let centre = full * Point::new(im.width as f64 / 2.0, im.height as f64 / 2.0);
        if im.xf.determinant().abs() > 1e-12 {
            return im.xf * full.inverse() * Affine::translate(centre - natural.center());
        }
    }
    let c = old.geometric_bounds().map_or(natural.center(), |b| b.center());
    Affine::translate(c - natural.center())
}

/// `rect` fitted with `natural`'s aspect, centred.
fn fit(natural: Rect, r: Rect) -> Affine {
    let ratio = |a: f64, b: f64| (b > 1e-9).then(|| a / b);
    let k = [ratio(r.width(), natural.width()), ratio(r.height(), natural.height())].into_iter().flatten().reduce(f64::min).unwrap_or(1.0);
    Affine::translate(r.center().to_vec2()) * Affine::scale(k) * Affine::translate(-natural.center().to_vec2())
}

fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let replace = bool_or(p, "replace", false);
    let template = bool_or(p, "template", false);
    let at = point_param(p, "at");
    let rect = rect_param(p)?;
    if replace && (template || at.is_some() || rect.is_some()) {
        return Err(bad(PLACE, "replace keeps the replaced object's place and transform: drop template, at and rect"));
    }
    let corners = [at, rect.map(|r| r.origin()), rect.map(|r| Point::new(r.x1, r.y1))];
    if corners.iter().flatten().any(|c| !(c.x.abs() <= MAX_COORD && c.y.abs() <= MAX_COORD)) {
        return Err(bad(PLACE, "at/rect lie outside the canvas"));
    }
    let st = s.doc()?;
    let old = match (replace, &st.selection.objects[..]) {
        (false, _) => None,
        (true, [id]) => Some(*id),
        (true, _) => return Err(bad(PLACE, "replace: select the one object to replace")),
    };
    let target = board_at(&st.doc, at);
    let mut loaded = load(p, PLACE, target)?;
    let old_node = old.and_then(|id| st.doc.node(id));
    // Type reflows rather than scales: its frame takes the size it is placed at.
    let is_text = matches!(loaded.art, Art::Text(_));
    if is_text {
        let size = match (old_node.and_then(Node::geometric_bounds), rect) {
            (Some(b), _) => b.size(),
            (None, Some(r)) => r.size(),
            (None, None) => text_frame(&st.doc, at).size(),
        };
        loaded.natural = Rect::from_origin_size(Point::ORIGIN, size);
    }
    let natural = loaded.natural;
    let m = match (old_node, rect) {
        (Some(o), _) => replace_xf(&st.doc, o, natural, !is_text),
        (None, Some(r)) => fit(natural, r),
        // Its artboard's bottom-left corner on the target's.
        (None, None) if let (Some(src), Some(to)) = (loaded.board, target) => Affine::translate((to.x0 - src.x0, to.y1 - src.y1)),
        (None, None) => {
            let c = at.or_else(|| st.doc.artboards.first().map(|a| a.rect.center())).unwrap_or_default();
            Affine::translate(c - natural.center())
        }
    };
    let parent = st.insertion_parent();
    let below = st.current_layer().or_else(|| parent.and_then(|p| st.doc.layer_of(p)));
    let link = loaded.link.take().filter(|_| bool_or(p, "link", true));
    let linked = link.is_some();
    let (name, format, warnings) = (loaded.name.clone(), loaded.format.id, std::mem::take(&mut loaded.warnings));
    let (id, size) = s.edit("Place", |d, sel| {
        let node = transformed(build(d, loaded, link), m);
        let size = node.geometric_bounds().unwrap_or_default();
        let (into, index) = match old {
            Some(old) => {
                let (par, idx, _) = d.position(old).ok_or(EngineError::NoNode(old))?;
                d.remove(old)?;
                (par, idx)
            }
            None if template => (Some(template_layer(d, below, &name)?), usize::MAX),
            None => (parent, usize::MAX),
        };
        let id = d.insert(into, index, node)?;
        // Template art is locked: nothing to select.
        if template {
            sel.clear()
        } else {
            sel.set([id])
        }
        Ok((id, size))
    })?;
    Ok(
        json!({ "ids": [id.0], "name": name, "format": format, "linked": linked, "width": size.width(), "height": size.height(), "warnings": warnings }),
    )
}

/// The artboard under `at`, else the first.
fn board_at(d: &Document, at: Option<Point>) -> Option<Rect> {
    at.and_then(|c| d.artboards.iter().find(|a| a.rect.contains(c))).or(d.artboards.first()).map(|a| a.rect)
}

/// Placed type's frame without `rect` or Replace: the artboard under `at` (else the first) less a
/// 36 pt margin.
fn text_frame(d: &Document, at: Option<Point>) -> Rect {
    let board = board_at(d, at).unwrap_or(TEXT_FRAME);
    Rect::from_center_size(board.center(), ((board.width() - 72.0).max(72.0), (board.height() - 72.0).max(72.0)))
}

/// A new locked template layer named after the file, right below the layer `below` (else at the
/// bottom).
fn template_layer(d: &mut Document, below: Option<NodeId>, name: &str) -> Result<NodeId> {
    let id = d.add_layer(Some(&format!("Template {name}")));
    let at = below.and_then(|b| d.layers.iter().position(|l| l.id == b)).unwrap_or(0);
    d.move_node(id, None, at)?;
    if let Some(n) = d.node_mut(id) {
        n.locked = true;
        if let NodeKind::Layer { template, .. } = &mut n.kind {
            *template = true;
        }
    }
    Ok(id)
}

// ---------- file.place.queue ----------

/// The files `file.place.queue` names, as `file.place` params.
fn queued_files(p: &Value) -> Result<Vec<Value>> {
    let list = |k: &str| p.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut out = vec![];
    for v in list("paths") {
        let path = v.as_str().ok_or_else(|| bad(QUEUE, "paths must be strings"))?;
        out.push(json!({ "path": path }));
    }
    for f in list("files") {
        let (Some(name), Some(data)) = (str_param(f, "name"), str_param(f, "dataBase64")) else {
            return Err(bad(QUEUE, "files must be {name, dataBase64}"));
        };
        out.push(json!({ "name": name, "dataBase64": data }));
    }
    match out.len() {
        0 => Err(bad(QUEUE, "give paths or files")),
        n if n > MAX_QUEUE => Err(bad(QUEUE, format!("at most {MAX_QUEUE} files"))),
        _ => Ok(out),
    }
}

fn queue(s: &mut Session, p: &Value) -> Result<Value> {
    let thumbnail = p.get("thumbnail").and_then(Value::as_f64);
    let (mut entries, mut files, mut skipped) = (vec![], vec![], vec![]);
    for mut q in queued_files(p)? {
        for k in ["link", "template", "page", "crop", "text", "dxf"] {
            if let Some(v) = p.get(k) {
                q[k] = v.clone();
            }
        }
        match load(&q, QUEUE, None) {
            Ok(l) => {
                let (w, h) = (l.natural.width(), l.natural.height());
                let mut f = json!({ "name": l.name, "format": l.format.id, "width": w, "height": h });
                entries.push(json!({ "params": q, "name": l.name, "width": w, "height": h }));
                if let Some(px) = thumbnail {
                    f["thumbnailBase64"] = json!(thumbnail_png(l, px)?);
                }
                files.push(f);
            }
            Err(e) => {
                let name = str_param(&q, "path").or(str_param(&q, "name")).map(fileio::file_name).unwrap_or_default();
                skipped.push(json!({ "name": name, "error": e.to_string() }));
            }
        }
    }
    if entries.is_empty() {
        let why = skipped.first().and_then(|e| e["error"].as_str()).unwrap_or("nothing to place");
        return Err(bad(QUEUE, why));
    }
    // Reloading the cursor keeps the tool to return to.
    let prev = Some(s.tool_id()).filter(|t| *t != "place");
    let view = s.last_view;
    s.select_tool("place", view)?;
    s.set_tool_option("queue", &json!({ "entries": entries, "prev": prev }));
    Ok(json!({ "count": files.len(), "files": files, "skipped": skipped }))
}

// ---------- queries ----------

/// The colour mode an image file stores its pixels in.
fn color_mode(bytes: &[u8]) -> &'static str {
    use image::{ExtendedColorType as E, ImageDecoder as _};
    let kind = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .map(|d| d.original_color_type());
    match kind {
        Some(E::L1 | E::La1 | E::L2 | E::La2 | E::L4 | E::La4 | E::L8 | E::La8 | E::L16 | E::La16) => "Grayscale",
        Some(E::Cmyk8 | E::Cmyk16) => "CMYK",
        _ => "RGB",
    }
}

/// `l` rendered as a PNG at most `px` (≤ [`MAX_THUMBNAIL`]) pixels on its longer side, base64.
fn thumbnail_png(l: Loaded, px: f64) -> Result<String> {
    let natural = l.natural;
    let mut d = Document::new(natural.width().max(1.0), natural.height().max(1.0));
    let layer = d.layers.first().map(|l| l.id);
    let node = build(&mut d, l, None);
    d.insert(layer, 0, node)?;
    let k = px.clamp(1.0, MAX_THUMBNAIL) / natural.width().max(natural.height()).max(1e-9);
    let png = vectorcraft_render::Renderer::new().render_region(&d, natural, k, false).to_png().map_err(EngineError::Other)?;
    Ok(vectorcraft_format::base64_encode(&png))
}

fn info(_: &mut Session, p: &Value) -> Result<Value> {
    let mut l = load(p, "file.place.info", None)?;
    let natural = l.natural;
    let mut out = json!({
        "name": l.name,
        "format": l.format.id,
        "width": natural.width(),
        "height": natural.height(),
        "warnings": std::mem::take(&mut l.warnings),
    });
    if let Art::Image(img) = &l.art {
        let (x, y) = img.ppi.unwrap_or((72.0, 72.0));
        out["pixelWidth"] = json!(img.width);
        out["pixelHeight"] = json!(img.height);
        out["ppi"] = json!([x, y]);
        out["colorMode"] = json!(color_mode(&img.blob.bytes));
    }
    if let Some(px) = p.get("thumbnail").and_then(Value::as_f64) {
        out["thumbnailBase64"] = json!(thumbnail_png(l, px)?);
    }
    Ok(out)
}

pub(crate) fn image_info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "image.info";
    let st = s.doc()?;
    let id = match (id_param(p, "id"), &st.selection.objects[..]) {
        (Some(id), _) => id,
        (None, [id]) => *id,
        _ => return Err(bad(C, "select one image or pass id")),
    };
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &n.kind else { return Err(bad(C, format!("object {} is not an image", id.0))) };
    // Effective resolution: 72 pt per inch over the points one pixel spans along each axis.
    let [a, b, c, d, ..] = im.xf.as_coeffs();
    let ppi = |x: f64, y: f64| Some(x.hypot(y)).filter(|l| *l > 1e-12).map_or(0.0, |l| 72.0 / l);
    let size = n.geometric_bounds().unwrap_or_default();
    Ok(json!({
        "id": id.0,
        "name": n.display_name(),
        "linked": im.link.is_some(),
        "link": im.link.as_ref().map(|l| &l.path),
        "colorMode": st.doc.images.get(&im.key).map_or("RGB", |b| color_mode(&b.bytes)),
        "pixelWidth": im.width,
        "pixelHeight": im.height,
        "ppi": [ppi(a, b), ppi(c, d)],
        "width": size.width(),
        "height": size.height(),
    }))
}
