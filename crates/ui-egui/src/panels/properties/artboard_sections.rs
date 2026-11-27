//! The Properties panel's Edit Artboards sections (the Artboard tool) and the artboard size controls
//! the Control bar and Artboard Options share.

use craft::behavior::{IconId, Mixed};
use craft::color_ui::ui::{Paint, PaintChip};
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing, Trigger};
use craft::ui::controls::disclosure::DisclosureGlyph;
use craft::ui::form::Field;
use craft::ui::layout_components::{Grid, Row};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_engine::cmd::newdoc;

use super::object_sections::HALVES;
use crate::VectorcraftApp;
use crate::page_glyphs::PageGlyph;
use crate::panels::{artboards, transform};

/// Edit Artboards (the Artboard tool): a Transform section with the active artboard's reference
/// point, position and size (a new W or H, preset or orientation goes to every selected artboard,
/// in one undo step), its preset and orientation, and Move and Scale Artwork with Artboard; then an
/// Artboards section with its name (edited to rename it), the Artboard Options icon, its Fill
/// (also for every selected artboard), New Artboard and Delete Artboard; and Quick Actions:
/// Rearrange All and Exit back to the Selection tool.
pub(super) fn artboard_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let i = artboards::selected(app, n);
    let ab = app.session.active().and_then(|d| d.doc.artboards.get(i)).cloned();
    if let Some(ab) = &ab {
        super::section(tl!("Transform")).show(ui, |ui, _| {
            transform::artboard_fields(app, ui, i, ab.rect);
            artboard_size_row(app, ui, i, ab.rect.width(), ab.rect.height());
            artboards::art_options(app, ui);
        });
    }
    super::section(tl!("Artboards")).show(ui, |ui, _| {
        if let Some(ab) = &ab {
            Field::new(tl!("Name:")).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    // The field shows the name as it is (names are never translated). A blank name is refused:
                    // the field shows the old one again.
                    let out = TextField::new(tl!("Name"), Bind::given(ab.name.clone())).show(ui);
                    if let Some(text) = out.value() {
                        let name = text.trim();
                        if !name.is_empty() && name != ab.name {
                            app.run("artboard.setProps", json!({ "index": i, "name": name })).ok();
                        }
                    }
                    IconButton::command(IconId::new("dc-artboard-options"), CommandRef::with_params("tool.options", json!({ "tool": "artboard" })))
                        .label(tl!("Artboard Options"))
                        .show(ui);
                });
            });
            artboard_fill_row(app, ui, i, ab.color);
        }
        Grid::new(&HALVES).show(ui, |ui, _| {
            if Button::new(tl!("New Artboard")).sizing(Sizing::FillWidth).show(ui).clicked() {
                artboards::add_new(app);
            }
            let deletable = n > 1 && !artboards::deletable(app, n).is_empty();
            if Button::new(tl!("Delete Artboard")).enabled(deletable).sizing(Sizing::FillWidth).show(ui).clicked() {
                artboards::delete_selected(app);
            }
        });
    });
    super::section(tl!("Quick Actions")).show(ui, |ui, _| {
        Grid::new(&HALVES).show(ui, |ui, _| {
            rearrange_button(ui, n);
            Button::command(CommandRef::with_params("tool.select", json!({ "tool": "selection" })))
                .label(tl!("Exit"))
                .sizing(Sizing::FillWidth)
                .show(ui);
        });
    });
}

/// Rearrange All: opens Rearrange All Artboards; disabled with fewer than two artboards.
pub(super) fn rearrange_button(ui: &mut Ui, artboards: usize) {
    Button::command(CommandRef::with_params("ui.menuDialog", json!({ "command": "artboard.rearrange" })))
        .label(tl!("Rearrange All"))
        .enabled(artboards > 1)
        .sizing(Sizing::FillWidth)
        .show(ui);
}

/// A preset `pw` × `ph` turned to an artboard's orientation (`landscape`, or portrait; a square
/// preset as it is).
fn turned(landscape: bool, pw: f64, ph: f64) -> (f64, f64) {
    if (pw > ph) == landscape || pw == ph { (pw, ph) } else { (ph, pw) }
}

/// Artboard `i`'s size preset (New Document's saved and built-in presets, by category, either way
/// round; Custom when its `w` × `h` points match none) and its orientation, here and in the Control
/// bar. A preset sizes each selected artboard in its own orientation; an orientation swaps the
/// width and height of each selected artboard that is not square and not already in it. Each
/// top-left corner stays put.
pub(crate) fn artboard_size_row(app: &mut VectorcraftApp, ui: &mut Ui, i: usize, w: f64, h: f64) {
    let (Some(choice), _) = artboard_size_controls(ui, &artboard_presets(app), w, h, None) else { return };
    let scale_art = artboards::scale_art(app);
    let indices = artboards::resized_with(app, i);
    artboards::set_props(app, &indices, json!({ "scaleArt": scale_art }), |j, r| {
        let (width, height) = choice.size_for(r.width(), r.height())?;
        Some(json!({ "index": j, "width": width, "height": height }))
    })
    .ok();
}

/// A shared picker choice, applied separately to each artboard's orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ArtboardSizeChoice {
    Preset(f64, f64),
    Orientation(bool),
}

impl ArtboardSizeChoice {
    pub(crate) fn size_for(self, w: f64, h: f64) -> Option<(f64, f64)> {
        match self {
            Self::Preset(pw, ph) => Some(turned(w > h, pw, ph)),
            Self::Orientation(landscape) if (w > h) != landscape && w != h => Some((h, w)),
            Self::Orientation(_) => None,
        }
    }
}

/// The presets the size menu lists, by category: New Document's lists without Recent (it holds
/// documents made, not presets) and without empty ones.
pub(crate) fn artboard_presets(app: &VectorcraftApp) -> Vec<(&'static str, Vec<newdoc::DocSettings>)> {
    newdoc::category_names()
        .filter(|c| !c.eq_ignore_ascii_case("Recent"))
        .filter_map(|c| Some((c, newdoc::category(&app.session, c)?)))
        .filter(|(_, l)| !l.is_empty())
        .collect()
}

/// A portrait page, the Portrait button's icon.
static PORTRAIT: PageGlyph = PageGlyph { landscape: false };
/// A landscape page, the Landscape button's icon.
static LANDSCAPE: PageGlyph = PageGlyph { landscape: true };

/// The Preset menu and the Portrait and Landscape buttons. Dialogs keep the chosen size in their
/// draft until OK. With `fits` (Artboard Options) the menu starts with Fit to Artwork Bounds and Fit
/// to Selected Art, each enabled by its entry; the second return value is a chosen Fit: false for
/// all artwork, true for selected artwork.
pub(crate) fn artboard_size_controls(
    ui: &mut Ui,
    presets: &[(&'static str, Vec<newdoc::DocSettings>)],
    w: f64,
    h: f64,
    fits: Option<[bool; 2]>,
) -> (Option<ArtboardSizeChoice>, Option<bool>) {
    let landscape = w > h;
    let same = |s: &newdoc::DocSettings| {
        let (pw, ph) = turned(landscape, s.width, s.height);
        (pw - w).abs() < 0.01 && (ph - h).abs() < 0.01
    };
    let current = presets.iter().flat_map(|(_, l)| l).find(|s| same(s)).map_or(tl!("Custom"), |s| crate::dialogs::preset_name(&s.name)).to_string();
    // The preset's width and height as listed, or the orientation clicked, or the fit chosen.
    let mut preset = None;
    let mut orientation = None;
    let mut fit = None;
    Field::new(tl!("Preset:")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let button = Button::new(current.clone()).trailing_icon(DisclosureGlyph::menu_button()).show(ui);
            Menu::new(tl!("Preset")).opened_by(&button.response, Trigger::Click).show(ui, |ui, _| {
                if let Some(enabled) = fits {
                    for (selection, label) in [(false, tl!("Fit to Artwork Bounds")), (true, tl!("Fit to Selected Art"))] {
                        if MenuItem::new(label).enabled(enabled[usize::from(selection)]).show(ui).ran {
                            fit = Some(selection);
                        }
                    }
                    craft::ui::display::Divider::horizontal().show(ui);
                }
                for (cat, list) in presets {
                    Menu::submenu_row(ui, tl!(cat).to_string(), |ui, _| {
                        for s in list {
                            if MenuItem::new(crate::dialogs::preset_name(&s.name).to_owned()).radio_mark(same(s)).show(ui).ran {
                                preset = Some((s.width, s.height));
                            }
                        }
                    });
                }
            });
            // Two toggles rather than a segmented control: a click on the shown orientation must still turn
            // the other selected artboards.
            for (is_landscape, label, glyph) in [(false, tl!("Portrait"), &PORTRAIT), (true, tl!("Landscape"), &LANDSCAPE)] {
                let out = ToggleButton::new(label, Bind::given(landscape == is_landscape)).icon(IconSpec::glyph(glyph)).icon_only().show(ui);
                if out.value().is_some() {
                    orientation = Some(is_landscape);
                }
            }
        });
    });
    (preset.map(|(w, h)| ArtboardSizeChoice::Preset(w, h)).or_else(|| orientation.map(ArtboardSizeChoice::Orientation)), fit)
}

/// Artboard `i`'s Fill, its background `color`: the chip shows it (the None paint when the artboard
/// has none) and a click on it opens the Color Picker for the artboard. The list sets None, White or
/// Black (in the document's color mode) or opens the Color Picker.
fn artboard_fill_row(app: &mut VectorcraftApp, ui: &mut Ui, i: usize, color: Option<Color>) {
    let fills = artboards::fill_colors(app);
    let shown = artboards::fill_name(app, color);
    let mut entries: Vec<Entry<usize>> = artboards::FILLS.iter().enumerate().map(|(k, name)| Entry::new(k, *name)).collect();
    entries.push(Entry::new(artboards::FILLS.len(), "Color Picker…"));
    let current = artboards::FILLS.iter().position(|name| *name == shown).map_or(Mixed::Mixed, Mixed::Same);
    let mut set = None;
    let mut pick = false;
    Field::new(tl!("Fill:")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let paint = color.map_or(Paint::None, |c| Paint::Solid(Srgba::from_rgb_array(c.to_rgb())));
            // A click only reports: the app's Color Picker opens, not the chip's own popover. The chip is named apart from
            // the list beside it.
            let name = crate::i18n::fmt(tl!("{row} swatch"), &[("row", tl!("Fill"))]);
            let chip = PaintChip::new(name, Bind::given(paint), Srgba::rgb(0.0, 0.0, 0.0)).popover(false).show(ui);
            pick = chip.clicked.is_some();
            let list = Select::new(tl!("Fill"), Bind::given(current), &entries).placeholder(tl!("Custom")).show(ui);
            match list.value().copied() {
                Some(k) if k < fills.len() => set = Some(fills[k]),
                Some(_) => pick = true,
                None => {}
            }
        });
    });
    // A Fill chosen from the list goes to every selected artboard, in one undo step.
    if let Some(c) = set {
        let color = c.as_ref().map(super::super::color_json);
        let indices = artboards::renamed_with(app, i);
        artboards::set_props(app, &indices, json!({}), |j, _| Some(json!({ "index": j, "color": color }))).ok();
    }
    if pick {
        app.run("ui.colorPicker", json!({ "artboard": i })).ok();
    }
}
