//! Shell helpers running in this process: icons, shortcut targets, context
//! menus and the folder picker.
//!
//! Showing a shell context menu loads the registered shell extensions into
//! Bento Desk's own process. That is the documented behavior of
//! `IContextMenu` and affects no other process.

use std::ffi::c_void;

use bento_core::geometry::Point;
use bento_core::image::Image;
use windows::Win32::Foundation::{HWND, SIZE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS,
    FileOpenDialog, IContextMenu, IFileOpenDialog, IShellItem, IShellItemImageFactory, IShellLinkW,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{Interface, PCSTR};

use crate::util::{pcwstr, wide};
use crate::window::Hwnd;

fn shell_item(path: &str) -> Option<IShellItem> {
    let w = wide(path);
    // SAFETY: NUL-terminated path, no bind context.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-shcreateitemfromparsingname
    unsafe { SHCreateItemFromParsingName(pcwstr(&w), None) }.ok()
}

/// Icon of a file, folder or virtual item, `size` pixels square.
pub fn item_icon(path: &str, size: i32) -> Option<Image> {
    let item = shell_item(path)?;
    let factory: IShellItemImageFactory = item.cast().ok()?;
    // SAFETY: plain values; the returned HBITMAP is ours to delete.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage
    let hbm = unsafe {
        factory.GetImage(
            SIZE { cx: size, cy: size },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )
    }
    .ok()?;
    let mut bm = BITMAP::default();
    // SAFETY: `bm` has the size given.
    let ok = unsafe {
        GetObjectW(
            HGDIOBJ(hbm.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut c_void),
        )
    };
    let result = (|| {
        if ok == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
        // SAFETY: temporary DC; `px` holds w*h*4 bytes for a 32-bit
        // top-down request.
        let lines = unsafe {
            let dc = CreateCompatibleDC(None);
            let n = GetDIBits(
                dc,
                hbm,
                0,
                h as u32,
                Some(px.as_mut_ptr() as *mut c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            let _ = DeleteDC(dc);
            n
        };
        if lines == 0 {
            return None;
        }
        let mut img = Image {
            width: w as u32,
            height: h as u32,
            pixels: px,
        };
        // The factory usually returns premultiplied pixels; if any color
        // exceeds its alpha the data is straight alpha.
        let straight = img
            .pixels
            .chunks_exact(4)
            .any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]);
        if straight {
            img.premultiply();
        }
        Some(img)
    })();
    // SAFETY: bitmap returned by GetImage, owned by us.
    let _ = unsafe { DeleteObject(HGDIOBJ(hbm.0)) };
    result
}

/// Target path of a `.lnk` shortcut.
pub fn shortcut_target(path: &str) -> Option<String> {
    // SAFETY: in-process ShellLink object; COM initialized by the caller.
    let link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let file: IPersistFile = link.cast().ok()?;
    let w = wide(path);
    // SAFETY: NUL-terminated path, read-only mode.
    unsafe { file.Load(pcwstr(&w), STGM_READ) }.ok()?;
    let mut buf = vec![0u16; 1024];
    // SAFETY: buffer valid for its length; NULL find data.
    // SLGP_RAWPATH = 4 keeps environment variables unexpanded.
    unsafe { link.GetPath(&mut buf, std::ptr::null_mut(), 4) }.ok()?;
    let s = crate::util::from_wide(&buf);
    (!s.is_empty()).then_some(s)
}

/// Shows the shell context menu of `path` at screen point `pt`. Returns
/// true when a command was invoked.
pub fn show_context_menu(owner: Hwnd, path: &str, pt: Point) -> bool {
    let Some(item) = shell_item(path) else {
        return false;
    };
    // SAFETY: BHID_SFUIObject asks the item for its IContextMenu.
    let Ok(menu) = (unsafe { item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) })
    else {
        return false;
    };
    // SAFETY: standard IContextMenu protocol: fill a popup menu, track it
    // (owner window is ours), invoke the chosen command, destroy the menu.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu
    unsafe {
        let Ok(hmenu) = CreatePopupMenu() else {
            return false;
        };
        let mut invoked = false;
        if menu
            .QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)
            .is_ok()
        {
            owner.set_foreground();
            let cmd = TrackPopupMenuEx(
                hmenu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                pt.x,
                pt.y,
                owner.raw(),
                None,
            );
            if cmd.0 > 0 {
                let info = CMINVOKECOMMANDINFO {
                    cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                    hwnd: HWND(owner.0 as *mut c_void),
                    // MAKEINTRESOURCE(cmd - idCmdFirst)
                    lpVerb: PCSTR(std::ptr::without_provenance::<u8>((cmd.0 - 1) as usize)),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                invoked = menu.InvokeCommand(&info).is_ok();
            }
        }
        let _ = DestroyMenu(hmenu);
        invoked
    }
}

/// Folder picker. Returns the chosen file-system path.
pub fn pick_folder(owner: Hwnd) -> Option<String> {
    // SAFETY: in-process common dialog object.
    let dlg: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.ok()?;
    // SAFETY: plain COM calls; Show runs a modal loop on this thread.
    unsafe {
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        dlg.Show(Some(owner.raw())).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// File type filter of the open and save dialogs: (label, pattern).
pub type FileFilter<'a> = (&'a str, &'a str);

fn set_filter(dlg: &windows::Win32::UI::Shell::IFileDialog, filter: FileFilter<'_>) -> Option<()> {
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    let name = wide(filter.0);
    let spec = wide(filter.1);
    let specs = [COMDLG_FILTERSPEC {
        pszName: pcwstr(&name),
        pszSpec: pcwstr(&spec),
    }];
    // SAFETY: the dialog copies the strings, which outlive the call.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifiledialog-setfiletypes
    unsafe { dlg.SetFileTypes(&specs) }.ok()
}

fn dialog_result(dlg: &windows::Win32::UI::Shell::IFileDialog, owner: Hwnd) -> Option<String> {
    // SAFETY: Show runs a modal loop on this thread; the result string is
    // freed with the COM allocator.
    // https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifiledialog-getresult
    unsafe {
        dlg.Show(Some(owner.raw())).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// "Open" dialog for one file. Returns the chosen path.
pub fn pick_open_file(owner: Hwnd, filter: FileFilter<'_>) -> Option<String> {
    // SAFETY: in-process common dialog object.
    let dlg: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let base: windows::Win32::UI::Shell::IFileDialog = dlg.cast().ok()?;
    // SAFETY: plain COM calls.
    unsafe {
        let opts = base.GetOptions().ok()?;
        base.SetOptions(opts | FOS_FORCEFILESYSTEM | windows::Win32::UI::Shell::FOS_FILEMUSTEXIST)
            .ok()?;
    }
    set_filter(&base, filter)?;
    dialog_result(&base, owner)
}

/// "Save as" dialog. `default_name` is the proposed file name and
/// `extension` the extension added when the user types none.
pub fn pick_save_file(
    owner: Hwnd,
    filter: FileFilter<'_>,
    default_name: &str,
    extension: &str,
) -> Option<String> {
    use windows::Win32::UI::Shell::{FOS_OVERWRITEPROMPT, FileSaveDialog, IFileSaveDialog};
    // SAFETY: in-process common dialog object.
    let dlg: IFileSaveDialog =
        unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let base: windows::Win32::UI::Shell::IFileDialog = dlg.cast().ok()?;
    let name = wide(default_name);
    let ext = wide(extension);
    // SAFETY: plain COM calls; the strings outlive them.
    unsafe {
        let opts = base.GetOptions().ok()?;
        base.SetOptions(opts | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)
            .ok()?;
        base.SetFileName(pcwstr(&name)).ok()?;
        base.SetDefaultExtension(pcwstr(&ext)).ok()?;
    }
    set_filter(&base, filter)?;
    dialog_result(&base, owner)
}

/// Copies or moves files into `dest` with the shell's own file operation
/// (progress window, conflict questions, undo). Runs on a short-lived
/// thread so that a long copy never blocks the desktop. False if the
/// thread could not start.
/// <https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation>
pub fn transfer(paths: Vec<String>, dest: String, copy: bool) -> bool {
    std::thread::Builder::new()
        .name("bento-fileop".into())
        .spawn(move || {
            use windows::Win32::UI::Shell::{FOF_ALLOWUNDO, FileOperation, IFileOperation};
            let Ok(_com) = crate::com::ComApartment::sta() else {
                return;
            };
            let Some(target) = shell_item(&dest) else {
                return;
            };
            // SAFETY: in-process shell object on this STA thread.
            let Ok(op) = (unsafe {
                CoCreateInstance::<_, IFileOperation>(&FileOperation, None, CLSCTX_INPROC_SERVER)
            }) else {
                return;
            };
            // SAFETY: plain COM calls with live items.
            unsafe {
                let _ = op.SetOperationFlags(FOF_ALLOWUNDO);
                for p in &paths {
                    let Some(item) = shell_item(p) else { continue };
                    let _ = if copy {
                        op.CopyItem(&item, &target, windows::core::PCWSTR::null(), None)
                    } else {
                        op.MoveItem(&item, &target, windows::core::PCWSTR::null(), None)
                    };
                }
                let _ = op.PerformOperations();
            }
        })
        .is_ok()
}
