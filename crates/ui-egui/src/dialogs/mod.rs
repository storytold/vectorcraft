//! Modal dialogs. Fields live in `UiState::dialog` (string/number JSON) so agents can fill them
//! through `ui.dialog.set` and press OK with `ui.dialog.confirm`.
//!
//! Each dialog is a file here exporting a [`DialogSpec`] (heading, body, OK action, buttons) plus
//! a row in the `registry!` below that maps its `Dialog::kind` strings to it. Dialogs inside the
//! shared modal frame only supply a body and a confirm action; a few (Preferences, Keyboard
//! Shortcuts, Workspaces, Find Font) draw their own window.

mod about;
mod all_tools;
pub mod artboard_options;
pub mod blend_options;
mod brush_art_options;
pub mod brush_options;
pub mod color_balance;
pub mod color_guide_options;
mod color_picker;
mod command;
pub mod confirm;
pub mod corners;
mod document_setup;
pub mod dxf_import;
pub mod dxf_options;
pub mod edit_selection;
mod effect;
pub mod envelope;
pub mod eps_options;
pub mod expand;
mod export_as;
mod export_for_screens;
pub mod eyedropper;
pub mod file_info;
pub mod flare_options;
pub mod flatten;
pub mod flattener_presets;
mod form;
pub mod freehand;
mod gradient_stop;
pub mod graphic_style_options;
pub mod halftone;
pub mod import_pdf;
pub mod layer_options;
pub mod layers_panel_options;
pub mod liquify;
pub mod missing_fonts;
pub mod missing_links;
pub(crate) mod modal;
pub mod new_color_group;
mod new_document;
pub mod new_swatch;
pub mod office_export;
pub mod package;
mod path_ops;
pub mod pdf_presets;
pub mod perspective_grid;
pub mod perspective_options;
pub mod perspective_plane;
pub mod perspective_presets;
pub mod place;
pub mod placement_options;
pub mod plugin;
mod png_options;
pub mod print;
pub mod print_presets;
mod psd_options;
pub mod raster_effects;
pub mod rearrange_artboards;
pub mod recolor;
mod recovery;
mod revolve;
pub(crate) mod revolve_gizmo;
pub mod saturate;
mod save_changes;
pub mod save_for_web;
pub(crate) mod save_options;
mod save_pdf;
pub mod save_style_library;
pub mod save_swatch_library;
mod shapes;
pub mod slices;
pub mod spot_colors;
pub(crate) mod svg_options;
pub mod swatch_conflict;
pub mod swatch_options;
pub mod symbolism_options;
mod text_export;
pub mod text_import;
mod tiff_bmp_tga;
pub mod tile_edge_color;
mod tools;
mod transform;
pub mod transform_each;
pub mod variables;
pub mod width_point;

use serde_json::{Value, json};

pub use color_picker::open as open_color_picker;
pub use document_setup::open as open_document_setup;
pub use effect::open as open_effect_dialog;
pub use export_as::open as open_export_as;
pub use export_for_screens::open as open_export_for_screens;
pub(crate) use export_for_screens::{
    KIND as EXPORT_FOR_SCREENS, formats as screen_formats, open_assets as open_export_for_screens_assets, saved_rows as screen_saved_rows,
};
pub(crate) use new_document::preset_name;
pub use new_document::{open as open_new_document, preset_card};
pub use png_options::open as open_raster_options;
pub use save_pdf::{open as open_save_pdf, open_preset as open_pdf_preset};
pub use tools::open_tool_dialog;

use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

type DialogResult = Result<Value, String>;

/// The shared dialog frame's inner margin, and the least room it leaves at the window's edges.
const MARGIN: i8 = 22;
const EDGE_GAP: f32 = 8.0;

/// How a dialog draws and applies itself. Specs start from [`DialogSpec::FORM`] and override what
/// differs.
pub(crate) struct DialogSpec {
    /// Draws its own window instead of the shared frame (the frame fields below are then unused).
    pub window: Option<fn(&mut VectorcraftApp, &egui::Context)>,
    /// The heading (and window title), in the UI language: the spec translates its own text and
    /// leaves names in it (a document's, a plug-in's) as they are.
    pub heading: fn(&Dialog) -> String,
    /// Draws the fields. Returns true to close the dialog as Cancel would.
    pub body: fn(&mut VectorcraftApp, &mut egui::Ui, &mut Dialog) -> bool,
    /// OK, Enter or `ui.dialog.confirm`: applies the dialog and closes it when done.
    pub confirm: fn(&mut VectorcraftApp, &Dialog) -> DialogResult,
    /// The OK button's label. None: no OK button, Enter does nothing and Cancel reads "Close".
    pub ok: Option<&'static str>,
    /// An extra button left of Cancel that sets `discard: true` and confirms ("Don't Save").
    pub discard: Option<&'static str>,
    pub min_width: f32,
    pub max_width: Option<f32>,
    /// The body runs a live preview interaction that Cancel rolls back.
    pub preview: bool,
    /// The OK button's label when it depends on the app (Export for Screens' "Download" on the
    /// web); `ok` when none. Only with an `ok`.
    pub ok_label: Option<fn(&VectorcraftApp) -> &'static str>,
}

impl DialogSpec {
    /// A text field per value; OK just closes. Also the fallback for unregistered kinds.
    pub const FORM: Self = Self {
        window: None,
        heading: |_| tl!("Dialog").into(),
        body: |app, ui, d| {
            form::grid(ui, d, app.session.general_unit());
            false
        },
        confirm: |app, _| {
            app.ui.dialog = None;
            Ok(Value::Null)
        },
        ok: Some("OK"),
        discard: None,
        min_width: 320.0,
        max_width: None,
        preview: false,
        ok_label: None,
    };

    /// A dialog that draws its own window and confirms through its module.
    const fn window(show: fn(&mut VectorcraftApp, &egui::Context), confirm: fn(&mut VectorcraftApp, &Dialog) -> DialogResult) -> Self {
        Self { window: Some(show), confirm, ..Self::FORM }
    }
}

/// The dialog registry: one row per dialog with its [`DialogKind`] variant, the `Dialog::kind`
/// strings it handles and its [`DialogSpec`]. Adding a dialog is a new file plus one row.
macro_rules! registry {
    ($($variant:ident: [$($kind:pat),+] => $spec:expr,)+) => {
        /// Every registered dialog.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum DialogKind {
            $($variant,)+
        }

        impl DialogKind {
            /// The dialog handling a `Dialog::kind` string.
            pub fn of(kind: &str) -> Option<Self> {
                match kind {
                    $($($kind)|+ => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Every variant, in registry order.
            pub const ALL: &[DialogKind] = &[$(Self::$variant,)+];

            fn spec(self) -> &'static DialogSpec {
                match self {
                    $(Self::$variant => {
                        static SPEC: DialogSpec = $spec;
                        &SPEC
                    })+
                }
            }
        }
    };
}

registry! {
    NewDocument: [new_document::KIND] => new_document::SPEC,
    Shape: ["rectangle", "roundedRectangle", "ellipse", "polygon", "star", "lineSegment"] => shapes::SPEC,
    Transform: ["move", "rotate", "scale", "reflect", "shear"] => transform::SPEC,
    PathOp: ["average", "offsetPath", "simplify", "splitIntoGrid"] => path_ops::SPEC,
    DocumentSetup: ["documentSetup"] => document_setup::SPEC,
    ArtboardOptions: [artboard_options::KIND] => artboard_options::SPEC,
    AllTools: ["allTools"] => all_tools::SPEC,
    ExportForScreens: ["exportForScreens"] => export_for_screens::SPEC,
    Recolor: [recolor::KIND] => recolor::SPEC,
    Command: ["command"] => command::SPEC,
    Variables: [variables::KIND] => variables::SPEC,
    Effect: ["effect"] => effect::SPEC,
    SaveChanges: [crate::unsaved::KIND] => save_changes::SPEC,
    Preferences: ["preferences"] => DialogSpec::window(crate::prefs_dialog::show, |app, _| crate::prefs_dialog::confirm(app)),
    Shortcuts: ["shortcuts"] => DialogSpec::window(crate::shortcut_editor::show, |app, _| crate::shortcut_editor::confirm(app)),
    Workspaces: ["newWorkspace", "manageWorkspaces"] => DialogSpec::window(crate::workspaces::show, |app, _| crate::workspaces::confirm(app)),
    FindFont: ["findFont"] => DialogSpec::window(crate::find_font::show, |app, _| crate::find_font::confirm(app)),
    SwatchOptions: [swatch_options::KIND] => swatch_options::SPEC,
    Confirm: [confirm::KIND] => confirm::SPEC,
    Message: [confirm::MESSAGE] => confirm::MESSAGE_SPEC,
    NewSwatch: [new_swatch::KIND] => new_swatch::SPEC,
    NewColorGroup: [new_color_group::KIND] => new_color_group::SPEC,
    GradientStop: ["gradientStop"] => gradient_stop::SPEC,
    ColorPicker: [color_picker::KIND] => color_picker::SPEC,
    GraphicStyleOptions: [graphic_style_options::KIND] => graphic_style_options::SPEC,
    EffectExists: [effect::EXISTS] => effect::EXISTS_SPEC,
    ColorGuideOptions: [color_guide_options::KIND] => color_guide_options::SPEC,
    ColorBalance: [color_balance::KIND] => color_balance::SPEC,
    Saturate: [saturate::KIND] => saturate::SPEC,
    SaveSwatchLibrary: [save_swatch_library::KIND] => save_swatch_library::SPEC,
    TileEdgeColor: [tile_edge_color::KIND] => tile_edge_color::SPEC,
    EyedropperOptions: [eyedropper::KIND] => eyedropper::SPEC,
    FlattenTransparency: [flatten::KIND] => flatten::SPEC,
    FlattenerPresets: [flattener_presets::KIND] => flattener_presets::SPEC,
    SaveStyleLibrary: [save_style_library::KIND] => save_style_library::SPEC,
    Expand: [expand::KIND] => expand::SPEC,
    SpotColors: [spot_colors::KIND] => spot_colors::SPEC,
    TransformEach: [transform_each::KIND] => transform_each::SPEC,
    WidthPoint: [width_point::KIND] => width_point::SPEC,
    Corners: [corners::KIND] => corners::SPEC,
    SavePdf: [save_pdf::KIND] => save_pdf::SPEC,
    SvgOptions: [svg_options::KIND] => svg_options::SPEC,
    NewDocumentMore: [new_document::MORE] => new_document::MORE_SPEC,
    Place: [place::KIND] => place::SPEC,
    RasterOptions: ["pngOptions", "jpgOptions", "webpOptions", "gifOptions", "png8Options"] => png_options::SPEC,
    ExportAs: ["exportAs"] => export_as::SPEC,
    ImportPdf: [import_pdf::KIND] => import_pdf::SPEC,
    SwatchConflict: [swatch_conflict::KIND] => swatch_conflict::SPEC,
    FileInfo: [file_info::KIND] => file_info::SPEC,
    RasterEffectsSettings: [raster_effects::KIND] => raster_effects::SPEC,
    MissingLinks: [missing_links::KIND] => missing_links::SPEC,
    MissingFonts: [missing_fonts::KIND] => missing_fonts::SPEC,
    TextImport: [text_import::KIND] => text_import::SPEC,
    PdfPresets: [pdf_presets::KIND] => pdf_presets::SPEC,
    PdfPreset: [save_pdf::PRESET_KIND] => save_pdf::PRESET_SPEC,
    SaveOptions: [save_options::KIND] => save_options::SPEC,
    TextExport: [text_export::KIND] => text_export::SPEC,
    OfficeExport: [office_export::KIND] => office_export::SPEC,
    DxfOptions: [dxf_options::KIND] => dxf_options::SPEC,
    PlacementOptions: [placement_options::KIND] => placement_options::SPEC,
    Package: [package::KIND] => package::SPEC,
    SliceOptions: [slices::OPTIONS] => slices::OPTIONS_SPEC,
    DivideSlices: [slices::DIVIDE] => slices::DIVIDE_SPEC,
    EpsOptions: [eps_options::KIND] => eps_options::SPEC,
    Recovery: [crate::recovery::KIND] => recovery::SPEC,
    DxfImport: [dxf_import::KIND] => dxf_import::SPEC,
    RasterFormatOptions: ["tiffOptions", "bmpOptions", "tgaOptions"] => png_options::SPEC,
    SaveForWeb: [save_for_web::KIND] => save_for_web::SPEC,
    PsdOptions: ["psdOptions"] => png_options::SPEC,
    Print: [print::KIND] => print::SPEC,
    PrintPreset: [print::PRESET_KIND] => print::PRESET_SPEC,
    PrintPresets: [print_presets::KIND] => print_presets::SPEC,
    Plugin: [plugin::KIND] => plugin::SPEC,
    VectorHalftone: [halftone::KIND] => halftone::SPEC,
    PerspectiveGrid: [perspective_grid::KIND] => perspective_grid::SPEC,
    Envelope: [envelope::WARP, envelope::MESH, envelope::OPTIONS] => envelope::SPEC,
    LiquifyOptions: [liquify::KIND] => liquify::SPEC,
    FreehandOptions: [freehand::KIND] => freehand::SPEC,
    FlareOptions: [flare_options::KIND] => flare_options::SPEC,
    SymbolismOptions: [symbolism_options::KIND] => symbolism_options::SPEC,
    PerspectiveGridPresets: [perspective_presets::KIND] => perspective_presets::SPEC,
    PerspectiveGridOptions: [perspective_options::KIND] => perspective_options::SPEC,
    BlendOptions: [blend_options::KIND] => blend_options::SPEC,
    BrushOptions: [brush_options::KIND] => brush_options::SPEC,
    EditSelection: [edit_selection::KIND] => edit_selection::SPEC,
    PerspectivePlane: [perspective_plane::KIND] => perspective_plane::SPEC,
    LayerOptions: [layer_options::KIND] => layer_options::SPEC,
    LayersPanelOptions: [layers_panel_options::KIND] => layers_panel_options::SPEC,
    RearrangeArtboards: [rearrange_artboards::KIND] => rearrange_artboards::SPEC,
}

/// The button labels the shared dialog frame can show (OK, discard and the fixed Cancel/Close),
/// so the catalog tests can insist they are translated.
pub fn button_labels() -> Vec<&'static str> {
    let mut v = vec!["Cancel", "Close"];
    for spec in std::iter::once(&DialogSpec::FORM).chain(DialogKind::ALL.iter().map(|k| k.spec())) {
        v.extend(spec.ok);
        v.extend(spec.discard);
    }
    v.sort_unstable();
    v.dedup();
    v
}

/// The spec for a `Dialog::kind` ([`DialogSpec::FORM`] when unregistered).
fn spec(kind: &str) -> &'static DialogSpec {
    static FALLBACK: DialogSpec = DialogSpec::FORM;
    DialogKind::of(kind).map_or(&FALLBACK, DialogKind::spec)
}

/// Run a command and close the dialog (whether or not the command succeeded).
fn run_and_close(app: &mut VectorcraftApp, id: &str, params: Value) -> DialogResult {
    let r = app.run(id, params);
    app.ui.dialog = None;
    r
}

/// Close the open dialog as Cancel does, rolling back a live preview (`ui.dialog.cancel`).
pub fn cancel(app: &mut VectorcraftApp) {
    missing_fonts::requeue_replaced(app);
    if app.ui.dialog.take().is_some_and(|d| spec(&d.kind).preview) {
        let _ = app.session.cancel_interaction();
    }
    settle(app);
}

/// Apply the open dialog (OK).
pub fn confirm(app: &mut VectorcraftApp) -> DialogResult {
    missing_fonts::requeue_replaced(app);
    let Some(d) = app.ui.dialog.clone() else { return Err("no dialog open".into()) };
    // A file dialog it shows off the UI thread confirms the dialog as it is again.
    let r = crate::picks::as_entry(app, || crate::picks::Entry::Confirm(Box::new(d.clone())), |app| (spec(&d.kind).confirm)(app, &d));
    settle(app);
    r
}

/// After a dialog closes, and each frame: a Missing Fonts dialog that another dialog replaced waits
/// for its turn again, the search a Missing Fonts dialog started stops once that dialog is gone,
/// and with no dialog open the next document's Missing Fonts dialog opens (one dialog at a time).
pub(crate) fn settle(app: &mut VectorcraftApp) {
    missing_fonts::requeue_replaced(app);
    missing_fonts::stop_when_closed(app);
    missing_fonts::open_next(app);
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    about::show(app, ctx);
    settle(app);
    // The kind of dialog shown last frame: a different one (or none) means this one just opened, and
    // its first field is to take the keyboard focus (`focus_id`, until a field takes it).
    let (shown_id, focus_id) = (egui::Id::new("dialog-shown"), egui::Id::new("dialog-focus-pending"));
    let Some(mut d) = app.ui.dialog.clone() else {
        revolve_gizmo::clear(ctx);
        app.ui.dialog_file = None;
        ctx.data_mut(|m| {
            m.remove::<String>(shown_id);
            m.remove::<bool>(focus_id);
        });
        return;
    };
    if !revolve_gizmo::active(&d) {
        revolve_gizmo::clear(ctx);
    }
    let focus_first = ctx.data_mut(|m| {
        if m.get_temp::<String>(shown_id).as_deref() != Some(d.kind.as_str()) {
            m.insert_temp(shown_id, d.kind.clone());
            m.insert_temp(focus_id, true);
        }
        m.get_temp::<bool>(focus_id).unwrap_or(false)
    });
    let spec = spec(&d.kind);
    if let Some(window) = spec.window {
        return window(app, ctx);
    }
    let mut ok = false;
    let mut cancel = false;
    let mut discard = false;
    let heading = (spec.heading)(&d);
    let revolves = revolve_gizmo::active(&d);
    if revolves {
        revolve_gizmo::show(app, ctx, &mut d);
    }
    let initial = if revolves { egui::vec2((ctx.content_rect().width() / 2.0 - 226.0).max(0.0), -40.0) } else { egui::vec2(0.0, -40.0) };
    let id = egui::Id::new(("dialog", d.kind.as_str()));
    let id = if revolves { id.with("revolve") } else { id };
    modal::show_at(ctx, &heading, id, initial, MARGIN, |ui| {
        // Never wider than the window (a large UI scale in a small window): the text wraps.
        let room = (ctx.content_rect().width() - 2.0 * (f32::from(MARGIN) + EDGE_GAP)).max(EDGE_GAP);
        ui.set_min_width(spec.min_width.min(room));
        ui.set_max_width(spec.max_width.map_or(room, |w| w.min(room)));
        modal::heading(ui, &heading);
        ui.add_space(12.0);
        // Just opened: its first field takes the keyboard focus, as in the reference app (type a
        // value, press Enter). Only the body's fields can take it.
        if focus_first {
            ui.data_mut(|m| m.insert_temp(widgets::dialog_focus_flag(), true));
        }
        cancel = (spec.body)(app, ui, &mut d);
        // Taken (the flag is gone): done. Still there (the window's measuring frame): next frame.
        if focus_first && ui.data_mut(|m| m.remove_temp::<bool>(widgets::dialog_focus_flag())).is_none() {
            ui.data_mut(|m| m.remove::<bool>(focus_id));
        }
        ui.add_space(16.0);
        // The button row is as wide as the fields above it and as tall as the buttons: a
        // right-to-left layout would otherwise take all the room left in the window, so the
        // window could never shrink to its content.
        let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(label) = spec.ok.map(|ok| spec.ok_label.map_or(ok, |f| f(app)))
                && widgets::primary_button(ui, label).clicked()
            {
                ok = true;
            }
            ui.add_space(8.0);
            if widgets::secondary_button(ui, if spec.ok.is_some() { "Cancel" } else { "Close" }).clicked() {
                cancel = true;
            }
            if let Some(label) = spec.discard {
                ui.add_space(28.0);
                discard = widgets::secondary_button(ui, label).clicked();
            }
        });
    });
    if spec.ok.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        ok = true;
    }
    if discard {
        d.fields.insert("discard".into(), json!(true));
        ok = true;
    }
    app.ui.dialog = Some(d);
    if cancel {
        self::cancel(app);
    } else if ok && let Err(e) = confirm(app) {
        app.status(e);
    }
}

/// What a dropdown shows for `names`, a list mixing built-in labels (`builtin(index)`: translated
/// into `lang`) with names the user saved or a file or the system supplied (shown as they are).
pub(crate) fn shown_names<'a>(lang: crate::i18n::Lang, names: &[&'a str], builtin: impl Fn(usize) -> bool) -> Vec<&'a str> {
    names.iter().enumerate().map(|(k, n)| crate::i18n::label_or_name(lang, n, builtin(k))).collect()
}

/// [`widgets::dropdown_names`] over such a mixed list ([`shown_names`] in the UI language), showing
/// `current` as its entry reads; a `current` that isn't among `names` is shown as given (a caller
/// translates its own [Custom]). Returns the index chosen in `names`.
pub(crate) fn mixed_dropdown(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    names: &[&str],
    width: f32,
    builtin: impl Fn(usize) -> bool,
) -> Option<usize> {
    let shown = shown_names(crate::i18n::current(), names, builtin);
    let current = names.iter().position(|n| *n == current).and_then(|i| shown.get(i).copied()).unwrap_or(current);
    widgets::dropdown_names(ui, id, current, &shown, width)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_export;
#[cfg(test)]
mod tests_import_pdf;
#[cfg(test)]
mod tests_package;
#[cfg(test)]
mod tests_raster_formats;

#[cfg(test)]
mod tests_dxf;
#[cfg(test)]
mod tests_screen_assets;
#[cfg(test)]
mod tests_screens;

#[cfg(test)]
mod tests_eps;

#[cfg(test)]
mod tests_metafile;

#[cfg(test)]
mod tests_tiff_bmp_tga;

#[cfg(test)]
mod tests_save_for_web;

#[cfg(test)]
mod tests_psd;

#[cfg(test)]
mod tests_print;

#[cfg(test)]
mod tests_print_presets;

#[cfg(test)]
mod tests_print_advanced;

#[cfg(test)]
mod tests_scale;

#[cfg(test)]
mod tests_perspective;

#[cfg(test)]
mod tests_modal;
