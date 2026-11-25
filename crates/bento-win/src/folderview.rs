//! Reading and moving the real desktop icons through `IFolderView`.
//!
//! Method from Raymond Chen, "Manipulating the positions of desktop icons":
//! <https://devblogs.microsoft.com/oldnewthing/20130318-00/?p=4933>.
//! Since Windows 10 1809 `IFolderView::SelectAndPositionItems` is the only
//! supported way to move desktop icons:
//! <https://devblogs.microsoft.com/oldnewthing/20211122-00/?p=105948>.
//!
//! Every call is a cross-process COM call into Explorer. The object must
//! live on a dedicated worker thread so that a hung Explorer never blocks
//! the UI thread.

use bento_core::geometry::{Point, Size};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoCreateInstance, CoTaskMemFree, IServiceProvider,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::{
    FOLDERVIEWMODE, FWF_AUTOARRANGE, FWF_NOICONS, FWF_SNAPTOGRID, IEnumIDList, IFolderView2,
    IShellBrowser, IShellFolder, IShellWindows, SHGDN_FORPARSING, SHGDN_NORMAL,
    SID_STopLevelBrowser, SVGIO_ALLVIEW, SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    ShellWindows, StrRetToStrW,
};
use windows::core::Interface;

use crate::error::{Context, Result, WinError};
use crate::window::Hwnd;

/// Child item id list allocated by the shell, freed on drop.
#[derive(Debug)]
pub struct ChildPidl(*mut ITEMIDLIST);

// SAFETY: an ITEMIDLIST is a plain byte buffer allocated with the COM task
// allocator, which is usable from any thread.
unsafe impl Send for ChildPidl {}

impl ChildPidl {
    fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }
}

impl Drop for ChildPidl {
    fn drop(&mut self) {
        // SAFETY: the pointer comes from IEnumIDList::Next, which allocates
        // with the COM task allocator.
        // https://learn.microsoft.com/windows/win32/api/combaseapi/nf-combaseapi-cotaskmemfree
        unsafe { CoTaskMemFree(Some(self.0 as *const _)) };
    }
}

/// One icon of the desktop.
#[derive(Debug)]
pub struct DesktopItem {
    /// Absolute parsing name (stable identity).
    pub key: String,
    /// Name displayed under the icon.
    pub display: String,
    /// Top-left of the icon cell, screen coordinates.
    pub pos: Point,
    pub pidl: ChildPidl,
}

/// Connection to the folder view of the desktop.
pub struct DesktopFolderView {
    view: IFolderView2,
    folder: IShellFolder,
    listview: Hwnd,
}

impl std::fmt::Debug for DesktopFolderView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopFolderView")
            .field("listview", &self.listview)
            .finish_non_exhaustive()
    }
}

fn variant_i4(v: i32) -> VARIANT {
    let mut var = VARIANT::default();
    // SAFETY: writing the discriminant and the matching union member of a
    // zero-initialized VARIANT (VT_I4 + lVal).
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = v;
    }
    var
}

fn strret_to_string(
    folder: &IShellFolder,
    pidl: *const ITEMIDLIST,
    flags: windows::Win32::UI::Shell::SHGDNF,
) -> Option<String> {
    let mut sr = STRRET::default();
    // SAFETY: `pidl` is a valid child pidl of `folder`; `sr` is an out
    // pointer. StrRetToStrW frees the STRRET content and allocates the
    // string we free with CoTaskMemFree.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-getdisplaynameof
    unsafe {
        folder.GetDisplayNameOf(pidl, flags, &mut sr).ok()?;
        let mut out = windows::core::PWSTR::null();
        StrRetToStrW(&mut sr, Some(pidl), &mut out).ok()?;
        let s = out.to_string().ok();
        CoTaskMemFree(Some(out.0 as *const _));
        s
    }
}

impl DesktopFolderView {
    /// Connects to the desktop view of the running Explorer.
    /// <https://learn.microsoft.com/windows/win32/api/exdisp/nf-exdisp-ishellwindows-findwindowsw>
    pub fn connect(listview: Hwnd) -> Result<DesktopFolderView> {
        // SAFETY: standard COM activation of the ShellWindows object hosted
        // by Explorer (out-of-process server); COM is initialized on this
        // thread by the caller.
        let windows: IShellWindows =
            unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }
                .ctx("CoCreateInstance(ShellWindows)")?;
        let loc = variant_i4(0); // CSIDL_DESKTOP
        let root = VARIANT::default();
        let mut hwnd = 0i32;
        // SAFETY: both VARIANTs are valid for the call, `hwnd` is an out
        // pointer.
        let disp =
            unsafe { windows.FindWindowSW(&loc, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH) }
                .ctx("FindWindowSW")?;
        let sp: IServiceProvider = disp.cast().ctx("IServiceProvider")?;
        // SAFETY: QueryService with a valid GUID pointer.
        let browser: IShellBrowser =
            unsafe { sp.QueryService(&SID_STopLevelBrowser) }.ctx("QueryService")?;
        // SAFETY: plain COM call.
        let view = unsafe { browser.QueryActiveShellView() }.ctx("QueryActiveShellView")?;
        let view: IFolderView2 = view.cast().ctx("IFolderView2")?;
        // SAFETY: plain COM call.
        let folder: IShellFolder = unsafe { view.GetFolder() }.ctx("GetFolder")?;
        Ok(DesktopFolderView {
            view,
            folder,
            listview,
        })
    }

    /// Screen position of the list-view client origin.
    fn origin(&self) -> Point {
        self.listview.client_origin()
    }

    /// Every icon with its identity and position.
    pub fn items(&self) -> Result<Vec<DesktopItem>> {
        // SAFETY: plain COM call returning an enumerator.
        let en: IEnumIDList = unsafe { self.view.Items(SVGIO_ALLVIEW) }.ctx("Items")?;
        let origin = self.origin();
        let mut out = Vec::new();
        loop {
            let mut buf = [std::ptr::null_mut::<ITEMIDLIST>(); 1];
            let mut fetched = 0u32;
            // SAFETY: `buf` and `fetched` are valid out pointers; Next
            // returns S_FALSE at the end.
            let hr = unsafe { en.Next(&mut buf, Some(&mut fetched)) };
            if hr.is_err() || fetched == 0 || buf[0].is_null() {
                break;
            }
            let pidl = ChildPidl(buf[0]);
            // SAFETY: valid child pidl of this view.
            let pos = match unsafe { self.view.GetItemPosition(pidl.as_ptr()) } {
                Ok(p) => Point::new(p.x + origin.x, p.y + origin.y),
                Err(_) => continue,
            };
            let Some(key) = strret_to_string(&self.folder, pidl.as_ptr(), SHGDN_FORPARSING) else {
                continue;
            };
            let display =
                strret_to_string(&self.folder, pidl.as_ptr(), SHGDN_NORMAL).unwrap_or_default();
            out.push(DesktopItem {
                key,
                display,
                pos,
                pidl,
            });
        }
        Ok(out)
    }

    /// Moves icons. Positions are screen coordinates of the icon cell.
    pub fn set_positions(&self, moves: &[(&ChildPidl, Point)]) -> Result<()> {
        if moves.is_empty() {
            return Ok(());
        }
        let origin = self.origin();
        let pidls: Vec<*const ITEMIDLIST> = moves.iter().map(|(p, _)| p.as_ptr()).collect();
        let points: Vec<POINT> = moves
            .iter()
            .map(|(_, p)| POINT {
                x: p.x - origin.x,
                y: p.y - origin.y,
            })
            .collect();
        let n = u32::try_from(pidls.len()).map_err(|_| WinError::Other("too many items".into()))?;
        // SAFETY: both arrays have `n` elements and outlive the call; every
        // pidl is a child of this view.
        // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifolderview-selectandpositionitems
        unsafe {
            self.view.SelectAndPositionItems(
                n,
                pidls.as_ptr(),
                Some(points.as_ptr()),
                SVSI_POSITIONITEM.0 as u32,
            )
        }
        .ctx("SelectAndPositionItems")
    }

    /// Grid spacing of the desktop (size of one icon cell).
    pub fn spacing(&self) -> Result<Size> {
        let mut p = POINT::default();
        // SAFETY: `p` is a valid out pointer.
        unsafe { self.view.GetSpacing(&mut p) }.ctx("GetSpacing")?;
        Ok(Size::new(p.x, p.y))
    }

    /// Icon size in pixels.
    pub fn icon_size(&self) -> Result<i32> {
        let mut mode = FOLDERVIEWMODE::default();
        let mut size = 0i32;
        // SAFETY: both are valid out pointers.
        unsafe { self.view.GetViewModeAndIconSize(&mut mode, &mut size) }
            .ctx("GetViewModeAndIconSize")?;
        Ok(size)
    }

    pub fn flags(&self) -> Result<u32> {
        // SAFETY: plain COM call.
        unsafe { self.view.GetCurrentFolderFlags() }.ctx("GetCurrentFolderFlags")
    }

    fn set_flags(&self, mask: u32, flags: u32) -> Result<()> {
        // SAFETY: plain COM call.
        unsafe { self.view.SetCurrentFolderFlags(mask, flags) }.ctx("SetCurrentFolderFlags")
    }

    /// Turns off "auto arrange" and "align to grid", required to place
    /// icons at exact positions. Returns the flags found before.
    pub fn free_positioning(&self) -> Result<u32> {
        let before = self.flags()?;
        let mask = (FWF_AUTOARRANGE.0 | FWF_SNAPTOGRID.0) as u32;
        if before & mask != 0 {
            self.set_flags(mask, 0)?;
        }
        Ok(before)
    }

    /// Restores "auto arrange" and "align to grid" from saved flags.
    pub fn restore_positioning(&self, saved: u32) -> Result<()> {
        let mask = (FWF_AUTOARRANGE.0 | FWF_SNAPTOGRID.0) as u32;
        self.set_flags(mask, saved & mask)
    }

    /// Shows or hides every desktop icon (quick-hide without exclusions).
    pub fn set_icons_hidden(&self, hidden: bool) -> Result<()> {
        let mask = FWF_NOICONS.0 as u32;
        self.set_flags(mask, if hidden { mask } else { 0 })
    }

    pub fn icons_hidden(&self) -> bool {
        self.flags().is_ok_and(|f| f & FWF_NOICONS.0 as u32 != 0)
    }

    /// Cheap liveness check: fails once Explorer has restarted.
    pub fn is_alive(&self) -> bool {
        self.listview.exists() && self.flags().is_ok()
    }
}
