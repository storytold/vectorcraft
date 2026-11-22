//! Dialogs. Each dialog is a file here that the dialog host owns and shows; [`host`] opens held
//! dialogs, serves every module once per frame and reaches the open ones for automation.

pub mod action_batch;
pub mod action_insert_menu_item;
pub mod action_new;
pub mod action_playback;
pub mod action_select_object;
pub mod action_set_options;
pub mod action_stop;
pub(crate) mod actions;
pub mod artboard_options;
pub mod attributes_panel_options;
pub mod blend_options;
pub(crate) mod brush_art_options;
pub mod brush_delete;
pub mod brush_options;
pub mod brush_replace_art;
pub mod bullets_numbering;
pub mod char_style_options;
pub mod check_spelling;
pub mod circle_options;
pub mod color_balance;
pub mod color_group_options;
pub mod color_guide_options;
pub(crate) mod color_picker;
pub(crate) mod command;
pub mod composite_fonts;
pub mod confirm;
pub mod corners;
pub mod custom_dictionary;
pub mod dimension;
mod document_setup;
mod duotone;
pub mod dxf_import;
pub mod dxf_options;
pub mod edit_selection;
pub mod edit_views;
pub(crate) mod effect;
pub(crate) mod effect_adjust;
pub mod effect_gallery;
pub(crate) mod effect_params;
pub(crate) mod effect_rows;
pub mod envelope;
pub mod eps_options;
pub mod expand;
mod export_as;
mod export_for_screens;
pub(crate) mod export_host;
pub mod eyedropper;
pub mod file_info;
pub mod find_links;
pub mod find_replace;
pub mod flare_options;
pub mod flatten;
pub mod flattener_presets;
pub(crate) mod form;
pub(crate) mod free_distort;
pub mod freehand;
pub(crate) mod gradient_stop;
pub mod graph_data;
pub mod graph_design;
pub mod graph_type;
pub mod graphic_style_options;
pub mod halftone;
pub(crate) mod host;
pub mod hpgl_options;
pub mod hyphenation;
pub mod import_pdf;
pub mod indexed_color;
pub mod javascript_files;
pub mod justification;
pub mod kinsoku_sets;
pub(crate) mod layer_color;
pub mod layer_options;
pub mod layers_panel_options;
pub mod liquify;
pub mod live_paint_bucket;
pub mod live_paint_gaps;
pub mod live_paint_selection;
pub mod missing_fonts;
pub mod missing_links;
pub mod navigator_panel_options;
pub mod new_brush;
pub mod new_color_group;
mod new_document;
pub mod new_swatch;
pub mod new_text_style;
pub(crate) mod object_dialogs;
pub mod object_mosaic;
pub mod objects_on_path_options;
pub mod office_export;
pub(crate) mod option_form;
pub mod package;
pub mod para_style_options;
pub(crate) mod path_ops;
pub mod pathfinder_options;
pub mod pdf_presets;
pub mod perspective_grid;
pub mod perspective_options;
pub mod perspective_plane;
pub mod perspective_presets;
pub mod pixel_snap_options;
pub mod place;
pub mod placement_options;
pub mod plugin;
mod png_options;
pub(crate) mod preset_manager;
pub(crate) mod preview;
pub(crate) mod preview_folders;
pub mod print;
pub mod print_presets;
pub mod profile_mismatch;
pub mod proof_setup;
pub mod psd_import;
mod psd_options;
pub mod raster_effects;
pub mod rearrange_artboards;
pub mod recolor;
pub mod relink;
pub mod rename_artboards;
pub(crate) mod revolve;
pub(crate) mod revolve_gizmo;
pub(crate) mod revolve_lights;
pub mod safe_mode;
pub mod saturate;
pub mod save_brush_library;
pub mod save_for_web;
pub(crate) mod save_library;
pub(crate) mod save_options;
pub(crate) mod save_pdf;
pub mod save_style_library;
pub mod save_swatch_library;
pub mod save_symbol_library;
pub(crate) mod saved_names;
pub(crate) mod screen_formats;
pub mod script_check;
pub mod script_run;
pub mod shape_builder_options;
pub(crate) mod shapes;
pub mod shortcut_import;
pub(crate) mod simplify_slider;
pub mod slices;
pub mod smart_punctuation;
pub(crate) mod smooth_slider;
pub mod spot_colors;
pub mod svg_filter;
pub(crate) mod svg_options;
pub mod swatch_conflict;
pub mod swatch_options;
pub mod symbol_delete;
pub mod symbol_options;
pub mod symbolism_options;
pub mod tate_chu_yoko;
mod text_export;
pub mod text_import;
pub(crate) mod three_d;
mod three_d_classic;
pub(crate) mod three_d_materials;
pub mod three_point_arc_options;
mod tiff_bmp_tga;
pub mod tile_edge_color;
pub(crate) mod tool_options;
mod tools;
pub mod tracing_presets;
pub(crate) mod track_cube;
pub(crate) mod transform;
pub mod transform_each;
pub mod variables;
pub(crate) mod variation;
pub mod warichu;
mod warp_effect;
pub mod width_point;

pub use color_picker::open as open_color_picker;
pub use document_setup::open as open_document_setup;
pub use effect::open as open_effect_dialog;
pub(crate) use effect_params::{FILM_GRAIN_FIELDS, PAINT_DAUB_BRUSH_TYPE_LABELS};
pub use export_as::{export_selection, open as open_export_as};
pub(crate) use export_for_screens::drop_unread_screen_settings;
pub use export_for_screens::open as open_export_for_screens;
pub(crate) use export_for_screens::{
    ExportForScreensDialog, open_assets as open_export_for_screens_assets, saved_options as screen_saved_options, saved_rows as screen_saved_rows,
};
pub(crate) use new_document::preset_name;
pub use new_document::{create_without_dialog as new_document_without_dialog, open as open_new_document};
pub use png_options::open as open_raster_options;
pub use save_pdf::{open as open_save_pdf, open_preset as open_pdf_preset};
pub use tools::open_tool_dialog;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_accessibility;
#[cfg(test)]
mod tests_export;
#[cfg(test)]
mod tests_export_selection;
#[cfg(test)]
mod tests_import_pdf;
#[cfg(test)]
mod tests_messages;
#[cfg(test)]
mod tests_package;
#[cfg(test)]
mod tests_plugin_export;
#[cfg(test)]
mod tests_plugin_fields;
#[cfg(test)]
mod tests_plugin_import;
#[cfg(test)]
mod tests_raster_formats;
#[cfg(test)]
mod tests_raster_supplied_options;

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
mod tests_hpgl;

#[cfg(test)]
mod tests_tiff_bmp_tga;

#[cfg(test)]
mod tests_psd;

#[cfg(test)]
mod tests_print;

#[cfg(test)]
mod tests_print_presets;

#[cfg(test)]
mod tests_print_advanced;

#[cfg(test)]
mod tests_perspective;

#[cfg(test)]
mod tests_modal;

#[cfg(test)]
mod tests_rectangular_grid;

#[cfg(test)]
mod tests_shortcut_import;

#[cfg(test)]
mod tests_warp;
