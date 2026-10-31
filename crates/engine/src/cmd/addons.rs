//! VectorCraft's built-in add-ons: extra tools beyond the standard menus, each a module of its own
//! with commands under `addon.<name>.*` (see `docs/mcp.md`).

mod cleanup;
mod color;
mod pathtools;
mod pen;

use super::CommandSpec;

/// The commands of every built-in add-on.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = cleanup::specs();
    v.extend(color::specs());
    v.extend(pathtools::specs());
    v.extend(pen::specs());
    v
}
