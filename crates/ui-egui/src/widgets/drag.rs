use super::{Paint, Rect, Response, Stroke, StrokeKind, Tokens, Ui, Vec2};

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

/// What a click on a row of a list whose rows highlight (the Layers and Appearance panels) means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowClick<K> {
    /// Highlight only this row (a plain click).
    Only(K),
    /// Highlight this row, or take its highlight off (Ctrl/Cmd).
    Toggle(K),
    /// Highlight these rows instead, listed top-down (Shift).
    Range(Vec<K>),
    /// Highlight these rows too, listed top-down (Shift+Ctrl/Cmd).
    AddRange(Vec<K>),
}

impl<K> RowClick<K> {
    /// Does the clicked row become the anchor Shift ranges start from? Every click but a Shift range.
    pub fn moves_anchor(&self) -> bool {
        matches!(self, Self::Only(_) | Self::Toggle(_))
    }
}

/// What a click on row `clicked` with modifiers `m` means, the rows listed top-down in `order`:
/// Shift ranges from `anchor` (else from the last of the `highlighted` rows) and Shift+Ctrl/Cmd adds
/// the range; Ctrl/Cmd toggles the row; a plain click, or Shift with no start in `order`, highlights
/// only it (Ctrl/Cmd: toggles it).
pub fn row_click<K: Copy + PartialEq>(order: &[K], highlighted: &[K], anchor: Option<K>, clicked: K, m: egui::Modifiers) -> RowClick<K> {
    let at = |k: K| order.iter().position(|r| *r == k);
    if m.shift
        && let Some(a) = anchor.or(highlighted.last().copied())
        && let (Some(i), Some(j)) = (at(a), at(clicked))
    {
        let range = order.get(i.min(j)..=i.max(j)).unwrap_or_default().to_vec();
        return if m.command { RowClick::AddRange(range) } else { RowClick::Range(range) };
    }
    if m.command { RowClick::Toggle(clicked) } else { RowClick::Only(clicked) }
}

/// Apply `click` to the `highlighted` rows, kept in highlight order as `layer.highlight` keeps the
/// Layers panel's: a toggled row is appended (or taken off, even the last one), a range replaces the
/// rows or adds those not there yet, top-down.
pub fn apply_row_click<K: Copy + PartialEq>(highlighted: &mut Vec<K>, click: &RowClick<K>) {
    let add = |rows: &mut Vec<K>, add: &[K]| {
        for k in add {
            if !rows.contains(k) {
                rows.push(*k);
            }
        }
    };
    match click {
        RowClick::Only(k) => *highlighted = vec![*k],
        RowClick::Toggle(k) if highlighted.contains(k) => highlighted.retain(|r| r != k),
        RowClick::Toggle(k) => highlighted.push(*k),
        RowClick::Range(rows) => {
            highlighted.clear();
            add(highlighted, rows);
        }
        RowClick::AddRange(rows) => add(highlighted, rows),
    }
}

/// Dimmed copies of dragged rows ([`drag_ghost`]) are drawn at this opacity.
pub const DRAG_GHOST_OPACITY: f32 = 0.5;

/// Which of the dragged rows the dimmed copy shows: the first of them in highlight order (`dragged`),
/// as the Layers panel has always done, else the grabbed one. The one place to change it (to show the
/// grabbed row, say) for both panels.
pub fn ghost_row<K: Copy>(dragged: &[K], grabbed: K) -> K {
    dragged.first().copied().unwrap_or(grabbed)
}

/// A dimmed copy of a dragged row at `rect`, above everything else, with the grabbing cursor (the
/// Layers and Appearance panels): the highlighted row's fill and an accent edge; `contents` draws
/// the row's own contents on it.
pub fn drag_ghost(ctx: &egui::Context, id: egui::Id, rect: Rect, contents: impl FnOnce(&Ui, Rect)) {
    let t = Tokens::get(ctx);
    // A plain Ui on the tooltip layer, not an Area: a new Area is invisible for a frame and fades in.
    let mut ui = Ui::new(ctx.clone(), id, egui::UiBuilder::new().layer_id(egui::LayerId::new(egui::Order::Tooltip, id)).max_rect(rect));
    ui.set_opacity(DRAG_GHOST_OPACITY);
    ui.painter().rect_filled(rect, 0.0, t.row_selected);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    contents(&ui, rect);
}

/// The outline a panel button shows while dragged rows hang over it and it would take them (the
/// Layers panel's trash and New Layer, the Appearance panel's Duplicate and Delete).
pub fn drop_target_outline(ui: &Ui, rect: Rect) {
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
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

#[cfg(test)]
mod row_click_tests {
    use egui::Modifiers;

    use super::{RowClick, apply_row_click, row_click};

    const ROWS: [u32; 5] = [10, 20, 30, 40, 50];
    const SHIFT_CMD: Modifiers = Modifiers { shift: true, command: true, ..Modifiers::NONE };

    /// A plain click highlights only the clicked row and Ctrl/Cmd toggles it; both move the anchor.
    #[test]
    fn a_plain_click_highlights_only_the_row_and_ctrl_toggles_it() {
        assert_eq!(row_click(&ROWS, &[20, 40], Some(20), 30, Modifiers::NONE), RowClick::Only(30));
        assert_eq!(row_click(&ROWS, &[20, 40], Some(20), 30, Modifiers::COMMAND), RowClick::Toggle(30));
        // Neither moves past the anchor rules: both make the clicked row the new anchor.
        assert!(RowClick::Only(30).moves_anchor() && RowClick::Toggle(30).moves_anchor());
    }

    /// Shift highlights the range from the anchor and Shift+Ctrl/Cmd adds it; neither moves the anchor.
    #[test]
    fn shift_ranges_from_the_anchor_and_shift_ctrl_adds_the_range() {
        // Top-down order whichever way the range runs.
        assert_eq!(row_click(&ROWS, &[40], Some(40), 20, Modifiers::SHIFT), RowClick::Range(vec![20, 30, 40]));
        assert_eq!(row_click(&ROWS, &[20], Some(20), 40, SHIFT_CMD), RowClick::AddRange(vec![20, 30, 40]));
        assert!(!RowClick::Range(vec![20]).moves_anchor() && !RowClick::AddRange(vec![20]).moves_anchor());
    }

    /// Without an anchor, Shift starts at the last highlighted row, or acts as a plain click when none is.
    #[test]
    fn without_an_anchor_shift_starts_at_the_last_highlighted_row_else_acts_as_a_plain_click() {
        assert_eq!(row_click(&ROWS, &[10, 50], None, 30, Modifiers::SHIFT), RowClick::Range(vec![30, 40, 50]));
        assert_eq!(row_click(&ROWS, &[], None, 30, Modifiers::SHIFT), RowClick::Only(30));
        assert_eq!(row_click(&ROWS, &[], None, 30, SHIFT_CMD), RowClick::Toggle(30));
        // An anchor (or last row) that isn't in the list: as without one.
        assert_eq!(row_click(&ROWS, &[99], Some(99), 30, Modifiers::SHIFT), RowClick::Only(30));
    }

    /// Applied clicks keep the rows in highlight order, as `layer.highlight` does.
    #[test]
    fn applying_a_click_keeps_highlight_order_as_layer_highlight_does() {
        let applied = |mut rows: Vec<u32>, c: RowClick<u32>| {
            apply_row_click(&mut rows, &c);
            rows
        };
        assert_eq!(applied(vec![40, 20], RowClick::Only(30)), [30]);
        // Toggling appends in click order; a highlighted row (even the last) goes off.
        assert_eq!(applied(vec![40, 20], RowClick::Toggle(10)), [40, 20, 10]);
        assert_eq!(applied(vec![40, 20], RowClick::Toggle(40)), [20]);
        assert_eq!(applied(vec![40], RowClick::Toggle(40)), Vec::<u32>::new());
        // A range replaces the rows, or adds the ones not there yet, top-down.
        assert_eq!(applied(vec![50], RowClick::Range(vec![20, 30])), [20, 30]);
        assert_eq!(applied(vec![30, 50], RowClick::AddRange(vec![20, 30, 40])), [30, 50, 20, 40]);
    }
}
