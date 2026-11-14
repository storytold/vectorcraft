//! Cutting Tools (`addon.cutting.*`), a built-in add-on that prepares art for cutting. Stencil
//! Bridges cuts bridges out of the selected art so that a stencil cut from it holds together
//! ([`stencil`]), and Weed Box and Lines adds a weed box and weeding lines around the selected art
//! for vinyl cutting ([`weed`]). Banner Hems and Grommets marks the hems and grommets of a banner
//! ([`banner`]). Its submenu (Object › Plug-ins › Cutting Tools) and dialogs are the UI's.

mod banner;
mod stencil;
mod weed;

use super::super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = stencil::specs();
    v.extend(weed::specs());
    v.extend(banner::specs());
    v
}
