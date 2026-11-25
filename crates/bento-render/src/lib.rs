//! Drawing of every surface of Bento Desk: fence backgrounds and frames,
//! Fluent style menus, folder portals, Peek and small overlays.
//!
//! The crate is safe code on top of `bento_win::gfx`. Layout functions
//! (hit testing, row positions) are pure and unit tested.

#![forbid(unsafe_code)]

pub mod fence;
pub mod icons;
pub mod menu;
pub mod overlay;
pub mod portal;
pub mod theme;

pub use theme::Theme;
