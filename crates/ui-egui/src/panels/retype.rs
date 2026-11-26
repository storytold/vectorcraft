//! Retype panel (Window › Retype): the fonts whose glyphs match the selected outlined text, ranked
//! by how many of its shapes each matches (`type.retypeFonts`), each with a sample
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>,
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/apply-retype-suggested-fonts-to-live-text.html>).
//! Find Matching Fonts lists ten, after reading the installed fonts not loaded yet off the UI
//! thread ([`crate::retype_runs`]), and Find more font matches at the end of the list five more. The
//! list stays after the selection changes, so choosing a font in it sets that font on the live text
//! selected then, as the font menus do. The sample field sets the text the samples show, and the
//! panel menu sets their size; Reset Retype Panel clears the list. `retype.find`, `retype.list`,
//! `retype.preview` and `retype.reset` do the same.

use craft::behavior::{EditEvent, Mixed, keyed};
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, Link};
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::menu::PanelRow;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::style::Variant;
use craft::ui::text::TextField;
use egui::{Rect, Ui};
use serde_json::{Value, json};

use crate::VectorcraftApp;

pub const ID: &str = "retype";

/// The fonts Find Matching Fonts lists.
const FIRST: usize = 10;
/// The fonts Find more font matches adds.
const MORE: usize = 5;
/// The most fonts the list holds (`type.retypeFonts`' largest count).
const MAX: usize = 96;
/// The longest sample text, in characters.
const MAX_TEXT: usize = 100;

/// How large the samples are drawn (the panel menu's three sizes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

/// Each size with the name `retype.preview` takes for it and its thumbnail size.
const SIZES: [(Size, &str, ThumbSize); 3] =
    [(Size::Small, "small", ThumbSize::Small), (Size::Medium, "medium", ThumbSize::Medium), (Size::Large, "large", ThumbSize::Large)];

/// One font in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub family: String,
    pub style: String,
    /// How many of the shapes it matched.
    pub matched: u64,
}

/// What the panel shows, kept while the app runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// A search has run since the panel was last reset.
    pub searched: bool,
    /// The fonts the list shows.
    pub fonts: Vec<Found>,
    /// The fonts the last search found after those, in rank order. Find more font matches moves
    /// the next five of them to the list.
    pub rest: Vec<Found>,
    /// The shapes the last search compared.
    pub shapes: u64,
    /// The last search left out shapes past `type.retype`'s limits (2,048 shapes, 16 nested
    /// groups).
    pub truncated: bool,
    pub size: Size,
    /// The samples' text (empty: a generic sample).
    pub text: String,
}

fn size_name(size: Size) -> &'static str {
    SIZES.iter().find(|s| s.0 == size).map_or("medium", |s| s.1)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut run: Option<(&str, Value)> = None;
    let has_path = vectorcraft_engine::cmd::has_selected_path(&app.session).is_ok();
    // The button searches the installed fonts off the UI thread, as `retype.find` from a menu does.
    if Button::new(tl!("Find Matching Fonts")).variant(Variant::PRIMARY).enabled(has_path).show(ui).clicked()
        && let Err(e) = find_in_background(app, &json!({}))
    {
        app.status(e);
    }
    let typed = TextField::new(tl!("Type your text to preview"), Bind::given(Mixed::Same(app.ui.retype.text.clone())))
        .placeholder(tl!("Type your text to preview"))
        .limit(MAX_TEXT)
        .show(ui);
    if let Some(text) = typed.events.committed() {
        run = Some(("retype.preview", json!({ "text": text })));
    }
    Divider::horizontal().show(ui);
    let view = app.ui.retype.clone();
    if view.searched && view.truncated {
        Text::new(tl!("Shapes past the first 2,048, or more than 16 groups deep, were not compared.")).shade(Shade::Muted).show(ui);
    }
    if !view.searched {
        Text::new(tl!("Select outlined text, then Find Matching Fonts.")).shade(Shade::Muted).show(ui);
    } else if view.fonts.is_empty() {
        Text::new(tl!("No font matches the selected shapes.")).shade(Shade::Muted).show(ui);
    } else {
        let sample = (!view.text.trim().is_empty()).then_some(view.text.as_str());
        let size = SIZES.iter().find(|s| s.0 == view.size).map_or(ThumbSize::Medium, |s| s.2);
        let on_white = app.session.prefs.font_samples_on_white;
        let indexed: Vec<(usize, &Found)> = view.fonts.iter().enumerate().collect();
        let source = keyed(&indexed, |(at, _)| *at);
        // A click activates a row, so the same row chosen again sets its font again.
        let list = List::<usize>::new(tl!("Matching fonts")).select(SelectMode::Single).activate(ActivateOn::Click).show_items(
            ui,
            &source,
            &mut |key, _| {
                let (family, style) = indexed.get(*key).map_or(("", ""), |(_, f)| (f.family.as_str(), f.style.as_str()));
                ListRow::new(format!("{family} {style}")).thumb_aspect(size, ThumbAspect::new(8, 1), move |cx: &ContentPainterCx, rect: Rect| {
                    crate::font_menu::paint_family_sample(cx, rect, family, style, sample, on_white);
                })
            },
        );
        let chosen = list.events.iter().find_map(|e| match e {
            CollectionEvent::Activate(at) => view.fonts.get(*at),
            _ => None,
        });
        if let Some(f) = chosen
            && live_text_selected(app)
        {
            // PLAN: ty.S4 gives `EditEvent::Set(family)`; the framework's `Set` is a struct variant, built with `EditEvent::set`.
            let event = EditEvent::set(Mixed::Mixed, f.family.clone(), f.family.as_str().into(), None);
            crate::font_menu::apply_family(app, event, Some(f.style.clone()));
        }
        if !view.rest.is_empty() && Link::new(tl!("Find more font matches")).show(ui).clicked() {
            run = Some(("retype.find", json!({ "more": true })));
        }
    }
    if let Some((id, p)) = run
        && let Err(e) = app.run(id, p)
    {
        app.status(e);
    }
}

/// Live type is selected, inside a selected group too, or being edited: what choosing a font sets
/// it on, as the font menus do.
fn live_text_selected(app: &VectorcraftApp) -> bool {
    crate::panels::character::text_editing(app).is_some() || !crate::panels::character::selected_stories(app).is_empty()
}

/// The panel menu's name for `size`.
fn size_label(size: Size) -> &'static str {
    match size {
        Size::Small => tl!("Small"),
        Size::Medium => tl!("Medium"),
        Size::Large => tl!("Large"),
    }
}

/// The panel menu's rows: the three sample sizes, then Reset Retype Panel.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let sizes = SIZES.iter().map(|(_, name, _)| PanelRow::Command(CommandRef::with_params("retype.preview", json!({ "size": name }))));
    sizes.chain([PanelRow::Separator, PanelRow::Command(CommandRef::new("retype.reset"))]).collect()
}

/// The titles and checks of the size rows, and Reset enabled while a list shows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for (size, name, _) in SIZES {
        let state = CommandState::new().title(size_label(size)).checked(app.ui.retype.size == size);
        states.set(CommandRef::with_params("retype.preview", json!({ "size": name })), state);
    }
    states.set(CommandRef::new("retype.reset"), CommandState::new().enabled(app.ui.retype.searched));
}

/// The Retype panel's UI commands: `retype.find`, `retype.list`, `retype.preview` and
/// `retype.reset`.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "retype.find" => find(app, p),
        "retype.list" => Ok(list(&app.ui.retype)),
        "retype.preview" => preview(app, p),
        "retype.reset" => {
            // A search Find Matching Fonts started does not fill the list again.
            crate::retype_runs::drop_panel_search(app);
            let v = &mut app.ui.retype;
            (v.searched, v.fonts, v.rest, v.shapes, v.truncated) = (false, vec![], vec![], 0, false);
            Ok(list(&app.ui.retype))
        }
        _ => return None,
    })
}

/// `retype.list`: what the panel shows → `{searched, fonts: [{family, style, matched}], shapes,
/// more, truncated, size, text}`.
fn list(v: &View) -> Value {
    json!({
        "searched": v.searched,
        "fonts": v.fonts.iter().map(|f| json!({"family": f.family, "style": f.style, "matched": f.matched})).collect::<Vec<_>>(),
        "shapes": v.shapes,
        "more": !v.rest.is_empty(),
        "truncated": v.truncated,
        "size": size_name(v.size),
        "text": v.text,
    })
}

/// `retype.find {more?, installed?}`: show the panel and list the first ten fonts matching the
/// selected outlined text (with `installed` false, the loaded fonts only), or with `more` add the
/// next five the last search found, whatever is selected now → what `retype.list` returns. The
/// list stays as it was when the search fails.
fn find(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let more = crate::menus::opt_bool(p, "more")?.unwrap_or(false);
    // A bad installed is an error even when no search runs.
    if p.get("installed").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
        return Err("`installed` must be true, false or null".into());
    }
    if more && app.ui.retype.searched {
        // The panel shows first: a failure leaves the list as it was.
        crate::panel_docking::reveal_panel(app, ID)?;
        let v = &mut app.ui.retype;
        let next: Vec<Found> = v.rest.drain(..v.rest.len().min(MORE)).collect();
        v.fonts.extend(next);
        return Ok(list(&app.ui.retype));
    }
    // A search Find Matching Fonts started in the background does not replace this one's list.
    crate::retype_runs::drop_panel_search(app);
    search(app, &search_params(p))
}

/// Find Matching Fonts as the user asks for it (the panel's button, `retype.find` from a menu or the
/// command palette, `p` its params without `more`): the installed fonts are searched off the UI
/// thread ([`crate::retype_runs`]), then the list is filled.
pub(crate) fn find_in_background(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    // A bad more is an error here too, as `retype.find` has it.
    crate::menus::opt_bool(p, "more")?;
    crate::retype_runs::start(app, "type.retypeFonts", search_params(p), true)
}

/// `type.retypeFonts`' params for a new search with `retype.find`'s params `p`.
fn search_params(p: &Value) -> Value {
    let mut q = json!({ "count": MAX });
    if let Some(installed) = p.get("installed") {
        q["installed"] = installed.clone();
    }
    q
}

/// Shows the panel and lists the first ten fonts `type.retypeFonts` with params `p` finds for the
/// selected outlined text → what `retype.list` returns. The list stays as it was when the search
/// fails.
pub(crate) fn search(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let r = app.session.execute("type.retypeFonts", p).map_err(|e| e.to_string())?;
    crate::panel_docking::reveal_panel(app, ID)?;
    let mut fonts: Vec<Found> = r["fonts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(Found {
                        family: f["family"].as_str()?.to_string(),
                        style: f["style"].as_str()?.to_string(),
                        matched: f["matched"].as_u64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rest = fonts.split_off(fonts.len().min(FIRST));
    let v = &mut app.ui.retype;
    (v.searched, v.fonts, v.rest, v.shapes) = (true, fonts, rest, r["shapes"].as_u64().unwrap_or(0));
    v.truncated = r["truncated"].as_bool().unwrap_or(false);
    Ok(list(&app.ui.retype))
}

/// `retype.preview {size?, text?}`: the samples' size (small, medium or large) and text (at most
/// 100 characters; empty for a generic sample) → what `retype.list` returns. A bad value is an
/// error and changes nothing.
fn preview(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let size = match p.get("size") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(|s| SIZES.iter().find(|x| x.1 == s)).map(|x| x.0).ok_or("`size` must be small, medium or large")?),
    };
    let text = match p.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.chars().count() <= MAX_TEXT => Some(s.clone()),
        Some(_) => return Err("`text` must be a string of at most 100 characters".into()),
    };
    let v = &mut app.ui.retype;
    if let Some(s) = size {
        v.size = s;
    }
    if let Some(t) = text {
        v.text = t;
    }
    Ok(list(&app.ui.retype))
}

#[cfg(test)]
mod tests;
