//! Separations Preview panel: process plates C/M/Y/K plus spot plates with eye toggles (one visible
//! plate renders as grayscale ink coverage; Alt-click an eye to show only that plate, and Alt-click
//! the eye of the plate shown alone to show every plate), and the
//! colour-management controls behind Edit → Color Settings… / Assign Profile… and View → Proof
//! Setup (working spaces, color management policies, intent, black-point compensation, proof
//! target, gamut check).

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource};
use craft::host::{PickRequest, Picked};
use craft::keymap::CommandRef;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Lazy, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
    ToggleColumn, clicked,
};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbSize, ToggleCell};
use craft::ui::menu::PanelRow;
use craft::ui::notices::Notice;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use egui::{Color32, Ui};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, Policy, ProfileKind};
use vectorcraft_render::proof;

use crate::VectorcraftApp;
use crate::file_asks::{Owner, ask};

const PROOF_TARGETS: [(&str, &str); 7] = [
    ("workingCmyk", "Working CMYK"),
    ("legacyMacRgb", "Legacy Macintosh RGB (Gamma 1.8)"),
    ("srgb", "Internet Standard RGB (sRGB)"),
    ("monitorRgb", "Monitor RGB"),
    ("protanopia", "Color blindness – Protanopia-type"),
    ("deuteranopia", "Color blindness – Deuteranopia-type"),
    ("tritanopia", "Color blindness – Tritanopia-type"),
];

/// The eye column of the Plates list.
const EYE: ColumnKey = ColumnKey::new("eye");
/// The column that holds the plate's chip and name.
const NAME: ColumnKey = ColumnKey::new("name");
/// The command of the panel menu's Show All Plates row.
const SHOW_ALL_PLATES: &str = "ui.separations.showAllPlates";
/// Runs command `id` with `p`; a failure goes to the status bar, so it returns `None`.
fn act(app: &mut VectorcraftApp, id: &str, p: Value) -> Option<Value> {
    match app.run(id, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            None
        }
    }
}

/// A check box named `label` over `on`; the value the user set this pass.
fn check(ui: &mut Ui, label: &'static str, on: bool, enabled: bool) -> Option<bool> {
    Checkbox::new(Bind::given(on)).label(label).enabled(enabled).id_salt(label).show(ui).value().copied()
}

/// A select named `name` (shown before it when `caption`) over `items` of `(value, shown name)`; the
/// value the user picked this pass.
fn pick<T: Clone + PartialEq + Send + Sync + 'static>(
    ui: &mut Ui,
    name: &str,
    caption: bool,
    enabled: bool,
    current: T,
    items: Vec<(T, String)>,
) -> Option<T> {
    let entries: Vec<Entry<T>> = items.into_iter().map(|(value, label)| Entry::new(value, label)).collect();
    let label = Label::from(name.to_owned());
    let mut picked = None;
    Row::new().show(ui, |ui, _| {
        if caption {
            Text::new(label.clone()).show(ui);
        }
        picked = Select::new(label.clone(), Bind::given(current.clone()), &entries).enabled(enabled).id_salt(name).show(ui).value().cloned();
    });
    picked
}

/// One row of the Plates list: the CMYK composite or a plate.
struct PlateRow {
    shown: String,
    /// The plate's ink; the composite has none.
    chip: Option<Color32>,
    visible: bool,
}

/// The Plates list's rows: the CMYK composite (key 0), then plate `n` at key `n + 1`.
struct Plates(Vec<PlateRow>);

impl DisplayOrder<usize> for Plates {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|row| Cow::Borrowed(row.shown.as_str()))
    }
}

impl ItemSource<usize> for Plates {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.0.get(at).is_some_and(|row| row.visible)
    }
}

/// The ink `rgb` (0..1 per channel) as a color.
fn chip(rgb: [f32; 3]) -> Color32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

/// The Plates section: the separations preview switch and the plates with their eyes.
fn plates_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let v = proof::view();
    let on = v.separations.is_some();
    if let Some(want) = check(ui, tl!("Overprint Preview (separations)"), on, app.session.active().is_some()) {
        act(app, "view.separationsPreview", json!({"on": want}));
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        Notice::new(Tone::Neutral, tl!("No document open.")).show(ui);
        return;
    };
    let plates = proof::plates(&doc);
    let visible = |n: &str| v.separations.as_ref().is_none_or(|s| s.iter().any(|x| x == n));
    let mut rows = vec![PlateRow { shown: tl!("CMYK").to_string(), chip: None, visible: plates.iter().all(|p| visible(&p.name)) }];
    rows.extend(plates.iter().map(|p| PlateRow {
        shown: if p.spot { crate::i18n::fmt(tl!("{name} (spot)"), &[("name", &p.name)]) } else { p.name.clone() },
        chip: Some(chip(p.rgb)),
        visible: visible(&p.name),
    }));
    let rows = Plates(rows);
    let columns = vec![
        ColumnSpec::toggle(EYE, Label::Empty, ToggleColumn::default()).icon(IconId::new("eye")),
        ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)),
    ];
    // Disabled while the preview is off: the eyes then take no press and the rows read dim.
    let out = List::new(tl!("Plates"))
        .id_salt("plates")
        .select(SelectMode::None)
        .sizing(Sizing::Content)
        .natural_height(true)
        .enabled(on)
        .columns(columns)
        .show_items(ui, &rows, &mut |key, item| {
            let Some(row) = rows.0.get(*key) else { return ListRow::new(Label::Empty) };
            let tooltip =
                if *key == 0 { tl!("Show all plates") } else { tl!("Show/hide plate (Alt-click: only this plate; Alt-click again: all plates)") };
            let mut list_row = ListRow::new(row.shown.clone()).toggle(ToggleCell::eye(EYE, row.visible).tooltip(tooltip));
            if let Some(ink) = row.chip {
                list_row = list_row.thumb(ThumbSize::Small, move |painter, rect| {
                    painter.painter.rect_filled(rect, 2.0, ink);
                });
            }
            list_row.cells(item.cells()).state(item.row.clone())
        });
    for event in out.events {
        // A flip lands once: a click's set, or a sweep's commit at the release. It carries the Alt held at the press.
        let CollectionEvent::Toggle(EditEvent::Set { after: edit, .. } | EditEvent::Commit { after: edit, .. }) = event else { continue };
        let p = match edit.key.checked_sub(1).and_then(|n| plates.get(n)) {
            Some(plate) => eye_click(&plate.name, edit.mods.alt),
            None => json!({"plates": plates.iter().map(|p| p.name.clone()).collect::<Vec<_>>()}),
        };
        act(app, "view.separationsPreview", p);
    }
}

/// The `view.separationsPreview` params for a click on plate `name`'s eye: with Alt, that plate
/// alone, or every plate when it already shows alone; without, the plate shown or hidden.
fn eye_click(name: &str, alt: bool) -> Value {
    if alt { json!({"soloToggle": name}) } else { json!({"toggle": name}) }
}

/// The `kind` profiles after `extra` (a built-in entry, shown in the UI language) as `(name, shown
/// name)`; profile names are shown as they are.
pub(crate) fn profile_entries(kind: ProfileKind, extra: Option<&str>) -> Vec<(String, String)> {
    extra
        .map(str::to_string)
        .into_iter()
        .chain(cms::profiles().into_iter().filter(|p| p.kind == kind).map(|p| p.name))
        .map(|n| {
            let shown = super::label_or_name(&n, extra == Some(n.as_str())).to_string();
            (n, shown)
        })
        .collect()
}

/// The Color Settings section's rows: the RGB and CMYK working spaces, intent, black point
/// compensation, the RGB and CMYK color management policies, the Ask boxes of Profile Mismatches and
/// Missing Profiles, Load Profile… and Gamut Check, and the profile the active document is assigned.
fn settings_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    let st = cms::working_settings();
    for (key, name, kind, current) in [("rgb", tl!("RGB:"), ProfileKind::Rgb, &st.rgb), ("cmyk", tl!("CMYK:"), ProfileKind::Cmyk, &st.cmyk)] {
        if let Some(n) = pick(ui, name, true, true, current.clone(), profile_entries(kind, None)) {
            act(app, "edit.colorSettings", json!({ key: n }));
        }
    }
    let intents = Intent::ALL.iter().enumerate().map(|(i, x)| (i, x.label().to_string())).collect();
    let current = Intent::ALL.iter().position(|i| i.id() == st.intent.id()).unwrap_or_default();
    if let Some(i) = pick(ui, tl!("Intent:"), true, true, current, intents)
        && let Some(intent) = Intent::ALL.get(i)
    {
        act(app, "edit.colorSettings", json!({"intent": intent.id()}));
    }
    if let Some(on) = check(ui, tl!("Use Black Point Compensation"), st.bpc, true) {
        act(app, "edit.colorSettings", json!({"bpc": on}));
    }
    // Color management policies: what opening a document does with a profile it is tagged with
    // that is not the working space.
    let policies: [(&str, &str, Policy, &[Policy]); 2] =
        [("rgbPolicy", tl!("RGB Policy:"), st.rgb_policy, &Policy::RGB), ("cmykPolicy", tl!("CMYK Policy:"), st.cmyk_policy, &Policy::CMYK)];
    for (key, label, current, choices) in policies {
        let items = choices.iter().enumerate().map(|(i, p)| (i, policy_label(*p).to_string())).collect();
        let at = choices.iter().position(|p| p.id() == current.id()).unwrap_or_default();
        if let Some(i) = pick(ui, label, true, true, at, items)
            && let Some(p) = choices.get(i)
        {
            act(app, "edit.colorSettings", json!({ key: p.id() }));
        }
    }
    // Profile Mismatches and Missing Profiles: what opening and pasting ask about.
    let asks: [(&str, &str, &'static str, bool); 3] = [
        ("askMismatchOpen", tl!("Profile Mismatches:"), tl!("Ask When Opening"), st.ask_mismatch_open),
        ("askMismatchPaste", "", tl!("Ask When Pasting"), st.ask_mismatch_paste),
        ("askMissingOpen", tl!("Missing Profiles:"), tl!("Ask When Opening"), st.ask_missing_open),
    ];
    for (key, heading, label, on) in asks {
        if !heading.is_empty() {
            Text::new(heading).show(ui);
        }
        // The two "Ask When Opening" boxes share a label: the key tells them apart.
        let value = Checkbox::new(Bind::given(on)).label(label).id_salt(key).show(ui).value().copied();
        if let Some(on) = value {
            act(app, "edit.colorSettings", json!({ key: on }));
        }
    }
    Row::new().show(ui, |ui, _| {
        if clicked(&Button::new(tl!("Load Profile…")).id_salt("load-profile").show(ui)) {
            load_profile(app);
        }
        if clicked(&Button::new(tl!("Gamut Check")).id_salt("gamut-check").show(ui)) {
            gamut_check(app);
        }
    });
    if let Some(row) = app.session.active().map(|d| AssignRow::of(&d.doc)) {
        assign_rows(app, ui, &row);
    }
}

/// Asks for an ICC profile file, off the UI thread; the answer loads it.
fn load_profile(app: &mut VectorcraftApp) {
    let request = PickRequest::open("profile").filter("ICC Profiles", &["icc", "icm"]);
    // The answer comes later; what `ask` returns now is only the pending marker.
    let _ = ask(app, request, Owner::Free, |app, picked| match picked {
        Picked::Paths(paths) => match paths.first() {
            Some(path) => app.run("color.loadProfile", json!({ "path": path })),
            None => Ok(Value::Null),
        },
        Picked::Contents(files) if !files.is_empty() => Err("loading profiles from disk isn't available in the browser".into()),
        _ => Ok(Value::Null),
    });
}

/// Runs Gamut Check and reports how many colors fall outside the CMYK gamut on the status bar.
fn gamut_check(app: &mut VectorcraftApp) {
    if let Some(r) = act(app, "color.gamutCheck", json!({})) {
        let n = r["outOfGamut"].as_array().map_or(0, Vec::len);
        app.ui.status = gamut_check_status(n);
    }
}

/// Edit › Assign Profile…'s three choices for the active document's color mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssignChoice {
    /// Don't Color Manage This Document: no profile, so the colors show through the working space.
    None,
    /// The working space of the mode, assigned to the document as its profile.
    Working,
    /// Another profile of the mode's kind.
    Profile,
}

/// What the Assign rows show for a document: the profile of its color mode (`key` is the
/// `edit.assignProfile` param), the working space of that mode, and the other profiles of that kind.
#[derive(Debug)]
struct AssignRow {
    key: &'static str,
    mode: vectorcraft_doc::ColorMode,
    assigned: Option<String>,
    working: String,
    others: Vec<String>,
}

impl AssignRow {
    fn of(doc: &vectorcraft_doc::Document) -> Self {
        let (rgb, cmyk) = vectorcraft_engine::cmd::colormgmt::doc_profiles(doc);
        let st = cms::working_settings();
        let (key, kind, assigned, working) = match doc.color_mode {
            vectorcraft_doc::ColorMode::Rgb => ("rgb", ProfileKind::Rgb, rgb, st.rgb),
            vectorcraft_doc::ColorMode::Cmyk => ("cmyk", ProfileKind::Cmyk, cmyk, st.cmyk),
        };
        let working = cms::canonical_name(&working).to_string();
        let others = cms::profiles().into_iter().filter(|p| p.kind == kind && p.name != working).map(|p| p.name).collect();
        AssignRow { key, mode: doc.color_mode, assigned, working, others }
    }

    fn choice(&self) -> AssignChoice {
        match &self.assigned {
            None => AssignChoice::None,
            Some(n) if *n == self.working => AssignChoice::Working,
            Some(_) => AssignChoice::Profile,
        }
    }

    /// The `edit.assignProfile` params for `choice` (with `profile`, the profile picked for
    /// [`AssignChoice::Profile`], else the first of the list), or `None` when the document already
    /// has it.
    fn params(&self, choice: AssignChoice, profile: Option<&str>) -> Option<Value> {
        let v = match choice {
            AssignChoice::None => Value::Null,
            AssignChoice::Working => json!(self.working),
            AssignChoice::Profile => json!(profile.or(self.others.first().map(String::as_str))?),
        };
        (v != json!(self.assigned)).then(|| json!({ self.key: v }))
    }
}

/// The Assign rows: Don't Color Manage This Document, the working space of the document's color
/// mode, and Profile with a list of the other profiles of that kind. A pick runs `edit.assignProfile`.
fn assign_rows(app: &mut VectorcraftApp, ui: &mut Ui, row: &AssignRow) {
    let choice = row.choice();
    let shown = if choice == AssignChoice::Profile { row.assigned.as_deref() } else { row.others.first().map(String::as_str) };
    let working = match row.mode {
        vectorcraft_doc::ColorMode::Rgb => tl!("Working RGB: {name}"),
        vectorcraft_doc::ColorMode::Cmyk => tl!("Working CMYK: {name}"),
    };
    let working = crate::i18n::fmt(working, &[("name", &row.working)]);
    let mut listed = None;
    Text::new(tl!("Assign:")).show(ui);
    let group = RadioGroup::new(tl!("Assign:"), Bind::given(choice)).show(ui, |ui, _| {
        Radio::new(AssignChoice::None).label(tl!("Don't Color Manage This Document")).show(ui);
        Radio::new(AssignChoice::Working).label(working.clone()).show(ui);
        Row::new().show(ui, |ui, _| {
            Radio::new(AssignChoice::Profile).label(tl!("Profile:")).enabled(!row.others.is_empty()).show(ui);
            let items = row.others.iter().map(|n| (n.clone(), n.clone())).collect();
            listed = pick(ui, tl!("Profile:"), false, !row.others.is_empty(), shown.unwrap_or_default().to_string(), items);
        });
    });
    let radio = group.output.value().copied().and_then(|c| row.params(c, shown));
    if let Some(p) = listed.and_then(|n| row.params(AssignChoice::Profile, Some(&n))).or(radio) {
        act(app, "edit.assignProfile", p);
    }
}

/// The name of color management policy `p` in the UI language.
fn policy_label(p: Policy) -> &'static str {
    match p {
        Policy::Off => tl!("Off"),
        Policy::PreserveEmbedded => tl!("Preserve Embedded Profiles"),
        Policy::Convert => tl!("Convert to Working Space"),
        Policy::PreserveNumbers => tl!("Preserve Numbers"),
    }
}

/// The status bar's line after Gamut Check finds `n` colors outside the CMYK gamut.
pub(crate) fn gamut_check_status(n: usize) -> String {
    if n == 0 {
        "All colors are within the CMYK gamut".into()
    } else {
        crate::i18n::msg_n(n as u64, "{n} color is out of the CMYK gamut", "{n} colors are out of the CMYK gamut", &[])
    }
}

/// The Proof Setup section: the target, Proof Colors, Simulate Paper and Overprint Preview.
fn proof_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let v = proof::view();
    let cur = v.setup.target.id();
    let mut targets: Vec<(String, String)> = PROOF_TARGETS.iter().map(|(id, l)| (id.to_string(), tl!(*l).to_string())).collect();
    // A target that isn't one of ours (a loaded profile) shows its name as it is.
    if !targets.iter().any(|(id, _)| *id == cur) {
        targets.push((cur.clone(), cur.clone()));
    }
    if let Some(id) = pick(ui, tl!("Proof Setup"), false, true, cur, targets) {
        act(app, "view.proofSetup", json!({"target": id}));
    }
    Row::new().show(ui, |ui, _| {
        if let Some(on) = check(ui, tl!("Proof Colors"), v.proof_colors, true) {
            act(app, "view.proofColors", json!({"on": on}));
        }
        if let Some(on) = check(ui, tl!("Simulate Paper"), v.setup.simulate_paper, v.setup.target.is_cmyk()) {
            act(app, "view.proofSetup", json!({"simulatePaper": on}));
        }
    });
    if let Some(on) = check(ui, tl!("Overprint Preview"), v.overprint, true) {
        act(app, "view.overprintPreview", json!({"on": on}));
    }
}

/// The panel: Plates, Proof Setup and Color Settings, each a fixed section. Edit › Color Settings… /
/// Assign Profile… ask for the last one to be scrolled into view ([`crate::state::UiState::color_settings_scroll`]):
/// the plates and Proof Setup above it fill a short window (#6419).
// PLAN: the sheet says `SectionStack`, which lets the user reorder sections; the old panel had no
// such thing, so the sections sit in a plain `Stack`.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The flag is read by every pass of the frame, so a second pass reveals as well; only a real
    // pass (not a sizing one) spends it.
    let reveal = app.ui.color_settings_scroll;
    Stack::new().label(tl!("Separations Preview")).fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
        crate::panels::section(tl!("Separations Preview")).show(ui, |ui, _| plates_section(app, ui));
        crate::panels::section(tl!("Proof Setup")).show(ui, |ui, _| proof_section(app, ui));
        // The scroll target is separate from the controls so they are registered for keyboard navigation
        // even before the section has scrolled into view.
        Lazy::new("color-settings", 0.0).reveal(reveal).show(ui, |_| {});
        crate::panels::section(tl!("Color Settings")).show(ui, |ui, _| settings_rows(app, ui));
    });
    if reveal && !ui.is_sizing_pass() {
        app.ui.color_settings_scroll = false;
    }
}

/// The rows of the panel's menu: Overprint Preview and Proof Colors (their checks come from the
/// menu state) and Show All Plates.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    ["view.overprintPreview", "view.proofColors", SHOW_ALL_PLATES].into_iter().map(|id| PanelRow::Command(CommandRef::from(id))).collect()
}

/// Show All Plates is available while the separations preview is on.
pub(crate) fn menu_states(_app: &VectorcraftApp, states: &mut CommandStates) {
    states.set(SHOW_ALL_PLATES, CommandState::new().enabled(proof::view().separations.is_some()));
}

/// Runs the panel's own menu command `id`; `None` for any other id. Show All Plates turns the
/// preview off and on again, which shows every plate.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    (id == SHOW_ALL_PLATES).then(|| {
        app.run("view.separationsPreview", json!({"on": false}))?;
        app.run("view.separationsPreview", json!({"on": true}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    #[test]
    fn browser_profile_pick_reports_why_it_cannot_load() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let picker = crate::tests::picks::scripted(&mut app);
        app.status("Previous status");

        load_profile(&mut app);
        picker.cancel();
        crate::file_asks::poll(&mut app);
        assert_eq!(app.ui.status, "Previous status");

        load_profile(&mut app);
        assert_eq!(picker.requests().len(), 2);
        picker.answer_contents(&[("proof.icc", b"test profile")]);
        crate::file_asks::poll(&mut app);
        assert_eq!(app.ui.status, "loading profiles from disk isn't available in the browser");
    }

    /// The panel's Proof Setup list offers the targets of View › Proof Setup, in the same order and
    /// with the same labels, and each one is a target `view.proofSetup` accepts.
    #[test]
    fn proof_targets_match_the_proof_setup_menu() {
        let app = VectorcraftApp::new(Session::new(), Default::default());
        let menu: Vec<(String, String)> = crate::menus::menu_entries(&app)
            .into_iter()
            .filter(|e| e.path == ["View", "Proof Setup"] && e.command.as_deref() == Some("view.proofSetup"))
            .map(|e| (e.params["target"].as_str().unwrap_or_default().to_string(), e.label))
            .collect();
        let panel: Vec<(String, String)> = PROOF_TARGETS.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
        assert_eq!(menu, panel);
        assert!(panel.iter().any(|(id, _)| id == "tritanopia"));
        assert!(panel.iter().all(|(id, _)| cms::ProofTarget::parse(id).is_some_and(|t| t.id() == *id)));
    }

    /// Assign Profile's three choices: no profile, the working space assigned as the profile, or
    /// another profile; a choice the document already has runs nothing.
    #[test]
    fn assign_choices_give_the_profile_params_of_the_color_mode() {
        let row = |assigned: Option<&str>| AssignRow {
            key: "rgb",
            mode: vectorcraft_doc::ColorMode::Rgb,
            assigned: assigned.map(str::to_string),
            working: "W".into(),
            others: vec!["A".into(), "B".into()],
        };
        assert_eq!(row(None).choice(), AssignChoice::None);
        assert_eq!(row(Some("W")).choice(), AssignChoice::Working);
        assert_eq!(row(Some("B")).choice(), AssignChoice::Profile);
        assert_eq!(row(None).params(AssignChoice::None, None), None);
        assert_eq!(row(Some("B")).params(AssignChoice::None, None), Some(json!({"rgb": null})));
        assert_eq!(row(None).params(AssignChoice::Working, None), Some(json!({"rgb": "W"})));
        assert_eq!(row(Some("W")).params(AssignChoice::Working, None), None);
        assert_eq!(row(None).params(AssignChoice::Profile, None), Some(json!({"rgb": "A"})));
        assert_eq!(row(Some("W")).params(AssignChoice::Profile, Some("B")), Some(json!({"rgb": "B"})));
        assert_eq!(row(Some("B")).params(AssignChoice::Profile, Some("B")), None);
        let none = AssignRow { others: vec![], ..row(None) };
        assert_eq!(none.params(AssignChoice::Profile, None), None, "no other profile to pick");
    }

    /// The Assign rows follow the document's color mode: an RGB document lists the RGB working space
    /// and the RGB profiles and assigns its RGB profile, a CMYK document the CMYK ones, and the other
    /// model's profile stays as it was.
    #[test]
    fn assign_rows_follow_the_document_color_mode() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        for (mode, key, kind, other) in [("rgb", "rgb", ProfileKind::Rgb, "cmyk"), ("cmyk", "cmyk", ProfileKind::Cmyk, "rgb")] {
            app.run("edit.assignProfile", json!({"rgb": null, "cmyk": null})).unwrap();
            app.run("file.documentColorMode", json!({"mode": mode, "convert": false})).unwrap();
            let row = AssignRow::of(&app.session.active().unwrap().doc);
            assert_eq!((row.key, row.choice()), (key, AssignChoice::None));
            assert!(!row.others.is_empty() && !row.others.contains(&row.working));
            assert!(row.others.iter().chain([&row.working]).all(|n| cms::profile(n).is_some_and(|p| p.kind == kind)), "{row:?}");
            app.run("edit.assignProfile", row.params(AssignChoice::Profile, None).unwrap()).unwrap();
            let doc = &app.session.active().unwrap().doc;
            let (rgb, cmyk) = vectorcraft_engine::cmd::colormgmt::doc_profiles(doc);
            let (mine, theirs) = if key == "rgb" { (rgb, cmyk) } else { (cmyk, rgb) };
            assert_eq!((mine.as_deref(), theirs), (row.others.first().map(String::as_str), None), "{other} stays untagged");
        }
    }

    /// #6419: Edit › Assign Profile… and Color Settings… show the panel scrolled to its Color
    /// Settings section, which a window shorter than the panel (a floated panel stops at 320
    /// points) otherwise shows only after scrolling past the plates and Proof Setup. Once: the
    /// request is spent by the pass that read it.
    #[test]
    fn color_settings_and_assign_profile_scroll_the_color_settings_into_view() {
        const BPC: &str = "Use Black Point Compensation";
        const HEIGHT: f32 = 200.0;
        for id in ["edit.assignProfile", "edit.colorSettings"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({})).unwrap();
            let mut panel = Panel::new(app, show);
            panel.room(egui::vec2(300.0, HEIGHT));
            panel.frame();
            // The settings controls start below the view.
            assert!(panel.rect(BPC).top() >= HEIGHT, "the section starts below the view");
            panel.app().run(id, json!({})).unwrap();
            panel.frame();
            assert!(!panel.app().ui.color_settings_scroll, "{id}: the pass that read the request spent it");
            // The scroll glides; let it end.
            panel.advance(1.0);
            panel.frame();
            panel.frame();
            let shown = panel.rect(BPC);
            assert!(panel.in_view(BPC) && shown.top() >= 0.0 && shown.bottom() <= HEIGHT, "{id} scrolls the section into the view: {shown:?}");
            // PLAN: the harness has no scroll driver, so "a later scroll to the top stays there" is
            // covered only as far as the spent request not scrolling again.
            panel.frame();
            assert_eq!(panel.rect(BPC), shown, "{id}: nothing scrolls it again");
        }
    }

    #[test]
    fn a_short_separations_panel_scrolls_to_the_color_settings_actions() {
        use craft::ui::testing::{Fixture, pointer};

        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        let mut fixture = Fixture::new(move |ui| show(&mut app, ui));
        fixture.room(egui::vec2(220.0, 320.0)).settle();
        let at = |fixture: &Fixture, name| fixture.by_label(name).unwrap().rect;
        assert!(at(&fixture, "Load Profile…").top() >= 320.0, "Color Settings starts below the view");
        for _ in 0..6 {
            fixture.frame(vec![pointer::moved(egui::pos2(120.0, 250.0)), pointer::wheel(-300.0)]).settle();
        }
        for name in ["Load Profile…", "Gamut Check"] {
            let rect = at(&fixture, name);
            assert!(rect.top() >= 0.0 && rect.bottom() <= 320.0, "{name} scrolls into the view: {rect:?}");
        }
    }

    #[test]
    fn tab_reaches_color_settings_before_any_manual_scroll() {
        use craft::ui::testing::Fixture;

        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        let mut fixture = Fixture::new(move |ui| show(&mut app, ui));
        fixture.room(egui::vec2(300.0, 120.0)).settle();
        let focused_gamut_check = |fixture: &Fixture| fixture.tree().name_of(fixture.tree().focused_key()) == Some("Gamut Check");
        for _ in 0..32 {
            fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
            if focused_gamut_check(&fixture) {
                break;
            }
        }
        assert!(focused_gamut_check(&fixture));
        let gamut = fixture.by_label("Gamut Check").unwrap().rect;
        assert!(gamut.top() >= 0.0 && gamut.bottom() <= 120.0, "focused action scrolls into the view: {gamut:?}");
    }
}
