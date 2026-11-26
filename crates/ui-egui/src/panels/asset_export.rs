//! Asset Export panel (Window → Asset Export): the document's assets as thumbnails with their
//! names. Art dragged in off the canvas becomes assets, one per object (with Alt held, one asset
//! of them all), and so does the selection with the + button. Clicking selects assets (Shift or
//! Cmd adds), double-clicking one renames it and the bin removes the selected ones. Below are the
//! export settings: the format rows shared with Export for Screens (one set per document), and
//! Export, which writes the selected assets (else all of them) into a picked folder (on the
//! desktop with Background Export on, on the background thread); the web downloads them. The
//! screen button opens Export for Screens on its Assets tab.
//!
//! Export 3D Object (the panel menu, and the 3D and Materials panel's button) collects the selected objects with a 3D effect as 3D assets,
//! marked 3D on their tiles. While the panel's assets include 3D assets, a Format row picks their
//! mesh format (USDA, USDZ, glTF or OBJ), and Export writes them as mesh files beside the pictures.
//!
//! The panel menu also holds Format Settings… (Export for Screens showing the first row's format
//! settings), and Create Sub-folders and Open Location after Export, the dialog's two checkboxes,
//! which Export follows. Create Sub-folders sorts the pictures into a folder per format row, or per
//! format or item when Export for Screens remembers that choice; 3D assets' mesh files go in the
//! folder itself.
//!
//! A tile's context menu exports or removes the selected assets, renames the tile, selects its
//! artwork on the canvas, or adds the selected artwork as assets.
//!
//! Everything runs through the `assets.*` commands (the canvas selection through `select.set`);
//! the menu rows are the `ui.assetExport.*` commands ([`menu_rows`], [`menu_states`], [`run`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, GesturePhase, IconId, ItemSource, Mixed, Selection};
use craft::host::{PickKind, PickRequest};
use craft::keymap::CommandRef;
use craft::theme::Variant;
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, RowAlign, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, TooltipSpec, clicked,
    request_rename,
};
use craft::ui::dialog::StackDialog as _;
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::{DropAction, FooterAction, FooterBar};
use craft::ui::tile_grid::{BadgeKind, BadgeSlot, Tile, TileBadge, TileGrid};
use egui::{Color32, Rect, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, ExportAsset, Node, NodeId};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::fileio::{FORMATS_3D, SCREEN_SETTINGS_KEYS, format_3d_label, screen_export_preset_list};

use crate::dialogs::screen_formats::{self, PresetPick};
use crate::dialogs::{ExportForScreensDialog, drop_unread_screen_settings, open_export_for_screens_assets, screen_saved_options, screen_saved_rows};
use crate::file_asks::{self, Owner};
use crate::panel_drag::PanelDrag;
use crate::{VectorcraftApp, io};

pub const ID: &str = "assetExport";
/// The id prefix of the panel menu's commands.
const PREFIX: &str = "ui.assetExport.";

/// The Asset Export panel's own state, kept while the app runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct AssetExportPanel {
    /// The state of each document, by its `uid`: two documents can hold the same asset ids.
    pub(crate) docs: HashMap<u64, AssetExportDoc>,
}

/// What the panel remembers about one document.
#[derive(Clone, Debug, Default)]
pub(crate) struct AssetExportDoc {
    /// The assets selected in the panel.
    pub(crate) selection: Selection<u64>,
    /// The export preset applied last (none, or a name).
    pub(crate) applied: Option<String>,
    /// The name being typed to save a preset (none when not saving).
    pub(crate) saving: Option<String>,
    /// The tile whose context menu the grid last asked for.
    pub(crate) menu: Option<u64>,
}

/// The active document's panel state, made on first use.
pub(crate) fn doc_panel(app: &mut VectorcraftApp) -> Option<&mut AssetExportDoc> {
    let uid = app.session.active()?.uid;
    Some(app.ui.asset_export_panel.docs.entry(uid).or_default())
}

/// The assets selected in the panel that are still in the document.
pub(crate) fn selected(app: &VectorcraftApp) -> Vec<u64> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let held = app.ui.asset_export_panel.docs.get(&st.uid).map(|d| d.selection.items.as_slice()).unwrap_or_default();
    held.iter().copied().filter(|id| st.doc.asset(*id).is_some()).collect()
}

/// A selection of exactly `ids`.
fn pick(ids: Vec<u64>) -> Selection<u64> {
    Selection::new(ids.clone(), ids.last().copied(), ids.first().copied())
}

/// Selects exactly `ids` in the active document's panel.
fn select(app: &mut VectorcraftApp, ids: Vec<u64>) {
    if let Some(d) = doc_panel(app) {
        d.selection = pick(ids);
    }
}

/// The addresses of an asset's objects, back to front: the identity of its look.
type Look = Vec<usize>;
/// A document (uid) at one revision.
type Revision = (u64, u64);

/// Asset `a` of `doc` (document uid and revision `revision`) fitted into `r` on white. Rendered once
/// per look of its objects (an edited object is a new allocation, so its address is a free change
/// detector, as in the Layers panel).
pub(crate) fn paint_thumb(painter: &egui::Painter, revision: Revision, doc: &Document, a: &ExportAsset, r: Rect) {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        /// The addresses of each asset's objects in one document revision (no tree walks per frame).
        static KEYS: RefCell<(Revision, HashMap<u64, Look>)> = RefCell::new(Default::default());
        static CACHE: crate::graphics::TexCache<HashMap<(Look, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    painter.rect_filled(r, 0.0, Color32::WHITE);
    if !painter.clip_rect().intersects(r) {
        return;
    }
    let ids = || doc.paint_order(a.nodes.iter().copied());
    let nodes = KEYS.with(|k| {
        let mut k = k.borrow_mut();
        if k.0 != revision {
            *k = (revision, HashMap::new());
        }
        k.1.entry(a.id).or_insert_with(|| ids().iter().filter_map(|id| doc.node(*id)).map(|n| std::ptr::from_ref(n) as usize).collect()).clone()
    });
    let px = (r.width() * painter.ctx().pixels_per_point()).round().max(8.0) as u32;
    let key = (nodes, px);
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).or_else(|| {
        let art: Vec<&Node> = ids().iter().filter_map(|id| doc.node(*id)).collect();
        // Same Art Optimized supersample as assets.export (#983): abutting fills leave no seam in
        // the tile (Layers row thumbs stay cheap without it).
        let opts = vectorcraft_render::RenderOptions { image_mips: true, precise: true, supersample: true, ..Default::default() };
        let img = RENDERER.with(|rr| match art.as_slice() {
            [one] => rr.borrow_mut().render_node_thumbnail_with(doc, one, px, None, &opts),
            many => {
                let group = Node::group(NodeId(0), many.iter().map(|n| Arc::new((*n).clone())).collect());
                rr.borrow_mut().render_node_thumbnail_with(doc, &group, px, None, &opts)
            }
        })?;
        let color = crate::canvas::color_image(&img);
        let tex = painter.ctx().load_texture(format!("asset-thumb-{}", a.id), color, egui::TextureOptions::LINEAR);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 512 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        Some(tex)
    });
    if let Some(t) = tex {
        painter.image(t.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The document's assets as the tile grid's items.
struct Assets {
    ids: Vec<u64>,
    names: Vec<String>,
}

impl Assets {
    fn of(st: &DocState) -> Self {
        Assets { ids: st.doc.assets.iter().map(|a| a.id).collect(), names: st.doc.assets.iter().map(|a| a.name.clone()).collect() }
    }
}

impl DisplayOrder<u64> for Assets {
    fn len(&self) -> usize {
        self.ids.len()
    }

    fn key(&self, at: usize) -> u64 {
        self.ids.get(at).copied().unwrap_or_default()
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.names.get(at).map(|name| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<u64> for Assets {}

/// The tile of asset `id`: its art, name, and a 3D badge for a 3D asset.
fn tile(st: &DocState, id: u64) -> Tile<'_> {
    let Some(a) = st.doc.asset(id) else { return Tile::new("") };
    let mut tile = Tile::new(a.name.clone())
        .tooltip(TooltipSpec::new(format!("{}\n{}", a.name, tl!("Double-click to rename"))))
        .thumb(ThumbAspect::SQUARE, move |cx, r| paint_thumb(&cx.painter, (st.uid, st.revision), &st.doc, a, r));
    if a.format_3d.is_some() {
        tile = tile.badge(TileBadge::new(BadgeSlot::StartTop, BadgeKind::Text("3D".into()), Tone::Neutral, None));
    }
    tile
}

/// Run `cmd`, reporting a failure in the status bar.
fn run_command(app: &mut VectorcraftApp, cmd: &str, params: Value) -> Option<Value> {
    app.run(cmd, params).map_err(|e| app.status(e)).ok()
}

/// The asset ids an `assets.add` or `assets.add3D` answer names.
fn assets_of(answer: &Value) -> Vec<u64> {
    answer["assets"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()
}

/// Export 3D Object: collects objects `ids` (else the selection) as 3D assets (`assets.add3D`), and
/// selects their assets in the panel, including those that were assets already.
pub(crate) fn add_3d(app: &mut VectorcraftApp, ids: Option<&[u64]>) -> Result<(), String> {
    let p = ids.map_or_else(|| json!({}), |ids| json!({ "ids": ids }));
    let r = app.run("assets.add3D", p)?;
    select(app, assets_of(&r));
    Ok(())
}

/// Collect objects `ids` (else the selection) as assets (one of them all with `single`), and select
/// them in the panel.
pub(crate) fn collect(app: &mut VectorcraftApp, ids: Option<&[u64]>, single: bool) {
    let mut p = json!({ "multiple": !single });
    if let Some(ids) = ids {
        p["ids"] = json!(ids);
    }
    if let Some(r) = run_command(app, "assets.add", p) {
        select(app, assets_of(&r));
    }
}

/// Does the export of 3D asset `asset` draw a texture for the graphics mapped onto its faces
/// ([`vectorcraft_engine::cmd::fileio::draws_textures`])? Only objects whose 3D effects map graphics
/// are built to find out.
fn writes_textures(doc: &vectorcraft_doc::Document, asset: &ExportAsset) -> bool {
    let maps = |e: &vectorcraft_doc::Effect| e.visible && !vectorcraft_render::effects::mapped_graphics(&e.params).is_empty();
    let nodes: Vec<&Node> = asset.nodes.iter().filter_map(|id| doc.node(*id)).collect();
    nodes.iter().any(|n| n.appearance.effects.iter().chain(n.appearance.items.iter().flat_map(|i| i.effects())).any(maps))
        && vectorcraft_render::effects::three_d_meshes_of_in(Some(doc), nodes.iter().copied())
            .is_ok_and(|parts| vectorcraft_engine::cmd::fileio::draws_textures(doc, &parts))
}

/// Export assets `ids` (`files_each`: the files one asset makes) into a picked folder (without a
/// folder picker: the last folder Export for Screens used, else the Desktop); the web downloads
/// them, several as one zip.
pub(crate) fn export(app: &mut VectorcraftApp, ids: Vec<u64>, files_each: usize) {
    if io::is_web(app) || !file_asks::can(app, PickKind::Folder) {
        return export_now(app, ids, files_each, None);
    }
    // The folder is picked off the UI thread; a cancelled pick drops the export.
    file_asks::ask(app, PickRequest::folder("export"), Owner::Document, move |app, picked| {
        if let Some(path) = picked.first_path() {
            export_now(app, ids, files_each, Some(path.to_string_lossy().into_owned()));
        }
        Ok(Value::Null)
    });
}

/// Writes assets `ids` into `folder`, else the remembered folder or the Desktop (the web needs none).
fn export_now(app: &mut VectorcraftApp, ids: Vec<u64>, files_each: usize, folder: Option<String>) {
    // 3D assets are written as mesh files (`assets.export3D`), the others as pictures.
    let web = io::is_web(app);
    let Some(st) = app.session.active() else { return };
    let format = |id: &u64| st.doc.asset(*id).and_then(|a| a.format_3d.clone());
    let (three_d, ids): (Vec<u64>, Vec<u64>) = ids.into_iter().partition(|id| format(id).is_some());
    // An OBJ is two files, and an OBJ or a USDA whose graphics draw a texture writes it beside
    // them. Only the web, which zips several files, needs the count.
    let mesh_files: usize = three_d
        .iter()
        .map(|id| {
            let format = format(id);
            let textures = web && matches!(format.as_deref(), Some("obj" | "usda")) && st.doc.asset(*id).is_some_and(|a| writes_textures(&st.doc, a));
            usize::from(format.as_deref() == Some("obj")) + 1 + usize::from(textures)
        })
        .sum();
    // Create Sub-folders as the panel menu checks it: on by default, and remembered folders (per
    // row, format or item) are used as Export for Screens uses them (a remembered `subfolders`
    // false beside them would alone put every file in the folder itself).
    let saved = &st.doc.export_settings;
    let mut p = keep_subfolders(saved, json!({ "assets": ids }));
    if let Some(f) = saved.get("folders").and_then(Value::as_str).filter(|f| !matches!(*f, "none" | "")) {
        p["folders"] = json!(f);
    }
    // Match the shared dialog's TIFF defaults, retaining saved choices and other formats.
    if let Some(st) = app.session.active()
        && screen_saved_rows(&st.doc.export_settings)
            .0
            .iter()
            .any(|row| row["format"].as_str().and_then(vectorcraft_engine::cmd::fileio::format).is_some_and(|f| f.id == "tiff"))
    {
        let saved = &st.doc.export_settings;
        let mut settings = saved.get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
        settings.insert("tiff".into(), Value::Object(screen_saved_options(app, saved, "tiff")));
        // Without the keys the export no longer reads, which it would refuse.
        let mut given = Map::from_iter([("settings".to_string(), Value::Object(settings))]);
        drop_unread_screen_settings(&mut given);
        p["settings"] = given.remove("settings").unwrap_or_default();
    }
    if web {
        p["zip"] = json!(ids.len() * files_each > 1);
    } else {
        let saved = app.session.active().map(|st| st.doc.export_settings.clone()).unwrap_or_default();
        let remembered = || saved.get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()).map(str::to_string);
        let Some(folder) = folder.or_else(remembered).or_else(vectorcraft_engine::cmd::fileio::export_folder) else {
            return app.status("choose a folder to export to".to_string());
        };
        p["folder"] = json!(folder);
        p["openLocation"] = json!(open_location(&saved));
    }
    let mut meshes = json!({ "assets": three_d });
    if web {
        meshes["zip"] = json!(mesh_files > 1);
    } else {
        meshes["folder"] = p["folder"].clone();
        meshes["openLocation"] = p["openLocation"].clone();
    }
    // The meshes are written at once, before the pictures start on the background thread. When
    // they fail, the pictures are not exported and the status bar shows the error.
    if !three_d.is_empty()
        && let Err(e) = io::export_files(app, "assets.export3D", meshes)
    {
        return app.status(e);
    }
    if !ids.is_empty()
        && let Err(e) = io::export_files(app, "assets.export", p)
    {
        app.status(e);
    }
}

/// Create Sub-folders for the remembered settings `saved`, as Export for Screens opens on it: off
/// for remembered folders `none`, on for any other remembered folders (a folder per row, format or
/// item); without them the remembered `subfolders`, else on for a document without settings or
/// with a preset's rows.
fn subfolders_on(saved: &Map<String, Value>) -> bool {
    match saved.get("folders").and_then(Value::as_str) {
        // An export reads empty folders as none.
        Some("none" | "") => false,
        Some(_) => true,
        None => saved.get("subfolders").and_then(Value::as_bool).unwrap_or_else(|| saved.is_empty() || screen_saved_rows(saved).1.is_some()),
    }
}

/// `p` (params of `assets.settings.set` or `assets.export`) with `subfolders: true` when `saved`
/// makes no Create Sub-folders choice and it is on by default: the export would otherwise write
/// into the folder itself, and settings `p` adds would turn the default off.
fn keep_subfolders(saved: &Map<String, Value>, mut p: Value) -> Value {
    if !saved.contains_key("folders") && !saved.contains_key("subfolders") && subfolders_on(saved) {
        p["subfolders"] = json!(true);
    }
    p
}

/// Open Location after Export for the remembered settings `saved` (on until it is turned off).
fn open_location(saved: &Map<String, Value>) -> bool {
    saved.get("openLocation").and_then(Value::as_bool).unwrap_or(true)
}

/// Open Export for Screens on the Assets tab with assets `ids` checked, showing the Format Settings
/// of format `format`.
fn format_settings(app: &mut VectorcraftApp, ids: &[u64], format: &str) {
    open_export_for_screens_assets(app, Some(ids));
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(Err(e)) = dialogs.with(|h| h.get_mut::<ExportForScreensDialog>().map(|d| d.set_json_field(&ctx, "__settings", json!(format)))) {
        app.status(e);
    }
}

/// What the export settings rows asked for.
enum Settings {
    /// Change the shared settings (`assets.settings.set`).
    Set(Value),
    /// Show a format's Format Settings (in Export for Screens).
    Format(String),
}

/// The Format of the 3D assets among `ids`: a row shown only when there are some, set to their
/// format (blank when they differ). Returns those assets and the format picked for them.
fn format_3d(ui: &mut Ui, st: &DocState, ids: &[u64]) -> Option<(Vec<u64>, &'static str)> {
    let three_d: Vec<(u64, &str)> = ids.iter().filter_map(|id| Some((*id, st.doc.asset(*id)?.format_3d.as_deref()?))).collect();
    let first = three_d.first()?.1;
    let current = if three_d.iter().all(|(_, f)| *f == first) { FORMATS_3D.iter().position(|f| *f == first) } else { None };
    let entries: Vec<Entry<u8>> = FORMATS_3D.iter().enumerate().map(|(i, f)| Entry::new(i as u8, format_3d_label(f))).collect();
    let label = format!("3D {}", tl!("Format:"));
    let mut picked = None;
    Field::new(label.clone()).show(ui, |ui, _| {
        let value = current.map_or(Mixed::Mixed, |i| Mixed::Same(i as u8));
        let out = Select::new(label.clone(), Bind::given(value), &entries).show(ui);
        picked = out.events.committed().copied().and_then(|i| FORMATS_3D.get(usize::from(i)).copied());
    });
    picked.map(|f| (three_d.into_iter().map(|(id, _)| id).collect(), f))
}

/// The text of field `key` in `fields`.
fn text_of(fields: &Map<String, Value>, key: &str) -> String {
    fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The document's shared export format rows. Returns the change asked for and how many files one asset makes.
fn settings(ui: &mut Ui, st: &DocState) -> (Option<Settings>, usize) {
    let (rows, preset) = screen_saved_rows(&st.doc.export_settings);
    let preset = preset.unwrap_or_default().to_string();
    let n = rows.len();
    let mut fields =
        Map::from_iter([("formats".to_string(), json!(&rows)), ("preset".to_string(), json!(preset)), ("__settings".to_string(), json!(""))]);
    screen_formats::rows(ui, &mut fields, None);
    let (shown, picked) = (text_of(&fields, "__settings"), text_of(&fields, "preset"));
    let formats = fields.remove("formats").unwrap_or(Value::Null);
    let change = if !shown.is_empty() {
        Some(Settings::Format(shown))
    } else if picked != preset && !picked.is_empty() {
        Some(Settings::Set(json!({ "preset": picked, "subfolders": true })))
    } else if picked != preset || formats.as_array() != Some(&rows) {
        // Rows edited, or Custom picked: the rows as they are, without the preset.
        Some(Settings::Set(keep_subfolders(&st.doc.export_settings, json!({ "formats": formats, "preset": "" }))))
    } else {
        None
    };
    (change, n)
}

/// The Save As Preset row while a name is being typed (`saving`): the pick it made, if any. The row
/// keeps the typed name in `__savePreset` and removes it when saved or cancelled.
fn save_row(ui: &mut Ui, saving: &mut Option<String>) -> Option<PresetPick> {
    let name = saving.as_ref()?;
    let mut fields = Map::from_iter([("__savePreset".to_string(), json!(name))]);
    let picked = screen_formats::save_preset_row(ui, &mut fields);
    *saving = fields.get("__savePreset").and_then(Value::as_str).map(str::to_string);
    picked
}

/// What the panel saw this frame, acted on once the document is no longer borrowed.
#[derive(Default)]
struct Seen {
    renames: Vec<(u64, String)>,
    dropped: Option<(Vec<u64>, bool)>,
    change: Option<Settings>,
    files_each: usize,
    format_3d: Option<(Vec<u64>, &'static str)>,
    save: Option<PresetPick>,
    tile: Option<TileAct>,
}

/// What a tile's context menu runs.
enum TileAct {
    Export(Vec<u64>),
    Remove(Vec<u64>),
    Rename(u64),
    SelectArtwork(Vec<NodeId>),
    /// Add the selected artwork as assets: one of it all when `true`.
    Collect(bool),
}

/// The context menu of tile `a`, hanging from the grid, which picked the tile unless it was picked already: Export and
/// Remove for the picked assets (`sel`, else `a` alone), Rename, Select Artwork on Canvas, and the selected artwork
/// (`has_art`) added as one asset or as one per object.
fn tile_menu(ui: &mut Ui, opener: &egui::Response, a: &ExportAsset, sel: &[u64], has_art: bool) -> Option<TileAct> {
    let targets = if sel.contains(&a.id) { sel.to_vec() } else { vec![a.id] };
    let count = targets.len().to_string();
    let (export_label, remove_label) = if targets.len() > 1 {
        (crate::i18n::fmt(tl!("Export {count} Assets…"), &[("count", &count)]), crate::i18n::fmt(tl!("Remove {count} Assets"), &[("count", &count)]))
    } else {
        (tl!("Export Asset…").to_string(), tl!("Remove Asset").to_string())
    };
    let shown = Menu::context(a.name.clone(), opener).show(ui, |ui, _| {
        let mut act = None;
        if MenuItem::new(export_label.clone()).show(ui).ran {
            act = Some(TileAct::Export(targets.clone()));
        }
        if MenuItem::new(remove_label.clone()).show(ui).ran {
            act = Some(TileAct::Remove(targets.clone()));
        }
        if MenuItem::new(tl!("Rename")).show(ui).ran {
            act = Some(TileAct::Rename(a.id));
        }
        Divider::horizontal().show(ui);
        if MenuItem::new(tl!("Select Artwork on Canvas")).show(ui).ran {
            act = Some(TileAct::SelectArtwork(a.nodes.clone()));
        }
        Divider::horizontal().show(ui);
        if MenuItem::new(tl!("Add Selected Artwork as One Asset")).enabled(has_art).show(ui).ran {
            act = Some(TileAct::Collect(true));
        }
        if MenuItem::new(tl!("Add Selected Artwork as Multiple Assets")).enabled(has_art).show(ui).ran {
            act = Some(TileAct::Collect(false));
        }
        act
    });
    shown.inner.flatten()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Closed documents' entries go.
    let session = &app.session;
    app.ui.asset_export_panel.docs.retain(|uid, _| session.documents().iter().any(|d| d.uid == *uid));
    let Some(st) = app.session.active() else {
        Text::new(tl!("No document")).shade(Shade::Muted).show(ui);
        return;
    };
    let uid = st.uid;
    let model = app.ui.asset_export_panel.docs.entry(uid).or_default();
    model.selection.items.retain(|id| st.doc.asset(*id).is_some());
    let grid_id = ui.scope_id().with(("asset-export-grid", uid));
    let mut seen = Seen::default();
    let mut events = Vec::new();
    let mut opener = None;
    let alt = craft::ui::runtime::pointer::frame(&ctx).mods.alt;
    // The footer is measured before the grid: it reads the selection the grid starts the pass with.
    let shown_sel = model.selection.items.clone();
    let all: Vec<u64> = st.doc.assets.iter().map(|a| a.id).collect();
    let has_art = !st.selection.is_empty();
    let (selection, saving) = (&mut model.selection, &mut model.saving);
    let (mut export_clicked, mut footer, mut screens) = (false, None, false);
    // The grid takes the room the settings, Export and the footer bar leave.
    let footed = Stack::new().fill().footer(|ui, _| {
        crate::panels::section(tl!("Export Settings")).show(ui, |ui, _| {
            let (change, files_each) = settings(ui, st);
            (seen.change, seen.files_each) = (change, files_each);
            seen.format_3d = format_3d(ui, st, &sel_or_all(&shown_sel, &all));
            seen.save = save_row(ui, saving);
        });
        Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
            let label = if shown_sel.is_empty() || shown_sel.len() == all.len() {
                tl!("Export").to_string()
            } else {
                crate::i18n::fmt(tl!("Export {count} Selected"), &[("count", &shown_sel.len().to_string())])
            };
            let button = Button::new(label)
                .variant(Variant::PRIMARY)
                .enabled(!all.is_empty())
                .tooltip(TooltipSpec::new(tl!("Export the selected assets (else all of them)")))
                .show(ui);
            export_clicked = clicked(&button);
        });
        // The first action sits at the end edge, so Remove comes first and Add stands before it.
        let remove = FooterAction::new(IconSpec::svg(IconId::new("trash-2")), tl!("Remove Selected Assets")).enabled(!shown_sel.is_empty());
        let add = FooterAction::new(IconSpec::svg(IconId::new("plus")), tl!("Add Selected Artwork (Alt: as one asset)")).enabled(has_art);
        let (bar, leading) = FooterBar::actions([remove, add]).show_footer_leading(ui, |ui, _| {
            IconButton::new(IconSpec::svg(IconId::new("monitor"))).label(tl!("Export for Screens…")).show(ui).response
        });
        screens = leading.is_some_and(|r| clicked(&r));
        footer = Some(bar);
    });
    let zone = footed.show(ui, |ui, _| {
        let drop_art = DropAction::<PanelDrag>::new(move |p| matches!(p, PanelDrag::Art { document, .. } if *document == uid));
        drop_art.sizing(Sizing::Fill).show(ui, |ui, _| {
            if st.doc.assets.is_empty() {
                Notice::new(Tone::Neutral, tl!("Drag art here, or select it and click +. Hold Alt for one asset of several objects."))
                    .title(tl!("No assets"))
                    .icon(NoticeIcon::Custom(IconSpec::svg(IconId::new("share-2"))))
                    .layout(NoticeLayout::Centred)
                    .show(ui);
            } else {
                let items = Assets::of(st);
                let grid =
                    TileGrid::<u64>::new(tl!("Assets")).id(grid_id).select(SelectMode::Multi).selection(Bind::bound(&mut *selection)).rename(true);
                let out = grid.show_items(ui, &items, &mut |id, _| tile(st, *id));
                opener = Some(out.response.response.clone());
                events = out.response.events;
            }
        })
    });
    if let Some(PanelDrag::Art { ids, .. }) = zone.inner.and_then(|zone| zone.output.dropped) {
        seen.dropped = Some((ids.into_iter().map(|id| id.0).collect(), alt));
    }
    for event in events {
        match event {
            CollectionEvent::Activate(id) => request_rename(&ctx, grid_id, &id),
            CollectionEvent::ContextMenu(request) => model.menu = Some(request.key),
            CollectionEvent::Rename(edit) => {
                let ended = matches!(edit.phase(), Some(GesturePhase::Commit) | None);
                if let Some(rename) = edit.value().filter(|_| ended) {
                    let text = rename.name.trim();
                    if !text.is_empty() && st.doc.asset(rename.key).is_some_and(|a| a.name != text) {
                        seen.renames.push((rename.key, text.to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    let sel = model.selection.items.clone();
    if let (Some(id), Some(opener)) = (model.menu, &opener)
        && let Some(a) = st.doc.asset(id)
    {
        seen.tile = tile_menu(ui, opener, a, &sel, has_art);
    }
    let files_each = seen.files_each;
    // Act on what the panel saw (the document is no longer borrowed).
    for (id, name) in seen.renames {
        run_command(app, "assets.rename", json!({ "asset": id, "name": name }));
    }
    if let Some((ids, single)) = seen.dropped {
        collect(app, Some(&ids), single);
    }
    match seen.change {
        Some(Settings::Set(p)) => {
            run_command(app, "assets.settings.set", p);
        }
        Some(Settings::Format(f)) => format_settings(app, &sel_or_all(&sel, &all), &f),
        None => {}
    }
    if let Some(pick) = seen.save {
        preset(app, uid, pick);
    }
    if let Some((assets, format)) = seen.format_3d {
        run_command(app, "assets.set3DFormat", json!({ "assets": assets, "format": format }));
    }
    match footer.as_ref().and_then(|f| f.picked.map(|i| (i, f.mods.alt))) {
        Some((0, _)) => remove_selected(app, &sel),
        Some((_, alt)) => collect(app, None, alt),
        None => {}
    }
    if screens {
        open_export_for_screens_assets(app, Some(&sel_or_all(&sel, &all)));
    }
    if export_clicked {
        export(app, sel_or_all(&sel, &all), files_each);
    }
    match seen.tile {
        Some(TileAct::Export(targets)) => export(app, targets, files_each),
        Some(TileAct::Remove(targets)) => remove_selected(app, &targets),
        Some(TileAct::Rename(id)) => request_rename(&ctx, grid_id, &id),
        Some(TileAct::SelectArtwork(ids)) => {
            run_command(app, "select.set", json!({ "ids": ids }));
        }
        Some(TileAct::Collect(single)) => collect(app, None, single),
        None => {}
    }
}

/// Removes assets `sel` and clears the panel's selection.
fn remove_selected(app: &mut VectorcraftApp, sel: &[u64]) {
    run_command(app, "assets.remove", json!({ "assets": sel }));
    select(app, Vec::new());
}

/// Sets which preset document `doc` (uid) has applied.
fn set_applied(app: &mut VectorcraftApp, doc: u64, name: Option<String>) {
    app.ui.asset_export_panel.docs.entry(doc).or_default().applied = name;
}

/// Act on a preset pick for document `doc` (uid): a preset applied to the shared settings, the
/// shared settings saved as one, a saved one deleted, presets imported from a file or the current
/// one written to a file.
pub(crate) fn preset(app: &mut VectorcraftApp, doc: u64, pick: PresetPick) {
    let done = match pick {
        PresetPick::Apply(p) => {
            let name = p["name"].as_str().unwrap_or_default().to_string();
            app.run("assets.settings.set", json!({ "exportPreset": name })).map(|_| set_applied(app, doc, Some(name)))
        }
        PresetPick::Save(name) => {
            let settings = preset_settings(app, doc);
            // The name as saved (trimmed and capped).
            app.run("document.exportPresets.save", json!({ "name": name, "settings": settings })).map(|r| {
                let d = app.ui.asset_export_panel.docs.entry(doc).or_default();
                d.applied = Some(r["name"].as_str().unwrap_or_default().to_string());
                d.saving = None;
            })
        }
        PresetPick::Delete(name) => app.run("document.exportPresets.delete", json!({ "name": name })).map(|_| set_applied(app, doc, None)),
        // The file asks report when they are answered.
        PresetPick::Import => return screen_formats::import_presets(app, report::<Vec<String>>),
        PresetPick::Export(name) => return screen_formats::export_preset(app, &name, report::<String>),
    };
    report(app, done);
}

/// Reports a failed preset action in the status bar; a cancelled file ask says nothing.
fn report<T>(app: &mut VectorcraftApp, done: Result<T, String>) {
    if let Err(e) = done
        && e != "cancelled"
    {
        app.status(e);
    }
}

/// What Save Preset saves for document `doc` (uid): the preset the panel applied to this document,
/// with the document's current shared settings over it, replacing its format choice; else the
/// shared settings alone.
fn preset_settings(app: &VectorcraftApp, doc: u64) -> Value {
    let shared = shared_settings(app).as_object().cloned().unwrap_or_default();
    let current = app.ui.asset_export_panel.docs.get(&doc).and_then(|d| d.applied.clone()).filter(|name| !name.is_empty());
    let Some(current) = current else { return Value::Object(shared) };
    let list = screen_export_preset_list(&app.session.prefs.screen_export_presets);
    let Some(applied) = list.as_array().and_then(|rows| rows.iter().find(|p| p["name"] == current.as_str())).and_then(|p| p["settings"].as_object())
    else {
        return Value::Object(shared);
    };
    let Some(st) = app.session.active() else { return Value::Object(shared) };
    if st.uid != doc {
        return Value::Object(shared);
    }
    let mut settings = applied.clone();
    if shared.contains_key("preset") {
        settings.remove("formats");
    }
    if shared.contains_key("formats") {
        settings.remove("preset");
    }
    settings.extend(shared);
    drop_unread_screen_settings(&mut settings);
    Value::Object(settings)
}

/// The settings the panel shows, as a preset keeps them: the shared settings and effective folder
/// choice, with the row shown when the document has no rows or mobile/density preset of its own.
fn shared_settings(app: &VectorcraftApp) -> Value {
    let Some(st) = app.session.active() else { return json!({}) };
    let saved = &st.doc.export_settings;
    let mut settings: Map<String, Value> =
        SCREEN_SETTINGS_KEYS.iter().filter_map(|k| Some((k.to_string(), saved.get(*k).filter(|v| !v.is_null())?.clone()))).collect();
    let (rows, preset) = screen_saved_rows(saved);
    if let Some(folders) = saved.get("folders").filter(|v| !v.is_null()) {
        settings.insert("folders".into(), folders.clone());
    } else if preset.is_some() && settings.get("subfolders").and_then(Value::as_bool) == Some(false) {
        settings.insert("folders".into(), json!("none"));
    }
    if preset.is_none() {
        settings.remove("preset");
        if !settings.get("formats").and_then(Value::as_array).is_some_and(|a| a.iter().any(Value::is_object)) {
            settings.insert("formats".into(), json!(rows));
        }
    }
    drop_unread_screen_settings(&mut settings);
    Value::Object(settings)
}

/// The selected assets, else all of them.
fn sel_or_all(sel: &[u64], all: &[u64]) -> Vec<u64> {
    if sel.is_empty() { all.to_vec() } else { sel.to_vec() }
}

/// What the panel menu's rows and commands read from the active document.
struct Facts {
    uid: u64,
    has_art: bool,
    all: Vec<u64>,
    sel: Vec<u64>,
    subfolders: bool,
    open_location: bool,
    /// The format Format Settings opens on.
    format: &'static str,
    /// The preset applied to this document ("" for none).
    applied: String,
    /// The applied preset is a saved one, not a built-in.
    saved: bool,
    /// The applied preset is in the list.
    known: bool,
}

impl Facts {
    fn of(app: &VectorcraftApp) -> Option<Facts> {
        let st = app.session.active()?;
        let saved = &st.doc.export_settings;
        let applied = app.ui.asset_export_panel.docs.get(&st.uid).and_then(|d| d.applied.clone()).unwrap_or_default();
        let list = screen_export_preset_list(&app.session.prefs.screen_export_presets);
        let rows = list.as_array().map(Vec::as_slice).unwrap_or_default();
        let known = !applied.is_empty() && rows.iter().any(|p| p["name"] == applied.as_str());
        Some(Facts {
            uid: st.uid,
            has_art: !st.selection.is_empty(),
            all: st.doc.assets.iter().map(|a| a.id).collect(),
            sel: selected(app),
            subfolders: subfolders_on(saved),
            open_location: open_location(saved),
            format: screen_formats::settings_format(&screen_saved_rows(saved).0),
            saved: known && rows.iter().any(|p| p["name"] == applied.as_str() && p["builtIn"] == false),
            known,
            applied,
        })
    }
}

/// The command of panel row `name` (`ui.assetExport.<name>`) with `params`.
fn row(name: &str, params: Value) -> PanelRow {
    PanelRow::Command(CommandRef::with_params(format!("{PREFIX}{name}"), params))
}

/// The panel (≡) menu, in production order.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![
        row("add", json!({ "multiple": false })),
        row("add", json!({ "multiple": true })),
        row("add3D", Value::Null),
        PanelRow::Separator,
        row("selectAll", Value::Null),
        row("remove", Value::Null),
        PanelRow::Separator,
        row("exportForScreens", Value::Null),
        row("formatSettings", Value::Null),
        PanelRow::Separator,
        row("subfolders", Value::Null),
    ];
    // The web downloads the files: there is no location to open.
    if !io::is_web(app) {
        rows.push(row("openLocation", Value::Null));
    }
    rows.push(PanelRow::Separator);
    let list = screen_export_preset_list(&app.session.prefs.screen_export_presets);
    let names = list.as_array().into_iter().flatten().filter_map(|p| p["name"].as_str());
    rows.extend(names.map(|name| row("applyPreset", json!({ "name": name }))));
    rows.push(PanelRow::Separator);
    rows.extend(["savePresetAs", "saveChanges", "deletePreset", "importPreset", "exportPreset"].map(|name| row(name, Value::Null)));
    rows
}

/// The states of the panel menu's rows for the active document: all off without one.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let facts = Facts::of(app);
    for panel_row in menu_rows(app) {
        let PanelRow::Command(command) = panel_row else { continue };
        let Some(f) = &facts else {
            states.set(command, CommandState::new().enabled(false));
            continue;
        };
        let state = match command.id.strip_prefix(PREFIX).unwrap_or_default() {
            "add" | "add3D" => CommandState::new().enabled(f.has_art),
            "selectAll" => CommandState::new().enabled(!f.all.is_empty()),
            "remove" => CommandState::new().enabled(!f.sel.is_empty()),
            "subfolders" => CommandState::new().checked(f.subfolders),
            "openLocation" => CommandState::new().checked(f.open_location),
            "applyPreset" => {
                let name = command.params["name"].as_str().unwrap_or_default().to_string();
                CommandState::new().checked(f.applied == name).title(name)
            }
            "saveChanges" | "deletePreset" => CommandState::new().enabled(f.saved),
            "exportPreset" => CommandState::new().enabled(f.known),
            _ => continue,
        };
        states.set(command, state);
    }
}

/// Runs the panel menu command `id` (`ui.assetExport.*`); `None` when `id` is not one of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let name = id.strip_prefix(PREFIX)?;
    let Some(f) = Facts::of(app) else { return Some(Err(tl!("No document").to_string())) };
    let (uid, targets) = (f.uid, sel_or_all(&f.sel, &f.all));
    let done: Result<(), String> = match name {
        "add" => {
            collect(app, None, !params["multiple"].as_bool().unwrap_or(false));
            Ok(())
        }
        "add3D" => add_3d(app, None),
        "selectAll" => {
            select(app, f.all);
            Ok(())
        }
        "remove" => {
            remove_selected(app, &f.sel);
            Ok(())
        }
        "exportForScreens" => {
            open_export_for_screens_assets(app, Some(&targets));
            Ok(())
        }
        "formatSettings" => {
            format_settings(app, &targets, f.format);
            Ok(())
        }
        "subfolders" => app.run("assets.settings.set", json!({ "subfolders": !f.subfolders })).map(|_| ()),
        "openLocation" => app.run("assets.settings.set", json!({ "openLocation": !f.open_location })).map(|_| ()),
        "applyPreset" => {
            preset(app, uid, PresetPick::Apply(json!({ "name": params["name"] })));
            Ok(())
        }
        "savePresetAs" => {
            if let Some(d) = doc_panel(app) {
                d.saving = Some(String::new());
            }
            Ok(())
        }
        "saveChanges" if f.saved => {
            preset(app, uid, PresetPick::Save(f.applied));
            Ok(())
        }
        "deletePreset" if f.saved => {
            preset(app, uid, PresetPick::Delete(f.applied));
            Ok(())
        }
        "importPreset" => {
            preset(app, uid, PresetPick::Import);
            Ok(())
        }
        "exportPreset" if f.known => {
            preset(app, uid, PresetPick::Export(f.applied));
            Ok(())
        }
        "saveChanges" | "deletePreset" | "exportPreset" => Err(tl!("Apply a saved preset first").to_string()),
        _ => return None,
    };
    Some(done.map(|()| Value::Null))
}
