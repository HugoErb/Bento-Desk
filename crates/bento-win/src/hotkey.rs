//! Global shortcuts through `RegisterHotKey` (no keyboard hook).
//! <https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey>

use bento_core::hotkey::Hotkey;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

use crate::error::{Context, Result};
use crate::window::Hwnd;

/// Registered shortcut, unregistered on drop.
#[derive(Debug)]
pub struct HotkeyRegistration {
    hwnd: Hwnd,
    id: i32,
}

impl HotkeyRegistration {
    /// Registers `hk` for `hwnd`; WM_HOTKEY arrives with `wparam == id`.
    /// Fails when another application (or the shell) owns the shortcut.
    pub fn register(hwnd: Hwnd, id: i32, hk: Hotkey) -> Result<HotkeyRegistration> {
        let mut mods: HOT_KEY_MODIFIERS = MOD_NOREPEAT;
        if hk.ctrl {
            mods |= MOD_CONTROL;
        }
        if hk.alt {
            mods |= MOD_ALT;
        }
        if hk.shift {
            mods |= MOD_SHIFT;
        }
        if hk.win {
            mods |= MOD_WIN;
        }
        // SAFETY: the window belongs to this thread; plain values.
        unsafe { RegisterHotKey(Some(hwnd.raw()), id, mods, u32::from(hk.key)) }
            .ctx("RegisterHotKey")?;
        Ok(HotkeyRegistration { hwnd, id })
    }
}

impl Drop for HotkeyRegistration {
    fn drop(&mut self) {
        // SAFETY: same window and id as the registration.
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd.raw()), self.id) };
    }
}
