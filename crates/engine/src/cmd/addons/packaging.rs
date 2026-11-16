//! Packaging Tools (`addon.packaging.*`), a built-in add-on for the dielines of boxes and other
//! folded packaging. A path is a fold line when its top stroke is painted with the swatch of a line
//! role: Cut, Crease, Partial Cut or Perforation ([`roles`]). The document's role table maps each
//! role to its swatch, so every format that keeps the stroke's swatch keeps the role. Its submenu
//! (Object › Plug-ins › Packaging Tools) and its Fold Lines panel are the UI's.

mod roles;

pub(crate) use roles::{role_swatch_edited, role_swatches_merged};

use super::super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    roles::specs()
}

#[cfg(test)]
mod tests;
