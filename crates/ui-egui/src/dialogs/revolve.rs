//! The rows of the 3D and Materials effects: Extrude & Bevel, Revolve, Inflate and Rotate, and Extrude & Bevel
//! (Classic), Revolve (Classic) and Rotate (Classic), whose own rows are in `three_d_classic`. The dialog
//! ([`super::three_d`]) and the 3D and Materials panel show the same list; the object and the rotation and light
//! gizmos live on the artboard.

use std::sync::{Arc, RwLock};

use craft::behavior::{DisplayOrder, ItemSource, SelectionChange};
use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::contract::Entry;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamKind, ParamRows};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::LengthUnits;
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::value_drag::SliderScale;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{BEVEL_NAME, BEVEL_PROFILE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE, EXTRUDE, INFLATE, REVOLVE, ROTATION_PRESETS};

use super::form::json_of;
use super::three_d::Data;
use crate::VectorcraftApp;

/// Where the rows are shown: the dialog has the gizmos' companions (Show axis, Reset all); the panel adds Materials.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// The effect dialog.
    Dialog,
    /// The 3D and Materials panel.
    Panel,
}

/// `s` as the 3D effects mean it (a solid's cap, not a stroke's; a bevelled edge, not a join).
pub(super) fn c3(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "3d", s)
}

/// The pages of `effect`'s rows, as (key, title).
pub(crate) fn pages(effect: &str, place: Place) -> Vec<(&'static str, &'static str)> {
    let mut pages = vec![];
    if [EXTRUDE, INFLATE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE].contains(&effect) {
        pages.push(("object", tl!("Object")));
    }
    pages.push(("rotation", tl!("Rotation")));
    if place == Place::Panel && effect != CLASSIC_EXTRUDE {
        pages.push(("materials", tl!("Materials")));
    }
    pages.push(("lighting", tl!("Lighting")));
    pages
}

/// The index in `pages` of the page `fields["__page"]` names; the first when it names none.
fn page_at(fields: &DialogFields, pages: &[(&'static str, &'static str)]) -> usize {
    fields.text("__page").and_then(|p| pages.iter().position(|(k, _)| *k == p)).unwrap_or(0)
}

/// The titles of the page strip, keyed by position.
struct Titles(Vec<String>);

impl DisplayOrder<usize> for Titles {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn key(&self, at: usize) -> usize {
        at
    }
    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }
    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|t| std::borrow::Cow::Borrowed(t.as_str()))
    }
}

impl ItemSource<usize> for Titles {}

/// The row of page tabs; a pick writes `__page`.
fn page_row(pages: Vec<(&'static str, &'static str)>) -> Param {
    Param::custom("__page", "", move |ui, fields, _| {
        let titles = Titles(pages.iter().map(|(_, t)| (*t).to_string()).collect());
        let out = TabStrip::new(tl!("Page")).id_salt("three-d-page").kind(TabKind::Text).shown(page_at(fields, &pages)).show_items(
            ui,
            &titles,
            &mut |k, _| Tab::new(titles.0[*k].clone()).id_salt(("tab", *k)),
        );
        let picked = out.response.events.into_iter().find_map(|e| match e {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.last().copied(),
            _ => None,
        });
        if let Some((key, _)) = picked.and_then(|k| pages.get(k)) {
            fields.set("__page", (*key).to_string());
        }
        ui.response()
    })
    .full_width()
}

/// A field's value as JSON: structured values as stored, scalars by their kind.
fn value_of(fields: &DialogFields, key: &str) -> Option<Value> {
    match fields.get(key)? {
        FieldValue::Json(v) => Some(v.clone()),
        _ => fields.bool(key).map(Value::from).or_else(|| fields.number(key).map(Value::from)).or_else(|| fields.text(key).map(Value::from)),
    }
}

/// The value a row keeps for JSON `value`; none for `null`.
fn field_value(value: &Value) -> Option<FieldValue> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => FieldValue::from(*b),
        Value::Number(n) => FieldValue::from(n.as_f64().unwrap_or_default()),
        Value::String(s) => FieldValue::from(s.clone()),
        other => FieldValue::Json(other.clone()),
    })
}

/// Stores JSON `value` under `key` the way a row keeps it; `null` removes the key.
fn set_value(fields: &mut DialogFields, key: &str, value: &Value) {
    match field_value(value) {
        Some(v) => fields.set(key, v),
        None => {
            fields.remove(key);
        }
    }
}

/// A select over `titles`: the index picked this pass. `at` is the shown entry (none shows a placeholder).
fn pick(ui: &mut egui::Ui, label: &str, titles: &[String], at: Option<usize>, default: Option<usize>) -> Option<usize> {
    let entries: Vec<Entry<usize>> = titles.iter().enumerate().map(|(i, t)| Entry::new(i, t.clone())).collect();
    let mut bind = Bind::given(at.unwrap_or(usize::MAX));
    if let Some(d) = default {
        bind = bind.reset(d);
    }
    Select::new(label.to_owned(), bind, &entries).id_salt(label).show(ui).value().copied()
}

/// A select row of JSON `options` (value, title) for `key`.
fn select_row(key: &str, label: &'static str, options: Vec<(Value, String)>, default: Value) -> Param {
    let key_owned = key.to_owned();
    Param::custom(key, label, move |ui, fields, _| {
        let titles: Vec<String> = options.iter().map(|(_, t)| t.clone()).collect();
        let shown = (!fields.is_mixed(&key_owned)).then(|| value_of(fields, &key_owned).unwrap_or_else(|| default.clone()));
        let at = shown.and_then(|v| options.iter().position(|(o, _)| *o == v));
        let default_at = options.iter().position(|(o, _)| *o == default);
        if let Some((value, _)) = pick(ui, label, &titles, at, default_at).and_then(|i| options.get(i)) {
            set_value(fields, &key_owned, value);
        }
        ui.response()
    })
}

/// The built-in choices of `effect`'s `key`, as a choice row's options.
fn choices(effect: &str, key: &str) -> Vec<(String, craft::ui::runtime::env::Label)> {
    super::effect_params::own_choices(effect, key, false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (value.to_string(), label.into()))
        .collect()
}

/// The numeric and flag rows of one effect, with the defaults it resets to.
struct Rows<'a> {
    defaults: &'a Value,
    unit: Unit,
    bare_points: bool,
}

impl Rows<'_> {
    fn default_number(&self, key: &str) -> f64 {
        self.defaults.get(key).and_then(Value::as_f64).unwrap_or(0.0)
    }

    fn range(&self, key: &str, label: &'static str, range: (f64, f64), decimals: u32, unit: UnitKind) -> Param {
        self.plain(key, label, range, decimals).unit(unit)
    }

    /// A slider over a number shown with no unit after it, such as a count.
    fn plain(&self, key: &str, label: &'static str, (lo, hi): (f64, f64), decimals: u32) -> Param {
        Param::new(key, label, ParamKind::range(lo, hi, self.default_number(key), SliderScale::Linear)).decimals(decimals).resettable()
    }

    fn degrees(&self, key: &str, label: &'static str, range: (f64, f64), decimals: u32) -> Param {
        self.range(key, label, range, decimals, UnitKind::Degrees)
    }

    fn percent(&self, key: &str, label: &'static str, range: (f64, f64)) -> Param {
        self.range(key, label, range, 1, UnitKind::Percent)
    }

    /// A length in points, shown in the general unit.
    fn length(&self, key: &str, label: &'static str, range: (f64, f64)) -> Param {
        // Stored in points; a pixel is a point, as the rulers count it.
        let lengths = LengthUnits::new(UnitKind::Points, crate::canvas::rulers::unit_kind(self.unit), 72.0);
        self.range(key, label, range, 1, UnitKind::Pixels).length(crate::fields::bare_points(lengths, self.bare_points))
    }

    fn flag(&self, key: &str, label: &'static str) -> Param {
        Param::new(key, label, ParamKind::bool(self.defaults.get(key).and_then(Value::as_bool).unwrap_or(false))).resettable()
    }

    fn choice(&self, effect: &str, key: &str, label: &'static str) -> Param {
        let default = self.defaults.get(key).and_then(Value::as_str).map(str::to_owned);
        Param::new(key, label, ParamKind::choice(choices(effect, key), default)).resettable()
    }
}

/// The rows of `data.effect` for `fields["__page"]`: the page tabs and the page's rows.
pub(crate) fn params(data: &Data, fields: &DialogFields, place: Place) -> Vec<Param> {
    let Some(effect) = data.effect.as_deref() else { return vec![] };
    let classic = vectorcraft_effects::is_classic_three_d(effect);
    let defaults = vectorcraft_effects::default_params(effect).unwrap_or(json!({}));
    let rows = Rows { defaults: &defaults, unit: data.unit, bare_points: data.bare_points };
    let pages = pages(effect, place);
    // Map Art takes the place of the tabs until its Done.
    if vectorcraft_effects::takes_map_art(effect) && fields.text("__page") == Some("mapArt") {
        return super::three_d_classic::map_art_params(data, fields, effect);
    }
    let page = pages.get(page_at(fields, &pages)).map_or("rotation", |p| p.0);
    let mut out = vec![];
    match page {
        "object" => out.extend(object_rows(data, &rows, effect, place)),
        "materials" => out.push(super::three_d_materials::row(Arc::new(RwLock::new(data.clone())))),
        "lighting" if classic => out.extend(super::three_d_classic::surface_params(data, fields, effect)),
        "lighting" => out.extend(lighting_rows(data, fields, &rows, place, effect)),
        _ if effect == REVOLVE => out.extend(revolve_rows(&rows, effect, place)),
        _ => out.extend(view_rows(&rows, effect, classic)),
    }
    // In the dialog automation sets every setting Reset all puts back: the page's rows keep the keys they take, the
    // tabs take the others.
    let tabs = match place {
        Place::Dialog => {
            let free: Vec<String> = reset_defaults(effect)
                .into_iter()
                .filter(|(k, v)| !out.iter().any(|row| row.key == *k || field_value(v).is_some_and(|v| row.accepts(k, &v))))
                .map(|(k, _)| k)
                .collect();
            page_row(pages).companions(free)
        }
        Place::Panel => page_row(pages),
    };
    out.insert(0, tabs);
    out
}

/// The Object page of `effect`.
fn object_rows(data: &Data, rows: &Rows, effect: &str, place: Place) -> Vec<Param> {
    match effect {
        CLASSIC_EXTRUDE => return super::three_d_classic::object_params(data, &DialogFields::new(), effect),
        CLASSIC_REVOLVE => return super::three_d_classic::revolve_params(data, &DialogFields::new(), effect),
        _ => {}
    }
    let solid = [Value::Bool(true), Value::Bool(false)];
    let cap = select_row("cap", c3("Cap"), vec![(solid[0].clone(), c3("Solid").into()), (solid[1].clone(), c3("Hollow").into())], json!(true));
    let mut out = vec![
        Param::section(tl!("Object")),
        rows.length("depth", tl!("Depth"), (0.0, 2000.0)),
        rows.degrees("twist", tl!("Twist"), (0.0, 360.0), 1),
        rows.percent("taper", tl!("Taper"), (1.0, 100.0)),
        cap,
    ];
    if effect == INFLATE {
        out.push(rows.percent("volume", tl!("Volume"), (0.0, 100.0)));
        out.push(rows.flag("bothSides", tl!("Both sides")));
        return out;
    }
    // Every option but None takes the bevel's size settings.
    let mut shaped: Vec<String> = choices(effect, "bevel").into_iter().map(|(v, _)| v).filter(|v| v != "none").collect();
    if !shaped.iter().any(|v| v == "custom") {
        shaped.push("custom".into());
    }
    let sized = |p: Param| p.enabled_when_in("bevel", shaped.clone());
    out.push(bevel_heading());
    out.push(bevel_row(data, effect, place));
    out.push(sized(rows.length("bevelWidth", tl!("Width"), (0.0, 1000.0))));
    out.push(sized(rows.length("bevelHeight", tl!("Height"), (0.0, 1000.0))));
    out.push(sized(rows.plain("bevelRepeat", tl!("Repeat"), (1.0, 32.0), 0)));
    out.push(sized(rows.percent("bevelSpace", c3("Space"), (0.0, 90.0))));
    out.push(sized(rows.flag("bevelInside", tl!("Bevel Inside"))));
    out.push(sized(rows.flag("bevelBothSides", tl!("Bevel both sides"))));
    out
}

/// A Button of `label` that reports a press.
fn pressed(ui: &mut egui::Ui, label: &str, enabled: bool, tip: Option<&str>) -> bool {
    let mut button = Button::new(label.to_owned()).enabled(enabled);
    if let Some(tip) = tip {
        button = button.tooltip(tip.to_owned());
    }
    clicked(&button.show(ui))
}

/// The Bevel heading with its Reset button, which puts every bevel setting back to its default and leaves the other
/// settings as they are. The button is off while every bevel setting is at its default.
fn bevel_heading() -> Param {
    let defaults = bevel_defaults();
    let keys: Vec<String> = defaults.keys().cloned().collect();
    Param::custom("__bevelHeading", c3("Bevel"), move |ui, fields, _| {
        let mut reset = false;
        let changed = differs_from(fields, &defaults);
        let shown = crate::panels::section(c3("Bevel"))
            .id_salt("bevel")
            .controls(|ui, _| reset = pressed(ui, tl!("Reset"), changed, None))
            .show(ui, |_, _| {});
        if reset {
            defaults.iter().for_each(|(k, v)| set_value(fields, k, v));
        }
        shown.output.response
    })
    .companions(keys)
    .full_width()
}

/// A Bevel Shape menu entry: (shown label, value, the custom bevel's name and points).
pub(crate) type BevelChoice = (String, Value, Option<(String, Value)>);

/// The entries of the Bevel Shape menu for form `fields`, and how many are built in. The built-in labels are
/// translated; custom bevels show their names as they are. The effect's own custom bevel in `fields` (the 3D and
/// Materials panel gives the first selected effect's settings, unmarked by a mixed selection) is listed unless the
/// file lists it by name with the same points; while another entry has its label, " (document)" is added to it.
pub(crate) fn bevel_options(app: &VectorcraftApp, fields: &serde_json::Map<String, Value>, effect: &str) -> (Vec<BevelChoice>, usize) {
    let mut options: Vec<BevelChoice> = super::effect_params::own_choices(effect, "bevel", false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (tl!(label).to_string(), json!(value), None))
        .collect();
    let builtin = options.len();
    for b in app.session.bevels.bevels() {
        options.push((b.name.clone(), json!("custom"), Some((b.name.clone(), b.profile_json()))));
    }
    let custom = fields.get("bevel").and_then(Value::as_str) == Some("custom");
    let own = fields
        .get(BEVEL_PROFILE)
        .filter(|p| custom && !p.is_null())
        .map(|p| (fields.get(BEVEL_NAME).and_then(Value::as_str).unwrap_or_default().to_string(), p.clone()));
    if let Some((n, p)) = own
        && !options[builtin..].iter().any(|(_, _, c)| c.as_ref().is_some_and(|(cn, cp)| *cn == n && *cp == p))
    {
        let mut shown = if n.is_empty() { tl!("Custom").to_string() } else { n.clone() };
        // Each pass makes the label longer, so this ends once no entry has it.
        while options.iter().any(|(l, _, _)| *l == shown) {
            shown = crate::i18n::fmt(tl!("{name} (document)"), &[("name", &shown)]);
        }
        options.push((shown, json!("custom"), Some((n, p))));
    }
    (options, builtin)
}

/// The Bevel Shape menu of Extrude & Bevel and Extrude & Bevel (Classic) (`effect`): None and the built-in shapes,
/// then the custom bevels of the bevels file, and the effect's own custom bevel when the file does not list it. Picking
/// a custom bevel stores its name and a copy of its points. In the panel, Edit Bevels… opens the bevels file
/// (`__editBevels`, which the panel runs).
pub(crate) fn bevel_row(data: &Data, effect: &str, place: Place) -> Param {
    let options = data.bevels.clone();
    let file = data.bevels_file;
    let default = vectorcraft_effects::default_params(effect).and_then(|p| p.get("bevel").cloned()).unwrap_or(Value::Null);
    Param::custom("bevel", c3("Bevel Shape"), move |ui, fields, _| {
        // Marked by index, not label: a custom bevel's name can match another entry's label.
        let builtin = options.iter().take_while(|(_, _, c)| c.is_none()).count();
        let name = fields.text(BEVEL_NAME).unwrap_or_default().to_owned();
        let points = value_of(fields, BEVEL_PROFILE).filter(|v| !v.is_null());
        let current = if fields.is_mixed("bevel") {
            None
        } else if fields.text("bevel") == Some("custom") {
            options[builtin..]
                .iter()
                .position(|(_, _, c)| c.as_ref().is_some_and(|(n, p)| *n == name && Some(p) == points.as_ref()))
                .map(|i| i + builtin)
        } else {
            let value = fields.text("bevel").map_or_else(|| default.clone(), |t| json!(t));
            options[..builtin].iter().position(|(_, v, _)| *v == value)
        };
        let default_at = options[..builtin].iter().position(|(_, v, _)| *v == default);
        let titles: Vec<String> = options.iter().map(|(l, _, _)| l.clone()).collect();
        // The row's body may run more than once in a pass: it reports the pick and the press, which apply after it.
        let (mut picked, mut edit) = (None, false);
        let row = Row::new().show(ui, |ui, _| {
            picked = pick(ui, c3("Bevel Shape"), &titles, current, default_at);
            edit = place == Place::Panel && pressed(ui, tl!("Edit Bevels…"), file, None);
        });
        if let Some((_, v, c)) = picked.and_then(|i| options.get(i)) {
            set_value(fields, "bevel", v);
            if let Some((n, p)) = c {
                set_value(fields, BEVEL_NAME, &json!(n));
                set_value(fields, BEVEL_PROFILE, p);
            }
        }
        if edit {
            fields.set("__editBevels", true);
        }
        row.output
    })
    .companions([BEVEL_NAME, BEVEL_PROFILE, "__editBevels"])
}

/// Extrude & Bevel's bevel settings (every parameter named `bevel…`) with their defaults.
fn bevel_defaults() -> serde_json::Map<String, Value> {
    match vectorcraft_effects::default_params(EXTRUDE) {
        Some(Value::Object(m)) => m.into_iter().filter(|(k, _)| k.starts_with("bevel")).collect(),
        _ => serde_json::Map::new(),
    }
}

/// Does any setting in `defaults` differ from its default in `fields`? A setting the panel's selected effects hold with
/// different values (mixed) counts as changed.
fn differs_from(fields: &DialogFields, defaults: &serde_json::Map<String, Value>) -> bool {
    defaults.iter().any(|(k, v)| {
        fields.is_mixed(k)
            || match v.as_f64() {
                Some(default) => (fields.number(k).unwrap_or(default) - default).abs() > 1e-6,
                None => value_of(fields, k).is_some_and(|f| f != *v),
            }
    })
}

/// Revolve's Rotation page: shape, view and the advanced settings.
fn revolve_rows(rows: &Rows, effect: &str, place: Place) -> Vec<Param> {
    let mut advanced = vec![rows.plain("segments", tl!("Segments"), (8.0, 128.0), 0)];
    if place == Place::Dialog {
        advanced.push(rows.flag("__showAxis", tl!("Show axis")));
    }
    let keep = rows.flag("expandVisibleOnly", tl!("Keep visible surfaces only when expanding")).tooltip(tl!(
        "Trim covered geometry on Expand Appearance. Live preview keeps the full surface. Transparent paints and surfaces exceeding visibility limits retain all faces."
    ));
    let cap = rows.flag("cap", tl!("Cap")).tooltip(tl!("Close the circular ends of open profiles. Partial sweeps also close their cut sides."));
    let mut out = vec![
        Param::section(tl!("Shape")),
        rows.degrees("angle", tl!("Revolve angle"), (0.0, 360.0), 1),
        rows.choice(effect, "edge", tl!("Axis side")),
        rows.length("offset", tl!("Offset"), (0.0, 100000.0)),
        cap,
        Param::section(tl!("View")),
        rows.degrees("rotationX", tl!("Rotation X"), (-360.0, 360.0), 1),
        rows.degrees("rotationY", tl!("Rotation Y"), (-360.0, 360.0), 1),
        rows.degrees("rotationZ", tl!("Rotation Z"), (-360.0, 360.0), 1),
        rows.degrees("perspectiveAngle", tl!("Perspective"), (0.0, 160.0), 1),
        Param::group(tl!("Advanced")).initially_open(false),
    ];
    out.extend(advanced);
    out.push(keep);
    out
}

/// Rotation presets, rotation and perspective for Extrude & Bevel, Inflate and Rotate, and the classic 3D effects,
/// which have the track cube below the presets.
fn view_rows(rows: &Rows, effect: &str, classic: bool) -> Vec<Param> {
    let _ = effect;
    let mut out = vec![Param::section(tl!("Rotation")), presets_row()];
    if classic {
        out.push(super::track_cube::row());
    }
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        out.push(rows.degrees(key, name, (-180.0, 180.0), 2));
    }
    out.push(rows.degrees("perspectiveAngle", tl!("Perspective"), (0.0, 160.0), 1));
    out
}

/// The rotation presets by name, then Custom for angles none of them has.
fn presets_row() -> Param {
    let names = [
        tl!("Front"),
        tl!("Back"),
        tl!("Left"),
        tl!("Right"),
        tl!("Top"),
        tl!("Bottom"),
        tl!("Off-Axis Front"),
        tl!("Off-Axis Back"),
        tl!("Off-Axis Left"),
        tl!("Off-Axis Right"),
        tl!("Off-Axis Top"),
        tl!("Off-Axis Bottom"),
        tl!("Isometric Left"),
        tl!("Isometric Right"),
        tl!("Isometric Top"),
        tl!("Isometric Bottom"),
    ];
    Param::custom("__rotationPreset", tl!("Presets"), move |ui, fields, _| {
        let angles = ["rotationX", "rotationY", "rotationZ"].map(|k| fields.number(k).unwrap_or(0.0));
        let current = ROTATION_PRESETS.iter().position(|(_, a)| a.iter().zip(angles).all(|(a, b)| (a - b).abs() < 0.005));
        let mut titles: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
        titles.push(tl!("Custom").to_string());
        let at = current.unwrap_or(names.len());
        if let Some((_, angles)) = pick(ui, tl!("Presets"), &titles, Some(at), None).and_then(|i| ROTATION_PRESETS.get(i)) {
            for (key, angle) in ["rotationX", "rotationY", "rotationZ"].into_iter().zip(angles) {
                fields.set(key, *angle);
            }
        }
        ui.response()
    })
    .companions(["rotationX", "rotationY", "rotationZ"])
}

/// The Lighting page of the effects that shade (the classic ones have `three_d_classic::surface_params`).
fn lighting_rows(data: &Data, fields: &DialogFields, rows: &Rows, place: Place, effect: &str) -> Vec<Param> {
    // A mixed Shading leaves the light settings on.
    let shaded = |p: Param| if fields.is_mixed("shade") { p } else { p.enabled_when("shade", true) };
    let mut out = vec![
        rows.flag("shade", tl!("Shading")),
        shaded(lighting_presets_row(effect)),
        shaded(light_editor(data, place, false)),
        shaded(rows.percent("ambient", tl!("Ambient Light"), (0.0, 200.0))),
        shaded(rows.flag("shadow", tl!("Shadows"))),
    ];
    if fields.bool("shadow").unwrap_or(false) || fields.is_mixed("shadow") {
        out.push(shaded(rows.choice(effect, "shadowPosition", tl!("Position"))));
        out.push(shaded(rows.percent("shadowDistance", tl!("Distance from Object"), (0.0, 100.0))));
        out.push(shaded(rows.percent("shadowBounds", tl!("Shadow Bounds"), (10.0, 400.0))));
    }
    out
}

/// The lighting presets by name, then Custom for settings none of them has; a pick writes the preset's patch.
fn lighting_presets_row(effect: &str) -> Param {
    let keys: Vec<String> =
        vectorcraft_effects::default_params(effect).and_then(|p| p.as_object().map(|m| m.keys().cloned().collect())).unwrap_or_default();
    Param::custom("__lightingPreset", tl!("Presets"), move |ui, fields, _| {
        let names = [tl!("Standard"), tl!("Diffuse"), tl!("Top Left"), tl!("Right")];
        let params: serde_json::Map<String, Value> = keys.iter().filter_map(|k| value_of(fields, k).map(|v| (k.clone(), v))).collect();
        let current = vectorcraft_effects::matching_lighting_preset(&Value::Object(params));
        let mut titles: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
        titles.push(tl!("Custom").to_string());
        let at = current.unwrap_or(names.len());
        if let Some(preset) = pick(ui, tl!("Presets"), &titles, Some(at), None).and_then(|i| vectorcraft_effects::LIGHTING_PRESETS.get(i))
            && let Some(patch) = preset.params().as_object()
        {
            patch.iter().for_each(|(k, v)| set_value(fields, k, v));
            fields.set("__light", 0i64);
        }
        ui.response()
    })
    .companions(["__light"])
}

/// The id of the footer's Reset all button.
pub(crate) const RESET_ALL: &str = "resetAll";

/// What Reset all puts back: the effect's defaults but the mapped graphics, Map Art with its options and the render
/// settings, which stay.
fn reset_defaults(effect: &str) -> serde_json::Map<String, Value> {
    let mut kept = vectorcraft_effects::default_params(effect).and_then(|d| d.as_object().cloned()).unwrap_or_default();
    for key in [
        vectorcraft_effects::GRAPHICS,
        vectorcraft_effects::RENDER,
        vectorcraft_effects::MAP_ART,
        vectorcraft_effects::MAP_SHADE,
        vectorcraft_effects::MAP_INVISIBLE,
    ] {
        kept.remove(key);
    }
    kept
}

/// The dialog's Reset all, in its footer beside the page's hint as production placed it, so it stays in view however
/// long the page: off while every setting it puts back is at its default; none on the Map Art page.
pub(crate) fn reset_all_button(effect: &str, fields: &DialogFields) -> Option<(String, FooterButton)> {
    (fields.text("__page") != Some("mapArt")).then(|| {
        let tip =
            if effect == REVOLVE { tl!("Reset all Revolve settings to their defaults.") } else { tl!("Reset all 3D settings to their defaults.") };
        let enabled = differs_from(fields, &reset_defaults(effect));
        (RESET_ALL.to_owned(), FooterButton::new(tl!("Reset all"), DialogRole::Other).enabled(enabled).tooltip(tip))
    })
}

/// Reset all's press: the shape, view and light back to their defaults, the first light selected.
pub(crate) fn reset_all(dialog: &mut HostedParamDialog, effect: &str) {
    let kept = reset_defaults(effect);
    let mut fields = dialog.fields().clone();
    kept.iter().for_each(|(k, v)| set_value(&mut fields, k, v));
    fields.set("__light", 0i64);
    for key in kept.keys().map(String::as_str).chain(["__light"]) {
        match fields.get(key) {
            Some(value) if dialog.fields().get(key) != Some(value) => dialog.set_field(key, value.clone()),
            Some(_) => {}
            None => dialog.clear_field(key),
        }
    }
}

/// Drops from `params` the light settings `effect` doesn't take: the light editor lists all six, and the classic effects
/// take neither a colour nor a softness.
pub(super) fn drop_untaken_light_keys(effect: &str, params: &mut serde_json::Map<String, Value>) {
    for (_, key) in LIGHT_KEYS {
        if let Some(value) = params.get(key)
            && vectorcraft_engine::cmd::params::check_effect_params(
                "effect.apply",
                effect,
                &Value::Object([(key.to_owned(), value.clone())].into_iter().collect()),
            )
            .is_err()
        {
            params.remove(key);
        }
    }
}

const LIGHT_KEYS: [(&str, &str); 6] = [
    ("color", "lightColor"),
    ("intensity", "lightIntensity"),
    ("rotation", "lightRotation"),
    ("height", "lightHeight"),
    ("softness", "lightSoftness"),
    ("behind", "lightBehind"),
];

/// The explicit lights of `fields`, when it has a usable list.
pub(super) fn explicit_lights(fields: &DialogFields) -> Option<Vec<Value>> {
    let list: Vec<Value> = match fields.get("lights")? {
        FieldValue::Json(Value::Array(a)) => a.clone(),
        // A list of objects reads back as a list of `Json` values.
        FieldValue::List(items) => items
            .iter()
            .map(|i| match i {
                FieldValue::Json(v) => v.clone(),
                other => serde_json::to_value(other).unwrap_or(Value::Null),
            })
            .collect(),
        _ => return None,
    };
    (!list.is_empty() && list.len() <= vectorcraft_three_d::MAX_LIGHTS).then_some(list)
}

fn light_index(fields: &DialogFields, count: usize) -> usize {
    fields.int("__light").and_then(|v| usize::try_from(v).ok()).unwrap_or(0).min(count.saturating_sub(1))
}

/// Only the selected light's six settings are overlaid. Selection is UI state and never enters effect parameters, so
/// selecting another light creates no document history.
pub(super) fn selected_light(fields: &DialogFields) -> DialogFields {
    let mut selected = fields.clone();
    selected.remove("lights");
    if let Some(list) = explicit_lights(fields)
        && let Some(light) = list.get(light_index(fields, list.len()))
    {
        let defaults = vectorcraft_effects::default_params(fields.text("__effect").unwrap_or_default()).unwrap_or(json!({}));
        for (key, field) in LIGHT_KEYS {
            if let Some(value) = light.get(key).or_else(|| defaults.get(field)) {
                set_value(&mut selected, field, value);
            }
        }
    }
    selected
}

/// The selected light's options, as the gizmo reads them.
pub(super) fn light_options(fields: &DialogFields) -> vectorcraft_three_d::Revolve {
    vectorcraft_effects::revolve_options(&Value::Object(json_of(&[], &selected_light(fields))))
}

/// The six settings of a light held in `fields`, as a light record.
pub(super) fn light_record(fields: &DialogFields) -> Value {
    Value::Object(LIGHT_KEYS.iter().filter_map(|(key, field)| value_of(fields, field).map(|v| ((*key).into(), v))).collect())
}

/// Writes the selected light's settings from `selected` back into `fields`.
pub(super) fn store_selected(fields: &mut DialogFields, selected: &DialogFields) {
    if let Some(mut list) = explicit_lights(fields) {
        let index = light_index(fields, list.len());
        if let Some(light) = list.get_mut(index) {
            *light = light_record(selected);
        }
        fields.set("lights", FieldValue::Json(Value::Array(list)));
    } else {
        for (_, key) in LIGHT_KEYS {
            if let Some(value) = value_of(selected, key) {
                set_value(fields, key, &value);
            }
        }
    }
}

/// Sets one setting of the selected light.
pub(super) fn set_light_value(fields: &mut DialogFields, key: &str, value: Value) {
    let mut selected = selected_light(fields);
    set_value(&mut selected, key, &value);
    store_selected(fields, &selected);
}

/// The light's colour: a swatch that asks the host's picker; the answer arrives in `__lightPick`.
fn light_color_row() -> Param {
    Param::custom("lightColor", tl!("Color"), |ui, fields, cx| {
        let hex = fields.text("lightColor").unwrap_or("#ffffff").to_owned();
        let color = Srgba::from_hex(&hex).unwrap_or(Srgba::rgb(1.0, 1.0, 1.0));
        // The row's body may run more than once in a pass: the picker opens after it.
        let mut ask = false;
        let row = Row::new().show(ui, |ui, _| ask = pressed(ui, &hex.to_uppercase(), true, Some(tl!("Color"))));
        if ask {
            cx.pick_color("__lightPick", color);
        }
        row.output
    })
    .opens_with("#ffffff")
}

/// The lights: the list with Add and Delete, the sphere (the panel and the classic dialogs; the other dialogs move
/// lights with the rings on the artboard) and the selected light's six settings.
pub(crate) fn light_editor(_data: &Data, place: Place, classic: bool) -> Param {
    let settings = [
        light_color_row(),
        Param::new("lightIntensity", tl!("Intensity"), ParamKind::range(0.0, 100.0, 100.0, SliderScale::Linear))
            .decimals(1)
            .unit(UnitKind::Percent)
            .resettable(),
        Param::new("lightRotation", tl!("Rotation"), ParamKind::range(-180.0, 180.0, 0.0, SliderScale::Linear))
            .decimals(1)
            .unit(UnitKind::Degrees)
            .resettable(),
        Param::new("lightHeight", tl!("Height"), ParamKind::range(0.0, 90.0, 90.0, SliderScale::Linear))
            .decimals(1)
            .unit(UnitKind::Degrees)
            .resettable(),
        Param::new("lightSoftness", tl!("Softness"), ParamKind::range(0.0, 100.0, 0.0, SliderScale::Linear))
            .decimals(1)
            .unit(UnitKind::Percent)
            .resettable(),
        Param::new("lightBehind", tl!("Light behind the object"), ParamKind::bool(false)).resettable(),
    ];
    // The classic effects' lights have no colour.
    let settings: Vec<Param> = settings.into_iter().filter(|p| !(classic && p.key == "lightColor")).collect();
    Param::custom("lights", tl!("Lights"), move |ui, fields, cx| {
        if let Some(Value::String(hex)) = fields.color("__lightPick").map(|c| json!(Srgba::from_rgb_array(c).to_hex())) {
            fields.remove("__lightPick");
            set_light_value(fields, "lightColor", json!(hex));
        }
        super::form::stacked(ui, fields, |ui, fields| {
            let mut list = explicit_lights(fields).unwrap_or_else(|| vec![light_record(fields)]);
            let mut index = light_index(fields, list.len());
            let titles: Vec<String> = (1..=list.len()).map(|i| format!("{} {i}", c3("Light"))).collect();
            if let Some(next) = pick(ui, tl!("Lights"), &titles, Some(index), None) {
                index = next;
            }
            // Add Light is off while a setting of the single light is mixed.
            let mixed = explicit_lights(fields).is_none() && LIGHT_KEYS.iter().any(|(_, key)| fields.is_mixed(key));
            // The row's body may run more than once in a pass: it reports the presses, which apply after it.
            let (mut add, mut delete) = (false, false);
            Row::new().show(ui, |ui, _| {
                add = pressed(ui, tl!("Add Light"), list.len() < vectorcraft_three_d::MAX_LIGHTS && !mixed, None);
                delete = pressed(ui, tl!("Delete Light"), list.len() > 1, None);
            });
            if add {
                let mut added = selected_light(fields);
                let defaults = vectorcraft_effects::default_params(fields.text("__effect").unwrap_or_default()).unwrap_or(json!({}));
                for (_, key) in LIGHT_KEYS {
                    if let Some(value) = defaults.get(key) {
                        set_value(&mut added, key, value);
                    }
                }
                set_value(&mut added, "lightRotation", &json!(0.0));
                set_value(&mut added, "lightHeight", &json!(90.0));
                list.push(light_record(&added));
                index = list.len() - 1;
                fields.set("lights", FieldValue::Json(Value::Array(list.clone())));
            }
            if delete && index < list.len() {
                list.remove(index);
                index = index.min(list.len().saturating_sub(1));
                fields.set("lights", FieldValue::Json(Value::Array(list.clone())));
            }
            fields.set("__light", i64::try_from(index).unwrap_or(0));
            if place == Place::Panel || classic {
                super::revolve_lights::sphere(ui, fields, cx, classic);
            }
            let mut selected = selected_light(fields);
            let before = light_record(&selected);
            let out = ParamRows::new(tl!("Light Settings"), &settings, Bind::bound(&mut selected)).show(ui);
            cx.gesture(&out.events);
            if light_record(&selected) != before {
                store_selected(fields, &selected);
            }
        })
    })
    .companions(LIGHT_KEYS.iter().map(|(_, k)| *k).chain(["__light", "__lightPick"]))
    .on_set(set_light_field)
    .full_width()
}

/// The light editor's set-time rule: a light setting set from outside goes to the selected light, as its rows do.
fn set_light_field(fields: &mut DialogFields, key: &str, value: Value) -> Result<(), String> {
    if LIGHT_KEYS.iter().any(|(_, k)| *k == key) {
        set_light_value(fields, key, value);
    } else {
        set_value(fields, key, &value);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
