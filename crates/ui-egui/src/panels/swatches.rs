//! Swatches panel: proxy, Recent Colors, find field, thumbnail grid (15.5 pt tiles on a 17 pt pitch)
//! or list, colour groups as folders, None/Registration first, bottom bar and panel menu.
//!
//! Swatches drag to reorder, into and out of colour groups (`swatch.move`) and onto art (the active
//! proxy's paint command with the object's id) and onto the Gradient panel's ramp; a Fill/Stroke
//! proxy or the Gradient panel's thumbnail dropped on the panel becomes a swatch (`swatch.new`).

use std::sync::Arc;

use egui::{Color32, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::libraries::{BuiltinLibrary, GRADIENT_LIBRARIES, SWATCH_LIBRARIES};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, GradientKind, Paint, Swatch, SwatchLibrary};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::swatchlib;

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::{active_paint, pstate, set_pstate};
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, SwatchRows, menu_item, swatch_tile};
use crate::{VectorcraftApp, icons};

mod actions;
mod body;
mod dnd;
mod libraries;
mod tiles;

pub use actions::menu;
#[cfg(test)]
use actions::{deletable, delete, mergeable, new_swatch, selected_group};
use body::body;
#[cfg(test)]
use body::{active_swatch, find_id, focus_swatch, kept_swatch};
use dnd::drag_of;
#[cfg(test)]
use dnd::highlighted;
pub(crate) use dnd::{click_selection, drag_preview};
pub(crate) use libraries::{
    DOCUMENT_SWATCHES, SwatchLibraries, USER_SLOT, colour_libraries, limit_key, limit_name, load_library, open_library, other_library, user_library,
    window_menu,
};
#[cfg(test)]
pub(crate) use tiles::describe;
pub(crate) use tiles::{draw_folder, folder_tile, global_mark, list_row, list_row_name, tile_grid};

/// View modes of the swatch list (and of the library panels: `ui.libraryPanel {view}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    SmallThumb,
    #[default]
    MediumThumb,
    LargeThumb,
    SmallList,
    LargeList,
}

impl View {
    pub(crate) const ALL: [(View, &'static str); 5] = [
        (View::SmallThumb, "Small Thumbnail View"),
        (View::MediumThumb, "Medium Thumbnail View"),
        (View::LargeThumb, "Large Thumbnail View"),
        (View::SmallList, "Small List View"),
        (View::LargeList, "Large List View"),
    ];
    pub(crate) fn is_list(self) -> bool {
        matches!(self, View::SmallList | View::LargeList)
    }
    /// (tile, pitch) in points. Medium is the measured 15.5 / 17 pt.
    pub(crate) fn tile(self) -> (f32, f32) {
        match self {
            View::SmallThumb => (11.0, 12.5),
            View::MediumThumb => (15.5, 17.0),
            View::LargeThumb => (30.0, 32.0),
            View::SmallList => (12.0, 17.0),
            View::LargeList => (18.0, 25.0),
        }
    }
}

/// Swatch-kind filter (Show Swatch Kinds menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    All,
    Color,
    Gradient,
    Pattern,
    Groups,
}

impl Kind {
    const ALL: [(Kind, &'static str); 5] = [
        (Kind::All, "Show All Swatches"),
        (Kind::Color, "Show Color Swatches"),
        (Kind::Gradient, "Show Gradient Swatches"),
        (Kind::Pattern, "Show Pattern Swatches"),
        (Kind::Groups, "Show Color Groups"),
    ];
    /// Does a swatch with `paint` (in a group or not) pass the filter?
    pub fn accepts(self, paint: &Paint, in_group: bool) -> bool {
        match self {
            Kind::All => true,
            Kind::Color => matches!(paint, Paint::Solid { .. } | Paint::None),
            Kind::Gradient => matches!(paint, Paint::Gradient(_)),
            Kind::Pattern => matches!(paint, Paint::Pattern { .. }),
            Kind::Groups => in_group,
        }
    }
}

/// A row of the swatch list: special tiles, plain swatches and group folders.
#[derive(Clone, Debug)]
enum Entry {
    Registration,
    Swatch { name: String, paint: Paint, global: bool, spot: bool },
    Folder(String),
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Registration => REGISTRATION,
            Entry::Swatch { name, .. } | Entry::Folder(name) => name,
        }
    }
    fn is_folder(&self) -> bool {
        matches!(self, Entry::Folder(_))
    }
}

/// The panel's rows for the kind filter and the find field's `query` (a case-insensitive name
/// match; a colour group whose name matches shows all its swatches).
fn entries(app: &VectorcraftApp, kind: Kind, query: &str) -> Vec<Entry> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    let q = query.trim().to_lowercase();
    let found = |n: &str| q.is_empty() || n.to_lowercase().contains(&q);
    let mut out = vec![];
    // A tint swatch shows its base's kind (global or spot).
    let sw = |s: &vectorcraft_color::Swatch| {
        let (global, spot) = s.tint_of().and_then(|(base, _)| d.swatch(base)).map_or((s.global, s.spot), |b| (b.global, b.spot));
        // A spot colour shows the colour its art shows (a Lab one's CMYK equivalent when the Spot
        // Colors options use those).
        let paint = match s.paint {
            Paint::Solid { color, swatch: None, .. } if spot => Paint::solid(d.linked_color(color, true)),
            _ => s.paint.clone(),
        };
        Entry::Swatch { name: s.name.clone(), paint, global, spot }
    };
    // None first, then Registration, then the rest (the reference app's order).
    let (specials, rest): (Vec<_>, Vec<_>) = d.swatches.iter().partition(|s| s.paint.is_none());
    for s in specials.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)) {
        out.push(sw(s));
    }
    if matches!(kind, Kind::All | Kind::Color) && found(REGISTRATION) {
        out.push(Entry::Registration);
    }
    for s in rest.iter().filter(|s| kind.accepts(&s.paint, false) && found(&s.name)) {
        out.push(sw(s));
    }
    for g in &d.swatch_groups {
        let all = found(&g.name);
        let items: Vec<_> = g.swatches.iter().filter(|s| kind.accepts(&s.paint, true) && (all || found(&s.name))).collect();
        // Empty groups show only among all swatches or groups, and when the find field names them.
        if items.is_empty() && !(all && matches!(kind, Kind::Groups | Kind::All)) {
            continue;
        }
        out.push(Entry::Folder(g.name.clone()));
        out.extend(items.into_iter().map(sw));
    }
    out
}

/// The paint command params that apply entry `e` (`paint.setFill`); none for a colour group.
fn swatch_params(e: &Entry) -> Option<Value> {
    Some(match e {
        Entry::Registration => json!({"swatch": REGISTRATION}),
        Entry::Swatch { paint, .. } if paint.is_none() => json!({"none": true}),
        Entry::Swatch { name, .. } => json!({"swatch": name}),
        Entry::Folder(_) => return None,
    })
}

/// Apply a clicked swatch to the active proxy (Alt: the inactive one). An Alt-click on a color
/// swatch with a gradient stop selected recolors the stop as dropping the swatch on it does,
/// and with a freeform gradient point selected it recolors the point.
fn apply(app: &mut VectorcraftApp, ui: &Ui, e: &Entry) {
    let Some(params) = swatch_params(e) else { return };
    if super::alt_held(ui)
        && let Some(c) = drag_of(e, vec![]).color()
        && super::gradient::recolor_selected(app, c, &params)
    {
        return;
    }
    super::apply_click(app, ui, params);
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui, TILE);
}

/// The panel in a Fill/Stroke chip popover (the Control bar's and Properties'): the same body, its
/// tiles under their own ids so it can be open beside the panel.
pub(crate) fn popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    body(app, ui, POPOVER_TILE);
}

/// Id salts of the panel's tiles and of a popover's.
const TILE: &str = "swatch-tile";
const POPOVER_TILE: &str = "swatch-pop-tile";

#[cfg(test)]
mod tests;
