//! Foreground tracking and the low-level mouse hook.
//!
//! - [`ForegroundHook`] uses `SetWinEventHook` with `WINEVENT_OUTOFCONTEXT`:
//!   events are delivered to this process, no DLL is loaded anywhere else.
//!   <https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwineventhook>
//! - [`MouseHookThread`] owns a `WH_MOUSE_LL` hook on a dedicated thread.
//!   The hook procedure runs in this process; the application installs it
//!   only while the desktop is in the foreground, so it never sees game
//!   input. It must answer fast: Windows silently removes a low-level hook
//!   that exceeds `LowLevelHooksTimeout`.
//!   <https://learn.microsoft.com/windows/win32/winmsg/lowlevelmouseproc>

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::sync::{OnceLock, RwLock, mpsc};
use std::thread::JoinHandle;

use bento_core::geometry::{Point, Rect};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging as wm;

use crate::error::{Context, Result, WinError};
use crate::window::Hwnd;

static FOREGROUND_CB: OnceLock<fn(Hwnd)> = OnceLock::new();

unsafe extern "system" fn foreground_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _obj: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some(cb) = FOREGROUND_CB.get() {
        cb(Hwnd::from_raw(hwnd));
    }
}

/// Out-of-context hook on `EVENT_SYSTEM_FOREGROUND`, removed on drop.
/// The callback runs on the thread that installed the hook, from its
/// message loop.
#[derive(Debug)]
pub struct ForegroundHook(HWINEVENTHOOK);

impl ForegroundHook {
    pub fn install(cb: fn(Hwnd)) -> Result<ForegroundHook> {
        let _ = FOREGROUND_CB.set(cb);
        // SAFETY: out-of-context hook (no module), static callback.
        let h = unsafe {
            SetWinEventHook(
                wm::EVENT_SYSTEM_FOREGROUND,
                wm::EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_proc),
                0,
                0,
                wm::WINEVENT_OUTOFCONTEXT,
            )
        };
        if h.is_invalid() {
            return Err(WinError::Other("SetWinEventHook failed".into()));
        }
        Ok(ForegroundHook(h))
    }
}

impl Drop for ForegroundHook {
    fn drop(&mut self) {
        // SAFETY: handle returned by SetWinEventHook.
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
    }
}

/// Events forwarded by the mouse hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEvent {
    LeftDown,
    LeftUp,
    /// Right button pressed on an empty desktop area (swallowed).
    RightDown,
    RightMove,
    /// End of a swallowed right-button drag.
    RightUp,
    /// Wheel over a scroll zone (swallowed), delta in WHEEL_DELTA units.
    Wheel(i16),
    /// Cursor entered hover zone `n`, or left all zones (`None`).
    Hover(Option<u32>),
}

impl MouseEvent {
    fn encode(self) -> usize {
        match self {
            MouseEvent::LeftDown => 1,
            MouseEvent::LeftUp => 2,
            MouseEvent::RightDown => 3,
            MouseEvent::RightMove => 4,
            MouseEvent::RightUp => 5,
            MouseEvent::Wheel(d) => 6 | ((d as u16 as usize) << 16),
            MouseEvent::Hover(z) => 7 | ((z.map(|z| z as usize + 1).unwrap_or(0)) << 16),
        }
    }

    /// Decodes the `wparam` of a hook notification.
    pub fn decode(wparam: usize) -> Option<MouseEvent> {
        let hi = wparam >> 16;
        Some(match wparam & 0xFF {
            1 => MouseEvent::LeftDown,
            2 => MouseEvent::LeftUp,
            3 => MouseEvent::RightDown,
            4 => MouseEvent::RightMove,
            5 => MouseEvent::RightUp,
            6 => MouseEvent::Wheel((hi & 0xFFFF) as u16 as i16),
            7 => MouseEvent::Hover(if hi == 0 {
                None
            } else {
                u32::try_from(hi - 1).ok()
            }),
            _ => return None,
        })
    }
}

/// Packs a screen point in an `LPARAM` (64-bit).
pub fn pack_point(p: Point) -> isize {
    ((p.x as u32 as u64) | ((p.y as u32 as u64) << 32)) as isize
}

/// Inverse of [`pack_point`].
pub fn unpack_point(lp: isize) -> Point {
    let v = lp as u64;
    Point::new((v & 0xFFFF_FFFF) as u32 as i32, (v >> 32) as u32 as i32)
}

/// Geometry the hook needs to decide what to do with an event.
#[derive(Clone, Debug, Default)]
pub struct HitData {
    pub listview: Hwnd,
    /// Rectangles of every visible icon (screen coordinates).
    pub icons: Vec<Rect>,
    /// Fence bodies where the wheel scrolls.
    pub wheel_zones: Vec<Rect>,
    /// Zones reported by `MouseEvent::Hover` (fence bodies).
    pub hover_zones: Vec<Rect>,
    /// Right-drag on empty desktop draws a fence.
    pub draw_gesture: bool,
}

static HIT: RwLock<Option<HitData>> = RwLock::new(None);
static TARGET: AtomicIsize = AtomicIsize::new(0);
static TARGET_MSG: AtomicU32 = AtomicU32::new(0);
static RIGHT_DRAG: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicI32 = AtomicI32::new(-1);

/// Replaces the hit-test data used by the hook.
pub fn set_hit_data(data: HitData) {
    if let Ok(mut g) = HIT.write() {
        *g = Some(data);
    }
}

fn post(ev: MouseEvent, p: Point) {
    let target = Hwnd(TARGET.load(Ordering::Relaxed));
    target.post(
        TARGET_MSG.load(Ordering::Relaxed),
        ev.encode(),
        pack_point(p),
    );
}

fn over_listview(hit: &HitData, p: Point) -> bool {
    // SAFETY: WindowFromPoint takes a plain value.
    let w = Hwnd::from_raw(unsafe { wm::WindowFromPoint(POINT { x: p.x, y: p.y }) });
    !hit.listview.is_null() && w == hit.listview
}

/// Returns `true` when the event must be swallowed.
fn handle(msg: u32, p: Point, data: u32) -> bool {
    // Never block: if the UI thread is updating the data, let it pass.
    let Ok(guard) = HIT.try_read() else {
        return false;
    };
    let Some(hit) = guard.as_ref() else {
        return false;
    };
    match msg {
        wm::WM_MOUSEMOVE => {
            if RIGHT_DRAG.load(Ordering::Relaxed) {
                post(MouseEvent::RightMove, p);
            }
            let zone = hit
                .hover_zones
                .iter()
                .position(|r| r.contains(p))
                .and_then(|i| i32::try_from(i).ok())
                .unwrap_or(-1);
            if HOVER.swap(zone, Ordering::Relaxed) != zone {
                post(MouseEvent::Hover(u32::try_from(zone).ok()), p);
            }
            false
        }
        wm::WM_RBUTTONDOWN => {
            if hit.draw_gesture && over_listview(hit, p) && !hit.icons.iter().any(|r| r.contains(p))
            {
                RIGHT_DRAG.store(true, Ordering::Relaxed);
                post(MouseEvent::RightDown, p);
                return true;
            }
            false
        }
        wm::WM_RBUTTONUP => {
            if RIGHT_DRAG.swap(false, Ordering::Relaxed) {
                post(MouseEvent::RightUp, p);
                return true;
            }
            false
        }
        wm::WM_LBUTTONDOWN => {
            if over_listview(hit, p) {
                post(MouseEvent::LeftDown, p);
            }
            false
        }
        wm::WM_LBUTTONUP => {
            post(MouseEvent::LeftUp, p);
            false
        }
        wm::WM_MOUSEWHEEL => {
            if over_listview(hit, p) && hit.wheel_zones.iter().any(|r| r.contains(p)) {
                let delta = ((data >> 16) & 0xFFFF) as u16 as i16;
                post(MouseEvent::Wheel(delta), p);
                return true;
            }
            false
        }
        _ => false,
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && lp.0 != 0 {
        // SAFETY: for WH_MOUSE_LL, lParam points to an MSLLHOOKSTRUCT valid
        // during the call.
        let info = unsafe { &*(lp.0 as *const wm::MSLLHOOKSTRUCT) };
        let p = Point::new(info.pt.x, info.pt.y);
        if handle(wp.0 as u32, p, info.mouseData) {
            return LRESULT(1);
        }
    }
    // SAFETY: forwarding the unchanged arguments.
    unsafe { wm::CallNextHookEx(None, code, wp, lp) }
}

const CMD_INSTALL: u32 = wm::WM_APP + 1;
const CMD_REMOVE: u32 = wm::WM_APP + 2;

/// Thread that installs and removes the low-level mouse hook on demand.
#[derive(Debug)]
pub struct MouseHookThread {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl MouseHookThread {
    /// Starts the thread. Events are posted to `target` as `msg` with the
    /// encoded [`MouseEvent`] in wparam and the packed point in lparam.
    pub fn spawn(target: Hwnd, msg: u32) -> Result<MouseHookThread> {
        TARGET.store(target.0, Ordering::Relaxed);
        TARGET_MSG.store(msg, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("mouse-hook".into())
            .spawn(move || hook_thread(tx))
            .map_err(|e| WinError::Other(format!("spawn hook thread: {e}")))?;
        let thread_id = rx
            .recv()
            .map_err(|_| WinError::Other("hook thread did not start".into()))?;
        Ok(MouseHookThread {
            thread_id,
            join: Some(join),
        })
    }

    fn command(&self, cmd: u32) {
        // SAFETY: posting a plain message to our own thread.
        let _ = unsafe { wm::PostThreadMessageW(self.thread_id, cmd, WPARAM(0), LPARAM(0)) };
    }

    /// Installs the hook (no effect if already installed).
    pub fn enable(&self) {
        self.command(CMD_INSTALL);
    }

    /// Removes the hook (no effect if not installed).
    pub fn disable(&self) {
        RIGHT_DRAG.store(false, Ordering::Relaxed);
        self.command(CMD_REMOVE);
    }
}

impl Drop for MouseHookThread {
    fn drop(&mut self) {
        self.command(wm::WM_QUIT);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct HookHandle(wm::HHOOK);

impl Drop for HookHandle {
    fn drop(&mut self) {
        // SAFETY: handle returned by SetWindowsHookExW on this thread.
        let _ = unsafe { wm::UnhookWindowsHookEx(self.0) };
    }
}

fn hook_thread(ready: mpsc::Sender<u32>) {
    let mut msg = wm::MSG::default();
    // SAFETY: PeekMessage creates the message queue of this thread before
    // anyone posts to it.
    unsafe {
        let _ = wm::PeekMessageW(&mut msg, None, 0, 0, wm::PM_NOREMOVE);
    }
    // SAFETY: no argument.
    let _ = ready.send(unsafe { GetCurrentThreadId() });
    let mut hook: Option<HookHandle> = None;
    loop {
        // SAFETY: valid out pointer; thread messages included.
        let r = unsafe { wm::GetMessageW(&mut msg, None, 0, 0) };
        if r.0 <= 0 {
            break;
        }
        match msg.message {
            CMD_INSTALL if hook.is_none() => {
                // SAFETY: low-level hooks take no module and no thread id;
                // the procedure is a static function of this process.
                // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwindowshookexw
                match unsafe { wm::SetWindowsHookExW(wm::WH_MOUSE_LL, Some(mouse_proc), None, 0) }
                    .ctx("SetWindowsHookExW(WH_MOUSE_LL)")
                {
                    Ok(h) => hook = Some(HookHandle(h)),
                    Err(e) => tracing::warn!("{e}"),
                }
            }
            CMD_REMOVE => {
                hook = None;
                HOVER.store(-1, Ordering::Relaxed);
            }
            _ => {
                // SAFETY: message filled by GetMessageW.
                unsafe {
                    wm::DispatchMessageW(&msg);
                }
            }
        }
    }
    drop(hook);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_round_trip() {
        for ev in [
            MouseEvent::LeftDown,
            MouseEvent::RightUp,
            MouseEvent::Wheel(-120),
            MouseEvent::Wheel(240),
            MouseEvent::Hover(None),
            MouseEvent::Hover(Some(3)),
        ] {
            assert_eq!(MouseEvent::decode(ev.encode()), Some(ev));
        }
    }

    #[test]
    fn point_round_trip() {
        for p in [
            Point::new(-1920, 5),
            Point::new(3000, -200),
            Point::new(0, 0),
        ] {
            assert_eq!(unpack_point(pack_point(p)), p);
        }
    }
}
