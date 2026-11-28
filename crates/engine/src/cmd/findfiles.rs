//! Looking through a folder the user picks, and its subfolders, for files of some kind, on separate
//! threads: up to 8 walker threads share one stack of folders and list each folder once. They
//! follow no links and don't enter other apps' or the system's folders, hidden folders or folders
//! whose contents aren't on the disk ([`Rules`]); the time, the entries listed, the folders queued,
//! the depth and the files read are capped ([`Limits`]). [`start`] returns at once,
//! [`Search::progress`] reports where a search stands and [`Search::stop`] ends it. A search reads
//! the files [`Visitor::wants`] accepts by their names and [`Visitor::worth_reading`] by the files
//! themselves, and keeps those [`Visitor::items`] gives wanted items for.
//! The web has no file system: [`start`] fails there.

mod rules;
#[cfg(test)]
mod tests;
mod walk;

pub use rules::{Rules, plain};
pub use walk::{End, Limits, Progress, Search, Visitor, start, threads};
