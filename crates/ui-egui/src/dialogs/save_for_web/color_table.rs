//! The Save for Web colour table model: the edits the table's buttons and menu make, applied to the
//! engine's `colorTable` value (`webexport::ColorTable`), and the marks the table shows from it.
//!
//! The value is user input (automation sets it), so every read skips what is not a colour.
//! Entries compare as the colours they name, however each is written (`#F00`, `#ff0000`), as the
//! engine reads them; the edits write lowercase `#rrggbb`.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::webexport;
use vectorcraft_render::encode::web::ColorSort;

use craft::ui::patterns::form_dialogs::{ColorTableEdit, TableMarks, TableOrder};

/// `c` as lowercase `#rrggbb`.
fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The colour `s` names, read as the engine reads the table's colours: `None` for text that is not
/// a colour and for a spelling with alpha (four or eight hex digits).
fn parse(s: &str) -> Option<[u8; 3]> {
    let t = s.trim();
    let digits = t.strip_prefix('#').unwrap_or(t);
    if matches!(digits.len(), 4 | 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let [r, g, b, _] = vectorcraft_color::Color::from_hex(t)?.to_rgba8(1.0);
    Some([r, g, b])
}

/// Whether `a` and `b` name one colour, however each is written; text that names no colour matches
/// only the same text in any case.
fn same(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// Whether `list` holds an entry that names `color`.
fn holds(list: &[String], color: &str) -> bool {
    list.iter().any(|c| same(c, color))
}

/// The strings of the table's list `key`, lowercase.
fn list(table: &Value, key: &str) -> Vec<String> {
    table.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect()
}

/// The table's `shifted` map.
fn shifts(table: &Value) -> Map<String, Value> {
    table.get("shifted").and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Set one key of the table, which becomes an object when it is not one.
fn set_table(table: &mut Value, key: &str, value: Value) {
    if !table.is_object() {
        *table = json!({});
    }
    table[key] = value;
}

/// Put `colors` into the table's list `key` when any of them is missing, take them all out when
/// every one is in: one button press on a selection reads the same on every colour.
fn toggle(table: &mut Value, key: &str, colors: &[String]) {
    if colors.is_empty() {
        return;
    }
    let mut v = list(table, key);
    if colors.iter().all(|c| holds(&v, c)) {
        v.retain(|c| !holds(colors, c));
    } else {
        let missing: Vec<String> = colors.iter().filter(|c| !holds(&v, c)).cloned().collect();
        v.extend(missing);
    }
    set_table(table, key, json!(v));
}

/// Take `colors` out of the table's list `key` (every entry that names one of them); the list is
/// left alone when none is in it.
fn remove(table: &mut Value, key: &str, colors: &[String]) {
    let mut v = list(table, key);
    let before = v.len();
    v.retain(|c| !holds(colors, c));
    if v.len() != before {
        set_table(table, key, json!(v));
    }
}

/// Take the shift by hand of the source colour `source` away: every key that names that colour.
fn unshift(table: &mut Value, source: &str) {
    let mut map = shifts(table);
    let before = map.len();
    map.retain(|k, _| !same(k, source));
    if map.len() != before {
        set_table(table, "shifted", Value::Object(map));
    }
}

/// Apply `event` to the table. `shown` are the colours the table shows, by source: a selected
/// colour it lacks (the selection outlives a re-encode) is skipped, and an edit left with no colour
/// changes nothing. Lock, Transparent and Web Shift put the colours in their list when any is
/// missing and take them all out otherwise; Lock leaves a colour shifted by hand alone (it stays
/// locked until it is unshifted), and Web Shift takes the hand shift of the colours it adds away.
/// Unshift All and Sort act on the whole table. Delete is refused with [`webexport::KEEP_ONE`],
/// and nothing changes, when no colour would be left that is neither deleted nor transparent.
pub(crate) fn apply(table: &mut Value, edit: &ColorTableEdit, colors: &[[u8; 3]], shown: &[[u8; 3]]) -> Result<(), String> {
    match edit {
        ColorTableEdit::UnshiftAll => {
            set_table(table, "shifted", json!({}));
            set_table(table, "webShift", json!([]));
            return Ok(());
        }
        ColorTableEdit::Sort(order) => {
            let sort = match order {
                TableOrder::Hue => ColorSort::Hue,
                TableOrder::Luminance => ColorSort::Luminance,
                TableOrder::Popularity => ColorSort::Popularity,
                _ => ColorSort::None,
            };
            set_table(table, "sort", json!(sort.id()));
            return Ok(());
        }
        ColorTableEdit::Restore => {
            restore(table);
            return Ok(());
        }
        // PLAN: assumed `ColorTableEdit::Shift([u8; 3])` (ex.A11 pending change): the colour the
        // first selected colour is shifted to.
        ColorTableEdit::Shift(to) => {
            if let Some(&from) = colors.first().filter(|c| shown.contains(c)) {
                return shift(table, from, *to);
            }
            return Ok(());
        }
        _ => {}
    }
    let mut picked: Vec<String> = Vec::new();
    for c in colors.iter().filter(|c| shown.contains(c)) {
        let c = hex(*c);
        if !picked.contains(&c) {
            picked.push(c);
        }
    }
    if picked.is_empty() {
        return Ok(());
    }
    match edit {
        ColorTableEdit::Lock => {
            let by_hand = shifts(table);
            let free: Vec<String> = picked.into_iter().filter(|c| !by_hand.keys().any(|k| same(k, c))).collect();
            toggle(table, "locked", &free);
        }
        ColorTableEdit::Transparent => toggle(table, "transparent", &picked),
        ColorTableEdit::WebShift => {
            let web = list(table, "webShift");
            let adds = !picked.iter().all(|c| holds(&web, c));
            toggle(table, "webShift", &picked);
            if adds {
                picked.iter().for_each(|c| unshift(table, c));
            }
        }
        ColorTableEdit::Unshift => remove(table, "webShift", &picked),
        ColorTableEdit::Delete => return delete(table, &picked, shown),
        _ => {}
    }
    Ok(())
}

/// Delete Color: `picked` join `deleted` and leave the other lists. The first deletion fixes the
/// palette (`customPalette`) to `shown`, so the colours left do not change.
fn delete(table: &mut Value, picked: &[String], shown: &[[u8; 3]]) -> Result<(), String> {
    let mut custom = list(table, "customPalette");
    if custom.is_empty() {
        custom = shown.iter().map(|c| hex(*c)).collect();
    }
    let mut deleted = list(table, "deleted");
    for c in picked {
        if !holds(&deleted, c) {
            deleted.push(c.clone());
        }
    }
    let clear: Vec<String> = list(table, "transparent").into_iter().filter(|c| !holds(picked, c)).collect();
    if custom.iter().all(|c| holds(&deleted, c) || holds(&clear, c)) {
        return Err(webexport::KEEP_ONE.into());
    }
    for key in ["locked", "transparent", "webShift"] {
        remove(table, key, picked);
    }
    for c in picked {
        unshift(table, c);
    }
    set_table(table, "customPalette", json!(custom));
    set_table(table, "deleted", json!(deleted));
    Ok(())
}

/// Shift the source colour `from` to `to`, wherever it is in the image; it is no longer
/// web-shifted. Shifting a colour to itself takes its shift away. A shift that would leave more
/// than 256 colours shifted is refused with the engine's limit error, and nothing changes. Entries the
/// engine drops (a colour shifted to itself, a deleted colour) do not count, and a new shift writes
/// the map without them.
pub(crate) fn shift(table: &mut Value, from: [u8; 3], to: [u8; 3]) -> Result<(), String> {
    let (source, to) = (hex(from), hex(to));
    let deleted: Vec<[u8; 3]> = list(table, "deleted").iter().filter_map(|c| parse(c)).collect();
    let map = shifts(table);
    // Entries are dropped only from a map the engine accepts (at most 256 entries, every key and
    // value a colour, no two keys naming one colour); any other map is kept whole for the engine to
    // refuse.
    let mut keys: Vec<[u8; 3]> = Vec::new();
    let valid = map.len() <= webexport::MAX_TABLE
        && map.iter().all(|(k, v)| match (parse(k), v.as_str().and_then(parse)) {
            (Some(k), Some(_)) if !keys.contains(&k) => {
                keys.push(k);
                true
            }
            _ => false,
        });
    let live = |k: &str, v: &Value| {
        let (true, Some(key), Some(to)) = (valid, parse(k), v.as_str().and_then(parse)) else { return true };
        to != key && !deleted.contains(&key)
    };
    let count = map.iter().filter(|(k, v)| live(k, v)).count();
    let held = usize::from(map.iter().any(|(k, v)| live(k, v) && same(k, &source)));
    if to != source && count - held >= webexport::MAX_TABLE {
        return Err(webexport::too_many("shifted"));
    }
    unshift(table, &source);
    if to != source {
        let mut map = shifts(table);
        map.retain(|k, v| live(k, v));
        map.insert(source.clone(), json!(to));
        set_table(table, "shifted", Value::Object(map));
    }
    remove(table, "webShift", &[source]);
    Ok(())
}

/// Restore Generated Table: no deleted colours and no fixed palette; the reduction makes the
/// palette again.
pub(crate) fn restore(table: &mut Value) {
    set_table(table, "deleted", json!([]));
    set_table(table, "customPalette", json!([]));
}

/// What the table shows of `table`: its locked, transparent and shifted colours. An entry that is
/// not a colour is skipped, and a colour written more than once in a list is listed once.
pub(crate) fn marks(table: &Value) -> TableMarks {
    let colors = |key: &str| {
        let mut out: Vec<[u8; 3]> = Vec::new();
        for c in list(table, key).iter().filter_map(|c| parse(c)) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        out
    };
    let shifted = shifts(table).iter().filter_map(|(k, v)| Some((parse(k)?, parse(v.as_str()?)?))).collect();
    // PLAN: TableMarks lacks `web_shifted` (ex.W9 request); the Web Shift/Unshift label needs it.
    let mut marks = TableMarks::default();
    marks.locked = colors("locked");
    marks.transparent = colors("transparent");
    marks.shifted = shifted;
    marks
}
