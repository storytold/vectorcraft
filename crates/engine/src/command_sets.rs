use super::{NodeId, Value, cmd};

/// The commands that act on the active artboard (#693), which the app passes as their `artboard`:
/// Paste in Place, in Front and in Back paste onto it, Align to Artboard aligns to it, Distribute
/// Objects and Distribute Spacing with Align to Artboard distribute objects across it, and All on
/// Active Artboard selects on it. A `command.batch` given `artboard` passes it on to each of their
/// steps that names none.
pub const ON_ACTIVE_ARTBOARD: [&str; 7] = [
    "edit.pasteInPlace",
    "edit.pasteInFront",
    "edit.pasteInBack",
    "object.align",
    "object.distribute",
    "object.distributeSpacing",
    "select.allOnArtboard",
];

/// The commands that resize the artboards the app passes (Object › Artboards): the app passes the
/// selected artboards to Fit to Artwork Bounds as its `indices`, and the active artboard to Fit to
/// Selected Art as its `index`. A `command.batch` given `artboard` passes it on, as `index`, to
/// each of their steps that names none.
pub const FIT_ACTIVE_ARTBOARD: [&str; 2] = ["artboard.fitToArt", "artboard.fitToSelection"];

/// `(command, param)` pairs the journal records only so a replay reproduces the original run
/// (dates from the clock: `cmd::clock_date`; the artboards the app passes to the
/// [`ON_ACTIVE_ARTBOARD`] and [`FIT_ACTIVE_ARTBOARD`] commands and to a batch); an action leaves
/// them out ([`Session::journal_for_action`]).
const REPLAY_ONLY: [(&str, &str); 14] = [
    ("file.new", "created"),
    ("document.save", "modified"),
    ("file.saveAs", "modified"),
    ("edit.pasteInPlace", "artboard"),
    ("edit.pasteInFront", "artboard"),
    ("edit.pasteInBack", "artboard"),
    ("object.align", "artboard"),
    ("object.distribute", "artboard"),
    ("object.distributeSpacing", "artboard"),
    ("artboard.fitToArt", "index"),
    ("artboard.fitToArt", "indices"),
    ("artboard.fitToSelection", "index"),
    ("select.allOnArtboard", "artboard"),
    ("command.batch", "artboard"),
];

/// `p`, the params of command `id`, without its [`REPLAY_ONLY`] values (a batch: its steps').
pub(crate) fn strip_replay_only(id: &str, p: &mut Value) {
    let strip = |id: &str, p: &mut Value| {
        if let Value::Object(m) = p {
            m.retain(|k, _| !REPLAY_ONLY.iter().any(|&(c, key)| c == id && key == k));
        }
    };
    strip(id, p);
    // Batches don't nest: one level of steps.
    if id == "command.batch"
        && let Some(Value::Array(steps)) = p.get_mut("commands")
    {
        for step in steps {
            let id = step.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(p) = step.get_mut("params") {
                strip(&id, p);
            }
        }
    }
}

/// Commands that act on the selection and take `ids` by selecting them first (`select.set`, with
/// its checks), then running as on the selection: the result and the selection afterwards are the
/// menu command's (#2130).
pub(crate) const SELECTS_GIVEN_IDS: &[&str] = &[
    "edit.duplicate",
    "object.arrange.bringForward",
    "object.arrange.bringToFront",
    "object.arrange.sendBackward",
    "object.arrange.sendToBack",
    "object.arrange.sendToCurrentLayer",
    "object.clippingMask.make",
    "object.clippingMask.release",
    "object.compoundPath.make",
    "object.compoundPath.release",
    "object.group",
    "object.hide",
    "object.lock",
    "object.move",
    "object.pathfinder.crop",
    "object.pathfinder.divide",
    "object.pathfinder.exclude",
    "object.pathfinder.intersect",
    "object.pathfinder.merge",
    "object.pathfinder.minusBack",
    "object.pathfinder.minusFront",
    "object.pathfinder.outline",
    "object.pathfinder.trap",
    "object.pathfinder.trim",
    "object.pathfinder.unite",
    "object.reflect",
    "object.rotate",
    "object.scale",
    "object.shear",
    "object.ungroup",
];

/// The non-empty `ids` given to a command whose params description names `ids` as a key (the
/// key callers outside the app may pass it, [`cmd::params::takes_key`]), or its `id` as `ids` of
/// one when no `ids` are given and the description takes `id` so
/// ([`cmd::params::id_stands_for_ids`]), as [`cmd::targets`] reads them.
pub(crate) fn given_ids(description: &'static str, params: &Value) -> Option<Vec<NodeId>> {
    if !cmd::params::takes_key(description, "ids") {
        return None;
    }
    match params.get("ids").filter(|v| !v.is_null()) {
        Some(_) => cmd::ids_param(params, "ids"),
        None => cmd::id_param(params, "id").filter(|_| cmd::params::id_stands_for_ids(description)).map(|id| vec![id]),
    }
    .filter(|ids| !ids.is_empty())
}
