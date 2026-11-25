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
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
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
    pub fn decode(lparam: isize) -> TrayEvent {
        match (lparam & 0xFFFF) as u32 {
            0x0400 | 0x0202 => TrayEvent::Select, // NIN_SELECT, WM_LBUTTONUP
            0x007B | 0x0205 => TrayEvent::Menu,   // WM_CONTEXTMENU, WM_RBUTTONUP
            0x0203 => TrayEvent::DoubleClick,     // WM_LBUTTONDBLCLK
            _ => TrayEvent::Other,
        }
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
