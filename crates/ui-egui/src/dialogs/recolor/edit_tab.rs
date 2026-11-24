//! The Recolor Artwork dialog's Edit tab: the harmony rules, the link of the harmony colours and the New Color Group
//! menu; the wheel of the new colours (smooth or segmented, hue with saturation or with brightness) or their colour
//! bars, with the Add Color and Remove Color tools and the right-click menu of a colour; the Prominent Colors bar; the
//! colour sliders; and the colour chips with Set Current Color As Base Color. Also the Color Theme Picker's canvas
//! tool, which takes clicks and drags on the document canvas while the picker is on.

use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, Cursor, CursorIcon, EditEvent, IconId, Selection, SelectionChange, keyed};
use craft::color_ui::ui::{DiscLook, DiscRadius, HarmonyWheel, HarmonyWords};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::buttons::{IconButton, PopupMark, ToggleButton};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft::ui::choice::{CellMarks, Segmented, Swatch, SwatchTable};
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, ReorderRule, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::notices::Notice;
use craft::ui::pickers::Select;
use craft::ui::primitives::Draw;
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::{Harmony, in_model_of};
use vectorcraft_color::recolor::ColorKey;

use super::prominent::{ProminentBar, ProminentPart, weights};
use super::{
    COMPS, DRAG, Feed, Fields, GLOBAL, MAX_PICKS, MenuChoice, RecolorDialog, SAVE_GROUP, SHADE_COLS, SHADE_ROWS, SliderTarget, THEME_PICKER,
    THEME_PICKS, WEIGHTS, WHEEL_AXES, WHEEL_TOOL, active_rows, add_color, added_color, apply_rule, base_row, can_set_base, choose, edit_slider_row,
    from_colors, global_sliders, in_group, keeps_rows, mode_menu, move_bar, move_color, prominent_parts, remove_color, removed, set_base, shade_at,
    sliders, to_color, wheel_axes,
};
use crate::VectorcraftApp;
use crate::panels::{c32, srgba};

/// The `display` ids: the smooth wheel, the segmented wheel and the colour bars.
const SMOOTH: &str = "smooth";
const SEGMENTED: &str = "segmented";
const BARS: &str = "bars";
/// The display buttons: id, icon and name.
const DISPLAYS: [(&str, &str, &str); 3] =
    [(SMOOTH, "circle", "Show smooth color wheel"), (SEGMENTED, "target", "Show segmented color wheel"), (BARS, "list", "Show color bars")];
/// The `wheelTool` ids of the Add Color and Remove Color tools.
const ADD: &str = "add";
const REMOVE: &str = "remove";
/// The wheel tools: id, icon and name.
const TOOLS: [(&str, &str, &str); 2] = [(ADD, "plus", "Add Color Tool"), (REMOVE, "minus", "Remove Color Tool")];
/// A colour bar is this many times as wide as it is high.
const BAR_ASPECT: u16 = 6;
/// Bars shown before the list scrolls: as tall as the wheel the bars replace.
const BAR_ROWS: u16 = 11;
/// The target of the wheel's right-click menu: `{marker, line, hs}` (row indexes and the hue and
/// saturation clicked).
const MENU_AT: &str = "__wheelMenu";

/// The name of row `row` in the wheel's menus, the bars, the chips and the Prominent Colors bar: "Color N".
pub(super) fn color_name(row: usize) -> String {
    format!("{} {}", tl!("Color"), row + 1)
}

/// The display `display` names: the smooth wheel unless it names the segmented wheel or the bars.
fn shown_display(d: &Fields) -> &'static str {
    match d.str("display").as_str() {
        SEGMENTED => SEGMENTED,
        BARS => BARS,
        _ => SMOOTH,
    }
}

/// The Edit tab. Returns whether the rows changed, and the row whose new colour a colour's menu asked the Color Picker
/// for (`false`: the picker, not Pick from Swatches). `feed` is what the pump read of the app, which the sliders share
/// with the Assign tab.
pub(super) fn show(ui: &mut Ui, d: &mut Fields, feed: &Feed, rows: &mut Vec<Value>) -> (bool, Option<(usize, bool)>) {
    let linked = d.bool("linked");
    let active = active_rows(rows);
    // The harmony rules build from the base colour; the sliders and the wheel's second channel edit the selected
    // colour, the base colour when the tab does not show the first selected row.
    let base = base_row(rows, &active);
    let sel = edit_slider_row(d, rows, &active, base);
    let display = shown_display(d);
    let axes = wheel_axes(d);
    let target = SliderTarget::at(feed, d, rows, sel);
    let mut changed = top_row(ui, d, &target, rows, &active, base);
    tool_row(ui, d, display, axes);

    let shown = sel.and_then(|s| rows.get(s)).and_then(to_color);
    let segmented = display == SEGMENTED;
    let tool = d.str(WHEEL_TOOL);
    let (wheel, opener) = if display == BARS {
        let (moved, opener) = bars(ui, d, rows, &active, base, sel);
        changed |= moved;
        (None, opener)
    } else {
        let models: Vec<Color> = active.iter().map(|&i| rows.get(i).and_then(to_color).unwrap_or_default()).collect();
        let colors: Vec<Srgba> = models.iter().map(srgba).collect();
        // A marker's colour as its row keeps it: in the row's model, to its key's precision.
        let stored = |k: usize, c: Srgba| -> Srgba { models.get(k).map_or(c, |&m| srgba(&ColorKey::of(&in_model_of(m, c)).color())) };
        let at = |row: Option<usize>| row.and_then(|r| active.iter().position(|&i| i == r));
        let look = if segmented { DiscLook::Segmented } else { DiscLook::Smooth };
        let name = if segmented { tl!("Segmented color wheel") } else { tl!("Color wheel") };
        let out = HarmonyWheel::new(name, &colors).selected_marker(at(sel)).base(at(base)).look(look).radius(axes).stored_as(&stored).show(ui);
        // A right-click on the disc: a marker's menu, or Add New Color at the point, tied to the spoke it is on.
        if let Some(hit) = out.context {
            let row = |k: Option<usize>| k.and_then(|k| active.get(k)).copied();
            d.fields.insert(MENU_AT.into(), json!({"marker": row(hit.marker), "line": row(hit.spoke), "hs": [hit.hue, hit.radial]}));
        }
        let opener = out.response.clone();
        (Some(out), Some(opener))
    };

    let mut pick = None;
    match opener.and_then(|opener| row_menu(ui, &opener, d, rows, &active, base)) {
        // A menu's choice changes the rows the wheel's indexes were for, so the wheel's clicks and drags of this pass
        // are left alone.
        Some(choice) => {
            let (did, picked) = choose(d, rows, choice, base, shown, segmented, axes);
            changed |= did;
            pick = picked.map(|i| (i, false));
        }
        None => {
            if let Some(out) = wheel {
                // A click with a wheel tool adds a colour there, or removes the colour of the marker clicked; it
                // selects no marker: Add Color selects the new row, and Remove Color keeps the base colour.
                match (out.clicked, tool.as_str()) {
                    (Some(hit), ADD) => {
                        let i = add_color(rows, &added_color(shown, hit.hue, hit.radial, segmented, axes));
                        d.fields.insert("__sel".into(), json!([i]));
                        d.fields.remove(COMPS);
                        changed = true;
                    }
                    (Some(hit), REMOVE) => {
                        let keep = keeps_rows(d);
                        if let Some(b) = hit.marker.and_then(|k| active.get(k)).and_then(|&i| remove_color(rows, i, base, keep)) {
                            removed(d, b);
                            changed = true;
                        }
                    }
                    _ => {
                        // A press with a tool on selects nothing either; the press of a click can be reported after
                        // the click, when the rows it counted from have changed.
                        if let Some(&i) = out.pressed.filter(|_| !matches!(tool.as_str(), ADD | REMOVE)).and_then(|k| active.get(k)) {
                            d.fields.insert("__sel".into(), json!([i]));
                        }
                        changed |= apply_edits(d, rows, &active, out.edits, linked);
                    }
                }
            }
        }
    }

    if d.bool("__art") {
        prominent(ui, d, rows);
    }
    // A colour added or removed above changed the rows, so the sliders and the chips read them afresh.
    let active = active_rows(rows);
    let base = base_row(rows, &active);
    let sel = edit_slider_row(d, rows, &active, base);
    if d.str("sliderMode") == GLOBAL {
        // Global Adjust changes every new colour at once; its rails show the selected colour.
        let sample = sel.and_then(|s| rows.get(s)).and_then(to_color).unwrap_or(Color::rgb(0.8, 0.45, 0.25));
        global_sliders(ui, d, sample);
    } else {
        changed |= sliders(ui, feed, d, rows, sel);
    }
    changed |= chips(ui, d, rows, &active, base, sel);
    (changed, pick)
}

/// The row over the wheel: Harmony Rules, the link of the harmony colours, the New Color Group menu (which files
/// `__saveGroup` for the pump's `save_group`) and the Color Mode of the sliders. Returns whether a rule gave the rows
/// new colours.
fn top_row(ui: &mut Ui, d: &mut Fields, target: &SliderTarget, rows: &mut [Value], active: &[usize], base: Option<usize>) -> bool {
    let words = Env::of(ui).words::<HarmonyWords>();
    let rules: Vec<Entry<Harmony>> = Harmony::ALL.iter().map(|&rule| Entry::new(rule, words.rule_name(rule).clone())).collect();
    let rule = d.str("rule");
    let current = Harmony::ALL.into_iter().find(|r| r.id() == rule).unwrap_or_default();
    let linked = d.bool("linked");
    let all = rows.iter().any(|r| in_group(r) && to_color(r).is_some());
    let any_prominent = d.bool("__art") && prominent_parts(d, rows).iter().any(|p| rows.get(p.0).and_then(to_color).is_some());
    let (mut picked, mut link, mut save) = (None, None, None);
    Row::new().show(ui, |ui, _| {
        picked = Select::new(tl!("Harmony Rules"), Bind::given(current), &rules).show(ui).value().copied();
        let tip = if linked { tl!("Unlink harmony colors") } else { tl!("Link harmony colors") };
        link = ToggleButton::new(tl!("Link harmony colors"), Bind::given(linked))
            .icon(IconId::new("unlink"))
            .on_icon(IconId::new("link"))
            .icon_only()
            .tooltip(tip)
            .show(ui)
            .value()
            .copied();
        let button = IconButton::new(IconId::new("new_folder"))
            .label(tl!("New Color Group"))
            .tooltip(tl!("New Color Group"))
            .popup(PopupKind::Menu)
            .popup_mark(PopupMark::Menu)
            .show(ui);
        save = Menu::new(tl!("New Color Group"))
            .opened_by(&button.response, Trigger::Press)
            .show(ui, |ui, _| {
                let all_ran = MenuItem::new(tl!("Save All Colors")).enabled(all).show(ui).ran;
                let prominent_ran = MenuItem::new(tl!("Save Prominent Colors")).enabled(any_prominent).show(ui).ran;
                if all_ran {
                    Some("all")
                } else if prominent_ran {
                    Some("prominent")
                } else {
                    None
                }
            })
            .inner
            .flatten();
        mode_menu(ui, d, target, true);
    });
    let mut changed = false;
    if let Some(rule) = picked {
        d.fields.insert("rule".into(), json!(rule.id()));
        if let Some((b, c)) = base.and_then(|b| Some((b, rows.get(b).and_then(to_color)?))) {
            apply_rule(rows, rule, b, c, active);
            changed = true;
        }
    }
    if let Some(on) = link {
        d.fields.insert("linked".into(), json!(on));
    }
    if let Some(which) = save {
        d.fields.insert(SAVE_GROUP.into(), json!(which));
    }
    changed
}

/// The row over the wheel's area: the display (smooth wheel, segmented wheel or colour bars), the Add Color and Remove
/// Color tools, and hue with brightness instead of saturation. The tools and the axes act on either wheel, not on the
/// bars; switching the axes changes no colour.
fn tool_row(ui: &mut Ui, d: &mut Fields, display: &'static str, axes: DiscRadius) {
    let on_wheel = display != BARS;
    let tool = d.str(WHEEL_TOOL);
    let bright = axes == DiscRadius::Brightness;
    let displays = DISPLAYS.map(|(id, icon, name)| Entry::new(id, tl!(name)).icon(IconId::new(icon)));
    let (mut shown, mut tools, mut flipped) = (None, [None; 2], None);
    Row::new().show(ui, |ui, _| {
        shown = Segmented::new(tl!("Color display"), Bind::given(display), &displays).show(ui).value().copied();
        // PLAN: the sheet names a `Segmented` for the tools; a segment cannot be turned off again, so each tool is a `ToggleButton` and a second click puts the tool away, as in production.
        for (slot, (id, icon, name)) in tools.iter_mut().zip(TOOLS) {
            *slot = ToggleButton::new(tl!(name), Bind::given(on_wheel && tool == id))
                .icon(IconId::new(icon))
                .icon_only()
                .tooltip(tl!(name))
                .enabled(on_wheel)
                .show(ui)
                .value()
                .copied();
        }
        let tip = if bright { tl!("Show saturation and hue on wheel") } else { tl!("Show brightness and hue on wheel") };
        // The button is named for what a click shows next, as its tooltip says.
        flipped = ToggleButton::new(tip, Bind::given(on_wheel && bright))
            .icon(IconId::new("sun"))
            .icon_only()
            .tooltip(tip)
            .enabled(on_wheel)
            .show(ui)
            .value()
            .copied();
    });
    if let Some(id) = shown {
        d.fields.insert("display".into(), json!(id));
    }
    for ((id, ..), on) in TOOLS.into_iter().zip(tools) {
        if let Some(on) = on {
            d.fields.insert(WHEEL_TOOL.into(), if on { json!(id) } else { Value::Null });
        }
    }
    if let Some(on) = flipped {
        d.fields.insert(WHEEL_AXES.into(), json!(if on { "hueBrightness" } else { "hueSaturation" }));
    }
}

/// Applies the wheel's edits (a marker dragged or keyed, or its second channel's row) to the rows. A gesture moves the
/// rows as they were when it began, kept in [`DRAG`] until it commits or cancels: a new colour read back from its
/// model (CMYK through the colour profile) isn't exactly the colour written, so building on the last pass's rows
/// drifted the linked colours (#1733). A cancel puts the rows back. Returns whether the rows changed.
fn apply_edits(d: &mut Fields, rows: &mut Vec<Value>, active: &[usize], edits: Vec<(usize, EditEvent<[Option<f32>; 3]>)>, linked: bool) -> bool {
    let mut changed = false;
    for (k, event) in edits {
        match event {
            EditEvent::Begin { .. } => {
                d.fields.insert(DRAG.into(), Value::Array(rows.clone()));
            }
            EditEvent::Cancel { .. } => {
                if let Some(Value::Array(start)) = d.fields.remove(DRAG)
                    && start.len() == rows.len()
                {
                    *rows = start;
                    changed = true;
                }
            }
            event => {
                let (Some(&i), Some(set)) = (active.get(k), event.value().copied()) else { continue };
                let start = match d.fields.get(DRAG).and_then(Value::as_array) {
                    Some(start) if start.len() == rows.len() => start.clone(),
                    _ => rows.clone(),
                };
                // What the control didn't set comes from the start too, not from the last pass's colour.
                if let Some(old) = start.get(i).and_then(to_color).map(|c| c.to_hsb()) {
                    let hsb = [0, 1, 2].map(|n| set[n].unwrap_or(old[n]));
                    *rows = start;
                    move_color(rows, i, hsb, linked);
                    changed = true;
                }
                if matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                    d.fields.remove(DRAG);
                }
            }
        }
    }
    changed
}

/// The colour bars: a row per shown colour in row order, its new colour as a bar, the base colour's row marked. A
/// click selects a bar's row for the sliders; a drag moves the row to where it is dropped, keeping every new colour;
/// a right-click asks for the colour's menu. With every row left out a notice says so. Returns whether the rows moved,
/// and the bars' response, which the colour's menu hangs from.
fn bars(ui: &mut Ui, d: &mut Fields, rows: &mut [Value], active: &[usize], base: Option<usize>, sel: Option<usize>) -> (bool, Option<Response>) {
    if active.is_empty() {
        Notice::new(Tone::Neutral, tl!("No colors to recolor (preserved colors are left out).")).show(ui);
        return (false, None);
    }
    let selection = sel.filter(|s| active.contains(s)).map_or_else(Selection::default, Selection::single);
    let shown: &[Value] = rows;
    let out = List::new(tl!("Color bars"))
        .select(SelectMode::Single)
        .max_rows(BAR_ROWS)
        .reorder(ReorderRule::new())
        .selection(Bind::given(selection))
        .show_items(ui, &keyed(active, |&i| i), &mut |key, item| {
            let i = *key;
            let color = shown.get(i).and_then(to_color).map(|c| c32(&c)).unwrap_or_default();
            let mut state = item.row.clone();
            state.emphasized = Some(i) == base;
            ListRow::new(color_name(i)).state(state).thumb_aspect(ThumbSize::Small, ThumbAspect::new(BAR_ASPECT, 1), move |cx, rect| {
                cx.painter.rect_filled(rect, 0.0, color);
            })
        });
    let mut moved = false;
    for event in out.events {
        match event {
            CollectionEvent::Selection { change, .. } => {
                let picked = match change {
                    SelectionChange::Set(keys) => keys.last().copied(),
                    SelectionChange::Toggle(key) => Some(key),
                    SelectionChange::Clear => None,
                    _ => continue,
                };
                d.fields.insert("__sel".into(), json!(picked.into_iter().collect::<Vec<_>>()));
            }
            CollectionEvent::Reorder(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                let Some(from) = active.iter().position(|&i| i == after.key) else { continue };
                // `to` counts the places left once the bar is taken out; `move_bar` counts them among the bars as
                // they are, so a bar dropped after its own place lands one further.
                let to = if after.to >= from { after.to + 1 } else { after.to };
                if let Some(row) = move_bar(rows, active, from, to) {
                    d.fields.insert("__sel".into(), json!([row]));
                    // The slider values held are those of a row index that now holds another row.
                    d.fields.remove(COMPS);
                    moved = true;
                }
            }
            CollectionEvent::ContextMenu(request) => {
                d.fields.insert(MENU_AT.into(), json!({"marker": request.key}));
            }
            _ => {}
        }
    }
    (moved, Some(out.response))
}

/// The menu of a marker or a colour bar, or of a point of the wheel away from the markers (Add New Color, tied to the
/// marker whose spoke it is on), hung from `opener`. It shows while [`MENU_AT`] names its target, which a pick or a
/// close drops. Returns the item picked.
fn row_menu(ui: &mut Ui, opener: &Response, d: &mut Fields, rows: &[Value], active: &[usize], base: Option<usize>) -> Option<MenuChoice> {
    let at = d.fields.get(MENU_AT).cloned()?;
    let row = |k: &str| at[k].as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| active.contains(i));
    let (marker, line) = (row("marker"), row("line"));
    let hs = at["hs"].as_array().and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]));
    let mut open = true;
    let shown = Menu::context(tl!("Color"), opener).open(Bind::bound(&mut open)).show(ui, |ui, _| match marker {
        Some(i) => marker_items(ui, rows, i, base),
        None => {
            let ran = MenuItem::new(tl!("Add New Color")).enabled(hs.is_some()).show(ui).ran;
            hs.filter(|_| ran).map(|hs| MenuChoice::Add(hs, line))
        }
    });
    let choice = shown.inner.flatten();
    if !open || choice.is_some() || shown.output.closed.is_some() {
        d.fields.remove(MENU_AT);
    }
    choice
}

/// The items of row `i`'s menu: Set As Base Color and Remove Color (neither for the base colour `base`), Select Shade
/// and Color Picker.
fn marker_items(ui: &mut Ui, rows: &[Value], i: usize, base: Option<usize>) -> Option<MenuChoice> {
    let mut choice = None;
    if MenuItem::new(tl!("Set As Base Color")).enabled(can_set_base(rows, i)).show(ui).ran {
        choice = Some(MenuChoice::Base(i));
    }
    if let Some(c) = rows.get(i).and_then(to_color) {
        // The row's own output says only whether its submenu is open, which the menu level keeps.
        let _ = Menu::submenu_row(ui, tl!("Select Shade"), |ui, _| {
            if let Some(sv) = shade_grid(ui, &c) {
                choice = Some(MenuChoice::Shade(i, sv));
            }
        });
    }
    if MenuItem::new(tl!("Color Picker")).show(ui).ran {
        choice = Some(MenuChoice::Picker(i));
    }
    if MenuItem::new(tl!("Remove Color")).enabled(Some(i) != base).show(ui).ran {
        choice = Some(MenuChoice::Remove(i));
    }
    choice
}

/// Select Shade's grid of `c`'s hue, saturations across and brightnesses down, the cell nearest `c` marked. Returns
/// the saturation and brightness of the cell picked.
fn shade_grid(ui: &mut Ui, c: &Color) -> Option<[f32; 2]> {
    let [h, s0, v0] = c.to_hsb();
    // Cell `at` is column `at % SHADE_COLS` of row `at / SHADE_COLS`.
    let near_col = (s0 * (SHADE_COLS - 1) as f32).round() as usize;
    let near_row = (((1.0 - v0) * SHADE_ROWS as f32).round() as usize).min(SHADE_ROWS - 1);
    let near = near_row * SHADE_COLS + near_col;
    let cells: Vec<(Label, Srgba)> = (0..SHADE_ROWS * SHADE_COLS)
        .map(|at| {
            let [s, v] = shade_at(at % SHADE_COLS, at / SHADE_COLS);
            let name = format!("{} {}%, {} {}%", tl!("Saturation"), (s * 100.0).round(), tl!("Brightness"), (v * 100.0).round());
            (Label::from(name), srgba(&Color::from_hsb(h, s, v)))
        })
        .collect();
    let out = SwatchTable::new(tl!("Select Shade"), &cells)
        .per_row(SHADE_COLS)
        .marks(move |&at| CellMarks::default().edge(at == near))
        .tips(true)
        .selection(Bind::given(Selection::default()))
        .show_all(ui);
    out.selected.map(|at| shade_at(at % SHADE_COLS, at / SHADE_COLS))
}

/// The Prominent Colors bar: the art's new colours, each as wide as its row's share of the art's uses. The commit of
/// an edge's drag files the parts' widths as the rows' `weights`, which the pump moves the rows' current colours
/// toward.
fn prominent(ui: &mut Ui, d: &mut Fields, rows: &[Value]) {
    let shares = prominent_parts(d, rows);
    let total: f64 = shares.iter().map(|&(_, share)| share).sum();
    let parts: Vec<ProminentPart> = shares
        .iter()
        .map(|&(row, _)| {
            let color = rows.get(row).and_then(|r| to_color(r).or_else(|| from_colors(r).first().copied())).unwrap_or_default();
            ProminentPart { row, color: srgba(&color) }
        })
        .collect();
    // Every share is above 0, so a bar with parts has a total above 0.
    let widths: Vec<f64> = shares.iter().map(|&(_, share)| share / total).collect();
    let edit = ProminentBar::new(tl!("Prominent Colors"), &parts, Bind::given(widths)).show(ui);
    if let Some(widths) = edit.events.committed() {
        d.fields.insert(WEIGHTS.into(), json!(weights(&parts, widths, rows.len())));
    }
}

/// The colour chips: Set Current Color As Base Color, then a swatch per shown colour, the selected one ringed and the
/// base colour marked. A click on a chip selects its row. Returns whether the base colour moved.
fn chips(ui: &mut Ui, d: &mut Fields, rows: &mut [Value], active: &[usize], base: Option<usize>, sel: Option<usize>) -> bool {
    let can = sel.is_some_and(|s| can_set_base(rows, s));
    let shown: Vec<(usize, Srgba, String)> = active
        .iter()
        .filter_map(|&i| {
            let row = rows.get(i)?;
            Some((i, srgba(&to_color(row).unwrap_or_default()), row["to"].as_str().unwrap_or_default().to_owned()))
        })
        .collect();
    let (mut make_base, mut chosen) = (false, None);
    Row::new().wrap(true).show(ui, |ui, _| {
        chosen = None;
        make_base = IconButton::new(IconId::new("target"))
            .label(tl!("Set Current Color As Base Color"))
            .tooltip(tl!("Set Current Color As Base Color"))
            .enabled(can)
            .show(ui)
            .clicked();
        for (i, color, hex) in &shown {
            // Named apart from the colour's bar and marker ("Color N"), which the menus and drags find by that name.
            let chip = Swatch::new(format!("{} {}", tl!("New Color"), i + 1), *color)
                .selected(Some(*i) == sel)
                .tooltip(hex.clone())
                .marks(CellMarks::default().edge(Some(*i) == base));
            if chip.show(ui).clicked {
                chosen = Some(*i);
            }
        }
    });
    if let Some(i) = chosen {
        d.fields.insert("__sel".into(), json!([i]));
    }
    match sel {
        Some(i) if make_base => set_base(rows, i),
        _ => false,
    }
}

/// Files `pick`, one Color Theme Picker pick (`{at: [x, y]}` or `{rect: [x0, y0, x1, y1]}` in document points), in
/// `picks`: after the picks so far with `add` (Shift held), else in their place. At most [`MAX_PICKS`] stay, the oldest
/// dropped first.
pub(crate) fn theme_pick(picks: &mut Vec<Value>, pick: Value, add: bool) {
    if !add {
        picks.clear();
    }
    picks.push(pick);
    let extra = picks.len().saturating_sub(MAX_PICKS);
    picks.drain(..extra);
}

/// The Color Theme Picker's canvas tool, which runs on the document canvas in place of the document tool while the
/// picker is on: a click picks the object under it and a drag the art inside its marquee, whose sides follow the
/// document's axes; Shift adds the pick to the picks so far. It keeps only the gesture under way between passes.
#[derive(Debug, Default)]
pub(crate) struct ThemePickTool {
    /// Where the press went down, in document points; `None` between gestures.
    from: Option<Point>,
    /// Where a drag is now, in document points; `None` until the press moves past the slop, so a release without it
    /// is a click.
    to: Option<Point>,
    /// The picks released in this pass, each with whether Shift adds it, for [`with_theme_pick`] to file.
    picked: Vec<(Value, bool)>,
}

impl CanvasTool for ThemePickTool {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        self.from = Some(space.view.to_doc(pointer.pos));
        self.to = None;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.from.is_some()
            && let Some(space) = cx.space()
        {
            self.to = Some(space.view.to_doc(pointer.pos));
        }
    }

    /// The host calls it only in the pass that receives the button's release, so a second pass of the frame files no
    /// pick again.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some(from), Some(space)) = (self.from.take(), cx.space()) else {
            self.to = None;
            return;
        };
        let at = space.view.to_doc(pointer.pos);
        let pick = match self.to.take() {
            None => json!({"at": [at.x, at.y]}),
            Some(_) => json!({"rect": [from.x, from.y, at.x, at.y]}),
        };
        self.picked.push((pick, pointer.mods.shift));
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.from = None;
        self.to = None;
    }

    // PLAN: the sheet names `Marquee::rect`, a canvas control shown with a `Ui` at the canvas's rect; a tool's `paint` gets only `Draw`, so the drag draws the tool host's marching ants, as the gallery's Marquee tool does.
    fn paint(&self, _: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(space), Some(a), Some(b)) = (cx.space(), self.from, self.to) else { return };
        let corners = [(a.x, a.y), (b.x, a.y), (b.x, b.y), (a.x, b.y)].map(|(x, y)| space.view.to_screen(Point::new(x, y)));
        cx.marching_ants(&corners, true);
    }

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Recolor Artwork dialog's themePicks field")
    }
}

/// Runs `run` with the open Recolor Artwork dialog's [`ThemePickTool`] while its Color Theme Picker is on, then files
/// the picks the tool released into the dialog's `themePicks` and gives the tool back. Returns whether the picker was
/// on: the canvas adapter runs the document tool only when it was not.
pub(crate) fn with_theme_pick(app: &mut VectorcraftApp, run: impl FnOnce(&mut VectorcraftApp, &mut ThemePickTool)) -> bool {
    let Some(dialogs) = app.dialogs() else { return false };
    let taken =
        dialogs.with(|host| host.get_mut::<RecolorDialog>().filter(|d| d.fields.bool(THEME_PICKER)).map(|d| std::mem::take(&mut d.theme_tool)));
    let Some(mut tool) = taken else { return false };
    run(app, &mut tool);
    let picked = std::mem::take(&mut tool.picked);
    dialogs.with(|host| {
        // A dialog closed during `run` takes neither the tool nor its picks.
        let Some(d) = host.get_mut::<RecolorDialog>() else { return };
        if !picked.is_empty() {
            let mut picks = d.fields.fields.get(THEME_PICKS).and_then(Value::as_array).cloned().unwrap_or_default();
            for (pick, add) in picked {
                theme_pick(&mut picks, pick, add);
            }
            d.fields.fields.insert(THEME_PICKS.into(), Value::Array(picks));
        }
        d.theme_tool = tool;
    });
    true
}
