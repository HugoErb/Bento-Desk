//! Win32 and COM interop of Bento Desk.
//!
//! This is the only crate of the workspace that contains `unsafe` code.
//! Every `unsafe` block states why it is sound and links the Microsoft
//! documentation of the API it calls. The public API is safe: handles are
//! wrapped in RAII types that release them on `Drop`.
//!
//! Nothing here loads code into another process. Allowed cross-process
//! operations are window messages, `SetParent`-style child windows, COM
//! calls into Explorer through its documented interfaces, out-of-context
//! WinEvent hooks and the low-level mouse hook (`WH_MOUSE_LL`), whose
//! callback runs in this process.

pub mod com;
pub mod desktop;
pub mod dirwatch;
pub mod display;
pub mod dragdrop;
pub mod dwm;
pub mod error;
pub mod folderview;
pub mod gfx;
pub mod hooks;
pub mod hotkey;
pub mod input;
pub mod shell;
pub mod system;
pub mod tray;
pub mod wallpaper;
pub mod window;

pub use error::{Result, WinError};
pub use window::Hwnd;

pub(crate) mod util {
    use windows::core::PCWSTR;

    /// Null-terminated UTF-16 copy of a string.
    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Borrowed `PCWSTR` for a buffer from [`wide`]. The buffer must outlive
    /// every use of the returned pointer.
    pub fn pcwstr(buf: &[u16]) -> PCWSTR {
        PCWSTR(buf.as_ptr())
    }

    /// String from a UTF-16 buffer, stopping at the first NUL.
    pub fn from_wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }
}
