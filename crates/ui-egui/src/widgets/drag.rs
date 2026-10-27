use super::*;

/// What panels drag onto art and onto each other. One payload type, so the canvas has one drop
/// handler for all of them (`canvas::panel_drop`); a chip of a dragged paint follows the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelDrag {
    /// A paint: Swatches panel tiles, a Fill/Stroke proxy or the Gradient panel's thumbnail. Art it
    /// is dropped on takes `params` (`{swatch}`, `{color}`, `{gradient}`…; null for a colour group,
    /// which paints nothing) through the active proxy's `paint.setFill`/`paint.setStroke`; the
    /// Gradient panel's ramp takes its colour as a stop; the Swatches panel moves `rows`, or makes
    /// a swatch of a paint dragged from elsewhere. `library`: the Libraries panel colour it is
    /// (library id, colour name), which that panel moves to where it is dropped.
    Paint { paint: Paint, params: serde_json::Value, rows: Option<SwatchRows>, library: Option<(String, String)> },
    /// The Appearance panel's thumbnail: art it is dropped on takes object `0`'s appearance
    /// (`appearance.copyFrom`); the Graphic Styles panel makes a style of it.
    Appearance(vectorcraft_doc::NodeId),
    /// A Graphic Styles panel style: art it is dropped on takes it (`graphicStyle.apply` with its
    /// `ids`); the panel moves it to where it is dropped.
    GraphicStyle(String),
    /// The selected art, dragged off the canvas with the Selection tool: the Graphic Styles panel
    /// makes a style of the first object (`graphicStyle.new`), the Symbols panel a symbol of it all
    /// (`symbol.new`), the Swatches panel a pattern swatch of a copy (`object.pattern.make`) and the
    /// Brushes panel an Art brush (`brush.new`).
    Art(Vec<vectorcraft_doc::NodeId>),
    /// A Symbols panel symbol: the canvas places an instance of it centred where it is dropped
    /// (`symbol.place`).
    Symbol(String),
    /// A symbol of a symbol library panel (`name`: its name in `library`): the canvas adds it to the
    /// document's Symbols and places an instance where it is dropped (`symbol.addFromLibrary` with
    /// `place`).
    LibrarySymbol { library: String, name: String },
    /// A Brushes panel brush (`def`: its definition, for the chip at the pointer): the path it is
    /// dropped on takes it (`brush.apply`).
    Brush { name: String, def: serde_json::Value },
    /// A brush of a brush library panel (`name`: its name in `library`): the path it is dropped on
    /// takes it (`brush.addFromLibrary` with `apply`), and the Brushes panel adds it.
    LibraryBrush { library: String, name: String },
    /// A Libraries panel graphic (`item`: its id in `library`): the canvas places a copy centred
    /// where it is dropped (`library.use`); the panel moves it to where it is dropped.
    LibraryGraphic { library: String, item: String },
    /// A Libraries panel text style (`item`: its name in `library`), which the panel moves to where
    /// it is dropped.
    LibraryItem { library: String, kind: vectorcraft_engine::cmd::library::ItemKind, item: String },
    /// A Libraries panel group, by its header, which the panel moves to where it is dropped.
    LibraryGroup { library: String, group: String },
}

/// A panel list `zone` that takes art dragged off the canvas: outlined while art is held over it;
/// → the dragged ids when it is released there.
pub fn art_drop(ui: &Ui, zone: &Response) -> Option<Vec<u64>> {
    let drag = zone.dnd_hover_payload::<PanelDrag>()?;
    let PanelDrag::Art(ids) = &*drag else { return None };
    ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    zone.dnd_release_payload::<PanelDrag>().map(|_| ids.iter().map(|id| id.0).collect())
}

/// The Swatches panel rows a drag from that panel moves: the swatch, None, Registration or colour
/// group under the pointer when the drag started (`grabbed`) and `names`, the panel selection when
/// the grabbed one is part of it (only colour groups, or only swatches, like the grabbed one), in
/// panel order.
#[derive(Clone, Debug, PartialEq)]
pub struct SwatchRows {
    pub grabbed: String,
    pub names: Vec<String>,
    /// The names are colour groups.
    pub groups: bool,
}

impl PanelDrag {
    /// `paint` dragged from a Fill/Stroke proxy or the Gradient panel's thumbnail. A gradient
    /// carries no placement: it fits the art it lands on (freeform points are placed afresh there,
    /// coloured like the dragged ones).
    pub fn paint(mut paint: Paint) -> Self {
        if let Paint::Gradient(g) = &mut paint {
            g.geom = None;
            g.freeform = None;
        }
        Self::Paint { params: crate::panels::paint_params(&paint), paint, rows: None, library: None }
    }
    /// The dragged colour (a solid paint's), which the Gradient panel's ramp takes.
    pub fn color(&self) -> Option<vectorcraft_color::Color> {
        match self {
            Self::Paint { paint, .. } => paint.color(),
            _ => None,
        }
    }
}

/// Make `resp` (which senses drags) a source of the [`PanelDrag`] `drag` builds when a drag starts
/// on it.
pub fn drag_source(ui: &Ui, resp: &Response, drag: impl FnOnce() -> PanelDrag) {
    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), drag());
    }
}

/// A small document rendered to a texture at the screen's pixel density (stroke previews: brushes,
/// arrowheads), transparent where nothing paints. `build` makes the `w`×`h` pt document only when
/// the texture for `key` at this size isn't cached yet.
pub fn doc_preview(ui: &Ui, key: &str, size: Vec2, build: impl FnOnce(f64, f64) -> Option<vectorcraft_doc::Document>) -> Option<egui::TextureHandle> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<String, egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let ppp = ui.ctx().pixels_per_point() as f64;
    let (w, h) = (size.x as f64, size.y as f64);
    let key = format!("{w}x{h}@{ppp}:{key}");
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let doc = build(w, h)?;
    let img = RENDERER.with(|r| r.borrow_mut().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, w, h), ppp, false));
    let color = crate::canvas::color_image(&img);
    let tex = ui.ctx().load_texture(format!("preview-{key}"), color, egui::TextureOptions::LINEAR);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    Some(tex)
}
