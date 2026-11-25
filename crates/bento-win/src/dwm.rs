//! Desktop Window Manager attributes: dark title, backdrops, corners.
//! <https://learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute>

use std::ffi::c_void;

use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmGetColorizationColor, DwmSetWindowAttribute,
};
use windows::core::BOOL;

use bento_core::color::Rgba;

use crate::window::Hwnd;

/// System backdrop material (Windows 11 22H2 and later).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Mica,
    Acrylic,
}

fn set<T>(hwnd: Hwnd, attr: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE, value: &T) -> bool {
    // SAFETY: `value` points to a `T` of the size given; DWM copies it.
    unsafe {
        DwmSetWindowAttribute(
            hwnd.raw(),
            attr,
            value as *const T as *const c_void,
            size_of::<T>() as u32,
        )
    }
    .is_ok()
}

/// Dark window frame. Ignored by older systems.
pub fn set_dark(hwnd: Hwnd, dark: bool) {
    let v = BOOL::from(dark);
    set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v);
}

/// Applies a backdrop. Returns false where unsupported (Windows 10).
pub fn set_backdrop(hwnd: Hwnd, b: Backdrop) -> bool {
    let v = match b {
        Backdrop::None => DWMSBT_NONE,
        Backdrop::Mica => DWMSBT_MAINWINDOW,
        Backdrop::Acrylic => DWMSBT_TRANSIENTWINDOW,
    };
    set(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &v)
}

/// Rounded corners on Windows 11.
pub fn set_round_corners(hwnd: Hwnd) {
    set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND);
}

/// Current accent (colorization) color.
pub fn accent_color() -> Option<Rgba> {
    let mut c = 0u32;
    let mut opaque = BOOL(0);
    // SAFETY: valid out pointers.
    unsafe { DwmGetColorizationColor(&mut c, &mut opaque) }.ok()?;
    Some(Rgba::rgb(
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    ))
}
