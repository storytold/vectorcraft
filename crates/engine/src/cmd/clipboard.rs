//! The internal clipboard and system clipboard interchange.
//!
//! Copy keeps the objects together with the document resources they use ([`Clipboard`]), so
//! pasting into any document brings those along (see `resources`).
//!
//! System clipboard: the copied objects as SVG markup (what other apps paste), and SVG markup from
//! other apps turned into clipboard objects (pasted with the Paste commands). The UI owns the
//! platform clipboard; these commands only convert. Within VectorCraft the internal clipboard stays
//! lossless (live effects, masks, symbols); SVG is the outside format, with PNG, PDF and plain
//! text besides it (see `flavours`).

mod flavours;
mod resources;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_brush::Brush;
use vectorcraft_color::Swatch;
use vectorcraft_doc::{Document, GraphicStyle, ImageBlob, Node, NodeId, PatternDef, Symbol, TextStyleDef};
use vectorcraft_geom::Rect;

pub use flavours::{AI, BITMAP, EMF, EPS, FILE_HEAD, Flavour, PASTE_ORDER, PDF, PNG, SVG, TEXT, file_flavour, is_address, names_a_picture};
pub(crate) use resources::SwatchChoices;

use super::*;
use crate::DocState;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        cmd!(query "clipboard.exportSvg", "Clipboard as SVG", [], None, "{} → {svg} the copied objects as standalone SVG (null when the clipboard is empty)", always, export_svg),
        cmd!(
            query "clipboard.importSvg",
            "Load SVG into Clipboard",
            [],
            None,
            "{svg | dataBase64 (SVG or SVGZ bytes), center?: [x, y]} replace the clipboard with the SVG's objects (and the images, patterns and symbols they use), centred on `center` (default: the first artboard) → {count}; then run edit.pasteInPlace",
            has_doc,
            import_svg
        ),
        cmd!(
            query "clipboard.conflicts",
            "Swatch Conflicts",
            [],
            None,
            "{} → {swatches: [{name, document, clipboard, spot}]} the global or spot swatches the copied objects use whose name the active document gives another colour (`document`/`clipboard`: #rrggbb): Paste asks Merge or Add for each (pass the answer as edit.paste* {swatchConflict}). Pasting back into the document the objects came from raises none",
            has_doc,
            conflicts
        ),
    ];
    v.extend(flavours::specs());
    v
}

/// Is `text` SVG markup (as other apps put on the clipboard)?
pub fn looks_like_svg(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<svg") || t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE svg")) && t.contains("<svg")
}

/// What Copy puts on the internal clipboard: the objects and every document resource they use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipboard {
    /// The copied objects in paint order (document coordinates).
    pub nodes: Vec<Node>,
    /// Blobs of the images the objects show, by key.
    pub images: BTreeMap<String, ImageBlob>,
    /// Symbols the objects place (also inside other symbols and patterns).
    pub symbols: Vec<Symbol>,
    /// Patterns the objects paint with (pasting makes their swatches again).
    pub patterns: Vec<PatternDef>,
    /// Global and spot swatches the objects' colours link to, the tint swatches of the tints they
    /// use and the gradient swatches their gradients came from. Never the built-in [Registration]
    /// swatch, which every document has.
    pub swatches: Vec<Swatch>,
    /// Graphic styles the objects are linked to.
    pub graphic_styles: Vec<GraphicStyle>,
    pub char_styles: Vec<TextStyleDef>,
    pub para_styles: Vec<TextStyleDef>,
    /// Brushes the objects' strokes use.
    pub brushes: Vec<Brush>,
    /// The artboard the objects were copied from (none when they lie on no artboard): Paste on All
    /// Artboards keeps their offset to it.
    pub source_artboard: Option<Rect>,
    /// Per object: the name of the layer it was copied from (Paste Remembers Layers).
    pub source_layers: Vec<Option<String>>,
    /// The open document the objects came from ([`DocState::uid`]): pasting back into it uses its
    /// own resources as they are now.
    pub source_doc: Option<u64>,
    /// An artboard copied with the Artboard tool (`artboard.copy`); `nodes` are then its art. Pasting
    /// adds a copy of it with the art.
    pub artboard: Option<vectorcraft_doc::Artboard>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.artboard.is_none()
    }

    /// Objects `roots` of `st`'s document with their resources, source artboard and layers.
    pub(crate) fn copy(st: &DocState, roots: &[NodeId]) -> Self {
        let d = &*st.doc;
        let mut nodes = vec![];
        let mut source_layers = vec![];
        for id in roots {
            let (Some(n), Some(anc)) = (d.node(*id), d.ancestry(*id)) else { continue };
            nodes.push(n.clone());
            // The nearest enclosing layer (or sublayer).
            source_layers.push(anc.iter().rev().skip(1).find_map(|a| d.node(*a).filter(|l| l.is_layer())).and_then(|l| l.name.clone()));
        }
        let bounds = nodes.iter().filter_map(Node::visual_bounds).reduce(|a, b| a.union(b));
        // Art on no artboard has no source artboard: in-place pastes put it back where it was (#1924).
        let board = bounds.and_then(|b| d.artboard_at(b.center()));
        Self {
            source_artboard: board.and_then(|i| d.artboards.get(i)).map(|a| a.rect),
            source_layers,
            source_doc: Some(st.uid),
            ..Self::gather(d, nodes)
        }
    }

    /// Every object of `src` (each top-level layer's contents) with its resources, for Place and
    /// pasting outside SVG. Their layers don't count for Paste Remembers Layers.
    pub fn from_document(src: &Document) -> Self {
        let nodes = src.layers.iter().flat_map(|l| l.children().into_iter().flatten()).map(|n| (**n).clone()).collect();
        Self::gather(src, nodes)
    }

    /// Union of the objects' visual bounds.
    pub fn bounds(&self) -> Option<Rect> {
        self.nodes.iter().filter_map(Node::visual_bounds).reduce(|a, b| a.union(b))
    }

    /// The swatch name conflicts pasting into `st` raises, as (pasted, document) swatches: none
    /// back in the document the objects came from.
    pub fn swatch_conflicts<'a>(&'a self, st: &'a DocState) -> Vec<(&'a Swatch, &'a Swatch)> {
        if self.source_doc == Some(st.uid) {
            return vec![];
        }
        self.conflicts(&st.doc, &self.used())
    }

    /// A standalone document holding the objects and their resources (SVG export).
    pub fn to_document(&self) -> Document {
        let mut d = Document::new(100.0, 100.0);
        d.images = self.images.clone();
        d.symbols = self.symbols.clone();
        d.patterns = self.patterns.clone();
        d.swatches.extend(self.swatches.iter().cloned());
        d.graphic_styles.extend(self.graphic_styles.iter().cloned());
        d.char_styles = self.char_styles.clone().into();
        d.para_styles = self.para_styles.clone().into();
        if !self.brushes.is_empty() {
            let mut lib = vectorcraft_brush::library(&d);
            for b in &self.brushes {
                match lib.iter_mut().find(|x| x.name == b.name) {
                    Some(x) => *x = b.clone(),
                    None => lib.push(b.clone()),
                }
            }
            vectorcraft_brush::store(&mut d, &lib);
        }
        // The layer's id is above every copied object's, so the document saves and reopens as
        // a native file.
        let mut top = 0;
        for n in &self.nodes {
            n.walk(&mut |c| top = top.max(c.id.0));
        }
        let mut layer = Node::layer(NodeId(top.saturating_add(1)), "Clipboard", vectorcraft_doc::LayerColor::Preset(0));
        if let Some(ch) = layer.children_mut() {
            *ch = self.nodes.iter().cloned().map(Arc::new).collect();
        }
        d.layers = vec![Arc::new(layer)];
        d.fix_next_id();
        d
    }

    /// The objects as standalone SVG (`None` when there are none).
    pub fn svg(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let d = self.to_document();
        // SVG has no filters for the Photoshop-style effects: their objects go in as images.
        let flat = super::rasterfx::flatten_pixel_effects(&d);
        Some(vectorcraft_svg::export(
            flat.as_ref().unwrap_or(&d),
            &vectorcraft_svg::ExportOptions { artboard: None, object_ids: vectorcraft_svg::ObjectIds::Minimal, ..Default::default() },
        ))
    }
}

impl Session {
    /// The internal clipboard as standalone SVG (`None` when it is empty).
    pub fn clipboard_svg(&self) -> Option<String> {
        self.clipboard.svg()
    }
}

fn export_svg(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "svg": s.clipboard_svg() }))
}

fn import_svg(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importSvg";
    let svg = match (str_param(p, "svg"), str_param(p, "dataBase64")) {
        (Some(svg), _) => std::borrow::Cow::Borrowed(svg),
        (None, Some(b64)) => {
            let bytes = vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad base64"))?;
            vectorcraft_svg::text_of(&bytes).map_err(|e| bad(C, e.to_string()))?.into_owned().into()
        }
        (None, None) => return Err(bad(C, "give svg, or dataBase64 of an SVG or SVGZ file")),
    };
    let (src, _) = super::fileio::import_svg(&svg, None).map_err(|e| bad(C, e.to_string()))?;
    let clip = Clipboard::from_document(&src);
    if clip.is_empty() {
        return Err(bad(C, "the SVG has no drawable objects"));
    }
    Ok(json!({ "count": s.load_clipboard(clip, p)? }))
}

fn conflicts(s: &mut Session, _: &Value) -> Result<Value> {
    let hex = |w: &Swatch| w.paint.color().map(|c| c.to_hex());
    let list: Vec<Value> = s
        .clipboard
        .swatch_conflicts(s.doc()?)
        .into_iter()
        .map(|(c, t)| json!({"name": c.name, "document": hex(t), "clipboard": hex(c), "spot": c.spot}))
        .collect();
    Ok(json!({ "swatches": list }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        s
    }

    /// The address a browser's Copy Image puts next to the picture (#597), and text that isn't one.
    #[test]
    fn an_address_alone_is_told_from_other_text() {
        for a in [
            "https://example.com/cat.png",
            " http://x.org/a?b=1
",
            "HTTPS://EXAMPLE.COM",
            "file:///C:/art/cat.png",
            "data:image/png;base64,iVBOR",
            "blob:https://x.org/1",
        ] {
            assert!(is_address(a), "{a}");
        }
        for t in ["Pasted words", "see https://example.com", "https://a.org https://b.org", "https://", "example.com/cat.png", ""] {
            assert!(!is_address(t), "{t}");
        }
    }

    /// The image markup some browsers put as text next to a copied picture names it too (#1089);
    /// markup with words in it is text.
    #[test]
    fn markup_of_only_images_names_a_picture() {
        for m in [
            r#"<img src="https://static.example.com/eagle.png" jsaction="" class="sFlh5c" style="max-width: 351px" alt="">"#,
            r#"<meta http-equiv="content-type" content="text/html; charset=utf-8"><img src="a.png">"#,
            "<html><body><!--StartFragment--><IMG SRC=\"a.png\"/><!--EndFragment--></body></html>",
            r#"<a href="https://x.org"><img src="a.png"></a>"#,
            "https://example.com/cat.png",
        ] {
            assert!(names_a_picture(m), "{m}");
        }
        for t in ["<p>Hello</p>", r#"<img src="a.png"> and a caption"#, "<b>bold</b>", "<imgx>", "<img", "plain words", "<br>", ""] {
            assert!(!names_a_picture(t), "{t}");
        }
    }

    #[test]
    fn copy_exports_svg_that_round_trips_through_paste() {
        let mut s = session();
        let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(s.execute("clipboard.exportSvg", &json!({})).unwrap()["svg"].is_null() || !s.clipboard.is_empty());
        s.execute("edit.copy", &json!({})).unwrap();
        let svg = s.execute("clipboard.exportSvg", &json!({})).unwrap()["svg"].as_str().unwrap().to_string();
        assert!(looks_like_svg(&svg) && svg.contains("#12ab34"), "{svg}");
        // As if pasted into another VectorCraft window.
        let mut t = session();
        assert_eq!(t.execute("clipboard.importSvg", &json!({"svg": svg, "center": [200, 150]})).unwrap()["count"], 1);
        t.execute("edit.pasteInPlace", &json!({})).unwrap();
        let st = t.doc().unwrap();
        let b = st.doc.bounds_of(&st.selection.in_paint_order(&st.doc), false).unwrap();
        assert!((b.center().x - 200.0).abs() < 1e-6 && (b.center().y - 150.0).abs() < 1e-6, "{b:?}");
        assert!((b.width() - 30.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6);
    }

    #[test]
    fn pasted_type_gets_its_layout_bounds_cache() {
        // Reported from Illustrator: pasted type kept no layout bounds, so its box (and alignment)
        // fell back to the rough estimate until the file was saved and reopened.
        let mut s = session();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="120"><text x="0" y="80" font-family="Source Sans 3" font-size="40">WWWWWWWW</text></svg>"#;
        s.execute("clipboard.importSvg", &json!({"svg": svg})).unwrap();
        s.execute("edit.paste", &json!({})).unwrap();
        let mut checked = 0;
        s.doc().unwrap().doc.walk(|n| {
            let vectorcraft_doc::NodeKind::Text(t) = &n.kind else { return };
            let cached = t.cached_bounds.expect("pasted type computed its bounds");
            let real = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).bounds;
            assert!((cached.width() - real.width()).abs() < 1e-6, "{cached:?} {real:?}");
            assert!(cached.width() > t.estimate_bounds().width() + 1.0, "{cached:?} is just the estimate");
            checked += 1;
        });
        assert_eq!(checked, 1, "one text object pasted");
    }

    #[test]
    fn paste_never_lands_in_an_object_that_reused_an_undone_layers_id() {
        // Found by the model-based test: undo restores the id counter, so a compound path made
        // after undoing New Layer gets the dead layer's id, which was still the active layer.
        let mut s = session();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(c, layer, "the scenario needs the id to be reused");
        s.execute("edit.paste", &json!({})).unwrap();
        let d = &s.doc().unwrap().doc;
        let compound = d.node(NodeId(c)).unwrap();
        assert!(compound.children().unwrap().iter().all(|ch| matches!(ch.kind, vectorcraft_doc::NodeKind::Path { .. })));
    }

    #[test]
    fn foreign_svg_and_non_svg_text() {
        let mut s = session();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle cx="5" cy="5" r="5" fill="red"/><rect width="2" height="2"/></svg>"##;
        assert!(looks_like_svg(svg) && looks_like_svg(&format!("<?xml version=\"1.0\"?>\n{svg}")));
        assert!(!looks_like_svg("hello <svg> world") && !looks_like_svg("plain text"));
        assert_eq!(s.execute("clipboard.importSvg", &json!({"svg": svg})).unwrap()["count"], 2);
        assert!(s.execute("clipboard.importSvg", &json!({"svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"})).is_err());
        assert!(s.execute("clipboard.importSvg", &json!({"svg": "not svg"})).is_err());
    }
}
