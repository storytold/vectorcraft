//! Navigator panel: the framework's [`NavigatorPanel`] (preview box with the view box you drag to
//! pan, zoom out/in buttons, a zoom slider and a zoom field) over a thumbnail of the document
//! rendered with `Renderer::render_region_with` and kept by the framework's [`Thumbnails`]: made on
//! a lane thread, held while an interaction is open. Panel Options… sets the view box's color and
//! whether the thumbnail draws dashed lines as solid lines, and the panel menu's View Artboard
//! Contents Only (on by default) limits the thumbnail to the art inside the artboards
//! ([`PanelOptions`]).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::ui::canvas::{AppPaint, NavigatorPreview, ProxyColor};
use craft::ui::contract::Bind;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::canvas::NavigatorPanel;
use craft::ui::primitives::{LoadedPicture, Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::Env;
use craft::ui::style::StyleCx;
use egui::{Color32, Rect, Shape, Ui, pos2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;
use vectorcraft_geom::Rect as DRect;

use crate::{CanvasCache, VectorcraftApp};

/// The view box's color by default (Light Red, the first of [`view_box_colors`]).
pub const VIEW_BOX_RED: [u8; 3] = [0xff, 0x30, 0x30];

/// The view box color presets of Panel Options: Light Red, the default, then the layer colors.
pub fn view_box_colors() -> impl Iterator<Item = (&'static str, [u8; 3])> {
    std::iter::once(("Light Red", VIEW_BOX_RED)).chain(LAYER_COLORS.iter().copied())
}

/// `color` (a preset's name, in any case, or `#rrggbb`) as RGB; `None` for anything else,
/// including a color with an alpha or a short `#rgb`.
pub fn resolve(color: &str) -> Option<[u8; 3]> {
    let color = color.trim();
    if let Some((_, rgb)) = view_box_colors().find(|(name, _)| name.eq_ignore_ascii_case(color)) {
        return Some(rgb);
    }
    let hex = color.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Navigator panel options (≡ › Panel Options… and ≡ › View Artboard Contents Only), kept with the
/// UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelOptions {
    /// The view box's color, `#rrggbb`.
    pub color: String,
    /// Draw Dashed Lines as Solid Lines (off by default): the thumbnail draws dashed strokes
    /// without their dashes. The canvas, printing and exports keep the dashes.
    pub solid_dashes: bool,
    /// View Artboard Contents Only (the panel menu; on by default, and on for UI state saved
    /// without it): the thumbnail fits the artboards and draws only the art inside them, cut at
    /// each artboard's edges. Off, it also fits and draws the art on the pasteboard.
    pub view_artboard_contents_only: bool,
}

impl Default for PanelOptions {
    fn default() -> Self {
        let [r, g, b] = VIEW_BOX_RED;
        Self { color: Color::rgb8(r, g, b).to_hex(), solid_dashes: false, view_artboard_contents_only: true }
    }
}

/// `ui.navigatorPanelOptions {color?, solidDashes?, viewArtboardContentsOnly?}`: with none, open
/// Panel Options on the panel's options; otherwise set the ones given and answer the options.
pub fn set_options(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (color, solid, contents_only) = (p.get("color"), p.get("solidDashes"), p.get("viewArtboardContentsOnly"));
    if color.is_none() && solid.is_none() && contents_only.is_none() {
        return crate::dialogs::navigator_panel_options::open(app);
    }
    let mut o = app.ui.navigator_panel.clone();
    if let Some(c) = color {
        let [r, g, b] = c.as_str().and_then(resolve).ok_or_else(|| {
            let color = c.as_str().map_or_else(|| c.to_string(), str::to_string);
            format!("`{color}` is not a colour (#rrggbb or a preset name)")
        })?;
        o.color = Color::rgb8(r, g, b).to_hex();
    }
    if let Some(s) = solid {
        let key = "solidDashes";
        o.solid_dashes = s.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?;
    }
    if let Some(v) = contents_only {
        let key = "viewArtboardContentsOnly";
        o.view_artboard_contents_only = v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))?;
    }
    app.ui.navigator_panel = o;
    Ok(serde_json::to_value(&app.ui.navigator_panel).unwrap_or_default())
}

/// The id of the command both panel-menu rows run.
const OPTIONS_COMMAND: &str = "ui.navigatorPanelOptions";

/// The panel menu's View Artboard Contents Only row while the option is `on`: it sets the opposite.
fn contents_only_row(on: bool) -> CommandRef {
    CommandRef::with_params(OPTIONS_COMMAND, json!({"viewArtboardContentsOnly": !on}))
}

/// The panel menu's Panel Options… row.
fn options_row() -> CommandRef {
    CommandRef::with_params(OPTIONS_COMMAND, json!({}))
}

/// The Navigator panel menu's rows.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let on = app.ui.navigator_panel.view_artboard_contents_only;
    vec![PanelRow::Command(contents_only_row(on)), PanelRow::Command(options_row())]
}

/// Titles, checks and enabled state of the panel menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let on = app.ui.navigator_panel.view_artboard_contents_only;
    states.set(contents_only_row(on), CommandState::new().title(tl!("View Artboard Contents Only")).checked(on).enabled(true));
    states.set(options_row(), CommandState::new().title(tl!("Panel Options…")).enabled(true));
}

/// What a navigator picture slot shows: document `uid`, the artboards only, dashed lines drawn as
/// solid lines. The document's revision and the region are the picture's version
/// ([`NavThumbs::revision`]), so a new revision replaces the picture of the same slot and the old
/// one stays as its stand-in until the new one lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NavKey {
    uid: u64,
    artboard_only: bool,
    solid_dashes: bool,
}

/// The slot of document `uid` under `options`.
fn nav_key(uid: u64, options: &PanelOptions) -> NavKey {
    NavKey { uid, artboard_only: options.view_artboard_contents_only, solid_dashes: options.solid_dashes }
}

/// The color the view box is drawn in: [`PanelOptions::color`], or Light Red when it is not a color.
fn proxy_color(options: &PanelOptions) -> ProxyColor {
    ProxyColor::Custom(resolve(&options.color).unwrap_or(VIEW_BOX_RED))
}

/// The document's navigator picture at one revision over one region.
struct NavThumbs {
    doc: Arc<vectorcraft_doc::Document>,
    revision: u64,
    region: DRect,
}

impl NavThumbs {
    /// The source of `doc` at `revision`, over `region` (document units).
    fn new(doc: &Arc<vectorcraft_doc::Document>, revision: u64, region: DRect) -> Self {
        Self { doc: doc.clone(), revision, region }
    }
}

impl ThumbSource<NavKey> for NavThumbs {
    fn revision(&self, _key: &NavKey, _request: &ThumbRequest) -> Revision {
        let mut hasher = DefaultHasher::new();
        self.revision.hash(&mut hasher);
        [self.region.x0, self.region.y0, self.region.x1, self.region.y1].map(f64::to_bits).hash(&mut hasher);
        Revision::new(hasher.finish())
    }

    fn render(&self, key: &NavKey, request: &ThumbRequest) -> ThumbRender {
        let (doc, region, key) = (self.doc.clone(), self.region, *key);
        // The longest side of the region gets `side` pixels.
        let scale = f64::from(request.side) / region.width().max(region.height()).max(f64::EPSILON);
        ThumbRender::Later(Box::new(move |_cancel| {
            let options = image_options(key);
            let renderer = if key.solid_dashes { &SOLID_RENDERER } else { &NAV_RENDERER };
            let image = renderer.with(|r| r.borrow_mut().render_region_with(&doc, region, scale, &options));
            Ok(LoadedPicture::Image(crate::canvas::color_image(&image)))
        }))
    }
}

/// The region (artboards, and the art unless `artboard_only`, with a margin) to show `doc` (`uid`
/// at `revision`) in; the art bounds walk the whole document, so the last one is kept in `cache`
/// per revision. While `busy` (a drag) the last one of the document stays: the picture held for
/// the drag was made for it.
fn region(doc: &vectorcraft_doc::Document, uid: u64, revision: u64, artboard_only: bool, busy: bool, cache: &mut CanvasCache) -> DRect {
    let key = (uid, revision, artboard_only);
    if let Some((k, r)) = cache.nav_region
        && (k == key || (busy && k.0 == uid && k.2 == artboard_only))
    {
        return r;
    }
    let mut region = doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap_or(DRect::new(0.0, 0.0, 612.0, 792.0));
    if !artboard_only && let Some(b) = doc.art_bounds() {
        region = region.union(b);
    }
    let region = region.inflate(region.width() * 0.04 + 4.0, region.height() * 0.04 + 4.0);
    cache.nav_region = Some((key, region));
    region
}

/// How the image `key` names is drawn: as exported (template layers and guides left out),
/// transparent, with images far smaller than they are sampling their mip levels, with Draw
/// Dashed Lines as Solid Lines, dashed strokes without their dashes, and with View Artboard
/// Contents Only, the art cut to the artboards (none in the gaps between them).
fn image_options(key: NavKey) -> vectorcraft_render::RenderOptions {
    vectorcraft_render::RenderOptions {
        skip_templates: true,
        image_mips: true,
        solid_dashes: key.solid_dashes,
        trim: key.artboard_only,
        ..Default::default()
    }
}

thread_local! {
    /// Renders the pictures on the thread of the lane that makes them.
    static NAV_RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
    /// Renders the images with Draw Dashed Lines as Solid Lines on, on the thread that renders
    /// them: each renderer draws every image it makes with one setting, so the art it keeps from
    /// one image is drawn the same way in the next.
    static SOLID_RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
}

/// `view` with its centre moved into the region's frame: the framework's document is
/// `(0, 0)..size`, the app's starts at the region's corner.
fn shifted(mut view: CanvasView, region: DRect) -> CanvasView {
    view.center.x -= region.x0;
    view.center.y -= region.y0;
    view
}

/// The `view.setZoom` params for `edited`, a view of the region's frame: the zoom as the status bar
/// shows it (in print size with `print`), and the centre back in document units.
fn view_params(edited: &CanvasView, region: DRect, print: bool) -> Value {
    json!({"zoom": crate::state::zoom_display_percent(edited.zoom, print), "center": [edited.center.x + region.x0, edited.center.y + region.y0]})
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "map", tl!("No document"), tl!("Open a document to navigate it."));
        return;
    };
    let (doc, uid, revision) = (st.doc.clone(), st.uid, st.revision);
    // A drag in progress: every pointer move makes a new revision. The picture waits for its end.
    let busy = st.interaction.is_some() || app.session.tool_busy();
    let Some(view) = app.canvas_view() else { return };
    let options = app.ui.navigator_panel.clone();
    let key = nav_key(uid, &options);
    let region = region(&doc, uid, revision, key.artboard_only, busy, &mut app.canvas);
    let given = shifted(view, region);
    let env = Env::of(ui);
    let pasteboard = StyleCx::of_env(ui.ctx(), &env).color(env.tokens.color.surface.canvas).egui();
    let paint = AppPaint::new(|cx, fitted: Rect| {
        let ctx = cx.painter.ctx();
        // Artboards (white) over the pasteboard, under the art, like the canvas.
        let scale = fitted.width() / region.width() as f32;
        let at = |x: f64, y: f64| pos2(fitted.min.x + (x - region.x0) as f32 * scale, fitted.min.y + (y - region.y0) as f32 * scale);
        let mut shapes = vec![Shape::rect_filled(fitted, 0.0, pasteboard)];
        shapes.extend(
            doc.artboards
                .iter()
                .map(|a| Shape::rect_filled(Rect::from_two_pos(at(a.rect.x0, a.rect.y0), at(a.rect.x1, a.rect.y1)), 0.0, Color32::WHITE)),
        );
        cx.data_shapes(shapes);
        let thumbs = Thumbnails::<NavKey>::in_context(ctx, "navigator", ThumbnailOptions::new());
        thumbs.hold(busy);
        let source = NavThumbs::new(&doc, revision, region);
        let request = ThumbRequest::for_cell(fitted.width().max(fitted.height()), ctx.pixels_per_point());
        let state = thumbs.state(ctx, &source, &key, &request, Priority::VISIBLE);
        thumbs.paint(cx, fitted, &state, &ThumbPaint::default().checker(false));
    });
    // PLAN: assumed `DRect::size()` is the `kurbo::Size` the panel takes
    let out = NavigatorPanel::new(tl!("Navigator"), Bind::given(given), region.size())
        .proxy(proxy_color(&options))
        .preview(NavigatorPreview::Paint(paint))
        .show(ui);
    if let Some(edited) = out.view.value()
        && *edited != given
    {
        let print = app.session.prefs.display_print_size;
        app.run("view.setZoom", view_params(edited, region, print)).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A centre edit of the shifted view reaches `view.setZoom` as the document point it shows, and
    /// an unedited view's centre is shifted by the region's corner.
    #[test]
    fn a_navigator_edit_moves_the_view_to_the_document_point_it_shows() {
        let region = DRect::new(-10.0, -20.0, 190.0, 180.0);
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(600.0, 400.0));
        let view = CanvasView::new(canvas, (200.0, 150.0).into(), 1.0);
        let given = shifted(view, region);
        assert_eq!((given.center.x, given.center.y), (210.0, 170.0), "the centre minus the region's corner");
        let mut edited = given;
        edited.center = (100.0, 100.0).into();
        assert_eq!(view_params(&edited, region, false)["center"], json!([90.0, 80.0]), "the edit plus the region's corner");
    }

    /// The panel menu's rows are titled, checked and enabled as the option says, and each row's
    /// command flips the option or opens the options dialog.
    #[test]
    fn the_panel_menu_rows_check_and_open_what_they_name() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        let states = |app: &VectorcraftApp| {
            let mut states = CommandStates::new();
            menu_states(app, &mut states);
            states
        };
        let rows = menu_rows(&app);
        assert_eq!(rows.len(), 2);
        let on = states(&app);
        let toggle = contents_only_row(true);
        assert_eq!(
            on.get(&toggle),
            Some(&CommandState::new().title(tl!("View Artboard Contents Only")).checked(true).enabled(true)),
            "checked while the option is on"
        );
        assert_eq!(on.get(&options_row()), Some(&CommandState::new().title(tl!("Panel Options…")).enabled(true)));
        app.run(&toggle.id, toggle.params.clone()).unwrap();
        assert!(!app.ui.navigator_panel.view_artboard_contents_only, "the row turns the option off");
        let off = states(&app);
        assert_eq!(
            off.get(&contents_only_row(false))
                .map(|s| s == &CommandState::new().title(tl!("View Artboard Contents Only")).checked(false).enabled(true)),
            Some(true)
        );
        let again = contents_only_row(false);
        app.run(&again.id, again.params.clone()).unwrap();
        assert!(app.ui.navigator_panel.view_artboard_contents_only, "the row turns it on again");
        let options = options_row();
        app.run(&options.id, options.params.clone()).unwrap();
        let open = app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<crate::dialogs::navigator_panel_options::NavigatorOptionsDialog>()));
        assert!(open, "Panel Options… opens the dialog");
    }

    /// `ui.navigatorPanelOptions` sets the view box color and Draw Dashed Lines as Solid Lines: the
    /// panel is given that color (Light Red by default) and the picture slot names the dashes. A
    /// value that is not a color, or not true or false, is refused and changes nothing.
    #[test]
    fn panel_options_set_the_view_box_color_and_solid_dashes() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(proxy_color(&app.ui.navigator_panel), ProxyColor::Custom(VIEW_BOX_RED), "Light Red by default");
        assert!(!nav_key(1, &app.ui.navigator_panel).solid_dashes);
        let set = app.run("ui.navigatorPanelOptions", json!({"color": "light blue", "solidDashes": true})).unwrap();
        assert_eq!(set, json!({"color": "#4f80ff", "solidDashes": true, "viewArtboardContentsOnly": true}));
        assert_eq!(proxy_color(&app.ui.navigator_panel), ProxyColor::Custom([0x4f, 0x80, 0xff]));
        assert!(nav_key(1, &app.ui.navigator_panel).solid_dashes, "the picture is made again with solid dashes");
        let bad = [
            json!({"color": 12}),
            json!({"color": "#zz0000"}),
            json!({"color": "#ff000080"}),
            json!({"color": "#f00"}),
            json!({"color": null}),
            json!({"color": "Red", "solidDashes": "yes"}),
            json!({"color": "Red", "viewArtboardContentsOnly": "no"}),
            json!({"solidDashes": false, "viewArtboardContentsOnly": 0}),
            json!({"viewArtboardContentsOnly": null}),
        ];
        for bad in bad {
            assert!(app.run("ui.navigatorPanelOptions", bad.clone()).is_err(), "{bad}");
        }
        let want = json!({"color": "#4f80ff", "solidDashes": true, "viewArtboardContentsOnly": true});
        assert_eq!(serde_json::to_value(&app.ui.navigator_panel).unwrap(), want, "nothing changed");
        app.run("ui.navigatorPanelOptions", json!({"color": "#123456"})).unwrap();
        assert!(app.ui.navigator_panel.solid_dashes, "an option left out keeps its value");
    }

    /// The thumbnail's render options drop a dashed stroke's dashes with Draw Dashed Lines as Solid
    /// Lines, so it draws as the same stroke without dashes; without the option the dashes stay.
    #[test]
    fn the_thumbnail_draws_dashed_lines_as_solid_lines_with_the_option() {
        use vectorcraft_color::Paint;
        use vectorcraft_doc::{Appearance, Document, Node};
        let line = |dash: bool| {
            let mut d = Document::new(100.0, 100.0);
            let mut bp = vectorcraft_geom::BezPath::new();
            bp.move_to((10.0, 50.0));
            bp.line_to((90.0, 50.0));
            let id = d.alloc_id();
            let mut n =
                Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 6.0));
            n.appearance.stroke_mut().unwrap().dash = dash.then(|| vectorcraft_doc::Dash { pattern: vec![8.0, 8.0], ..Default::default() });
            let l = d.layers[0].id;
            d.insert(Some(l), 0, n).unwrap();
            d
        };
        let key = NavKey { uid: 1, artboard_only: false, solid_dashes: false };
        let draw = |d: &Document, key: NavKey| {
            vectorcraft_render::Renderer::new().render_region_with(d, DRect::new(0.0, 0.0, 100.0, 100.0), 1.0, &image_options(key)).pixels
        };
        let solid = draw(&line(false), key);
        assert_ne!(draw(&line(true), key), solid, "the dashes show without the option");
        let on = NavKey { solid_dashes: true, ..key };
        assert!(image_options(on).solid_dashes && !image_options(key).solid_dashes);
        assert_eq!(draw(&line(true), on), solid, "the dashes go with the option");
        assert_eq!(draw(&line(false), on), solid, "a solid stroke draws as it did");
    }

    /// View Artboard Contents Only is on by default, and on for UI state saved without it, whether
    /// the panel's options are missing or saved before the option existed; a saved off stays off.
    #[test]
    fn view_artboard_contents_only_is_on_by_default_and_a_saved_off_stays_off() {
        assert!(PanelOptions::default().view_artboard_contents_only);
        let none: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert!(none.navigator_panel.view_artboard_contents_only, "UI state saved without the panel's options");
        let older: crate::UiState = serde_json::from_value(json!({"navigator_panel": {"color": "#123456", "solidDashes": true}})).unwrap();
        let o = &older.navigator_panel;
        assert!(o.view_artboard_contents_only && o.solid_dashes && o.color == "#123456", "options saved before the option existed: {o:?}");
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert!(app.ui.navigator_panel.view_artboard_contents_only);
        let set = app.run("ui.navigatorPanelOptions", json!({"viewArtboardContentsOnly": false})).unwrap();
        assert_eq!(set, json!({"color": "#ff3030", "solidDashes": false, "viewArtboardContentsOnly": false}));
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(!back.navigator_panel.view_artboard_contents_only, "a saved off stays off");
    }

    /// With View Artboard Contents Only the thumbnail draws only the art inside the artboards: a
    /// shape in the gap between two artboards is left out and one that crosses an artboard's edge
    /// is cut there. Without it both draw.
    #[test]
    fn the_thumbnail_draws_only_the_art_inside_the_artboards_with_the_option() {
        use vectorcraft_color::Paint;
        use vectorcraft_doc::{Appearance, Document, Node};
        let mut d = Document::new(100.0, 100.0);
        let mut second = d.artboards[0].clone();
        second.id = d.artboards.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        second.rect = DRect::new(150.0, 0.0, 250.0, 100.0);
        d.artboards.push(second);
        let l = d.layers[0].id;
        for (x0, y0, x1, y1) in [(110.0, 20.0, 140.0, 80.0), (20.0, 80.0, 60.0, 130.0)] {
            let mut bp = vectorcraft_geom::BezPath::new();
            bp.move_to((x0, y0));
            bp.line_to((x1, y0));
            bp.line_to((x1, y1));
            bp.line_to((x0, y1));
            bp.close_path();
            let id = d.alloc_id();
            let n = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
            d.insert(Some(l), 0, n).unwrap();
        }
        let alpha = |artboard_only: bool, x: u32, y: u32| {
            let key = NavKey { uid: 1, artboard_only, solid_dashes: false };
            vectorcraft_render::Renderer::new().render_region_with(&d, DRect::new(0.0, 0.0, 260.0, 140.0), 1.0, &image_options(key)).pixel(x, y)[3]
        };
        assert_eq!([alpha(true, 125, 50), alpha(true, 40, 90), alpha(true, 40, 120)], [0, 255, 0], "the gap and below the artboard are empty");
        assert_eq!([alpha(false, 125, 50), alpha(false, 40, 90), alpha(false, 40, 120)], [255, 255, 255], "all the art draws");
    }

    /// Turning View Artboard Contents Only off makes the picture again, fitted to the art on the
    /// pasteboard as well as the artboards; turning it on fits it to the artboards again.
    #[test]
    fn the_thumbnail_is_made_again_when_view_artboard_contents_only_changes() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 400, "y": 50, "width": 50, "height": 50})).unwrap();
        let (doc, uid, revision) = {
            let st = app.session.active().unwrap();
            (st.doc.clone(), st.uid, st.revision)
        };
        let request = ThumbRequest::for_cell(100.0, 1.0);
        let at = |app: &mut VectorcraftApp, only: bool| {
            let region = region(&doc, uid, revision, only, false, &mut app.canvas);
            let key = NavKey { uid, artboard_only: only, solid_dashes: false };
            (NavThumbs::new(&doc, revision, region).revision(&key, &request), region.x1)
        };
        let (on, right_on) = at(&mut app, true);
        assert!(right_on < 300.0, "fitted to the artboard: {right_on}");
        let (off, right_off) = at(&mut app, false);
        assert!(right_off > 450.0, "fitted to the art on the pasteboard too: {right_off}");
        assert_ne!(on, off, "the picture is a new one");
        assert_eq!(at(&mut app, true), (on, right_on), "turning it on restores both");
    }

    /// No picture is made while an interaction is open; one is made after it, of the document's new
    /// revision.
    #[test]
    fn no_image_is_rendered_during_a_drag_and_one_is_after_it() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        let ctx = crate::testing::ui_context();
        let request = ThumbRequest::for_cell(100.0, 1.0);
        // One pass of the panel's picture: the images the store uploaded in it, and the version asked for.
        let frame = |app: &mut VectorcraftApp, held: bool| {
            let (doc, uid, revision) = {
                let st = app.session.active().unwrap();
                (st.doc.clone(), st.uid, st.revision)
            };
            let region = region(&doc, uid, revision, true, held, &mut app.canvas);
            let source = NavThumbs::new(&doc, revision, region);
            let key = NavKey { uid, artboard_only: true, solid_dashes: false };
            let mut uploads = 0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let thumbs = Thumbnails::<NavKey>::in_context(ui.ctx(), "navigator", ThumbnailOptions::new().lanes(0));
                thumbs.hold(held);
                thumbs.state(ui.ctx(), &source, &key, &request, Priority::VISIBLE);
                uploads = thumbs.cache().statistics(ui.ctx()).uploads;
            });
            out.textures_delta.clear();
            (uploads, source.revision(&key, &request))
        };
        let first = (0..12).map(|_| frame(&mut app, false)).find(|(uploads, _)| *uploads > 0).expect("the first picture is made");
        // A move drag: every preview is a new revision.
        app.session.begin_interaction("Move").unwrap();
        for i in 1..=20 {
            app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i, 0]})).unwrap();
            assert_eq!(frame(&mut app, true).0, 0, "nothing is made while the drag goes on");
        }
        app.session.commit_interaction().unwrap();
        let after = (0..12).map(|_| frame(&mut app, false)).find(|(uploads, _)| *uploads > 0).expect("a picture after the drag");
        assert_ne!(after.1, first.1, "the picture is of the document's new revision");
    }
}
