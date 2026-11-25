//! Domain logic of Bento Desk.
//!
//! This crate is 100 % safe Rust and has no dependency on Windows APIs, so
//! every rule, layout and conversion can be unit tested on any machine.

#![forbid(unsafe_code)]

pub mod color;
pub mod display;
pub mod geometry;
pub mod grid;
pub mod hotkey;
pub mod i18n;
pub mod image;
pub mod ipc;
pub mod items;
pub mod model;
pub mod rules;
pub mod settings;
pub mod snap;
pub mod sort;

/// Current time in seconds since the Unix epoch, 0 if the clock is broken.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
