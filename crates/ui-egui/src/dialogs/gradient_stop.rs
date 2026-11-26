//! The gradient stop popover: double-clicking a stop on the Gradient tool's annotator (or the
//! Gradient panel's slider) edits it next to its chip — its colour (the Color panel's controls,
//! or a document swatch, which stays linked when it is a global or spot colour), opacity and
//! location. It edits the selected stop (`gradient.selectStop`) through `paint.editGradient`, and
//! closes on Escape or a press elsewhere. On a freeform gradient it edits the selected point
//! (`paint.freeform.selectPoint`) instead: its colour, opacity and spread, through
//! `paint.freeform.setPoint`.
//!
//! The popover is a `StackDialog` on the dialog host. It holds plain data that `pump` pushes each
//! frame (the target's values, the swatches, what the Color panel shows, where it goes) and files what
//! the user did for `pump` to apply, since `show` has no app.
//!
//! Fields: `tab` (`color` or `swatches`). Agents can set `color` (hex), `opacity` and `location` (a
//! point: `spread`) as percentages, and confirm; confirming reports no stop index.

use craft::behavior::IconId;
use craft::color_ui::ui::{ColorPanel, ColorPanelLayout, ColorPanelOutput, ColorPanelShown, ColorPanelState, DropperButton, SwatchPalette};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::notices::Notice;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::text::NumberField;
use egui::{Pos2, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::gradient::move_stop;
use vectorcraft_color::{Color, FreeformPoint, GradientStop, Paint};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::params::color_json;

use crate::VectorcraftApp;
use crate::panels::gradient::{edit_point, edits_points, set_stops, set_stops_keeping_models, shown_points};
use crate::panels::{Live, active_paint, c32};

/// The dialog kind of the gradient stop popover.
pub const KIND: &str = "gradientStop";

/// Where the popover opens: at a screen point (from the Gradient panel's chip) or beside the art, from the
/// chip of a stop on the canvas (a document point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Anchor {
    /// A screen point.
    Screen(Pos2),
    /// A stop's or point's chip on the canvas, in document coordinates.
    Chip(Point),
}

/// What the popover edits: the selected stop of the gradient behind the active proxy (with all
/// its stops), or the selected point of a freeform gradient.
enum Target {
    Stop(Vec<GradientStop>, usize),
    Point(FreeformPoint, usize),
}

impl Target {
    fn color(&self) -> Color {
        match self {
            Target::Stop(stops, i) => stops.get(*i).map_or(Color::BLACK, |s| s.color),
            Target::Point(p, _) => p.color,
        }
    }
    fn opacity(&self) -> f32 {
        match self {
            Target::Stop(stops, i) => stops.get(*i).map_or(1.0, |s| s.opacity),
            Target::Point(p, _) => p.opacity,
        }
    }
    /// The location of a stop or the spread of a point, 0 to 1.
    fn amount(&self) -> f32 {
        match self {
            Target::Stop(stops, i) => stops.get(*i).map_or(0.0, |s| s.offset),
            Target::Point(p, _) => p.spread,
        }
    }
}

fn selected(app: &VectorcraftApp) -> Option<Target> {
    let Paint::Gradient(g) = active_paint(app) else { return None };
    if edits_points(app, &g) {
        let f = shown_points(&g);
        let i = app.session.selected_freeform_point().filter(|i| *i < f.points.len())?;
        return Some(Target::Point(f.points[i], i));
    }
    let i = app.session.selected_stop().filter(|i| *i < g.gradient.stops.len())?;
    Some(Target::Stop(g.gradient.stops, i))
}

/// `stops` with stop `i` given `color`, linked to `link` (a global swatch and tint) or unlinked.
fn recolor(mut stops: Vec<GradientStop>, i: usize, color: Color, link: Option<(String, f32)>) -> Vec<GradientStop> {
    stops[i].set_color(color, link);
    stops
}

/// Solid colours of the document's swatches (groups included), with their names.
fn swatch_colors(app: &VectorcraftApp) -> Vec<(String, Color)> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    d.swatches.iter().chain(d.swatch_groups.iter().flat_map(|g| g.swatches.iter())).filter_map(|s| Some((s.name.clone(), s.paint.color()?))).collect()
}

/// The popover's two views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    /// The Color panel's controls.
    #[default]
    Color,
    /// The document's swatches.
    Swatches,
}

impl Tab {
    /// The tab's name in the `tab` field.
    fn id(self) -> &'static str {
        match self {
            Tab::Color => "color",
            Tab::Swatches => "swatches",
        }
    }
}

/// The fields set from outside the popover or edited in it, as percentages where the field is one; each `None` is
/// a field left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PopoverFields {
    color: Option<Color>,
    opacity: Option<f64>,
    location: Option<f64>,
    spread: Option<f64>,
}

/// The target's values and what the Color panel shows, as `pump` last pushed them.
#[derive(Clone)]
struct Feed {
    /// A freeform point (spread) rather than a stop (location).
    point: bool,
    color: Color,
    /// The colour as the panel and the swatches show it.
    current: Srgba,
    /// Percent.
    opacity: f64,
    /// Percent: the stop's location or the point's spread.
    amount: f64,
    swatches: Vec<(String, Color)>,
    /// `swatches` as the palette draws them, in the same order.
    tiles: Vec<(Label, Srgba)>,
    shown: ColorPanelShown,
}

/// What the user did in the popover since `pump` last took it.
#[derive(Default)]
struct Filed {
    fields: PopoverFields,
    swatch: Option<(String, Color)>,
    eyedropper: bool,
    panels: Vec<ColorPanelOutput>,
}

/// The gradient stop popover.
pub(crate) struct GradientStopPopover {
    /// Where it opened.
    pub(crate) anchor: Anchor,
    /// The view it shows.
    pub(crate) tab: Tab,
    /// The screen point it hangs from, as `pump` last placed it.
    at: Pos2,
    panel: ColorPanelState,
    feed: Option<Feed>,
    /// The fields set from outside, applied at confirm.
    set: PopoverFields,
    filed: Filed,
}

impl GradientStopPopover {
    fn new(anchor: Anchor) -> Self {
        Self {
            anchor,
            tab: Tab::default(),
            at: Pos2::ZERO,
            panel: ColorPanelState::default(),
            feed: None,
            set: PopoverFields::default(),
            filed: Filed::default(),
        }
    }

    fn take_filed(&mut self) -> Filed {
        std::mem::take(&mut self.filed)
    }
}

fn rejects(field: &str, value: &Value) -> String {
    FieldRefusal::Rejects(value).message(field)
}

impl StackDialog for GradientStopPopover {
    type Result = PopoverFields;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<PopoverFields>> {
        let Self { feed: Some(feed), tab, panel, filed, at, set, .. } = self else { return None };
        let mut open = true;
        let shown = Popover::new("Gradient stop", Bind::bound(&mut open))
            .at_point(*at)
            .close_on_enter(true)
            .show(ui, |ui, _| body(ui, feed, tab, panel, filed));
        match shown.output.closed {
            Some(CloseReason::Enter) => Some(DialogResult::Ok(set.clone())),
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({ "tab": self.tab.id() });
        if let Some(feed) = &self.feed {
            let (key, set) = if feed.point { ("spread", self.set.spread) } else { ("location", self.set.location) };
            fields["color"] = json!(self.set.color.unwrap_or(feed.color).to_hex());
            fields["opacity"] = json!(self.set.opacity.unwrap_or(feed.opacity));
            fields[key] = json!(set.unwrap_or(feed.amount));
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let number = || value.as_f64().ok_or_else(|| rejects(field, &value));
        match field {
            "tab" => {
                self.tab = match value.as_str() {
                    Some("color") => Tab::Color,
                    Some("swatches") => Tab::Swatches,
                    _ => return Err(rejects(field, &value)),
                }
            }
            "color" => {
                let hex = value.as_str().ok_or_else(|| rejects(field, &value))?;
                // Hex digits only: `Color::from_hex` also takes a few colour names.
                let color = Srgba::from_hex(hex).and_then(|_| Color::from_hex(hex));
                self.set.color = Some(color.ok_or_else(|| format!("bad colour `{hex}` (#rrggbb)"))?);
            }
            "opacity" => self.set.opacity = Some(number()?),
            "location" => self.set.location = Some(number()?),
            "spread" => self.set.spread = Some(number()?),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    /// OK (agents): the fields that were set, which `pump` applies; an untouched popover confirms to no change.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<PopoverFields> {
        Some(self.set.clone())
    }
}

/// The popover's body: the tabs and the eyedropper, the Color panel or the swatches, then opacity and location (a
/// point: spread). What the user edits is filed in `filed`.
fn body(ui: &mut Ui, feed: &Feed, tab: &mut Tab, panel: &mut ColorPanelState, filed: &mut Filed) {
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let tabs = [
                Entry::new(Tab::Color, tl!("Color").to_owned()).icon(IconId::new("palette")),
                Entry::new(Tab::Swatches, tl!("Swatches").to_owned()).icon(IconId::new("swatch-book")),
            ];
            Segmented::new("View", Bind::bound(&mut *tab), &tabs).show(ui);
            // A stop's colour can come from the art, as from the Gradient panel's eyedropper.
            if !feed.point {
                let tip = tl!("Eyedropper: click the art to sample the stop's colour").to_owned();
                filed.eyedropper |= DropperButton::new(tip, Bind::given(false)).show(ui).edit.value() == Some(&true);
            }
        });
        match *tab {
            // The Color panel edits the selected stop (or point) of the active gradient.
            Tab::Color => {
                filed.panels.push(
                    ColorPanel::new(Bind::bound(&mut *panel), [feed.current; 2])
                        .layout(ColorPanelLayout::FillStroke)
                        .shown(feed.shown.clone())
                        .show(ui),
                );
            }
            Tab::Swatches => swatches(ui, feed, filed),
        }
        Row::new().show(ui, |ui, _| {
            filed.fields.opacity = percent_field(ui, tl!("Opacity").to_owned(), feed.opacity).or(filed.fields.opacity);
            let (label, amount) = if feed.point { (tl!("Spread"), &mut filed.fields.spread) } else { (tl!("Location"), &mut filed.fields.location) };
            *amount = percent_field(ui, label.to_owned(), feed.amount).or(*amount);
        });
    });
}

/// Document swatches as tiles: a pick gives the stop (or point) that colour.
fn swatches(ui: &mut Ui, feed: &Feed, filed: &mut Filed) {
    if feed.tiles.is_empty() {
        Notice::new(Tone::Info, tl!("The document has no colour swatches.").to_owned()).show(ui);
        return;
    }
    let shown = SwatchPalette::new("Swatches", Bind::given(feed.current), &feed.tiles).show(ui);
    let Some(picked) = shown.edit.as_ref().and_then(|edit| edit.value()) else { return };
    // The first swatch of that colour: the palette reports colours, not cells.
    let at = feed.tiles.iter().position(|(_, c)| c.to_rgba() == picked.to_rgba());
    filed.swatch = at.and_then(|i| feed.swatches.get(i)).cloned();
}

/// A percent field named `label` over `value`; the number the user entered in this pass.
fn percent_field(ui: &mut Ui, label: String, value: f64) -> Option<f64> {
    let field = Field::new(label.clone()).show(ui, |ui, _| {
        NumberField::new(label.clone(), Bind::given(Some(value))).unit_kind(UnitKind::Percent).range(0.0..=100.0).step(1.0).show(ui)
    });
    field.inner?.value().copied().flatten()
}

/// The popover's size before it has been drawn, for placing it.
const POPOVER_SIZE: egui::Vec2 = vec2(272.0, 360.0);

/// The selected art's box on the screen, `quad` taking a document box to its corners on the screen.
fn selection_on_screen(app: &VectorcraftApp, quad: impl Fn(Rect) -> [Pos2; 4]) -> Option<egui::Rect> {
    let st = app.session.active()?;
    let b = st.doc.bounds_of(&st.selection.objects, false)?;
    Some(egui::Rect::from_points(&quad(b)))
}

/// Where the popover opened from a stop's chip at `chip` goes: beside the selected `art` on the
/// first side of it with room for a popover of `size` in `room` (right, left, below, above), so
/// the art it recolours stays in view (#862); else beside the chip.
fn beside_art(chip: egui::Pos2, art: egui::Rect, size: egui::Vec2, room: egui::Rect) -> egui::Pos2 {
    const GAP: f32 = 12.0;
    let x = (chip.x - size.x / 2.0).clamp(room.left(), (room.right() - size.x).max(room.left()));
    let y = (chip.y - size.y / 2.0).clamp(room.top(), (room.bottom() - size.y).max(room.top()));
    [
        egui::pos2(art.right() + GAP, y),
        egui::pos2(art.left() - GAP - size.x, y),
        egui::pos2(x, art.bottom() + GAP),
        egui::pos2(x, art.top() - GAP - size.y),
    ]
    .into_iter()
    .find(|p| room.contains_rect(egui::Rect::from_min_size(*p, size)))
    .unwrap_or(chip + vec2(14.0, 14.0))
}

/// The screen point the popover hangs from.
fn place(app: &VectorcraftApp, ctx: &egui::Context, anchor: Anchor) -> Pos2 {
    match anchor {
        Anchor::Screen(at) => at,
        // PLAN: assumed `CanvasView::{to_screen(Point) -> Pos2, quad(Rect) -> [Pos2; 4]}` and its `rect` field (cv.S4 gives names only).
        Anchor::Chip(chip) => match app.canvas_view() {
            Some(view) => {
                let at = view.to_screen(chip);
                selection_on_screen(app, |b| view.quad(b)).map_or(at + vec2(14.0, 14.0), |art| beside_art(at, art, POPOVER_SIZE, view.rect))
            }
            None => ctx.content_rect().center(),
        },
    }
}

/// Pushes `target`'s values, the swatches, what the Color panel shows and the place into the open popover `id`.
fn refresh(app: &VectorcraftApp, ctx: &egui::Context, id: DialogId, target: &Target) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(anchor) = dialogs.with(|host| host.get_by_id::<GradientStopPopover>(id).map(|popover| popover.anchor)) else { return };
    let shown = crate::panels::color::shown(app, &crate::panels::color::target_now(app));
    let swatches = swatch_colors(app);
    let feed = Feed {
        point: matches!(target, Target::Point(..)),
        color: target.color(),
        current: Srgba::from(c32(&target.color())),
        opacity: f64::from(target.opacity()) * 100.0,
        amount: f64::from(target.amount()) * 100.0,
        tiles: swatches.iter().map(|(name, c)| (Label::from(name.clone()), Srgba::from(c32(c)))).collect(),
        swatches,
        shown,
    };
    let at = place(app, ctx, anchor);
    dialogs.with(|host| {
        if let Some(popover) = host.get_mut_by_id::<GradientStopPopover>(id) {
            (popover.at, popover.feed) = (at, Some(feed));
        }
    });
}

/// Opens the popover on the selected stop (a freeform gradient's selected point) of the active gradient, the stop
/// at `index` when it is not the selected one. Nothing opens without a window or a stop to edit.
pub(crate) fn open_at(app: &mut VectorcraftApp, index: usize, anchor: Anchor) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let Some(dialogs) = app.dialogs() else { return };
    if let Paint::Gradient(g) = active_paint(app)
        && !edits_points(app, &g)
        && app.session.selected_stop() != Some(index)
    {
        // An index past the last stop selects nothing, and `selected` then finds nothing to edit.
        app.run("gradient.selectStop", json!({ "index": index })).ok();
    }
    let Some(target) = selected(app) else { return };
    dialogs.with(|host| {
        host.open(GradientStopPopover::new(anchor));
    });
    refresh(app, &ctx, dialog_id(KIND), &target);
}

/// Once per frame: applies what the user did in the popover and its confirmed answer, closes it when its stop or
/// point goes away, and pushes the target's values for the next frame. One line of `dialogs::host::pump`.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    match dialogs.with(|host| host.take_by_id::<GradientStopPopover>(id)) {
        Some(DialogResult::Ok(fields)) => {
            if fields != PopoverFields::default()
                && let Some(target) = selected(app)
            {
                apply(app, &target, &fields);
            }
            return;
        }
        Some(_) => return,
        None => {}
    }
    if !dialogs.with(|host| host.is_open::<GradientStopPopover>()) {
        return;
    }
    let Some(target) = selected(app) else {
        // The stop or point went away (undo, another object selected): nothing left to edit.
        dialogs.with(|host| host.cancel_by_id(ctx, id));
        return;
    };
    let filed = dialogs.with(|host| host.get_mut_by_id::<GradientStopPopover>(id).map(GradientStopPopover::take_filed)).unwrap_or_default();
    if filed.fields != PopoverFields::default() {
        apply(app, &target, &filed.fields);
    }
    if let Some((name, color)) = filed.swatch
        && let Some(target) = selected(app)
    {
        pick_swatch(app, &target, &name, color);
    }
    for output in &filed.panels {
        // PLAN: assumed `panels::color::apply_output(app, &ColorPanelOutput)` (color-w9's decision).
        crate::panels::color::apply_output(app, output);
    }
    if filed.eyedropper {
        dialogs.with(|host| host.cancel_by_id(ctx, id));
        crate::panels::gradient::stop_eyedropper(app);
        return;
    }
    if let Some(target) = selected(app) {
        refresh(app, ctx, id, &target);
    }
}

/// Gives the target the swatch `name`'s colour. A global or spot color (or a tint of one) stays linked, and the
/// swatch's color keeps its model.
fn pick_swatch(app: &mut VectorcraftApp, target: &Target, name: &str, color: Color) {
    match target {
        Target::Stop(stops, i) => {
            let link = crate::panels::gradient::dropped_link(app, &json!({ "swatch": name }));
            set_stops_keeping_models(app, &recolor(stops.clone(), *i, color, link), Some(*i), Live::Released);
        }
        Target::Point(_, i) => edit_point(app, "paint.freeform.setPoint", json!({ "index": i, "color": color_json(&color), "keepModel": true })),
    }
}

/// Apply the `color`, `opacity` and `location` or `spread` (percentages) set in `f` to the target as one edit.
fn apply(app: &mut VectorcraftApp, target: &Target, f: &PopoverFields) {
    let pct = |v: Option<f64>| v.map(|v| (v / 100.0).clamp(0.0, 1.0));
    match target {
        Target::Stop(stops, i) => {
            let (mut stops, mut i) = (stops.clone(), *i);
            if let Some(c) = f.color {
                stops = recolor(stops, i, c, None);
            }
            if let Some(o) = pct(f.opacity)
                && let Some(stop) = stops.get_mut(i)
            {
                stop.opacity = o as f32;
            }
            if let Some(l) = f.location {
                (stops, i) = move_stop(&stops, i, (l / 100.0) as f32);
            }
            set_stops(app, &stops, Some(i), Live::Released);
        }
        Target::Point(_, i) => {
            let mut p = json!({ "index": i });
            if let Some(c) = f.color {
                p["color"] = color_json(&c);
            }
            for (k, v) in [("opacity", pct(f.opacity)), ("spread", pct(f.spread))] {
                if let Some(v) = v {
                    p[k] = json!(v);
                }
            }
            edit_point(app, "paint.freeform.setPoint", p);
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::dialogs::host;

    /// A rectangle with a three-stop gradient from (100, 150) to (200, 150), the Gradient tool
    /// and stop 1 selected, in an app with the window context that hosts the popover.
    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 0.5, "color": "#00ff00"}, {"offset": 1, "color": "#000000"}]);
        app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [100, 150], "end": [200, 150]}})).unwrap();
        app.select_tool("gradient");
        app.run("gradient.selectStop", json!({"index": 1})).unwrap();
        app
    }

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().unwrap()
    }

    fn open_popover(app: &mut VectorcraftApp) {
        open_at(app, 1, Anchor::Screen(egui::pos2(200.0, 200.0)));
        assert!(is_open(app), "the popover opened");
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|host| host.is_open::<GradientStopPopover>())
    }

    fn set(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        app.dialogs().unwrap().with(|host| host.set_json_field(&ctx(app), dialog_id(KIND), field, value))
    }

    /// OK, then the frame that applies the answer.
    fn confirm(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, dialog_id(KIND)));
        host::pump(app, &ctx);
    }

    /// #862: the popover goes beside the art, on the first side with room, not over it.
    #[test]
    fn the_popover_goes_beside_the_art_it_edits() {
        let room = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let size = egui::vec2(272.0, 360.0);
        let place = |art: egui::Rect| egui::Rect::from_min_size(beside_art(art.center(), art, size, room), size);
        // Room on the right.
        let art = egui::Rect::from_min_max(egui::pos2(300.0, 300.0), egui::pos2(500.0, 450.0));
        assert!(place(art).left() > art.right() && room.contains_rect(place(art)));
        // Against the right edge: on the left.
        let art = egui::Rect::from_min_max(egui::pos2(900.0, 300.0), egui::pos2(1150.0, 450.0));
        assert!(place(art).right() < art.left());
        // As wide as the room: below it, or above it near the bottom.
        let art = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(1100.0, 300.0));
        assert!(place(art).top() > art.bottom());
        let art = egui::Rect::from_min_max(egui::pos2(100.0, 420.0), egui::pos2(1100.0, 780.0));
        assert!(place(art).bottom() < art.top());
        // Nowhere to go: beside the chip, as before.
        let art = room.shrink(10.0);
        assert_eq!(beside_art(art.center(), art, size, room), art.center() + egui::vec2(14.0, 14.0));
    }

    fn stops(app: &VectorcraftApp) -> Vec<GradientStop> {
        match selected(app) {
            Some(Target::Stop(stops, _)) => stops,
            _ => panic!("no stop selected"),
        }
    }

    #[test]
    fn confirm_applies_colour_opacity_and_location() {
        let mut app = app();
        open_popover(&mut app);
        for (field, value) in [("color", json!("#ff0000")), ("opacity", json!(40)), ("location", json!(80))] {
            set(&app, field, value).unwrap();
        }
        confirm(&mut app);
        let s = stops(&app);
        assert_eq!((s[1].color.to_hex(), s[1].opacity, (s[1].offset * 100.0).round()), ("#ff0000".to_string(), 0.4, 80.0));
        assert!(!is_open(&app));
        // Moving it past the last stop keeps it selected at its new index.
        open_popover(&mut app);
        set(&app, "location", json!(100)).unwrap();
        confirm(&mut app);
        assert_eq!((app.session.selected_stop(), stops(&app)[2].color.to_hex()), (Some(2), "#ff0000".to_string()));
        // A colour that is not `#rrggbb` is refused when it is set.
        open_at(&mut app, 2, Anchor::Screen(egui::pos2(200.0, 200.0)));
        assert!(set(&app, "color", json!("red")).is_err());
    }

    #[test]
    fn on_a_freeform_gradient_it_edits_the_selected_point() {
        let mut app = app();
        app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
        app.run("paint.freeform.addPoint", json!({"at": [150, 150], "color": "#0000ff"})).unwrap();
        let point = |app: &VectorcraftApp| match selected(app) {
            Some(Target::Point(p, i)) => (p, i),
            _ => panic!("no point selected"),
        };
        let (p, i) = point(&app);
        open_at(&mut app, i, Anchor::Chip(p.at));
        assert!(is_open(&app), "the popover opened");
        // Agents set its colour, opacity and spread.
        for (field, value) in [("color", json!("#ff0000")), ("opacity", json!(40)), ("spread", json!(25))] {
            set(&app, field, value).unwrap();
        }
        confirm(&mut app);
        let (p, _) = point(&app);
        assert_eq!((p.color.to_hex(), p.opacity, p.spread), ("#ff0000".to_string(), 0.4, 0.25));
        assert!(!is_open(&app));
    }

    #[test]
    fn the_popover_closes_when_its_stop_goes_away() {
        let mut app = app();
        open_popover(&mut app);
        app.run("gradient.selectStop", json!({"index": null})).unwrap();
        let ctx = ctx(&app);
        host::pump(&mut app, &ctx);
        assert!(!is_open(&app));
    }
}
