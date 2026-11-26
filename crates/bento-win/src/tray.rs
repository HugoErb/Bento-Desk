//! Notification area icon.
//! <https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw>

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, LoadIconW, LoadImageW,
};
use windows::core::PCWSTR;

use crate::window::Hwnd;

/// Tray icon, removed on drop.
#[derive(Debug)]
pub struct TrayIcon {
    hwnd: Hwnd,
    msg: u32,
    tip: String,
    added: bool,
    /// The shell accepted NOTIFYICON_VERSION_4.
    v4: bool,
}

/// Decoded tray callback (NOTIFYICON_VERSION_4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click or keyboard selection.
    Select,
    /// Right click or menu key.
    Menu,
    DoubleClick,
    Other,
}

fn app_icon() -> HICON {
    // SAFETY: icon resource 1 of the executable, shared handle; falls back
    // to the system application icon.
    unsafe {
        let module = GetModuleHandleW(None).ok();
        let small = LoadImageW(
            module.map(|m| HINSTANCE(m.0)),
            PCWSTR(std::ptr::without_provenance::<u16>(1)),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        );
        match small {
            Ok(h) => HICON(h.0),
            Err(_) => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        }
    }
}

fn copy_tip(dst: &mut [u16; 128], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(127).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

impl TrayIcon {
    pub fn new(hwnd: Hwnd, msg: u32, tip: &str) -> TrayIcon {
        let mut t = TrayIcon {
            hwnd,
            msg,
            tip: tip.to_owned(),
            added: false,
            v4: false,
        };
        t.add();
        t
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd.raw(),
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: self.msg,
            hIcon: app_icon(),
            ..Default::default()
        };
        copy_tip(&mut d.szTip, &self.tip);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        d
    }

    /// (Re)adds the icon, for example after `TaskbarCreated`.
    pub fn add(&mut self) {
        let d = self.data();
        // SAFETY: `d` is fully initialized for the call.
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &d).as_bool() {
                self.added = true;
                self.v4 = Shell_NotifyIconW(NIM_SETVERSION, &d).as_bool();
            }
        }
    }

    pub fn set_tip(&mut self, tip: &str) {
        self.tip = tip.to_owned();
        let d = self.data();
        // SAFETY: as above.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    /// Decodes the callback `lparam` (low word = event).
    ///
    /// With NOTIFYICON_VERSION_4 the shell sends WM_RBUTTONUP *and*
    /// WM_CONTEXTMENU for one right click (WM_LBUTTONUP and NIN_SELECT for
    /// a left click), so only one message of each pair is decoded:
    /// opening the menu twice destroyed the first popup, the shell took the
    /// foreground back and the second popup closed at once.
    pub fn decode(&self, lparam: isize) -> TrayEvent {
        decode_event((lparam & 0xFFFF) as u32, self.v4)
    }
}

fn decode_event(event: u32, v4: bool) -> TrayEvent {
    const NIN_SELECT: u32 = 0x0400;
    const NIN_KEYSELECT: u32 = 0x0401;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    match (event, v4) {
        (NIN_SELECT | NIN_KEYSELECT, true) | (WM_LBUTTONUP, false) => TrayEvent::Select,
        (WM_CONTEXTMENU, true) | (WM_RBUTTONUP, false) => TrayEvent::Menu,
        (WM_LBUTTONDBLCLK, _) => TrayEvent::DoubleClick,
        _ => TrayEvent::Other,
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        if self.added {
            let d = self.data();
            // SAFETY: same identity (hwnd + id) as the added icon.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_event_per_click() {
        // Version 4: right click = WM_RBUTTONUP then WM_CONTEXTMENU.
        assert_eq!(decode_event(0x0205, true), TrayEvent::Other);
        assert_eq!(decode_event(0x007B, true), TrayEvent::Menu);
        assert_eq!(decode_event(0x0202, true), TrayEvent::Other);
        assert_eq!(decode_event(0x0400, true), TrayEvent::Select);
        // Legacy version: only the mouse messages.
        assert_eq!(decode_event(0x0205, false), TrayEvent::Menu);
        assert_eq!(decode_event(0x0202, false), TrayEvent::Select);
        assert_eq!(decode_event(0x0203, false), TrayEvent::DoubleClick);
    }
}
