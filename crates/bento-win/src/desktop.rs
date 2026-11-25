//! Locating the desktop layer of Explorer and inserting windows into it.
//!
//! Windows 10 and Windows 11 up to 23H2: after the undocumented message
//! `0x052C` is sent to `Progman`, Explorer creates a top-level `WorkerW`
//! behind the `WorkerW` that hosts `SHELLDLL_DefView` (the icons). Animated
//! wallpaper apps draw in that back `WorkerW`; fence backgrounds do the
//! same.
//!
//! Windows 11 24H2 and later: `SHELLDLL_DefView` and the `WorkerW` are
//! children of `Progman`; a child of `Progman` is only drawn when it has
//! `WS_EX_LAYERED`, and fence backgrounds go just below `SHELLDLL_DefView`.
//! Sources: <https://github.com/lively-community/lively/issues/2074>,
//! <https://github.com/rocksdanister/lively/issues/2695>.
//!
//! The variant is detected from the window tree, not from the build number.

use bento_core::geometry::Point;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::HiDpi::{DPI_HOSTING_BEHAVIOR_MIXED, SetThreadDpiHostingBehavior};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
use windows::core::BOOL;

use crate::error::{Result, WinError};
use crate::window::{Hwnd, find_child, find_window};

/// How the desktop windows are arranged on this system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerVariant {
    /// Windows 10 style: DefView in a top-level `WorkerW`, wallpaper in
    /// another top-level `WorkerW` behind it.
    SplitWorkerW,
    /// Windows 11 24H2 style: DefView and `WorkerW` are children of
    /// `Progman`.
    ProgmanChildren,
    /// No `WorkerW` could be created: everything is inside `Progman`.
    ProgmanOnly,
}

/// Handles of the desktop windows Bento Desk works with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopLayer {
    pub variant: LayerVariant,
    pub progman: Hwnd,
    /// `SHELLDLL_DefView`.
    pub defview: Hwnd,
    /// `SysListView32` with the icons.
    pub listview: Hwnd,
    /// Parent of the fence backgrounds (below the icons).
    pub backdrop_parent: Hwnd,
    /// Parent of the fence frames (above the icons).
    pub frame_parent: Hwnd,
}

const SPAWN_WORKERW: u32 = 0x052C;

/// Finds the top-level `WorkerW` that hosts `SHELLDLL_DefView`.
fn find_defview_host() -> Option<(Hwnd, Hwnd)> {
    struct Found(Option<(Hwnd, Hwnd)>);
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let top = Hwnd::from_raw(hwnd);
        if let Some(dv) = find_child(Some(top), None, "SHELLDLL_DefView") {
            // SAFETY: `lp` is the address of the `Found` below, alive for
            // the whole EnumWindows call.
            let found = unsafe { &mut *(lp.0 as *mut Found) };
            found.0 = Some((top, dv));
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found = Found(None);
    // SAFETY: the callback only reads window handles and writes `found`,
    // which outlives the call.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-enumwindows
    let _ = unsafe { EnumWindows(Some(cb), LPARAM(&mut found as *mut Found as isize)) };
    found.0
}

impl DesktopLayer {
    /// Locates the desktop layer, asking Explorer to create the wallpaper
    /// `WorkerW` when needed.
    pub fn locate() -> Result<DesktopLayer> {
        let progman = find_window("Progman").ok_or(WinError::NotFound("Progman"))?;
        // Ask Progman to spawn the WorkerW. wParam 0xD / lParam 1 is the
        // form needed by 24H2; older builds accept it too. A second message
        // with zero arguments covers builds that ignore the first one.
        let _ = progman.send_timeout(SPAWN_WORKERW, 0xD, 1, 1000);
        let _ = progman.send_timeout(SPAWN_WORKERW, 0, 0, 1000);

        // 24H2: DefView is a child of Progman.
        if let Some(defview) = find_child(Some(progman), None, "SHELLDLL_DefView") {
            let listview = find_child(Some(defview), None, "SysListView32")
                .ok_or(WinError::NotFound("SysListView32"))?;
            let variant = if find_child(Some(progman), None, "WorkerW").is_some() {
                LayerVariant::ProgmanChildren
            } else {
                LayerVariant::ProgmanOnly
            };
            return Ok(DesktopLayer {
                variant,
                progman,
                defview,
                listview,
                backdrop_parent: progman,
                frame_parent: progman,
            });
        }

        // Windows 10: DefView lives in a top-level WorkerW.
        let (host, defview) = find_defview_host().ok_or(WinError::NotFound("SHELLDLL_DefView"))?;
        let listview = find_child(Some(defview), None, "SysListView32")
            .ok_or(WinError::NotFound("SysListView32"))?;
        // The wallpaper WorkerW is the next top-level WorkerW after the host.
        let wallpaper = find_child(None, Some(host), "WorkerW").unwrap_or(progman);
        Ok(DesktopLayer {
            variant: LayerVariant::SplitWorkerW,
            progman,
            defview,
            listview,
            backdrop_parent: wallpaper,
            frame_parent: host,
        })
    }

    /// True while every handle is still valid (false after an Explorer
    /// restart).
    pub fn is_alive(&self) -> bool {
        self.progman.exists()
            && self.defview.exists()
            && self.listview.exists()
            && self.backdrop_parent.exists()
            && self.frame_parent.exists()
    }

    /// Puts a background window at its place in the z-order.
    pub fn place_backdrop(&self, w: Hwnd) {
        match self.variant {
            // Background windows go right below the icons.
            LayerVariant::ProgmanChildren | LayerVariant::ProgmanOnly => {
                if w.set_z_after(self.defview).is_err() {
                    tracing::warn!("cannot place backdrop below DefView");
                }
            }
            // On top of the wallpaper inside the back WorkerW (still behind
            // the WorkerW with the icons).
            LayerVariant::SplitWorkerW => w.set_z_top(),
        }
    }

    /// Puts a frame window above the icons.
    pub fn place_frame(&self, w: Hwnd) {
        w.set_z_top();
    }

    /// Screen position of a parent's client origin, to convert screen
    /// rectangles into child coordinates.
    pub fn origin_of(parent: Hwnd) -> Point {
        parent.client_origin()
    }

    /// True for windows that belong to the desktop (used to detect that
    /// the desktop is in the foreground).
    pub fn is_desktop_window(&self, w: Hwnd) -> bool {
        if w.is_null() {
            return false;
        }
        let root = w.root();
        root == self.progman || root == self.frame_parent || root == self.backdrop_parent || {
            let c = root.class_name();
            c == "Progman" || c == "WorkerW"
        }
    }
}

/// Allows this thread to create child windows inside windows that use a
/// different DPI awareness (Explorer's desktop).
/// <https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setthreaddpihostingbehavior>
pub fn allow_mixed_dpi_hosting() {
    // SAFETY: plain value, affects only the calling thread.
    unsafe { SetThreadDpiHostingBehavior(DPI_HOSTING_BEHAVIOR_MIXED) };
}
