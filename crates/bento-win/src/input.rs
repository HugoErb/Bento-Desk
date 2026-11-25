//! Mouse and keyboard state queries.

use bento_core::geometry::{Point, Rect};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, GetKeyState, GetKeyboardLayoutList, VK_CONTROL,
    VK_LBUTTON, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXDOUBLECLK, SM_CXVIRTUALSCREEN, SM_CYDOUBLECLK,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Cursor position in screen coordinates.
pub fn cursor_pos() -> Point {
    let mut p = POINT::default();
    // SAFETY: valid out pointer.
    let _ = unsafe { GetCursorPos(&mut p) };
    Point::new(p.x, p.y)
}

/// Maximum delay between the clicks of a double-click, milliseconds.
pub fn double_click_ms() -> u32 {
    // SAFETY: no argument.
    unsafe { GetDoubleClickTime() }
}

/// Maximum distance between the clicks of a double-click, pixels.
pub fn double_click_size() -> (i32, i32) {
    // SAFETY: plain index values.
    unsafe {
        (
            GetSystemMetrics(SM_CXDOUBLECLK),
            GetSystemMetrics(SM_CYDOUBLECLK),
        )
    }
}

pub fn ctrl_down() -> bool {
    // SAFETY: plain value; the high bit means "down".
    let s = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) };
    s < 0
}

/// Left mouse button currently pressed (physical state).
pub fn left_button_down() -> bool {
    // SAFETY: plain value; the high bit means "down".
    let s = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) };
    s < 0
}

pub fn shift_down() -> bool {
    // SAFETY: plain value.
    let s = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) };
    s < 0
}

/// Number of installed keyboard layouts. With more than one, Win+Space
/// switches layouts and must not be taken by Peek.
pub fn keyboard_layout_count() -> usize {
    // SAFETY: a None buffer returns the count.
    usize::try_from(unsafe { GetKeyboardLayoutList(None) }).unwrap_or(0)
}

/// Bounding rectangle of all monitors.
pub fn virtual_screen() -> Rect {
    // SAFETY: plain index values.
    unsafe {
        Rect::from_xywh(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}
