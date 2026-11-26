//! Fold Preview's model (`addon.packaging.fold`): a blank folded into its 3D shape by
//! [`vectorcraft_three_d::fold::fold`], and what the Fold Preview window needs to draw it: the
//! document's blanks ([`fold_source`]) and the objects its print leaves out ([`print_hidden`]).

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_three_d::fold::{Blank, Issue, fold, pick};

use super::super::super::{CommandSpec, bad, cmd, finite_numbers, has_doc, number_or};
use super::check::{TOLERANCE, document_lines, found, issue_rows, lines, tolerance};
use super::roles::{RoleTable, can_carry};
use crate::{Result, Session};

const FOLD: &str = "addon.packaging.fold";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "addon.packaging.fold",
        "Fold",
        [],
        None,
        "{ids?: [id…] (default: the selection), base?: [x, y] (the panel under this point; default: the largest panel), amount?: 0..1 (1), tolerance?: pt 0.01..10 (0.1)} fold the blank that the fold lines among the objects make (as addon.packaging.check finds them; while nothing is selected and ids is left out, every fold line of the document) into its 3D shape. The base panel stays in place; every other panel turns about the hinges that join it to the base, each by its angle times amount: a Crease or a Perforation by 90° away from the printed side, a Partial Cut by 90° toward it. A hinge takes the role of the line with the most length in it, with the lowest id winning a tie. Hinges that close a loop of panels are not turned about: gap is how far apart their two placements end up when every hinge is folded all the way, whatever amount is, and closes is true when the gap is within the larger of tolerance and 0.5% of the distance between the hinge's two ends. Changes nothing in the document → {ok (no error), blank (the index of the blank folded, or null when none is), blanks, base, panels: [{outline (SVG path data, document coordinates), frame: {x, y, z, origin} (where the flat blank's x and y directions, its printed side and its origin go, in points: x right, y up, z toward the printed side of the flat blank), parent (null for the base and loose panels), depth}], hinges: [{a, b, angle (degrees), kind: tree|closing, closes, gap}], collisions: [[a, b]] (panels that pass through each other; panels stacked in one plane do not), issues (as addon.packaging.check)}. Errors: a base point in no panel",
        has_doc,
        fold_command
    )]
}

/// The blanks the shown fold lines of `d` make, and their issues (`addon.packaging.check` on the
/// whole document): what the Fold Preview window folds.
pub fn fold_source(d: &Document) -> (Vec<Blank>, Vec<Issue>) {
    found(&document_lines(d), TOLERANCE.1)
}

/// The objects the print on a folded blank leaves out: the layers that do not print and, unless
/// `fold_lines`, every object with a line role.
pub fn print_hidden(d: &Document, fold_lines: bool) -> Vec<NodeId> {
    fn walk(n: &Node, table: &RoleTable, fold_lines: bool, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Layer { printable: false, .. } => out.push(n.id),
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().for_each(|c| walk(c, table, fold_lines, out)),
            _ if !fold_lines && can_carry(n) && table.role_of(n).is_some() => out.push(n.id),
            _ => {}
        }
    }
    let table = RoleTable::of(d);
    let mut out = vec![];
    d.layers.iter().for_each(|l| walk(l, &table, fold_lines, &mut out));
    out
}

fn fold_command(s: &mut Session, p: &Value) -> Result<Value> {
    let tolerance = tolerance(p, FOLD)?;
    let amount = number_or(p, "amount", 1.0, FOLD)?;
    if !(0.0..=1.0).contains(&amount) {
        return Err(bad(FOLD, "`amount` must be from 0 to 1"));
    }
    let base = match p.get("base") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let [x, y] = finite_numbers::<2>(v).ok_or_else(|| bad(FOLD, "`base` must be a point: [x, y]"))?;
            Some(vectorcraft_geom::Point::new(x, y))
        }
    };
    let lines = lines(s, p, FOLD)?;
    let (blanks, issues) = found(&lines, tolerance);
    let ok = !issues.iter().any(|i| i.code.is_error());
    let picked = pick(&blanks, base);
    if base.is_some() && picked.is_none() {
        return Err(bad(FOLD, "`base` is in no panel"));
    }
    let folded = picked.and_then(|(b, panel)| Some((b, blanks.get(b)?, fold(blanks.get(b)?, Some(panel), amount, tolerance)?)));
    let Some((index, blank, f)) = folded else {
        return Ok(json!({
            "ok": ok, "blank": null, "blanks": blanks.len(), "base": null, "panels": [], "hinges": [], "collisions": [],
            "issues": issue_rows(&issues, lines.scope),
        }));
    };
    let column = |m: &[[f64; 3]; 3], k: usize| json!([m[0][k], m[1][k], m[2][k]]);
    let panels: Vec<Value> = blank
        .panels
        .iter()
        .zip(&f.panels)
        .map(|(panel, placed)| {
            json!({
                "outline": panel.outline.to_bezpath().to_svg(),
                "frame": {"x": column(&placed.pose.m, 0), "y": column(&placed.pose.m, 1), "z": column(&placed.pose.m, 2), "origin": placed.pose.t},
                "parent": placed.parent,
                "depth": placed.depth,
            })
        })
        .collect();
    let hinges: Vec<Value> = f
        .hinges
        .iter()
        .map(|h| json!({"a": h.a, "b": h.b, "angle": h.angle, "kind": if h.tree { "tree" } else { "closing" }, "closes": h.closes, "gap": h.gap}))
        .collect();
    Ok(json!({
        "ok": ok,
        "blank": index,
        "blanks": blanks.len(),
        "base": f.base,
        "panels": panels,
        "hinges": hinges,
        "collisions": f.collisions.iter().map(|(a, b)| [a, b]).collect::<Vec<_>>(),
        "issues": issue_rows(&issues, lines.scope),
    }))
}

#[cfg(test)]
mod tests;
