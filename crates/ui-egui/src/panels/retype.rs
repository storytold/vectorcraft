//! Retype panel (Window › Retype): the fonts whose glyphs match the selected outlined text, ranked
//! by how many of its shapes each matches (`type.retypeFonts`), each with a sample
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>,
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/apply-retype-suggested-fonts-to-live-text.html>).
//! Find Matching Fonts lists ten, and Find more font matches at the end of the list five more. The
//! list stays after the selection changes, so choosing a font in it sets that font on the live text
//! selected then, as the font menus do. The sample field sets the text the samples show, and the
//! panel menu sets their size; Reset Retype Panel clears the list. `retype.find`, `retype.list`,
//! `retype.preview` and `retype.reset` do the same.

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

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

/// Each size with the name `retype.preview` takes for it and its row height.
const SIZES: [(Size, &str, f32); 3] = [(Size::Small, "small", 22.0), (Size::Medium, "medium", 32.0), (Size::Large, "large", 48.0)];

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

fn row_height(size: Size) -> f32 {
    SIZES.iter().find(|s| s.0 == size).map_or(32.0, |s| s.2)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut run: Option<(&str, Value)> = None;
    let has_path = vectorcraft_engine::cmd::has_selected_path(&app.session).is_ok();
    if widgets::primary_button_enabled(ui, tl!("Find Matching Fonts"), has_path).clicked() {
        run = Some(("retype.find", json!({})));
    }
    ui.add_space(4.0);
    let mut text = app.ui.retype.text.clone();
    let field = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(ui.make_persistent_id("retype-sample"))
            .char_limit(MAX_TEXT)
            .hint_text(egui::RichText::new(tl!("Type your text to preview")).italics())
            .desired_width(ui.available_width()),
    );
    if field.changed() {
        run = Some(("retype.preview", json!({ "text": text })));
    }
    widgets::divider(ui);
    let view = app.ui.retype.clone();
    if view.searched && view.truncated {
        widgets::dim_label(ui, tl!("Shapes past the first 2,048, or more than 16 groups deep, were not compared."));
    }
    if !view.searched {
        widgets::dim_label(ui, tl!("Select outlined text, then Find Matching Fonts."));
    } else if view.fonts.is_empty() {
        widgets::dim_label(ui, tl!("No font matches the selected shapes."));
    } else {
        let live = live_text_selected(app);
        let sample = (!view.text.trim().is_empty()).then_some(view.text.as_str());
        let height = row_height(view.size);
        let on_white = app.session.prefs.font_samples_on_white;
        let mut chosen = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for f in &view.fonts {
                if font_row(ui, f, sample, height, on_white, live) {
                    chosen = Some(f.clone());
                }
            }
            if !view.rest.is_empty() && ui.link(tl!("Find more font matches")).clicked() {
                run = Some(("retype.find", json!({ "more": true })));
            }
        });
        if let Some(f) = chosen {
            crate::font_menu::apply(app, ui.ctx(), crate::font_menu::FontPick::Chosen(f.family, Some(f.style)));
        }
    }
    if let Some((id, p)) = run
        && let Err(e) = app.run(id, p)
    {
        app.status(e);
    }
}

/// A font's row: its name and style over its sample. A click chooses it when `live` text is
/// selected.
fn font_row(ui: &mut Ui, f: &Found, sample: Option<&str>, height: f32, on_white: bool, live: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let name = format!("{} {}", f.family, f.style);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height + 16.0), if live { Sense::click() } else { Sense::hover() });
    widgets::list_item_info(&resp, false, || name.clone());
    if live && resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, t.hover);
    }
    let label = egui::Rect::from_min_size(rect.min + vec2(4.0, 0.0), vec2(rect.width() - 8.0, 14.0));
    ui.painter().text(label.left_center(), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(11.0), t.text_dim);
    let slot = egui::Rect::from_min_size(rect.min + vec2(4.0, 15.0), vec2(rect.width() - 8.0, height));
    crate::font_menu::styled_sample(ui, slot, &f.family, &f.style, sample, on_white);
    live && resp.clicked()
}

/// Live type is selected, inside a selected group too, or being edited: what choosing a font sets
/// it on, as the font menus do.
fn live_text_selected(app: &VectorcraftApp) -> bool {
    crate::panels::character::text_editing(app).is_some() || !crate::panels::character::selected_stories(app).is_empty()
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    for (size, name, _) in SIZES {
        let label = match size {
            Size::Small => tl!("Small"),
            Size::Medium => tl!("Medium"),
            Size::Large => tl!("Large"),
        };
        if menu_item(ui, label, true, app.ui.retype.size == size)
            && let Err(e) = app.run("retype.preview", json!({ "size": name }))
        {
            app.status(e);
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Reset Retype Panel"), app.ui.retype.searched, false)
        && let Err(e) = app.run("retype.reset", json!({}))
    {
        app.status(e);
    }
}

/// The Retype panel's UI commands: `retype.find`, `retype.list`, `retype.preview` and
/// `retype.reset`.
pub(crate) fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "retype.find" => find(app, p),
        "retype.list" => Ok(list(&app.ui.retype)),
        "retype.preview" => preview(app, p),
        "retype.reset" => {
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

/// `retype.find {more?}`: show the panel and list the first ten fonts matching the selected
/// outlined text, or with `more` add the next five the last search found, whatever is selected
/// now → what `retype.list` returns. The list stays as it was when the search fails.
fn find(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let more = crate::menus::opt_bool(p, "more")?.unwrap_or(false);
    if more && app.ui.retype.searched {
        // The panel shows first: a failure leaves the list as it was.
        crate::panel_docking::reveal_panel(app, ID)?;
        let v = &mut app.ui.retype;
        let next: Vec<Found> = v.rest.drain(..v.rest.len().min(MORE)).collect();
        v.fonts.extend(next);
        return Ok(list(&app.ui.retype));
    }
    let r = app.session.execute("type.retypeFonts", &json!({ "count": MAX })).map_err(|e| e.to_string())?;
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
