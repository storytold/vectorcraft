//! Cutting Tools (`addon.cutting.*`), a built-in add-on that prepares art for cutting. Stencil
//! Bridges cuts bridges out of the selected art so that a stencil cut from it holds together
//! ([`stencil`]). Its submenu (Object › Plug-ins › Cutting Tools) and dialog are the UI's.

mod stencil;

use super::super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    stencil::specs()
}
