//! Monitors: rectangles, work areas, DPI and stable identities.

use bento_core::display::MonitorInfo;
use bento_core::geometry::Rect;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

use crate::util::from_wide;

/// GDI device name (`\\.\DISPLAY1`) to (device path, friendly name).
fn display_config_names() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut n_paths = 0u32;
    let mut n_modes = 0u32;
    // SAFETY: out pointers are valid.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-querydisplayconfig
    unsafe {
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes).is_err() {
            return out;
        }
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
    // SAFETY: buffers sized from GetDisplayConfigBufferSizes.
    let r = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            None,
        )
    };
    if r.is_err() {
        return out;
    }
    paths.truncate(n_paths as usize);
    for p in &paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source.header.adapterId = p.sourceInfo.adapterId;
        source.header.id = p.sourceInfo.id;
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        target.header.adapterId = p.targetInfo.adapterId;
        target.header.id = p.targetInfo.id;
        // SAFETY: headers describe the structures they start.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-displayconfiggetdeviceinfo
        let ok = unsafe {
            DisplayConfigGetDeviceInfo(&mut source.header) == 0
                && DisplayConfigGetDeviceInfo(&mut target.header) == 0
        };
        if ok {
            out.push((
                from_wide(&source.viewGdiDeviceName),
                from_wide(&target.monitorDevicePath),
                from_wide(&target.monitorFriendlyDeviceName),
            ));
        }
    }
    out
}

/// All active monitors.
pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn cb(h: HMONITOR, _dc: HDC, _r: *mut RECT, lp: LPARAM) -> BOOL {
        // SAFETY: `lp` is the address of the Vec below, alive during the
        // enumeration.
        let list = unsafe { &mut *(lp.0 as *mut Vec<HMONITOR>) };
        list.push(h);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: callback only pushes into `handles`.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-enumdisplaymonitors
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(cb),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    let names = display_config_names();
    handles
        .into_iter()
        .filter_map(|h| {
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: MONITORINFOEXW starts with MONITORINFO and cbSize says
            // which variant is passed.
            let ok =
                unsafe { GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) };
            if !ok.as_bool() {
                return None;
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            // SAFETY: valid monitor handle and out pointers.
            let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
            let gdi = from_wide(&mi.szDevice);
            let (id, name) = names
                .iter()
                .find(|(g, _, _)| *g == gdi)
                .map(|(_, path, friendly)| {
                    let id = if path.is_empty() {
                        gdi.clone()
                    } else {
                        path.clone()
                    };
                    let name = if friendly.is_empty() {
                        gdi.clone()
                    } else {
                        friendly.clone()
                    };
                    (id, name)
                })
                .unwrap_or_else(|| (gdi.clone(), gdi.clone()));
            let r = mi.monitorInfo.rcMonitor;
            let w = mi.monitorInfo.rcWork;
            Some(MonitorInfo {
                id,
                name,
                rect: Rect::new(r.left, r.top, r.right, r.bottom),
                work: Rect::new(w.left, w.top, w.right, w.bottom),
                dpi: dx.max(96),
                primary: mi.monitorInfo.dwFlags & 1 != 0,
            })
        })
        .collect()
}
