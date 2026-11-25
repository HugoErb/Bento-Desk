//! Windows of the app: class registration, creation, messages and timers.
//!
//! Every window of the process uses one window procedure that forwards the
//! message to a single dispatcher function set by the application. The
//! application decides what to do from the `HWND`; when the dispatcher
//! returns `None`, `DefWindowProcW` handles the message.

use std::ffi::c_void;
use std::sync::OnceLock;

use bento_core::geometry::{Point, Rect};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject, FW_NORMAL,
    HFONT, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging as wm;
use windows::core::PCWSTR;

use crate::error::{Context, Result, last_error};
use crate::util::{from_wide, pcwstr, wide};

/// Copyable, `Send` window handle. Only meaningful inside this process
/// (or as a target of messages for windows of other processes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hwnd(pub isize);

impl Hwnd {
    pub const NULL: Hwnd = Hwnd(0);

    pub(crate) fn raw(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }

    pub(crate) fn from_raw(h: HWND) -> Hwnd {
        Hwnd(h.0 as isize)
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    /// True if the handle still identifies an existing window.
    pub fn exists(self) -> bool {
        // SAFETY: IsWindow accepts any value, including stale handles.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-iswindow
        !self.is_null() && unsafe { wm::IsWindow(Some(self.raw())) }.as_bool()
    }

    pub fn is_visible(self) -> bool {
        // SAFETY: IsWindowVisible accepts any handle.
        unsafe { wm::IsWindowVisible(self.raw()) }.as_bool()
    }

    /// Window class name, empty on failure.
    pub fn class_name(self) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the buffer is valid for its whole length.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getclassnamew
        let n = unsafe { wm::GetClassNameW(self.raw(), &mut buf) };
        from_wide(&buf[..usize::try_from(n).unwrap_or(0).min(buf.len())])
    }

    /// Window rectangle in screen coordinates.
    pub fn rect(self) -> Option<Rect> {
        let mut r = RECT::default();
        // SAFETY: `r` is a valid out pointer.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getwindowrect
        unsafe { wm::GetWindowRect(self.raw(), &mut r) }.ok()?;
        Some(Rect::new(r.left, r.top, r.right, r.bottom))
    }

    /// Screen position of the client-area origin.
    pub fn client_origin(self) -> Point {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: `p` is a valid in/out pointer. ScreenToClient of (0,0)
        // gives minus the client origin.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-screentoclient
        let _ = unsafe { ScreenToClient(self.raw(), &mut p) };
        Point::new(-p.x, -p.y)
    }

    /// Process id owning the window.
    pub fn process_id(self) -> u32 {
        let mut pid = 0u32;
        // SAFETY: `pid` is a valid out pointer.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid
        unsafe { wm::GetWindowThreadProcessId(self.raw(), Some(&mut pid)) };
        pid
    }

    /// Thread id owning the window.
    pub fn thread_id(self) -> u32 {
        // SAFETY: no out pointer.
        unsafe { wm::GetWindowThreadProcessId(self.raw(), None) }
    }

    pub fn parent(self) -> Option<Hwnd> {
        // SAFETY: GetParent accepts any handle.
        let h = unsafe { wm::GetParent(self.raw()) }.ok()?;
        (!h.is_invalid()).then(|| Hwnd::from_raw(h))
    }

    /// Top-level ancestor (the window itself for top-level windows).
    pub fn root(self) -> Hwnd {
        // SAFETY: GetAncestor accepts any handle.
        Hwnd::from_raw(unsafe { wm::GetAncestor(self.raw(), wm::GA_ROOT) })
    }

    /// Moves and resizes without changing the z-order or activation.
    pub fn set_rect(self, r: Rect) {
        // SAFETY: plain values.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwindowpos
        let _ = unsafe {
            wm::SetWindowPos(
                self.raw(),
                None,
                r.left,
                r.top,
                r.width(),
                r.height(),
                wm::SWP_NOZORDER | wm::SWP_NOACTIVATE | wm::SWP_NOOWNERZORDER,
            )
        };
    }

    /// Places the window just below `after` in the z-order.
    pub fn set_z_after(self, after: Hwnd) -> Result<()> {
        // SAFETY: plain values.
        unsafe {
            wm::SetWindowPos(
                self.raw(),
                Some(after.raw()),
                0,
                0,
                0,
                0,
                wm::SWP_NOMOVE | wm::SWP_NOSIZE | wm::SWP_NOACTIVATE,
            )
        }
        .ctx("SetWindowPos(after)")
    }

    /// Places the window at the top of its siblings.
    pub fn set_z_top(self) {
        // SAFETY: plain values.
        let _ = unsafe {
            wm::SetWindowPos(
                self.raw(),
                Some(wm::HWND_TOP),
                0,
                0,
                0,
                0,
                wm::SWP_NOMOVE | wm::SWP_NOSIZE | wm::SWP_NOACTIVATE,
            )
        };
    }

    /// Places a top-level window above all non-topmost windows.
    pub fn set_topmost(self, topmost: bool) {
        let after = if topmost {
            wm::HWND_TOPMOST
        } else {
            wm::HWND_NOTOPMOST
        };
        // SAFETY: plain values.
        let _ = unsafe {
            wm::SetWindowPos(
                self.raw(),
                Some(after),
                0,
                0,
                0,
                0,
                wm::SWP_NOMOVE | wm::SWP_NOSIZE | wm::SWP_NOACTIVATE,
            )
        };
    }

    /// Shows without activating, or hides.
    pub fn show(self, visible: bool) {
        let cmd = if visible {
            wm::SW_SHOWNOACTIVATE
        } else {
            wm::SW_HIDE
        };
        // SAFETY: plain values.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-showwindow
        let _ = unsafe { wm::ShowWindow(self.raw(), cmd) };
    }

    /// Shows and activates.
    pub fn show_active(self) {
        // SAFETY: plain values.
        let _ = unsafe { wm::ShowWindow(self.raw(), wm::SW_SHOW) };
    }

    pub fn minimize(self) {
        // SAFETY: plain values.
        let _ = unsafe { wm::ShowWindow(self.raw(), wm::SW_SHOWMINNOACTIVE) };
    }

    /// Requests a WM_PAINT.
    pub fn invalidate(self) {
        // SAFETY: NULL rect means the whole client area.
        let _ = unsafe { InvalidateRect(Some(self.raw()), None, true) };
    }

    /// Posts a message to this window's queue.
    pub fn post(self, msg: u32, wparam: usize, lparam: isize) -> bool {
        // SAFETY: posting copies the plain values, no pointer is shared.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-postmessagew
        unsafe { wm::PostMessageW(Some(self.raw()), msg, WPARAM(wparam), LPARAM(lparam)) }.is_ok()
    }

    /// Sends a message with a timeout. Only plain values, no pointers.
    pub fn send_timeout(self, msg: u32, wparam: usize, lparam: isize, ms: u32) -> Option<usize> {
        let mut result = 0usize;
        // SAFETY: `result` is a valid out pointer, wparam/lparam carry
        // integers only.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw
        let r = unsafe {
            wm::SendMessageTimeoutW(
                self.raw(),
                msg,
                WPARAM(wparam),
                LPARAM(lparam),
                wm::SMTO_NORMAL | wm::SMTO_ABORTIFHUNG,
                ms,
                Some(&mut result),
            )
        };
        (r.0 != 0).then_some(result)
    }

    pub fn set_capture(self) {
        // SAFETY: plain value.
        unsafe { SetCapture(self.raw()) };
    }

    pub fn set_focus(self) {
        // SAFETY: plain value.
        let _ = unsafe { SetFocus(Some(self.raw())) };
    }

    /// Asks for activation. Windows may refuse when another app has focus.
    pub fn set_foreground(self) -> bool {
        // SAFETY: plain value.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setforegroundwindow
        unsafe { wm::SetForegroundWindow(self.raw()) }.as_bool()
    }

    /// Requests a WM_MOUSELEAVE when the cursor leaves the window.
    pub fn track_mouse_leave(self) {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.raw(),
            dwHoverTime: 0,
        };
        // SAFETY: `tme` is fully initialized.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-trackmouseevent
        let _ = unsafe { TrackMouseEvent(&mut tme) };
    }

    /// Starts a timer delivering WM_TIMER with `id`.
    pub fn set_timer(self, id: usize, ms: u32) {
        // SAFETY: no callback, the timer posts WM_TIMER to this window.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-settimer
        unsafe { wm::SetTimer(Some(self.raw()), id, ms, None) };
    }

    pub fn kill_timer(self, id: usize) {
        // SAFETY: killing an unknown timer is harmless.
        let _ = unsafe { wm::KillTimer(Some(self.raw()), id) };
    }

    /// Window text (used for the inline edit box).
    pub fn text(self) -> String {
        let mut buf = vec![0u16; 1024];
        // SAFETY: the buffer is valid for its length.
        let n = unsafe { wm::GetWindowTextW(self.raw(), &mut buf) };
        from_wide(&buf[..usize::try_from(n).unwrap_or(0).min(buf.len())])
    }

    pub fn set_text(self, text: &str) {
        let w = wide(text);
        // SAFETY: `w` is NUL terminated and outlives the call.
        let _ = unsafe { wm::SetWindowTextW(self.raw(), pcwstr(&w)) };
    }

    /// Destroys a window owned by this thread.
    pub fn destroy(self) {
        if self.exists() {
            // SAFETY: DestroyWindow on a valid handle of this thread.
            let _ = unsafe { wm::DestroyWindow(self.raw()) };
        }
    }
}

/// Releases mouse capture.
pub fn release_capture() {
    // SAFETY: no argument.
    let _ = unsafe { ReleaseCapture() };
}

/// Message given to the dispatcher.
#[derive(Clone, Copy, Debug)]
pub struct Message {
    pub hwnd: Hwnd,
    pub msg: u32,
    pub wparam: usize,
    pub lparam: isize,
}

impl Message {
    /// Client coordinates packed in `lparam` (mouse messages).
    pub fn point(&self) -> Point {
        lparam_point(self.lparam)
    }

    /// Screen coordinates packed in `lparam` (WM_MOUSEWHEEL, WM_CONTEXTMENU).
    pub fn wheel_delta(&self) -> i16 {
        ((self.wparam >> 16) & 0xFFFF) as u16 as i16
    }
}

/// Signed x/y packed in the low 32 bits of an `LPARAM`.
pub fn lparam_point(lp: isize) -> Point {
    let x = (lp & 0xFFFF) as u16 as i16;
    let y = ((lp >> 16) & 0xFFFF) as u16 as i16;
    Point::new(i32::from(x), i32::from(y))
}

/// Application callback for every message of every window.
pub type Dispatcher = fn(&Message) -> Option<isize>;

static DISPATCHER: OnceLock<Dispatcher> = OnceLock::new();

/// Installs the dispatcher. Only the first call has an effect.
pub fn set_dispatcher(d: Dispatcher) {
    let _ = DISPATCHER.set(d);
}

/// Window procedure shared by all classes of the app.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(d) = DISPATCHER.get() {
        let m = Message {
            hwnd: Hwnd::from_raw(hwnd),
            msg,
            wparam: wp.0,
            lparam: lp.0,
        };
        if let Some(r) = d(&m) {
            return LRESULT(r);
        }
    }
    // SAFETY: forwarding the unchanged arguments of this window procedure.
    unsafe { wm::DefWindowProcW(hwnd, msg, wp, lp) }
}

fn instance() -> Result<HINSTANCE> {
    // SAFETY: NULL module name returns the executable module.
    // https://learn.microsoft.com/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulehandlew
    let m = unsafe { GetModuleHandleW(None) }.ctx("GetModuleHandleW")?;
    Ok(HINSTANCE(m.0))
}

/// Registers a window class using the shared window procedure.
pub fn register_class(name: &str, double_clicks: bool) -> Result<()> {
    let hinst = instance()?;
    let class = wide(name);
    // SAFETY: IDC_ARROW is a predefined resource of the system.
    let cursor = unsafe { wm::LoadCursorW(None, wm::IDC_ARROW) }.ctx("LoadCursorW")?;
    let mut style = wm::CS_HREDRAW | wm::CS_VREDRAW;
    if double_clicks {
        style |= wm::CS_DBLCLKS;
    }
    // SAFETY: LoadImageW with resource id 1 (embedded icon); failure is
    // tolerated and falls back to no icon.
    let icon = unsafe {
        wm::LoadImageW(
            Some(hinst),
            PCWSTR(std::ptr::without_provenance::<u16>(1)),
            wm::IMAGE_ICON,
            0,
            0,
            wm::LR_DEFAULTSIZE | wm::LR_SHARED,
        )
    }
    .ok()
    .map(|h| wm::HICON(h.0));
    let wc = wm::WNDCLASSEXW {
        cbSize: size_of::<wm::WNDCLASSEXW>() as u32,
        style,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        hCursor: cursor,
        hIcon: icon.unwrap_or_default(),
        lpszClassName: pcwstr(&class),
        ..Default::default()
    };
    // SAFETY: `wc` is fully initialized and `class` outlives the call
    // (the system copies the name).
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerclassexw
    let atom = unsafe { wm::RegisterClassExW(&wc) };
    if atom == 0 {
        return Err(last_error("RegisterClassExW"));
    }
    Ok(())
}

/// Kind of window to create.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Invisible top-level window that still receives broadcasts.
    HiddenTop,
    /// Per-pixel alpha child window (fences inside the desktop layer).
    LayeredChild { parent: Hwnd },
    /// Per-pixel alpha popup (menus, Peek, rubber band, indicators).
    LayeredPopup {
        owner: Option<Hwnd>,
        topmost: bool,
        activate: bool,
    },
    /// Window with a taskbar button ("show as running task").
    TaskbarStub,
}

/// RAII window: destroyed on drop if it still exists.
#[derive(Debug)]
pub struct OwnedWindow(Hwnd);

impl OwnedWindow {
    pub fn hwnd(&self) -> Hwnd {
        self.0
    }
}

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// Creates a window of a registered class.
pub fn create(class: &str, title: &str, kind: Kind, rect: Rect) -> Result<OwnedWindow> {
    let hinst = instance()?;
    let class_w = wide(class);
    let title_w = wide(title);
    let (ex, style, parent) = match kind {
        Kind::HiddenTop => (wm::WS_EX_TOOLWINDOW, wm::WS_POPUP, None),
        Kind::LayeredChild { parent } => (
            wm::WS_EX_LAYERED | wm::WS_EX_NOACTIVATE,
            wm::WS_CHILD | wm::WS_CLIPSIBLINGS,
            Some(parent.raw()),
        ),
        Kind::LayeredPopup {
            owner,
            topmost,
            activate,
        } => {
            let mut ex = wm::WS_EX_LAYERED | wm::WS_EX_TOOLWINDOW;
            if topmost {
                ex |= wm::WS_EX_TOPMOST;
            }
            if !activate {
                ex |= wm::WS_EX_NOACTIVATE;
            }
            (ex, wm::WS_POPUP, owner.map(Hwnd::raw))
        }
        Kind::TaskbarStub => (wm::WS_EX_APPWINDOW, wm::WS_OVERLAPPEDWINDOW, None),
    };
    // SAFETY: class and title buffers are NUL terminated and outlive the
    // call; the parent (if any) is a window handle; for a parent owned by
    // Explorer this creates a cross-process child window, which is a
    // documented Win32 scenario (no code runs in Explorer).
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-createwindowexw
    let hwnd = unsafe {
        wm::CreateWindowExW(
            ex,
            pcwstr(&class_w),
            pcwstr(&title_w),
            style,
            rect.left,
            rect.top,
            rect.width().max(1),
            rect.height().max(1),
            parent,
            None,
            Some(hinst),
            None,
        )
    }
    .ctx("CreateWindowExW")?;
    Ok(OwnedWindow(Hwnd::from_raw(hwnd)))
}

/// Single-line edit control shown as a borderless popup, used to rename a
/// fence in place. Enter and Escape are reported to `notify` with
/// `notify_msg` (wparam 1 = Enter, 2 = Escape).
pub fn create_inline_edit(
    owner: Hwnd,
    rect: Rect,
    text: &str,
    font: &Font,
    notify: Hwnd,
    notify_msg: u32,
) -> Result<OwnedWindow> {
    let hinst = instance()?;
    let class_w = wide("EDIT");
    let text_w = wide(text);
    // SAFETY: as in `create`, EDIT is a system class.
    let hwnd = unsafe {
        wm::CreateWindowExW(
            wm::WS_EX_TOOLWINDOW | wm::WS_EX_TOPMOST,
            pcwstr(&class_w),
            pcwstr(&text_w),
            wm::WS_POPUP | wm::WS_BORDER | wm::WINDOW_STYLE(0x0080), // ES_AUTOHSCROLL
            rect.left,
            rect.top,
            rect.width().max(40),
            rect.height().max(16),
            Some(owner.raw()),
            None,
            Some(hinst),
            None,
        )
    }
    .ctx("CreateWindowExW(EDIT)")?;
    let edit = OwnedWindow(Hwnd::from_raw(hwnd));
    // SAFETY: WM_SETFONT with a valid HFONT that outlives the control
    // (the caller keeps `font` alive while the edit exists).
    unsafe {
        wm::SendMessageW(
            hwnd,
            wm::WM_SETFONT,
            Some(WPARAM(font.0.0 as usize)),
            Some(LPARAM(1)),
        );
        // EM_SETSEL 0..-1 selects everything.
        wm::SendMessageW(hwnd, 0x00B1, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    EDIT_TARGET.with(|t| t.set((notify, notify_msg)));
    // SAFETY: subclassing a window of this thread with a static procedure.
    // https://learn.microsoft.com/windows/win32/api/commctrl/nf-commctrl-setwindowsubclass
    let _ = unsafe { SetWindowSubclass(hwnd, Some(edit_subclass), 1, 0) };
    Ok(edit)
}

thread_local! {
    static EDIT_TARGET: std::cell::Cell<(Hwnd, u32)> = const { std::cell::Cell::new((Hwnd::NULL, 0)) };
}

unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let (target, notify) = EDIT_TARGET.with(std::cell::Cell::get);
    match msg {
        wm::WM_KEYDOWN if wp.0 == 0x0D || wp.0 == 0x1B => {
            let code = if wp.0 == 0x0D { 1 } else { 2 };
            target.post(notify, code, 0);
            return LRESULT(0);
        }
        // Swallow the matching WM_CHAR to avoid the error beep.
        wm::WM_CHAR if wp.0 == 0x0D || wp.0 == 0x1B => return LRESULT(0),
        wm::WM_KILLFOCUS => {
            target.post(notify, 3, 0);
        }
        _ => {}
    }
    // SAFETY: forwarding the unchanged arguments to the next procedure.
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// GDI font, deleted on drop.
#[derive(Debug)]
pub struct Font(HFONT);

impl Font {
    /// Font of `px` pixels height.
    pub fn new(family: &str, px: i32) -> Result<Font> {
        let f = wide(family);
        // SAFETY: `f` outlives the call; other arguments are plain values.
        // https://learn.microsoft.com/windows/win32/api/wingdi/nf-wingdi-createfontw
        let h = unsafe {
            CreateFontW(
                -px.abs(),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                pcwstr(&f),
            )
        };
        if h.is_invalid() {
            return Err(last_error("CreateFontW"));
        }
        Ok(Font(h))
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: the font was created by CreateFontW and is not selected
        // in any DC anymore when the owner drops it.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

/// Runs the message loop until WM_QUIT, returns the exit code.
pub fn run_message_loop() -> i32 {
    let mut msg = wm::MSG::default();
    loop {
        // SAFETY: `msg` is a valid out pointer; NULL hwnd = all windows of
        // this thread plus thread messages.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getmessagew
        let r = unsafe { wm::GetMessageW(&mut msg, None, 0, 0) };
        if r.0 <= 0 {
            break;
        }
        // SAFETY: `msg` was filled by GetMessageW.
        unsafe {
            let _ = wm::TranslateMessage(&msg);
            wm::DispatchMessageW(&msg);
        }
    }
    i32::try_from(msg.wParam.0).unwrap_or(0)
}

/// Posts WM_QUIT to the current thread.
pub fn post_quit(code: i32) {
    // SAFETY: plain value.
    unsafe { wm::PostQuitMessage(code) };
}

/// Registers (or retrieves) a system-wide message id.
pub fn register_message(name: &str) -> u32 {
    let w = wide(name);
    // SAFETY: `w` outlives the call.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerwindowmessagew
    unsafe { wm::RegisterWindowMessageW(pcwstr(&w)) }
}

/// Finds a top-level window by class name.
pub fn find_window(class: &str) -> Option<Hwnd> {
    let w = wide(class);
    // SAFETY: `w` outlives the call.
    let h = unsafe { wm::FindWindowW(pcwstr(&w), PCWSTR::null()) }.ok()?;
    (!h.is_invalid()).then(|| Hwnd::from_raw(h))
}

/// Finds a child window by class name, after `after` in z-order.
pub fn find_child(parent: Option<Hwnd>, after: Option<Hwnd>, class: &str) -> Option<Hwnd> {
    let w = wide(class);
    // SAFETY: `w` outlives the call; handles are plain values.
    // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-findwindowexw
    let h = unsafe {
        wm::FindWindowExW(
            parent.map(Hwnd::raw),
            after.map(Hwnd::raw),
            pcwstr(&w),
            PCWSTR::null(),
        )
    }
    .ok()?;
    (!h.is_invalid()).then(|| Hwnd::from_raw(h))
}

/// Top-level window under a screen point.
pub fn window_from_point(p: Point) -> Hwnd {
    // SAFETY: plain value.
    Hwnd::from_raw(unsafe { wm::WindowFromPoint(POINT { x: p.x, y: p.y }) })
}

/// Current foreground window.
pub fn foreground_window() -> Hwnd {
    // SAFETY: no argument.
    Hwnd::from_raw(unsafe { wm::GetForegroundWindow() })
}

/// Mouse cursor shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    Hand,
    SizeWe,
    SizeNs,
    SizeNwse,
    SizeNesw,
    SizeAll,
}

/// Sets the cursor shape (call on WM_SETCURSOR or WM_MOUSEMOVE).
pub fn set_cursor(c: Cursor) {
    let id = match c {
        Cursor::Arrow => wm::IDC_ARROW,
        Cursor::Hand => wm::IDC_HAND,
        Cursor::SizeWe => wm::IDC_SIZEWE,
        Cursor::SizeNs => wm::IDC_SIZENS,
        Cursor::SizeNwse => wm::IDC_SIZENWSE,
        Cursor::SizeNesw => wm::IDC_SIZENESW,
        Cursor::SizeAll => wm::IDC_SIZEALL,
    };
    // SAFETY: predefined system cursors, shared handles need no release.
    unsafe {
        if let Ok(h) = wm::LoadCursorW(None, id) {
            wm::SetCursor(Some(h));
        }
    }
}

/// Message constants re-exported for the application.
pub mod msg {
    use windows::Win32::UI::WindowsAndMessaging as wm;
    pub const WM_DESTROY: u32 = wm::WM_DESTROY;
    pub const WM_CLOSE: u32 = wm::WM_CLOSE;
    pub const WM_TIMER: u32 = wm::WM_TIMER;
    pub const WM_MOUSEMOVE: u32 = wm::WM_MOUSEMOVE;
    pub const WM_LBUTTONDOWN: u32 = wm::WM_LBUTTONDOWN;
    pub const WM_LBUTTONUP: u32 = wm::WM_LBUTTONUP;
    pub const WM_LBUTTONDBLCLK: u32 = wm::WM_LBUTTONDBLCLK;
    pub const WM_RBUTTONDOWN: u32 = wm::WM_RBUTTONDOWN;
    pub const WM_RBUTTONUP: u32 = wm::WM_RBUTTONUP;
    pub const WM_MBUTTONUP: u32 = wm::WM_MBUTTONUP;
    pub const WM_MOUSEWHEEL: u32 = wm::WM_MOUSEWHEEL;
    pub const WM_MOUSELEAVE: u32 = 0x02A3;
    pub const WM_KEYDOWN: u32 = wm::WM_KEYDOWN;
    pub const WM_SETCURSOR: u32 = wm::WM_SETCURSOR;
    pub const WM_CAPTURECHANGED: u32 = wm::WM_CAPTURECHANGED;
    pub const WM_ACTIVATE: u32 = wm::WM_ACTIVATE;
    pub const WM_ACTIVATEAPP: u32 = wm::WM_ACTIVATEAPP;
    pub const WM_KILLFOCUS: u32 = wm::WM_KILLFOCUS;
    pub const WM_DISPLAYCHANGE: u32 = wm::WM_DISPLAYCHANGE;
    pub const WM_DPICHANGED: u32 = wm::WM_DPICHANGED;
    pub const WM_SETTINGCHANGE: u32 = wm::WM_SETTINGCHANGE;
    pub const WM_POWERBROADCAST: u32 = wm::WM_POWERBROADCAST;
    pub const WM_HOTKEY: u32 = wm::WM_HOTKEY;
    pub const WM_COPYDATA: u32 = wm::WM_COPYDATA;
    pub const WM_APP: u32 = wm::WM_APP;
    pub const WM_SYSCOMMAND: u32 = wm::WM_SYSCOMMAND;
    pub const WM_ENDSESSION: u32 = wm::WM_ENDSESSION;
    pub const WM_MOUSEACTIVATE: u32 = wm::WM_MOUSEACTIVATE;
    pub const WM_NCHITTEST: u32 = wm::WM_NCHITTEST;
    pub const WM_DEVICECHANGE: u32 = 0x0219;
    pub const MA_NOACTIVATE: isize = 3;
    pub const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
    pub const PBT_APMRESUMESUSPEND: usize = 0x7;
    pub const SC_RESTORE: usize = 0xF120;
    pub const WA_INACTIVE: usize = 0;
    pub const MK_CONTROL: usize = 0x0008;
    pub const MK_SHIFT: usize = 0x0004;
}
