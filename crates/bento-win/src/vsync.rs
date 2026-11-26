//! Frame pacing on the display refresh.
//!
//! `SetTimer` only fires on the system tick (15.6 ms by default), and a timer
//! re-armed after drawing a frame often waits two ticks: animations then run
//! at about 32 frames per second, out of step with the screen. A [`Pacer`]
//! instead waits for each composition pass of the Desktop Window Manager
//! (`DwmFlush`) on its own thread and posts one message per frame to a
//! window, so animations follow the refresh rate of the screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Dwm::DwmFlush;

use crate::window::Hwnd;

#[derive(Default)]
struct Shared {
    /// True while frames are wanted.
    active: Mutex<bool>,
    wake: Condvar,
    /// A frame message is posted and not yet handled: no other one is
    /// posted, so a slow frame never queues a backlog of messages.
    pending: AtomicBool,
    quit: AtomicBool,
}

/// Posts `msg` to a window once per displayed frame while started.
pub struct Pacer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Pacer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pacer")
    }
}

impl Pacer {
    pub fn start(hwnd: Hwnd, msg: u32) -> std::io::Result<Pacer> {
        let shared = Arc::new(Shared::default());
        let s = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("vsync".into())
            .spawn(move || run(&s, hwnd, msg))?;
        Ok(Pacer {
            shared,
            thread: Some(thread),
        })
    }

    /// Starts sending frame messages (no effect when already running).
    pub fn resume(&self) {
        let mut a = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        if !*a {
            *a = true;
            self.shared.wake.notify_one();
        }
    }

    /// Stops sending frame messages after the current one.
    pub fn pause(&self) {
        *self.shared.active.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }

    /// To call when a frame message is handled, so the next one is posted.
    pub fn frame_done(&self) {
        self.shared.pending.store(false, Ordering::Release);
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        self.resume();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(s: &Shared, hwnd: Hwnd, msg: u32) {
    let mut quick = 0u32;
    loop {
        {
            let mut a = s.active.lock().unwrap_or_else(|e| e.into_inner());
            while !*a {
                a = s.wake.wait(a).unwrap_or_else(|e| e.into_inner());
            }
        }
        if s.quit.load(Ordering::Acquire) {
            return;
        }
        wait_frame(&mut quick);
        let active = *s.active.lock().unwrap_or_else(|e| e.into_inner());
        if active && !s.pending.swap(true, Ordering::AcqRel) && !hwnd.post(msg, 0, 0) {
            // Window gone: nothing will ever handle the frame.
            s.pending.store(false, Ordering::Release);
        }
    }
}

/// Blocks until the next composition pass. Falls back to a short sleep when
/// composition is unavailable, or when it returns at once several times in a
/// row (one quick return is normal when called just before a refresh).
fn wait_frame(quick: &mut u32) {
    let t = Instant::now();
    // SAFETY: no arguments; blocks the calling thread until DWM presents.
    // https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmflush
    let ok = unsafe { DwmFlush() }.is_ok();
    if ok && t.elapsed() >= Duration::from_millis(1) {
        *quick = 0;
        return;
    }
    *quick += 1;
    if !ok || *quick >= 2 {
        std::thread::sleep(Duration::from_millis(8));
    }
}
