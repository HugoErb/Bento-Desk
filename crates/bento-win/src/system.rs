//! Registry, theme, language, single instance, processes, IPC.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
    ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW, WM_COPYDATA,
};
use windows::core::{PCWSTR, PWSTR};

use crate::error::{Result, WinError};
use crate::util::{pcwstr, wide};
use crate::window::Hwnd;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let k = wide(subkey);
    let v = wide(value);
    let mut size = 0u32;
    // SAFETY: first call queries the size only.
    // https://learn.microsoft.com/windows/win32/api/winreg/nf-winreg-reggetvaluew
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            pcwstr(&k),
            pcwstr(&v),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if r.is_err() || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: `buf` holds `size` bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            pcwstr(&k),
            pcwstr(&v),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    r.is_ok().then(|| crate::util::from_wide(&buf))
}

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let k = wide(subkey);
    let v = wide(value);
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: `data` holds 4 bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            pcwstr(&k),
            pcwstr(&v),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    r.is_ok().then_some(data)
}

/// Command line registered under the Run key, if any.
pub fn autostart_command(name: &str) -> Option<String> {
    reg_string(RUN_KEY, name)
}

/// Adds or removes the `HKCU\...\Run` entry.
pub fn set_autostart(name: &str, command: Option<&str>) -> Result<()> {
    let k = wide(RUN_KEY);
    let n = wide(name);
    match command {
        Some(cmd) => {
            let data = wide(cmd);
            // SAFETY: `data` is a NUL-terminated UTF-16 string of the given
            // byte size.
            let r = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    pcwstr(&k),
                    pcwstr(&n),
                    REG_SZ.0,
                    Some(data.as_ptr() as *const c_void),
                    u32::try_from(data.len() * 2).unwrap_or(0),
                )
            };
            if r.is_err() {
                return Err(WinError::Other(format!("RegSetKeyValueW: {r:?}")));
            }
        }
        None => {
            // SAFETY: plain strings.
            let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, pcwstr(&k), pcwstr(&n)) };
        }
    }
    Ok(())
}

/// True when apps use the light theme (default when the value is missing).
pub fn apps_use_light_theme() -> bool {
    reg_dword(PERSONALIZE_KEY, "AppsUseLightTheme").is_none_or(|v| v != 0)
}

/// True when Explorer hides known file extensions.
pub fn hide_known_extensions() -> bool {
    reg_dword(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
        "HideFileExt",
    )
    .is_none_or(|v| v != 0)
}

/// Windows UI language id.
pub fn ui_language() -> u16 {
    // SAFETY: no argument.
    unsafe { GetUserDefaultUILanguage() }
}

/// Milliseconds since boot.
pub fn tick_ms() -> u64 {
    // SAFETY: no argument.
    unsafe { GetTickCount64() }
}

/// Named mutex held for the whole life of the process.
#[derive(Debug)]
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// `Ok(Some(_))` when this is the first instance, `Ok(None)` otherwise.
    pub fn acquire(name: &str) -> Result<Option<SingleInstance>> {
        let n = wide(name);
        // SAFETY: default security, name buffer outlives the call.
        // https://learn.microsoft.com/windows/win32/api/synchapi/nf-synchapi-createmutexw
        let h = unsafe { CreateMutexW(None, false, pcwstr(&n)) }.map_err(|e| WinError::Api {
            context: "CreateMutexW",
            source: e,
        })?;
        // SAFETY: reads the error code set by CreateMutexW.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: valid handle we own.
            let _ = unsafe { CloseHandle(h) };
            return Ok(None);
        }
        Ok(Some(SingleInstance(h)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: handle owned by this struct.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// True when the user runs a full-screen D3D app, a presentation or is
/// otherwise busy (quiet hours do not count).
pub fn fullscreen_app_running() -> bool {
    // SAFETY: no argument.
    // https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_BUSY || s == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}

/// File name of the executable owning a window (`game.exe`). Uses
/// `PROCESS_QUERY_LIMITED_INFORMATION`, the minimum access right.
pub fn process_name_of(hwnd: Hwnd) -> Option<String> {
    let pid = hwnd.process_id();
    if pid == 0 {
        return None;
    }
    // SAFETY: plain values; the handle is closed below.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = vec![0u16; 1024];
    let mut len = u32::try_from(buf.len()).unwrap_or(0);
    // SAFETY: `buf` holds `len` characters.
    let r = unsafe {
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    };
    // SAFETY: handle returned by OpenProcess.
    let _ = unsafe { CloseHandle(h) };
    r.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().map(str::to_owned)
}

/// Opens a file, folder or URL with its default handler.
pub fn shell_open(target: &str) -> bool {
    shell_verb(None, target, None)
}

/// Opens a folder in a new File Explorer window.
pub fn open_in_explorer(path: &str) -> bool {
    shell_verb(Some("explore"), path, None)
}

/// Starts an executable with arguments.
pub fn spawn(exe: &str, args: &str) -> bool {
    shell_verb(None, exe, Some(args))
}

fn shell_verb(verb: Option<&str>, file: &str, args: Option<&str>) -> bool {
    let v = verb.map(wide);
    let f = wide(file);
    let a = args.map(wide);
    // SAFETY: buffers outlive the call.
    // https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shellexecutew
    let r = unsafe {
        ShellExecuteW(
            None,
            v.as_deref().map(pcwstr).unwrap_or(PCWSTR::null()),
            pcwstr(&f),
            a.as_deref().map(pcwstr).unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    r.0 as isize > 32
}

/// File-system paths of the user and public desktop folders.
pub fn desktop_folders() -> Vec<PathBuf> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_PublicDesktop, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
    };
    [FOLDERID_Desktop, FOLDERID_PublicDesktop]
        .iter()
        .filter_map(|id| {
            // SAFETY: valid GUID; the string is freed after copy.
            // https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath
            unsafe {
                let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
                let s = p.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                s.map(PathBuf::from)
            }
        })
        .collect()
}

/// Tells the shell that a file appeared, so that Explorer refreshes its
/// view. Needed for files written by programs that bypass the shell on
/// some redirected (OneDrive) desktops.
/// <https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotify>
pub fn shell_notify_created(path: &str, is_dir: bool) {
    use windows::Win32::UI::Shell::{
        SHCNE_CREATE, SHCNE_MKDIR, SHCNF_FLUSHNOWAIT, SHCNF_PATHW, SHChangeNotify,
    };
    let w = wide(path);
    let ev = if is_dir { SHCNE_MKDIR } else { SHCNE_CREATE };
    // SAFETY: NUL-terminated path valid during the call; the shell copies it.
    unsafe {
        SHChangeNotify(
            ev,
            SHCNF_PATHW | SHCNF_FLUSHNOWAIT,
            Some(w.as_ptr() as *const c_void),
            None,
        );
    }
}

/// Tells the shell that a file disappeared (see [`shell_notify_created`]).
pub fn shell_notify_deleted(path: &str, is_dir: bool) {
    use windows::Win32::UI::Shell::{
        SHCNE_DELETE, SHCNE_RMDIR, SHCNF_FLUSHNOWAIT, SHCNF_PATHW, SHChangeNotify,
    };
    let w = wide(path);
    let ev = if is_dir { SHCNE_RMDIR } else { SHCNE_DELETE };
    // SAFETY: as in `shell_notify_created`.
    unsafe {
        SHChangeNotify(
            ev,
            SHCNF_PATHW | SHCNF_FLUSHNOWAIT,
            Some(w.as_ptr() as *const c_void),
            None,
        );
    }
}

/// Path of the running executable.
pub fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Sends bytes to a window of another process with WM_COPYDATA.
pub fn send_copydata(target: Hwnd, kind: usize, data: &[u8]) -> bool {
    let cds = COPYDATASTRUCT {
        dwData: kind,
        cbData: u32::try_from(data.len()).unwrap_or(0),
        lpData: data.as_ptr() as *mut c_void,
    };
    let mut result = 0usize;
    // SAFETY: WM_COPYDATA marshals `cds` and the buffer to the target
    // process; both outlive the synchronous call.
    // https://learn.microsoft.com/windows/win32/dataxchg/wm-copydata
    let r = unsafe {
        SendMessageTimeoutW(
            target.raw(),
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&cds as *const COPYDATASTRUCT as isize),
            SMTO_ABORTIFHUNG,
            3000,
            Some(&mut result),
        )
    };
    r.0 != 0
}

/// Reads the payload of a received WM_COPYDATA (`lparam`).
pub fn read_copydata(lparam: isize) -> Option<(usize, Vec<u8>)> {
    if lparam == 0 {
        return None;
    }
    // SAFETY: for WM_COPYDATA, lparam points to a COPYDATASTRUCT valid
    // until the window procedure returns; we copy the bytes out.
    unsafe {
        let cds = &*(lparam as *const COPYDATASTRUCT);
        if cds.lpData.is_null() || cds.cbData == 0 {
            return Some((cds.dwData, Vec::new()));
        }
        let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
        Some((cds.dwData, bytes.to_vec()))
    }
}
