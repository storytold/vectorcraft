//! Every angle or rotation a command takes says which way it turns (#6340): creation commands turn
//! clockwise and most edits counter-clockwise, so a client can't guess.

use crate::cmd::params::{is_key, object_body, split_top};
use crate::command_specs;

/// Params that are angles but not a turn, with why.
const NOT_A_TURN: &[(&str, &str, &str)] = &[
    ("addon.pathtools.cornerSet", "maxAngle", "the angle a corner's sides meet at"),
    ("addon.pathtools.cornerSet", "minAngle", "the angle a corner's sides meet at"),
    ("addon.shapes.box3d", "angleX", "a tilt out of the page: the description says which side turns away"),
    ("addon.shapes.box3d", "angleY", "a tilt out of the page: the description says which side turns away"),
    ("addon.shapes.configure", "angleX", "a tilt out of the page: the description says which side turns away"),
    ("addon.shapes.configure", "angleY", "a tilt out of the page: the description says which side turns away"),
    ("object.path.simplify", "cornerAngle", "the angle a corner's sides meet at"),
    ("perspective.grid.define", "angle", "the viewing angle, 0 to 90°"),
];

/// A param name that is an angle or a turn.
fn is_turn(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.ends_with("angle") || k.ends_with("angles") || k.starts_with("angle") || ["rotation", "rotate", "piestart", "pieend"].contains(&k.as_str())
}

/// The bodies of the `{…}` objects in `text` that are not inside another one.
fn objects_in(text: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let Ok((body, after)) = object_body(&rest[i..]) else { break };
        out.push(body);
        rest = after;
    }
    out
}

/// `(key, what the description says about it)` for each key the object `body` names, and for the
/// keys of the objects inside it. A key given without a note (`a?, b?: degrees`) shares the note of
/// the next key that has one.
fn keys_and_notes(body: &str, out: &mut Vec<(String, String)>) {
    let mut items: Vec<(&str, String)> = vec![];
    for entry in split_top(body, ',').into_iter().map(str::trim).filter(|e| !e.is_empty() && !e.starts_with('…')) {
        for (i, alternative) in split_top(entry, '|').into_iter().enumerate() {
            let head = alternative.split([':', '=', '(']).next().unwrap_or_default();
            let note = alternative.get(head.len()..).unwrap_or_default();
            let name = head.trim().trim_end_matches('?').trim_end();
            let a_key = is_key(name) && (i == 0 || head.trim_end().ends_with('?') || note.starts_with(':'));
            match items.last_mut() {
                Some((_, last)) if !a_key => last.push_str(alternative),
                _ if a_key => items.push((name, note.to_string())),
                _ => {}
            }
        }
    }
    for (i, (key, note)) in items.iter().enumerate() {
        let shared = items.get(i..).unwrap_or_default().iter().map(|(_, n)| n.trim()).find(|n| !n.is_empty()).unwrap_or_default();
        out.push((key.to_string(), shared.to_string()));
        for inner in objects_in(note) {
            keys_and_notes(inner, out);
        }
    }
}

/// The turn params of description `doc` whose note doesn't say which way they turn.
fn undirected(doc: &str) -> Vec<String> {
    let mut found = vec![];
    let mut next = Some(doc);
    // `{…} or {…}`: each object the command takes.
    while let Some(text) = next {
        let Ok((body, rest)) = object_body(text) else { break };
        keys_and_notes(body, &mut found);
        next = rest.trim_start().strip_prefix("or ").or_else(|| rest.trim_start().strip_prefix("| ")).filter(|r| r.starts_with('{'));
    }
    found.into_iter().filter(|(k, note)| is_turn(k) && !note.to_ascii_lowercase().contains("clockwise")).map(|(k, _)| k).collect()
}

#[test]
fn a_note_is_found_for_each_key() {
    assert_eq!(undirected("{angle: deg (counter-clockwise), origin?}"), Vec::<String>::new());
    assert_eq!(undirected("{x, rotation?: deg} → {rotation}"), ["rotation"]);
    // A shared note, an alternative, a nested object, and the object after `or`.
    assert_eq!(undirected("{inAngle?, outAngle?: degrees counter-clockwise, size?: pt}"), Vec::<String>::new());
    assert_eq!(undirected("{startAngle?, endAngle?, center?: [x,y] | hSpacing? | angle?}"), ["startAngle", "endAngle", "angle"]);
    assert_eq!(undirected("{gradient?: {kind?: linear|radial, angle?: deg}, id?}"), ["angle"]);
    assert_eq!(undirected("{id} or {ids, rotate?: deg}"), ["rotate"]);
    // Not a turn by its name, or a value rather than a key.
    assert_eq!(undirected("{autoRotate: true, orientation?: portrait|landscape, mode?: rotation|scale}"), Vec::<String>::new());
}

/// Each command param that is an angle or a rotation says in its description which way it turns
/// (clockwise or counter-clockwise), unless [`NOT_A_TURN`] says why it isn't a turn.
#[test]
fn every_turn_param_says_which_way_it_turns() {
    let mut missing = vec![];
    for c in command_specs() {
        for key in undirected(c.params) {
            if !NOT_A_TURN.iter().any(|(id, k, _)| *id == c.id && *k == key) {
                missing.push(format!("{}: {key}", c.id));
            }
        }
    }
    assert!(missing.is_empty(), "{} params don't say whether they turn clockwise or counter-clockwise:\n{}", missing.len(), missing.join("\n"));
    let stale: Vec<_> = NOT_A_TURN
        .iter()
        .filter(|(id, key, _)| !command_specs().iter().any(|c| c.id == *id && keys_named(c.params).iter().any(|k| k == key)))
        .collect();
    assert!(stale.is_empty(), "NOT_A_TURN names params no command takes: {stale:?}");
}

fn keys_named(doc: &str) -> Vec<String> {
    let mut found = vec![];
    if let Ok((body, _)) = object_body(doc) {
        keys_and_notes(body, &mut found);
    }
    found.into_iter().map(|(k, _)| k).collect()
}

/// Turn notes the key walk above doesn't reach: a numeric reflection axis and the Scatter brush's
/// `rotation [a, b]` range, whose head isn't a plain key.
#[test]
fn turns_outside_plain_keys_say_which_way() {
    let doc = |id: &str| command_specs().iter().find(|c| c.id == id).map(|c| c.params).unwrap_or_default();
    assert!(doc("object.reflect").contains("|deg (the mirror line, counter-clockwise from horizontal"), "object.reflect's numeric axis");
    assert!(doc("brush.options").contains("rotation [a, b] −180..180° (counter-clockwise)"), "the Scatter brush's rotation");
}
