//! OLE drop target for fence titles and tabs: accepts desktop icons dragged
//! from Explorer's desktop list view.
//!
//! The drop always answers `DROPEFFECT_NONE`: Bento Desk only records that
//! the item now belongs to the fence, and the drag source must never move
//! or delete the file. A drop on a folder portal is carried out by Bento
//! Desk itself with `IFileOperation` (see `shell::transfer`), still with
//! `DROPEFFECT_NONE` for the source.
//! <https://learn.microsoft.com/windows/win32/api/oleidl/nn-oleidl-idroptarget>

use std::sync::Mutex;

use bento_core::geometry::Point;
use windows::Win32::Foundation::POINTL;
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, RegisterDragDrop,
    RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{
    IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_DESKTOPABSOLUTEPARSING,
};
use windows::core::{Ref, implement};

use crate::error::{Context, Result};
use crate::hooks::pack_point;
use crate::window::Hwnd;

/// Kind of drag notification posted to the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEvent {
    Enter,
    Over,
    Leave,
    /// Items are available through [`take_dropped`].
    Drop,
}

impl DragEvent {
    pub fn decode(w: usize) -> Option<DragEvent> {
        Some(match w {
            1 => DragEvent::Enter,
            2 => DragEvent::Over,
            3 => DragEvent::Leave,
            4 => DragEvent::Drop,
            _ => return None,
        })
    }
}

static DROPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Parsing names of the last dropped items.
pub fn take_dropped() -> Vec<String> {
    DROPPED
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

#[implement(IDropTarget)]
struct Target {
    notify: Hwnd,
    msg: u32,
}

fn names(data: &IDataObject) -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: the data object is valid during Drop; the array and the
    // strings are released after use.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-shcreateshellitemarrayfromdataobject
    unsafe {
        let Ok(arr) = SHCreateShellItemArrayFromDataObject::<_, IShellItemArray>(data) else {
            return out;
        };
        let n = arr.GetCount().unwrap_or(0);
        for i in 0..n {
            if let Ok(item) = arr.GetItemAt(i)
                && let Ok(p) = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
            {
                if let Ok(s) = p.to_string() {
                    out.push(s);
                }
                CoTaskMemFree(Some(p.0 as *const _));
            }
        }
    }
    out
}

impl Target {
    fn post(&self, ev: usize, pt: &POINTL) {
        self.notify
            .post(self.msg, ev, pack_point(Point::new(pt.x, pt.y)));
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        _data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if !effect.is_null() {
            // SAFETY: OLE passes a valid in/out pointer.
            unsafe { *effect = DROPEFFECT_MOVE };
        }
        self.post(1, pt);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if !effect.is_null() {
            // SAFETY: as above.
            unsafe { *effect = DROPEFFECT_MOVE };
        }
        self.post(2, pt);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.post(3, &POINTL { x: 0, y: 0 });
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        DROP_KEYS.store(keys.0, std::sync::atomic::Ordering::Relaxed);
        if let Some(d) = data.as_ref() {
            let n = names(d);
            if let Ok(mut v) = DROPPED.lock() {
                *v = n;
            }
        }
        if !effect.is_null() {
            // SAFETY: as above. NONE: the source must not touch the file.
            unsafe { *effect = DROPEFFECT_NONE };
        }
        self.post(4, pt);
        Ok(())
    }
}

/// Drop target registration, revoked on drop.
#[derive(Debug)]
pub struct DropRegistration(Hwnd);

impl DropRegistration {
    /// Registers `hwnd` (a window of this thread, OLE initialized).
    /// Notifications go to `notify` as `msg` (wparam = [`DragEvent`] code,
    /// lparam = packed screen point).
    pub fn register(hwnd: Hwnd, notify: Hwnd, msg: u32) -> Result<DropRegistration> {
        let target: IDropTarget = Target { notify, msg }.into();
        // SAFETY: the window belongs to this thread and OLE is initialized
        // (ComApartment::ole_sta). OLE keeps its own reference.
        unsafe { RegisterDragDrop(hwnd.raw(), &target) }.ctx("RegisterDragDrop")?;
        Ok(DropRegistration(hwnd))
    }
}

impl Drop for DropRegistration {
    fn drop(&mut self) {
        if self.0.exists() {
            // SAFETY: same window as the registration.
            let _ = unsafe { RevokeDragDrop(self.0.raw()) };
        }
    }
}

/// Keys held when the last drop happened: (Ctrl, Shift).
pub fn dropped_keys() -> (bool, bool) {
    let k = DROP_KEYS.load(std::sync::atomic::Ordering::Relaxed);
    (k & 0x8 != 0, k & 0x4 != 0)
}

static DROP_KEYS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Absolute item id list of a path, freed on drop.
struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: allocated by SHParseDisplayName, freed once.
        unsafe { windows::Win32::UI::Shell::ILFree(Some(self.0)) };
    }
}

fn parse(path: &str) -> Option<Pidl> {
    let w = crate::util::wide(path);
    let mut p = std::ptr::null_mut();
    // SAFETY: NUL-terminated path and valid out pointer.
    // https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shparsedisplayname
    unsafe {
        windows::Win32::UI::Shell::SHParseDisplayName(
            crate::util::pcwstr(&w),
            None,
            &mut p,
            0,
            None,
        )
    }
    .ok()?;
    (!p.is_null()).then_some(Pidl(p))
}

/// Starts a drag of files from a portal. Blocks until the drop (OLE modal
/// loop on this thread). The drop target decides what to do with the
/// files, exactly as when dragging from an Explorer window.
pub fn drag_out(hwnd: Hwnd, paths: &[String]) -> bool {
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK};
    use windows::Win32::UI::Shell::{
        BHID_DataObject, SHCreateShellItemArrayFromIDLists, SHDoDragDrop,
    };
    let pidls: Vec<Pidl> = paths.iter().filter_map(|p| parse(p)).collect();
    if pidls.is_empty() {
        return false;
    }
    let raw: Vec<*const windows::Win32::UI::Shell::Common::ITEMIDLIST> =
        pidls.iter().map(|p| p.0 as *const _).collect();
    // SAFETY: the id lists stay alive until the end of the function; the
    // data object copies them. SHDoDragDrop supplies a default drop source.
    // https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop
    unsafe {
        let Ok(arr) = SHCreateShellItemArrayFromIDLists(&raw) else {
            return false;
        };
        let Ok(data) = arr.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else {
            return false;
        };
        SHDoDragDrop(
            Some(hwnd.raw()),
            &data,
            None,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
        )
        .is_ok()
    }
}
