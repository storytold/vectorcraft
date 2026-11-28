//! Packaging Tools (`addon.packaging.*`), a built-in add-on for the dielines of boxes and other
//! folded packaging. A path is a fold line when its top stroke is painted with the swatch of a line
//! role: Cut, Crease, Partial Cut or Perforation ([`roles`]). The document's role table maps each
//! role to its swatch, so every format that keeps the stroke's swatch keeps the role. Check Fold
//! Lines ([`check`]) finds the panels and hinges the lines make and what is wrong with them. Set
//! Fold Angle ([`angle`]) gives a fold line its own angle. Its submenu (Object › Plug-ins ›
//! Packaging Tools) and its Fold Lines panel are the UI's.

mod angle;
mod check;
mod fold;
mod roles;

pub use angle::{FoldAngles, selection_fold_angles, selection_fold_lines};
pub use fold::{fold_source, print_hidden};
pub(crate) use roles::{role_swatch_edited, role_swatches_merged};

use super::super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = roles::specs();
    v.extend(check::specs());
    v.extend(angle::specs());
    v.extend(fold::specs());
    v
}

#[cfg(test)]
mod tests;
