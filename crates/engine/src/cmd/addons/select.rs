//! Selection Tools (`addon.select.*`), a built-in add-on: Select by Artboards selects the objects
//! overlapping, fully inside or fully outside one or more chosen artboards (#5525). Its submenu
//! (Object › Plug-ins › Selection Tools) and dialog are the UI's.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::super::select::{on_artboard, selectable};
use super::super::{CommandSpec, bad, cmd, has_doc, point_indices, text_param};
use crate::{Result, Session};

/// How Select by Artboards relates an object to the chosen artboards (`relation`).
pub const RELATIONS: [&str; 3] = ["overlap", "inside", "outside"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.select.byArtboards",
        "Select by Artboards",
        ["Object", "Plug-ins", "Selection Tools"],
        None,
        "{artboards: [index…] (0-based, at least one), relation?: \"overlap\" (default: on at least one of them, as select.allOnArtboard takes it: the bounds overlap the artboard or the centre is on it) | \"inside\" (the bounds lie wholly within one of them, edges included) | \"outside\" (on none of them)} select those objects among the ones Select All takes (in isolation mode, the isolated container's); hidden and locked objects, objects on hidden, locked or template layers and art held by a locked artboard are left out. Bounds are geometric, a symbol instance's measured by its art → {count}",
        has_doc,
        by_artboards
    )]
}

fn by_artboards(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.select.byArtboards";
    let relation = text_param(p, "relation", C)?.unwrap_or("overlap");
    if !RELATIONS.contains(&relation) {
        return Err(bad(C, "`relation` must be overlap, inside or outside"));
    }
    let listed = p.get("artboards").ok_or_else(|| bad(C, "missing `artboards`"))?;
    let indices = point_indices(listed).ok_or_else(|| bad(C, "`artboards` must be a list of artboard indices"))?;
    if indices.is_empty() {
        return Err(bad(C, "choose at least one artboard"));
    }
    let st = s.doc()?;
    let rects =
        indices.iter().map(|i| st.doc.artboards.get(*i).map(|a| a.rect)).collect::<Option<Vec<_>>>().ok_or_else(|| bad(C, "no such artboard"))?;
    let symbols = st.doc.symbol_boxes();
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation)
        .into_iter()
        .filter(|id| {
            let Some(n) = st.doc.node(*id) else { return false };
            match relation {
                "inside" => n.geometric_bounds_with(&symbols).is_some_and(|b| {
                    rects.iter().any(|r| {
                        vectorcraft_doc::artboard_holds(*r, Point::new(b.x0, b.y0)) && vectorcraft_doc::artboard_holds(*r, Point::new(b.x1, b.y1))
                    })
                }),
                "outside" => !rects.iter().any(|r| on_artboard(n, &symbols, *r)),
                _ => rects.iter().any(|r| on_artboard(n, &symbols, *r)),
            }
        })
        .collect();
    s.select(|_, sel| sel.set(ids))?;
    Ok(json!({ "count": s.doc()?.selection.objects.len() }))
}
