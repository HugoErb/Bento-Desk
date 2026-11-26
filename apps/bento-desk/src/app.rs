//! Application state and message routing of the resident process.
//!
//! Everything runs on the UI thread. The window procedure forwards each
//! message to [`dispatch`], which borrows the thread-local [`App`]. When the
//! app is already borrowed (a message sent synchronously while the app is
//! working, or a modal loop) the message goes to `DefWindowProcW`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};

use bento_config::Paths;
use bento_core::display::{MonitorInfo, describe, display_key};
use bento_core::geometry::{Point, Rect, Size};
use bento_core::i18n::{Lang, Msg};
use bento_core::image::Image;
use bento_core::ipc::IpcCmd;
use bento_core::model::{DesktopState, FenceStyle, Layout};
use bento_core::settings::AppConfig;
use bento_render::Theme;
use bento_win::Hwnd;
use bento_win::desktop::{DesktopLayer, allow_mixed_dpi_hosting};
use bento_win::dirwatch::{DirWatcher, FsChange};
use bento_win::gfx::Gfx;
use bento_win::hooks::{
    ForegroundHook, HitData, MouseEvent, MouseHookThread, set_hit_data, unpack_point,
};
use bento_win::hotkey::HotkeyRegistration;
use bento_win::tray::{TrayEvent, TrayIcon};
use bento_win::window::{self, Kind, Message, OwnedWindow, msg};

use crate::fences::FenceUi;
use crate::popup::{Overlay, Popup, RubberBand};
use crate::sync::{self, DeskItem, LayoutInput, Placed};
use crate::worker::{Cmd, Listing, Reply, ShellWorker};

pub const CLASS_MAIN: &str = bento_core::ipc::DAEMON_CLASS;
pub const CLASS_FENCE: &str = "BentoDesk.Fence";
pub const CLASS_POPUP: &str = "BentoDesk.Popup";
pub const CLASS_TASK: &str = "BentoDesk.Task";

pub const WM_WORKER: u32 = msg::WM_APP + 1;
pub const WM_HOOK: u32 = msg::WM_APP + 2;
pub const WM_TRAY: u32 = msg::WM_APP + 3;
pub const WM_SHELL_CHANGE: u32 = msg::WM_APP + 4;
pub const WM_EDIT: u32 = msg::WM_APP + 5;
pub const WM_DRAG: u32 = msg::WM_APP + 6;
pub const WM_LISTING: u32 = msg::WM_APP + 8;
pub const WM_FOREGROUND: u32 = msg::WM_APP + 9;
pub const WM_WALLPAPER: u32 = msg::WM_APP + 10;

/// Timers of the main window. None of them repeats while the app is idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Timer {
    Save = 1,
    Retry = 2,
    Display = 3,
    AfterDrag = 4,
    IdleHide = 5,
    Edge = 6,
    Overlay = 7,
    Collapse = 8,
    Refresh = 9,
    Startup = 10,
}

impl Timer {
    fn from(id: usize) -> Option<Timer> {
        Some(match id {
            1 => Timer::Save,
            2 => Timer::Retry,
            3 => Timer::Display,
            4 => Timer::AfterDrag,
            5 => Timer::IdleHide,
            6 => Timer::Edge,
            7 => Timer::Overlay,
            8 => Timer::Collapse,
            9 => Timer::Refresh,
            10 => Timer::Startup,
            _ => return None,
        })
    }
}

const HK_PEEK: i32 = 1;
const HK_QUICK_HIDE: i32 = 2;
const HK_NEXT_PAGE: i32 = 3;
const HK_PREV_PAGE: i32 = 4;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Application messages that arrived while the app was busy in a modal
    /// loop (context menu, drag and drop, dialog), posted again afterwards.
    static DEFERRED: RefCell<Vec<(Hwnd, u32, usize, isize)>> = const { RefCell::new(Vec::new()) };
}

/// Window procedure entry point for every window of the app.
pub fn dispatch(m: &Message) -> Option<isize> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => {
            let r = guard.as_mut().and_then(|app| app.on_message(m));
            drop(guard);
            flush_deferred();
            r
        }
        Err(_) => {
            // Re-entered from a modal loop. Posted application messages
            // (worker replies, drops, folder changes) must not be lost.
            let posted = m.msg >= msg::WM_APP;
            if posted {
                DEFERRED.with(|d| {
                    if let Ok(mut d) = d.try_borrow_mut() {
                        d.push((m.hwnd, m.msg, m.wparam, m.lparam));
                    }
                });
                Some(0)
            } else {
                None
            }
        }
    })
}

fn flush_deferred() {
    let pending = DEFERRED.with(|d| d.try_borrow_mut().map(|mut d| std::mem::take(&mut *d)));
    if let Ok(pending) = pending {
        for (h, msg, w, l) in pending {
            h.post(msg, w, l);
        }
    }
}

/// Installs the app in the thread-local slot.
pub fn install(app: App) {
    APP.with(|cell| {
        if let Ok(mut g) = cell.try_borrow_mut() {
            *g = Some(app);
        }
    });
}

/// Drops the app (restores icons, releases handles) before exit.
pub fn uninstall() {
    APP.with(|cell| {
        if let Ok(mut g) = cell.try_borrow_mut()
            && let Some(mut app) = g.take()
        {
            app.shutdown();
        }
    });
}

static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn on_foreground(h: Hwnd) {
    let main = Hwnd(MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed));
    main.post(WM_FOREGROUND, h.0 as usize, 0);
}

/// Last left click on the desktop, for double-click detection.
#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub tick: u64,
    pub at: Point,
}

pub struct App {
    pub paths: Paths,
    pub config: AppConfig,
    pub state: DesktopState,
    pub lang: Lang,
    pub theme: Theme,
    pub gfx: Gfx,
    pub main: OwnedWindow,
    pub task_stub: Option<OwnedWindow>,
    pub tray: Option<TrayIcon>,
    pub layer: Option<DesktopLayer>,
    pub monitors: Vec<MonitorInfo>,
    pub layout_key: String,
    pub worker: ShellWorker,
    pub connected: bool,
    pub items: BTreeMap<String, DeskItem>,
    pub spacing: Size,
    pub icon_size: i32,
    pub expected: BTreeMap<String, Point>,
    pub fences: Vec<FenceUi>,
    pub hook: Option<MouseHookThread>,
    _fg_hook: Option<ForegroundHook>,
    pub desktop_fg: bool,
    watchers: Vec<DirWatcher>,
    pub fs_tx: Sender<(u64, Vec<FsChange>)>,
    fs_rx: Receiver<(u64, Vec<FsChange>)>,
    hotkeys: Vec<HotkeyRegistration>,
    pub popup: Option<Popup>,
    pub rubber: Option<RubberBand>,
    pub overlay: Option<Overlay>,
    pub edit: Option<crate::fences::InlineEdit>,
    pub peek: Option<crate::peek::Peek>,
    pub quick_hidden: bool,
    /// Quick-hide uses Explorer's "hide icons" flag (no exclusions).
    pub quick_hidden_global: bool,
    pub paused: bool,
    pub style_clip: Option<FenceStyle>,
    pub blur: HashMap<String, Image>,
    pub listing_tx: Sender<Listing>,
    listing_rx: Receiver<Listing>,
    wallpaper_tx: Sender<Vec<(String, Image)>>,
    wallpaper_rx: Receiver<Vec<(String, Image)>>,
    pub icon_cache: HashMap<String, Image>,
    pub last_click: Option<Click>,
    pub msg_taskbar_created: u32,
    pub msg_open_settings: u32,
    pub own_pid: u32,
    pub edges: Vec<(OwnedWindow, bool)>,
    pub pending_new: HashMap<String, u64>,
    pub pending_deleted: HashSet<String>,
    pub check_user_moves: bool,
    pub first_snapshot: bool,
    pub retries: u32,
    pub listing_token: u64,
    pub edge_side: Option<bool>,
}

impl App {
    pub fn new(paths: Paths) -> anyhow::Result<App> {
        let config = bento_config::load_config(&paths);
        let state = bento_config::load_state(&paths);
        let lang = config
            .general
            .language
            .resolve(bento_win::system::ui_language());
        window::set_dispatcher(dispatch);
        window::register_class(CLASS_MAIN, false)?;
        window::register_class(CLASS_FENCE, true)?;
        window::register_class(CLASS_POPUP, false)?;
        window::register_class(CLASS_TASK, false)?;
        let main = window::create(CLASS_MAIN, "Bento Desk", Kind::HiddenTop, Rect::default())?;
        let hwnd = main.hwnd();
        MAIN_HWND.store(hwnd.0, std::sync::atomic::Ordering::Relaxed);
        let worker = ShellWorker::start(hwnd, WM_WORKER)?;
        let (listing_tx, listing_rx) = mpsc::channel();
        let (wallpaper_tx, wallpaper_rx) = mpsc::channel();
        let (fs_tx, fs_rx) = mpsc::channel();
        let theme = system_theme(&config);
        let mut app = App {
            paths,
            config,
            state,
            lang,
            theme,
            gfx: Gfx::new()?,
            main,
            task_stub: None,
            tray: None,
            layer: None,
            monitors: Vec::new(),
            layout_key: String::new(),
            worker,
            connected: false,
            items: BTreeMap::new(),
            spacing: Size::new(75, 100),
            icon_size: 48,
            expected: BTreeMap::new(),
            fences: Vec::new(),
            hook: None,
            _fg_hook: None,
            desktop_fg: false,
            watchers: Vec::new(),
            fs_tx,
            fs_rx,
            hotkeys: Vec::new(),
            popup: None,
            rubber: None,
            overlay: None,
            edit: None,
            peek: None,
            quick_hidden: false,
            quick_hidden_global: false,
            paused: false,
            style_clip: None,
            blur: HashMap::new(),
            listing_tx,
            listing_rx,
            wallpaper_tx,
            wallpaper_rx,
            icon_cache: HashMap::new(),
            last_click: None,
            msg_taskbar_created: window::register_message("TaskbarCreated"),
            msg_open_settings: window::register_message("BentoDesk.OpenSettings"),
            own_pid: std::process::id(),
            edges: Vec::new(),
            pending_new: HashMap::new(),
            pending_deleted: HashSet::new(),
            check_user_moves: false,
            first_snapshot: true,
            retries: 0,
            listing_token: 0,
            edge_side: None,
        };
        app.tray = Some(TrayIcon::new(hwnd, WM_TRAY, Msg::AppName.text(lang)));
        match ForegroundHook::install(on_foreground) {
            Ok(h) => app._fg_hook = Some(h),
            Err(e) => tracing::warn!("foreground hook: {e}"),
        }
        match MouseHookThread::spawn(hwnd, WM_HOOK) {
            Ok(h) => app.hook = Some(h),
            Err(e) => tracing::warn!("mouse hook thread: {e}"),
        }
        app.apply_config_side_effects();
        app.try_attach();
        Ok(app)
    }

    pub fn hwnd(&self) -> Hwnd {
        self.main.hwnd()
    }

    pub fn t(&self, m: Msg) -> &'static str {
        m.text(self.lang)
    }

    pub fn set_timer(&self, t: Timer, ms: u32) {
        self.hwnd().set_timer(t as usize, ms);
    }

    pub fn kill_timer(&self, t: Timer) {
        self.hwnd().kill_timer(t as usize);
    }

    /// Schedules a save of `layouts.json`.
    pub fn mark_dirty(&self) {
        self.set_timer(Timer::Save, 800);
    }

    fn save_now(&self) {
        if let Err(e) = bento_config::save_state(&self.paths, &self.state) {
            tracing::error!("save layouts: {e}");
        }
    }

    // ----- configuration -------------------------------------------------

    /// Applies everything that depends on the configuration.
    pub fn apply_config_side_effects(&mut self) {
        self.theme = system_theme(&self.config);
        self.lang = self
            .config
            .general
            .language
            .resolve(bento_win::system::ui_language());
        crate::logging::set_level(self.config.general.log_level);
        self.apply_autostart();
        self.register_hotkeys();
        self.update_task_stub();
        let tip = if self.paused {
            self.t(Msg::TrayPaused)
        } else {
            self.t(Msg::AppName)
        };
        if let Some(t) = &mut self.tray {
            t.set_tip(tip);
        }
    }

    fn apply_autostart(&self) {
        let cmd = bento_win::system::exe_path().map(|p| format!("\"{}\"", p.display()));
        let want = self.config.general.start_with_windows && !self.paths.portable;
        let res = if want {
            match cmd {
                Some(c) => bento_win::system::set_autostart("BentoDesk", Some(&c)),
                None => Ok(()),
            }
        } else {
            bento_win::system::set_autostart("BentoDesk", None)
        };
        if let Err(e) = res {
            tracing::warn!("autostart: {e}");
        }
    }

    fn register_hotkeys(&mut self) {
        self.hotkeys.clear();
        let mut failed: Vec<String> = Vec::new();
        let hk = self.config.hotkeys.clone();
        let list = [
            (HK_PEEK, hk.peek),
            (HK_QUICK_HIDE, hk.quick_hide),
            (HK_NEXT_PAGE, hk.next_page),
            (HK_PREV_PAGE, hk.prev_page),
        ];
        for (id, h) in list {
            let Some(h) = h.filter(|h| h.is_valid()) else {
                continue;
            };
            // Win+Space switches keyboard layouts when several are
            // installed: never take it then.
            if h.win
                && h.key == 0x20
                && !h.ctrl
                && !h.alt
                && !h.shift
                && bento_win::input::keyboard_layout_count() > 1
            {
                tracing::warn!("Win+Space left to the keyboard layout switch");
                failed.push(h.to_string());
                continue;
            }
            match HotkeyRegistration::register(self.hwnd(), id, h) {
                Ok(r) => self.hotkeys.push(r),
                Err(e) => {
                    tracing::warn!("hotkey {h}: {e}");
                    failed.push(h.to_string());
                }
            }
        }
        if !failed.is_empty() && !self.monitors.is_empty() {
            let text = Msg::HotkeyFailed.fmt(self.lang, &failed.join(", "));
            self.show_toast(&text);
        }
    }

    fn update_task_stub(&mut self) {
        if self.config.general.show_in_taskbar {
            if self.task_stub.is_none()
                && let Ok(w) = window::create(
                    CLASS_TASK,
                    "Bento Desk",
                    Kind::TaskbarStub,
                    Rect::from_xywh(-32000, -32000, 200, 100),
                )
            {
                w.hwnd().minimize();
                self.task_stub = Some(w);
            }
        } else {
            self.task_stub = None;
        }
    }

    pub fn reload_config(&mut self) {
        self.config = bento_config::load_config(&self.paths);
        self.apply_config_side_effects();
        self.update_gestures();
        self.rebuild_fences();
        self.relayout();
    }

    // ----- desktop layer lifecycle --------------------------------------

    /// Locates the desktop and connects to Explorer; retries later on
    /// failure (Explorer not ready at login).
    pub fn try_attach(&mut self) {
        match DesktopLayer::locate() {
            Ok(layer) => {
                tracing::info!("desktop layer: {:?}", layer.variant);
                allow_mixed_dpi_hosting();
                self.layer = Some(layer);
                self.retries = 0;
                self.connected = false;
                self.first_snapshot = true;
                self.worker.send(Cmd::Connect(layer.listview));
                self.watchers.clear();
                for dir in bento_win::system::desktop_folders() {
                    match DirWatcher::spawn(
                        dir.clone(),
                        0,
                        self.fs_tx.clone(),
                        self.hwnd(),
                        WM_SHELL_CHANGE,
                    ) {
                        Ok(w) => self.watchers.push(w),
                        Err(e) => tracing::warn!("watch {}: {e}", dir.display()),
                    }
                }
                self.refresh_monitors(true);
                self.request_wallpaper();
            }
            Err(e) => {
                self.retries += 1;
                tracing::warn!("desktop not ready ({e}), retry {}", self.retries);
                if self.retries < 60 {
                    self.set_timer(Timer::Retry, 2000);
                }
            }
        }
    }

    fn on_explorer_restart(&mut self) {
        tracing::info!("Explorer restarted");
        if let Some(t) = &mut self.tray {
            t.add();
        }
        self.fences.clear();
        self.edges.clear();
        self.layer = None;
        self.connected = false;
        self.retries = 0;
        self.set_timer(Timer::Retry, 1500);
    }

    /// Reads monitors; switches layout when the configuration changed.
    pub fn refresh_monitors(&mut self, force: bool) {
        let monitors = bento_win::display::monitors();
        if monitors.is_empty() {
            return;
        }
        let key = display_key(&monitors);
        let changed = key != self.layout_key;
        self.monitors = monitors;
        if changed || force {
            if self.state.layout(&key).is_none() {
                // Unknown configuration: start from the current layout so
                // fences do not disappear; they are remapped to monitors
                // that exist.
                let mut l = self
                    .state
                    .layout(&self.layout_key)
                    .cloned()
                    .or_else(|| self.state.layouts.last().cloned())
                    .unwrap_or_default();
                l.key = key.clone();
                l.description = describe(&self.monitors);
                l.normalize();
                if self.config.layouts.auto_restore || self.state.layouts.is_empty() {
                    self.state.layouts.push(l);
                }
                self.mark_dirty();
            }
            if self.config.layouts.auto_restore || self.layout_key.is_empty() {
                self.layout_key = key;
            }
            if self.state.layout(&self.layout_key).is_none() {
                let mut l = Layout {
                    key: self.layout_key.clone(),
                    description: describe(&self.monitors),
                    ..Layout::default()
                };
                l.normalize();
                self.state.layouts.push(l);
            }
            tracing::info!("layout: {}", describe(&self.monitors));
        }
        self.rebuild_fences();
        self.rebuild_edges();
        self.relayout();
    }

    pub fn layout(&self) -> Option<&Layout> {
        self.state.layout(&self.layout_key)
    }

    pub fn layout_mut(&mut self) -> Option<&mut Layout> {
        self.state.layout_mut(&self.layout_key)
    }

    pub fn virtual_screen(&self) -> Rect {
        bento_core::display::virtual_screen(&self.monitors)
    }

    // ----- wallpaper ------------------------------------------------------

    pub fn request_wallpaper(&self) {
        let monitors = self.monitors.clone();
        let tx = self.wallpaper_tx.clone();
        let main = self.hwnd();
        let spawn = std::thread::Builder::new()
            .name("wallpaper".into())
            .spawn(move || {
                let _com = bento_win::com::ComApartment::mta().ok();
                let mut out = Vec::new();
                for m in monitors {
                    if let Some(mut img) = bento_win::wallpaper::monitor_wallpaper(m.rect, 8) {
                        img.blur(6);
                        out.push((m.id.clone(), img));
                    }
                }
                if tx.send(out).is_ok() {
                    main.post(WM_WALLPAPER, 0, 0);
                }
            });
        if let Err(e) = spawn {
            tracing::warn!("wallpaper thread: {e}");
        }
    }

    // ----- icons ----------------------------------------------------------

    /// Current page fences with their on-screen geometry.
    pub fn placed(&self) -> Vec<Placed> {
        self.fences.iter().map(FenceUi::placed).collect()
    }

    /// Recomputes every icon position and asks Explorer to move icons.
    pub fn relayout(&mut self) {
        if !self.connected || self.quick_hidden_global {
            self.update_hit_data();
            return;
        }
        let Some(layout) = self.state.layout(&self.layout_key) else {
            return;
        };
        let placed = self.placed();
        let inp = LayoutInput {
            layout,
            placed: &placed,
            items: &self.items,
            desktop_spacing: self.spacing,
            spacing_percent: self.config.layout.icon_spacing,
            virtual_screen: self.virtual_screen(),
            park_free: self.quick_hidden && !self.config.quick_hide.keep_free_icons,
        };
        let targets = sync::targets(&inp);
        let moves = sync::moves(&targets, &self.items);
        // Free icons are expected where they are.
        let mut expected = targets.clone();
        for (k, it) in &self.items {
            expected.entry(k.clone()).or_insert(it.pos);
        }
        for (k, p) in &targets {
            if let Some(it) = self.items.get_mut(k) {
                it.pos = *p;
            }
        }
        self.expected = expected;
        if !moves.is_empty() {
            tracing::debug!("moving {} icons", moves.len());
            self.worker.send(Cmd::Move(moves));
        }
        self.update_hit_data();
        for i in 0..self.fences.len() {
            self.render_frame(i);
        }
    }

    /// Geometry given to the mouse hook.
    pub fn update_hit_data(&self) {
        let vs = self.virtual_screen();
        let icons = if self.quick_hidden_global {
            Vec::new()
        } else {
            self.items
                .values()
                .filter(|it| !bento_core::grid::is_parked(&vs, it.pos))
                .map(|it| {
                    Rect::from_xywh(it.pos.x, it.pos.y, self.spacing.width, self.spacing.height)
                })
                .collect()
        };
        let bodies: Vec<Rect> = self
            .fences
            .iter()
            .filter(|f| f.visible)
            .map(|f| f.placed().body)
            .collect();
        set_hit_data(HitData {
            listview: self.layer.map(|l| l.listview).unwrap_or_default(),
            icons,
            wheel_zones: bodies.clone(),
            hover_zones: bodies,
            draw_gesture: self.config.general.desktop_gestures && !self.paused,
        });
    }

    fn on_snapshot(&mut self, snap: Vec<crate::worker::ItemSnap>) {
        tracing::debug!(
            "snapshot: {} icons ({} pending new)",
            snap.len(),
            self.pending_new.len()
        );
        let previous: HashSet<String> = self.items.keys().cloned().collect();
        let mut items = BTreeMap::new();
        for s in snap {
            items.insert(
                s.info.key.clone(),
                DeskItem {
                    info: s.info,
                    pos: s.pos,
                },
            );
        }
        // Icons whose file no longer exists: Explorer missed the deletion
        // (seen on OneDrive desktops). Tell the shell and look again.
        let ghosts: Vec<String> = items
            .keys()
            .filter(|k| !k.starts_with("::") && !std::path::Path::new(k.as_str()).exists())
            .cloned()
            .collect();
        if !ghosts.is_empty() {
            for k in &ghosts {
                items.remove(k);
                bento_win::system::shell_notify_deleted(k, false);
                self.pending_deleted.insert(k.clone());
            }
            self.set_timer(Timer::Refresh, 1000);
        }
        self.items = items;
        let placed = self.placed();
        let mut changed = false;

        if self.first_snapshot {
            self.first_snapshot = false;
            self.recover_parked_free_icons();
        } else {
            // Items that appeared since the last snapshot.
            let mut fresh: Vec<String> = self
                .items
                .keys()
                .filter(|k| !previous.contains(*k) && !self.is_fenced(k))
                .cloned()
                .collect();
            // Explorer shows a new file a little after it appears on disk:
            // keep waiting for pending items for up to 10 seconds.
            let now_ms = bento_win::system::tick_ms();
            let pending: Vec<(String, u64)> = self.pending_new.drain().collect();
            let mut waiting = false;
            for (k, since) in pending {
                if self.items.contains_key(&k) {
                    if !previous.contains(&k) {
                        fresh.push(k);
                    }
                } else if now_ms.saturating_sub(since) < 10_000 {
                    // Not in Explorer's view yet: tell the shell about it.
                    let is_dir = std::path::Path::new(&k).is_dir();
                    bento_win::system::shell_notify_created(&k, is_dir);
                    self.pending_new.insert(k, since);
                    waiting = true;
                }
            }
            if waiting {
                self.set_timer(Timer::Refresh, 1000);
            }
            fresh.sort();
            fresh.dedup();
            let now = bento_core::now_secs();
            for key in fresh {
                let Some(item) = self.items.get(&key).cloned() else {
                    continue;
                };
                if self.is_fenced(&key) {
                    continue;
                }
                let target = self.layout().and_then(|l| {
                    sync::fence_for_new_item(
                        l,
                        &self.config.sorting.rules,
                        self.config.sorting.auto_apply,
                        &item,
                        &placed,
                        self.spacing,
                        now,
                    )
                });
                tracing::debug!("new icon {key}: fence {target:?}");
                if let Some(t) = target
                    && let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(t))
                    && let Some(tab) = f.active_mut()
                    && let Some(list) = tab.content.desktop_items_mut()
                {
                    list.push(key.clone());
                    changed = true;
                }
            }
            // Deleted items leave their fence.
            let gone: Vec<String> = self
                .pending_deleted
                .drain()
                .filter(|k| !self.items.contains_key(k))
                .collect();
            if !gone.is_empty() {
                for l in &mut self.state.layouts {
                    for p in &mut l.pages {
                        for f in &mut p.fences {
                            for k in &gone {
                                changed |= f.remove_item(k);
                            }
                        }
                    }
                }
            }
        }

        if self.check_user_moves {
            self.check_user_moves = false;
            let moved = sync::moved_by_user(&self.expected, &self.items);
            let spacing = self.spacing;
            let pct = self.config.layout.icon_spacing;
            for (key, pos) in moved {
                if let Some(l) = self.state.layout_mut(&self.layout_key) {
                    changed |= sync::apply_user_move(l, &placed, &key, pos, spacing, pct);
                }
            }
        }
        if changed {
            self.sort_all_tabs();
            self.mark_dirty();
        }
        self.relayout();
        self.publish_items_if_requested();
    }

    pub fn is_fenced(&self, key: &str) -> bool {
        self.state
            .layouts
            .iter()
            .any(|l| l.fenced_items().any(|k| k == key))
    }

    /// Applies non-manual sort modes after membership changes.
    pub fn sort_all_tabs(&mut self) {
        let infos: HashMap<String, bento_core::items::ItemInfo> = self
            .items
            .iter()
            .map(|(k, v)| (k.clone(), v.info.clone()))
            .collect();
        let counts = self.state.open_counts.clone();
        if let Some(l) = self.state.layout_mut(&self.layout_key) {
            for p in &mut l.pages {
                for f in &mut p.fences {
                    for t in &mut f.tabs {
                        let mode = t.sort;
                        if let Some(list) = t.content.desktop_items_mut() {
                            bento_core::sort::sort_items(list, &infos, &counts, mode);
                        }
                    }
                }
            }
        }
    }

    /// Puts back free icons that were parked by a quick-hide interrupted
    /// by a crash or a shutdown.
    fn recover_parked_free_icons(&mut self) {
        if self.state.parked_free_icons.is_empty() || self.quick_hidden {
            return;
        }
        let moves: Vec<(String, Point)> = self
            .state
            .parked_free_icons
            .iter()
            .filter(|(k, _)| self.items.contains_key(*k) && !self.is_fenced(k))
            .map(|(k, p)| (k.clone(), *p))
            .collect();
        for (k, p) in &moves {
            if let Some(it) = self.items.get_mut(k) {
                it.pos = *p;
            }
        }
        self.state.parked_free_icons.clear();
        self.mark_dirty();
        if !moves.is_empty() {
            self.worker.send(Cmd::Move(moves));
        }
    }

    /// Puts every icon that is outside the visible screen back in view
    /// (menu "Recover all icons").
    pub fn recover_all_icons(&mut self) {
        let vs = self.virtual_screen();
        let Some(work) = self.monitors.iter().find(|m| m.primary).map(|m| m.work) else {
            return;
        };
        let lost: Vec<String> = self
            .items
            .iter()
            .filter(|(_, it)| !vs.contains(it.pos))
            .map(|(k, _)| k.clone())
            .collect();
        let occupied: Vec<Rect> = self
            .items
            .values()
            .filter(|it| vs.contains(it.pos))
            .map(|it| Rect::from_xywh(it.pos.x, it.pos.y, self.spacing.width, self.spacing.height))
            .chain(self.fences.iter().map(|f| f.rect))
            .collect();
        let slots = sync::free_slots(work, self.spacing, &occupied, lost.len());
        // Recovered icons leave their fences so they stay visible.
        for k in &lost {
            for l in &mut self.state.layouts {
                for p in &mut l.pages {
                    for f in &mut p.fences {
                        f.remove_item(k);
                    }
                }
            }
        }
        let moves: Vec<(String, Point)> = lost.into_iter().zip(slots).collect();
        self.state.parked_free_icons.clear();
        self.quick_hidden = false;
        if !moves.is_empty() {
            self.worker.send(Cmd::Move(moves));
        }
        self.mark_dirty();
        self.worker.send(Cmd::Snapshot);
        self.show_toast(self.t(Msg::Recovered));
    }

    /// Writes the list of desktop items for the rule tester of the
    /// settings window.
    fn publish_items_if_requested(&self) {
        let infos: Vec<&bento_core::items::ItemInfo> =
            self.items.values().map(|i| &i.info).collect();
        if let Ok(bytes) = serde_json::to_vec(&infos)
            && let Err(e) = bento_config::write_atomic(&self.paths.items_file(), &bytes)
        {
            tracing::debug!("publish items: {e}");
        }
    }

    fn on_worker(&mut self) {
        for r in self.worker.drain() {
            match r {
                Reply::Connected {
                    spacing,
                    icon_size,
                    flags,
                } => {
                    tracing::info!(
                        "connected to the desktop view, spacing {spacing:?}, icon {icon_size}, flags {flags:#x}"
                    );
                    self.connected = true;
                    self.spacing = spacing;
                    self.icon_size = icon_size;
                    if self.state.original_folder_flags.is_none() {
                        self.state.original_folder_flags = Some(flags);
                        self.mark_dirty();
                    }
                    let has_fences = self
                        .state
                        .layouts
                        .iter()
                        .any(|l| l.pages.iter().any(|p| !p.fences.is_empty()));
                    if has_fences {
                        self.worker.send(Cmd::FreePositioning);
                    }
                    self.worker.send(Cmd::Snapshot);
                }
                Reply::ConnectFailed(e) => {
                    tracing::warn!("cannot connect to the desktop view: {e}");
                    self.retries += 1;
                    if self.retries < 60 {
                        self.set_timer(Timer::Retry, 2000);
                    }
                }
                Reply::Snapshot(items) => self.on_snapshot(items),
                Reply::Positioning => {}
                Reply::Icons(icons) => {
                    for (k, img) in icons {
                        if let Some(img) = img {
                            self.icon_cache.insert(k, img);
                        }
                    }
                    self.render_peek();
                }
                Reply::Error(e) => {
                    tracing::warn!("{e}");
                    if !self.layer.is_some_and(|l| l.is_alive()) {
                        self.on_explorer_restart();
                    }
                }
            }
        }
    }

    // ----- foreground and gestures --------------------------------------

    fn on_foreground_change(&mut self, h: Hwnd) {
        let own = h.process_id() == self.own_pid;
        let desktop = self.layer.is_some_and(|l| l.is_desktop_window(h));
        self.desktop_fg = desktop || (own && self.desktop_fg);
        if !desktop && !own {
            // Close transient UI when another app takes the focus.
            self.close_popup();
            if self.config.general.pause_on_fullscreen {
                let excluded = bento_win::system::process_name_of(h).is_some_and(|n| {
                    self.config
                        .general
                        .game_mode_exclusions
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&n))
                });
                if excluded || bento_win::system::fullscreen_app_running() {
                    tracing::debug!("game mode");
                    self.kill_timer(Timer::IdleHide);
                }
            }
        }
        self.update_gestures();
    }

    /// Installs the mouse hook only while the desktop is in the foreground.
    pub fn update_gestures(&self) {
        let Some(h) = &self.hook else { return };
        let want = self.desktop_fg && !self.paused && self.layer.is_some();
        if want {
            h.enable();
        } else {
            h.disable();
        }
    }

    fn on_hook(&mut self, wparam: usize, lparam: isize) {
        let Some(ev) = MouseEvent::decode(wparam) else {
            return;
        };
        let p = unpack_point(lparam);
        if self.config.quick_hide.idle_seconds > 0 && !matches!(ev, MouseEvent::Hover(_)) {
            if self.quick_hidden && matches!(ev, MouseEvent::LeftDown) {
                self.set_quick_hide(false);
            }
            self.set_timer(
                Timer::IdleHide,
                self.config.quick_hide.idle_seconds.saturating_mul(1000),
            );
        }
        match ev {
            MouseEvent::LeftDown => self.on_desktop_click(p),
            MouseEvent::LeftUp => {
                if self.connected {
                    self.set_timer(Timer::AfterDrag, 250);
                }
            }
            MouseEvent::RightDown => self.begin_rubber(p),
            MouseEvent::RightMove => self.update_rubber(p),
            MouseEvent::RightUp => self.end_rubber(p),
            MouseEvent::Wheel(d) => self.on_body_wheel(p, d),
            MouseEvent::Hover(zone) => self.on_body_hover(zone.map(|z| z as usize)),
        }
    }

    fn on_desktop_click(&mut self, p: Point) {
        let now = bento_win::system::tick_ms();
        let (dx, dy) = bento_win::input::double_click_size();
        let dbl = self.last_click.is_some_and(|c| {
            now.saturating_sub(c.tick) <= u64::from(bento_win::input::double_click_ms())
                && (c.at.x - p.x).abs() <= dx
                && (c.at.y - p.y).abs() <= dy
        });
        self.last_click = if dbl {
            None
        } else {
            Some(Click { tick: now, at: p })
        };
        if !dbl {
            return;
        }
        // Double-click on an icon: count the launch.
        let hit = self.items.iter().find(|(_, it)| {
            Rect::from_xywh(it.pos.x, it.pos.y, self.spacing.width, self.spacing.height).contains(p)
        });
        if let Some((k, _)) = hit
            && !self.quick_hidden_global
        {
            *self.state.open_counts.entry(k.clone()).or_insert(0) += 1;
            self.mark_dirty();
            return;
        }
        let in_fence = self.fences.iter().any(|f| f.visible && f.rect.contains(p));
        if !in_fence && self.config.quick_hide.double_click {
            let hide = !self.quick_hidden;
            self.set_quick_hide(hide);
        }
    }

    /// Quick-hide: hides icons and fences (except exclusions).
    pub fn set_quick_hide(&mut self, hide: bool) {
        if hide == self.quick_hidden {
            return;
        }
        self.quick_hidden = hide;
        let exempt = self
            .layout()
            .and_then(Layout::page)
            .is_some_and(|p| p.fences.iter().any(|f| f.quick_hide_exempt));
        let global = !exempt && !self.config.quick_hide.keep_free_icons;
        if hide {
            if global {
                self.quick_hidden_global = true;
                self.worker.send(Cmd::SetIconsHidden(true));
            } else if !self.config.quick_hide.keep_free_icons {
                // Remember free icons for crash recovery before parking.
                let fenced: HashSet<String> = self
                    .state
                    .layouts
                    .iter()
                    .flat_map(|l| l.fenced_items().cloned())
                    .collect();
                self.state.parked_free_icons = self
                    .items
                    .iter()
                    .filter(|(k, _)| !fenced.contains(*k))
                    .map(|(k, it)| (k.clone(), it.pos))
                    .collect();
                self.mark_dirty();
            }
        } else if self.quick_hidden_global {
            self.quick_hidden_global = false;
            self.worker.send(Cmd::SetIconsHidden(false));
        } else if !self.state.parked_free_icons.is_empty() {
            let moves: Vec<(String, Point)> = self
                .state
                .parked_free_icons
                .iter()
                .map(|(k, p)| (k.clone(), *p))
                .collect();
            for (k, p) in &moves {
                if let Some(it) = self.items.get_mut(k) {
                    it.pos = *p;
                }
            }
            self.state.parked_free_icons.clear();
            self.worker.send(Cmd::Move(moves));
            self.mark_dirty();
        }
        let exempt_ids: Vec<bento_core::model::FenceId> = self
            .layout()
            .and_then(Layout::page)
            .map(|p| {
                p.fences
                    .iter()
                    .filter(|f| f.quick_hide_exempt)
                    .map(|f| f.id)
                    .collect()
            })
            .unwrap_or_default();
        for f in &mut self.fences {
            f.hidden = hide && !exempt_ids.contains(&f.id);
            f.apply_visibility();
        }
        self.relayout();
    }

    // ----- pages ---------------------------------------------------------

    pub fn switch_page(&mut self, delta: i32) {
        let Some(l) = self.layout_mut() else { return };
        let n = l.pages.len() as i32;
        if n <= 1 {
            return;
        }
        l.current_page = ((l.current_page as i32 + delta).rem_euclid(n)) as usize;
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
        self.show_page_indicator();
    }

    pub fn goto_page(&mut self, i: usize) {
        let Some(l) = self.layout_mut() else { return };
        if i >= l.pages.len() || i == l.current_page {
            return;
        }
        l.current_page = i;
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
        self.show_page_indicator();
    }

    pub fn new_page(&mut self) {
        let Some(l) = self.layout_mut() else { return };
        l.pages.push(bento_core::model::Page::default());
        let last = l.pages.len() - 1;
        self.goto_page(last);
    }

    pub fn delete_page(&mut self) {
        let Some(l) = self.layout_mut() else { return };
        if l.pages.len() <= 1 {
            return;
        }
        let cur = l.current_page;
        // Fences of the deleted page move to the first remaining page.
        let removed = l.pages.remove(cur);
        let target = if cur == 0 { 0 } else { cur - 1 };
        if let Some(p) = l.pages.get_mut(target) {
            p.fences.extend(removed.fences);
        }
        l.current_page = target;
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
        self.show_page_indicator();
    }

    /// Thin windows on the left and right screen edges for page switching.
    fn rebuild_edges(&mut self) {
        self.edges.clear();
        if !self.config.pages.enabled || !self.config.pages.edge_switch {
            return;
        }
        let Some(layer) = self.layer else { return };
        let vs = self.virtual_screen();
        let origin = layer.frame_parent.client_origin();
        for left in [true, false] {
            let x = if left { vs.left } else { vs.right - 1 };
            let r = Rect::from_xywh(x, vs.top, 1, vs.height());
            let Ok(w) = window::create(
                CLASS_POPUP,
                "",
                Kind::LayeredChild {
                    parent: layer.frame_parent,
                },
                r.offset(-origin.x, -origin.y),
            ) else {
                continue;
            };
            if let Ok(mut s) = bento_win::gfx::Surface::new(&self.gfx, 1, vs.height()) {
                let _ = s.draw(&self.gfx, |c| {
                    c.fill_rect(
                        bento_win::gfx::RectF::xywh(0.0, 0.0, 1.0, vs.height() as f32),
                        bento_core::color::Rgba::new(0, 0, 0, 1),
                    );
                });
                let _ = s.present(w.hwnd());
            }
            layer.place_frame(w.hwnd());
            w.hwnd().show(true);
            self.edges.push((w, left));
        }
    }

    // ----- messages ------------------------------------------------------

    pub fn on_message(&mut self, m: &Message) -> Option<isize> {
        if m.hwnd == self.hwnd() {
            return self.on_main_message(m);
        }
        if let Some(i) = self.fences.iter().position(|f| f.owns(m.hwnd)) {
            return self.on_fence_message(i, m);
        }
        if self.popup.as_ref().is_some_and(|p| p.hwnd() == m.hwnd) {
            return self.on_popup_message(m);
        }
        if self.peek.as_ref().is_some_and(|p| p.hwnd() == m.hwnd) {
            return self.on_peek_message(m);
        }
        if let Some((_, left)) = self.edges.iter().find(|(w, _)| w.hwnd() == m.hwnd) {
            if m.msg == msg::WM_MOUSEMOVE {
                self.edge_side = Some(*left);
                self.set_timer(Timer::Edge, 450);
            }
            return None;
        }
        if self.task_stub.as_ref().is_some_and(|w| w.hwnd() == m.hwnd) {
            return self.on_task_stub(m);
        }
        None
    }

    fn on_task_stub(&mut self, m: &Message) -> Option<isize> {
        match m.msg {
            msg::WM_SYSCOMMAND if m.wparam & 0xFFF0 == msg::SC_RESTORE => {
                self.open_settings(None);
                Some(0)
            }
            msg::WM_CLOSE => {
                self.config.general.show_in_taskbar = false;
                let _ = bento_config::save_config(&self.paths, &self.config);
                self.update_task_stub();
                Some(0)
            }
            _ => None,
        }
    }

    fn on_main_message(&mut self, m: &Message) -> Option<isize> {
        match m.msg {
            WM_WORKER => self.on_worker(),
            WM_HOOK => self.on_hook(m.wparam, m.lparam),
            WM_FOREGROUND => self.on_foreground_change(Hwnd(m.wparam as isize)),
            WM_TRAY => match TrayIcon::decode(m.lparam) {
                TrayEvent::Menu => self.show_tray_menu(),
                TrayEvent::Select | TrayEvent::DoubleClick => self.open_settings(None),
                TrayEvent::Other => {}
            },
            WM_SHELL_CHANGE => {
                let batches: Vec<(u64, Vec<FsChange>)> = self.fs_rx.try_iter().collect();
                for (tag, changes) in batches {
                    if tag != 0 {
                        // Folder portal of fence `tag`.
                        if let Some(i) = self.fences.iter().position(|f| f.id.0 == tag) {
                            self.refresh_portal(i);
                        }
                        continue;
                    }
                    for change in changes {
                        tracing::debug!("desktop change: {change:?}");
                        match change {
                            FsChange::Created(p) => {
                                self.pending_new.insert(p, bento_win::system::tick_ms());
                            }
                            FsChange::Deleted(p) => {
                                self.pending_deleted.insert(p);
                            }
                            FsChange::Renamed { from, to } => self.rename_item(&from, &to),
                            FsChange::Modified(_) | FsChange::Overflow => {}
                        }
                    }
                    self.set_timer(Timer::Refresh, 300);
                }
            }
            WM_EDIT => self.on_edit_key(m.wparam),
            WM_DRAG => self.on_drag_event(m.wparam, m.lparam),
            WM_LISTING => {
                let listings: Vec<Listing> = self.listing_rx.try_iter().collect();
                for l in listings {
                    self.on_listing(l);
                }
            }
            WM_WALLPAPER => {
                for batch in self.wallpaper_rx.try_iter().collect::<Vec<_>>() {
                    for (id, img) in batch {
                        self.blur.insert(id, img);
                    }
                }
                self.apply_chameleon();
                for i in 0..self.fences.len() {
                    self.render_backdrop(i);
                }
            }
            msg::WM_TIMER => self.on_timer(m.wparam),
            msg::WM_HOTKEY => match m.wparam as i32 {
                HK_PEEK => self.toggle_peek(),
                HK_QUICK_HIDE => {
                    let h = !self.quick_hidden;
                    self.set_quick_hide(h);
                }
                HK_NEXT_PAGE if self.config.pages.enabled => self.switch_page(1),
                HK_PREV_PAGE if self.config.pages.enabled => self.switch_page(-1),
                _ => {}
            },
            msg::WM_DISPLAYCHANGE | msg::WM_DPICHANGED | msg::WM_DEVICECHANGE => {
                self.set_timer(Timer::Display, 1500);
            }
            msg::WM_SETTINGCHANGE => {
                // SPI_SETDESKWALLPAPER = 0x14, SPI_SETWORKAREA = 0x2F
                if m.wparam == 0x14 {
                    self.request_wallpaper();
                } else if m.wparam == 0x2F {
                    self.set_timer(Timer::Display, 500);
                } else {
                    let theme = system_theme(&self.config);
                    if theme.dark != self.theme.dark || theme.accent != self.theme.accent {
                        self.theme = theme;
                        self.render_all();
                    }
                }
            }
            msg::WM_POWERBROADCAST => {
                if m.wparam == msg::PBT_APMRESUMEAUTOMATIC || m.wparam == msg::PBT_APMRESUMESUSPEND
                {
                    // Secondary screens may wake up late.
                    self.set_timer(Timer::Display, 3000);
                }
            }
            msg::WM_COPYDATA => {
                if let Some((kind, bytes)) = bento_win::system::read_copydata(m.lparam)
                    && kind == bento_core::ipc::COPYDATA_KIND
                    && let Some(cmd) = IpcCmd::from_bytes(&bytes)
                {
                    self.on_ipc(cmd);
                }
                return Some(1);
            }
            msg::WM_ENDSESSION if m.wparam != 0 => {
                self.save_now();
            }
            // `taskkill` (without /F) and installers close the app this way.
            msg::WM_CLOSE => window::post_quit(0),
            x if x == self.msg_taskbar_created && x != 0 => self.on_explorer_restart(),
            x if x == self.msg_open_settings && x != 0 => self.open_settings(None),
            _ => return None,
        }
        Some(0)
    }

    fn on_timer(&mut self, id: usize) {
        let Some(t) = Timer::from(id) else { return };
        self.kill_timer(t);
        match t {
            Timer::Save => self.save_now(),
            Timer::Retry => self.try_attach(),
            Timer::Display => {
                self.refresh_monitors(false);
                self.request_wallpaper();
            }
            Timer::AfterDrag => {
                self.check_user_moves = true;
                self.worker.send(Cmd::Snapshot);
            }
            Timer::Refresh => self.worker.send(Cmd::Snapshot),
            Timer::IdleHide => {
                if !self.quick_hidden {
                    self.set_quick_hide(true);
                }
            }
            Timer::Edge => {
                if let Some(left) = self.edge_side.take() {
                    let vs = self.virtual_screen();
                    let p = bento_win::input::cursor_pos();
                    let at_edge = if left {
                        p.x <= vs.left + 1
                    } else {
                        p.x >= vs.right - 2
                    };
                    if at_edge {
                        self.switch_page(if left { -1 } else { 1 });
                    }
                }
            }
            Timer::Overlay => self.overlay = None,
            Timer::Collapse => self.check_collapse(),
            Timer::Startup => {}
        }
    }

    fn rename_item(&mut self, from: &str, to: &str) {
        let mut changed = false;
        for l in &mut self.state.layouts {
            for p in &mut l.pages {
                for f in &mut p.fences {
                    changed |= f.rename_item(from, to);
                }
            }
        }
        if let Some(c) = self.state.open_counts.remove(from) {
            self.state.open_counts.insert(to.to_owned(), c);
            changed = true;
        }
        if let Some(it) = self.items.remove(from) {
            self.items.insert(to.to_owned(), it);
        }
        if let Some(p) = self.expected.remove(from) {
            self.expected.insert(to.to_owned(), p);
        }
        if changed {
            self.mark_dirty();
        }
    }

    // ----- IPC and settings ------------------------------------------------

    pub fn on_ipc(&mut self, cmd: IpcCmd) {
        tracing::debug!("ipc: {cmd:?}");
        match cmd {
            IpcCmd::ReloadConfig => self.reload_config(),
            IpcCmd::OpenSettings => self.open_settings(None),
            IpcCmd::SetFenceStyle { fence, style } => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(fence))
                    && let Some(t) = f.active_mut()
                {
                    t.style = style;
                }
                self.mark_dirty();
                self.apply_chameleon();
                self.render_all();
            }
            IpcCmd::SetAllFenceStyles { style } => {
                for l in &mut self.state.layouts {
                    for p in &mut l.pages {
                        for f in &mut p.fences {
                            for t in &mut f.tabs {
                                t.style = style.clone();
                            }
                        }
                    }
                }
                self.mark_dirty();
                self.apply_chameleon();
                self.render_all();
            }
            IpcCmd::RenameFence { fence, title } => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(fence))
                    && let Some(t) = f.active_mut()
                {
                    t.title = title;
                }
                self.mark_dirty();
                self.render_all();
            }
            IpcCmd::SetFenceOptions {
                fence,
                auto_rollup,
                quick_hide_exempt,
            } => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(fence)) {
                    f.auto_rollup = auto_rollup;
                    f.quick_hide_exempt = quick_hide_exempt;
                }
                self.mark_dirty();
                self.rebuild_fences();
                self.relayout();
            }
            IpcCmd::SaveSnapshot { name } => {
                let name = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                self.save_snapshot(name);
            }
            IpcCmd::RestoreSnapshot { name } => self.restore_snapshot(&name),
            IpcCmd::SortNow => self.sort_now(),
            IpcCmd::PublishItems => self.publish_items_if_requested(),
            IpcCmd::RecoverIcons => self.recover_all_icons(),
            IpcCmd::SwapScreens { a, b } => self.swap_screens(&a, &b),
            IpcCmd::DebugPackage => self.debug_package(),
            IpcCmd::Quit => window::post_quit(0),
            IpcCmd::RestoreExplorerOptions => {
                if let Some(f) = self.state.original_folder_flags {
                    self.worker.send(Cmd::RestorePositioning(f));
                }
            }
            IpcCmd::ReloadAll => {
                self.config = bento_config::load_config(&self.paths);
                self.state = bento_config::load_state(&self.paths);
                self.apply_config_side_effects();
                self.refresh_monitors(true);
            }
        }
    }

    /// Starts the settings window (it handles its own single instance).
    pub fn open_settings(&self, section: Option<&str>) {
        let Some(dir) =
            bento_win::system::exe_path().and_then(|p| p.parent().map(|d| d.to_owned()))
        else {
            return;
        };
        let exe = dir.join("bento-settings.exe");
        let args = section
            .map(|s| format!("--section {s}"))
            .unwrap_or_default();
        if !bento_win::system::spawn(&exe.to_string_lossy(), &args) {
            tracing::warn!("cannot start {}", exe.display());
        }
    }

    pub fn save_snapshot(&mut self, name: Option<String>) {
        let Some(layout) = self.layout().cloned() else {
            return;
        };
        let name = name.unwrap_or_else(|| {
            let n = bento_config::list_snapshots(&self.paths).len() + 1;
            format!("Snapshot {n}")
        });
        let snap = bento_core::model::Snapshot {
            name: name.clone(),
            created: bento_core::now_secs(),
            layout,
        };
        match bento_config::save_snapshot(&self.paths, &snap) {
            Ok(()) => self.show_toast(&format!("{} : {name}", self.t(Msg::TraySnapshots))),
            Err(e) => tracing::error!("snapshot: {e}"),
        }
    }

    pub fn restore_snapshot(&mut self, name: &str) {
        match bento_config::load_snapshot(&self.paths, name) {
            Ok(Some(s)) => {
                let key = self.layout_key.clone();
                let desc = describe(&self.monitors);
                if let Some(l) = self.layout_mut() {
                    *l = s.layout;
                    l.key = key;
                    l.description = desc;
                    l.normalize();
                }
                self.state.repair_ids();
                self.mark_dirty();
                self.rebuild_fences();
                self.relayout();
            }
            Ok(None) => tracing::warn!("snapshot {name} not found"),
            Err(e) => tracing::error!("snapshot: {e}"),
        }
    }

    pub fn swap_screens(&mut self, a: &str, b: &str) {
        let (Some(ma), Some(mb)) = (
            self.monitors.iter().find(|m| m.id == a).cloned(),
            self.monitors.iter().find(|m| m.id == b).cloned(),
        ) else {
            return;
        };
        if let Some(l) = self.layout_mut() {
            bento_core::display::swap_monitors(l, &ma, &mb);
        }
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
    }

    /// Applies the rules to every desktop icon that is not in a fence.
    pub fn sort_now(&mut self) {
        let now = bento_core::now_secs();
        let free: Vec<(String, DeskItem)> = self
            .items
            .iter()
            .filter(|(k, _)| !self.is_fenced(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let rules = self.config.sorting.rules.clone();
        let mut changed = false;
        for (k, it) in free {
            let Some(r) = bento_core::rules::first_match(&rules, &it.info, now) else {
                continue;
            };
            if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(r.target))
                && let Some(t) = f.active_mut()
                && let Some(list) = t.content.desktop_items_mut()
            {
                list.push(k);
                changed = true;
            }
        }
        if changed {
            self.sort_all_tabs();
            self.mark_dirty();
            self.relayout();
        }
    }

    /// Writes a troubleshooting folder and opens it.
    fn debug_package(&self) {
        let dir = self
            .paths
            .log_dir
            .parent()
            .map(|p| p.join(format!("debug-{}", bento_core::now_secs())))
            .unwrap_or_else(|| self.paths.data_dir.join("debug"));
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let mut report = String::new();
        report.push_str(&format!("Bento Desk {}\n", env!("CARGO_PKG_VERSION")));
        report.push_str(&format!("layer: {:?}\n", self.layer));
        report.push_str(&format!("connected: {}\n", self.connected));
        report.push_str(&format!("monitors: {:#?}\n", self.monitors));
        report.push_str(&format!("icons: {}\n", self.items.len()));
        report.push_str(&format!("fences on page: {}\n", self.fences.len()));
        report.push_str(&format!("spacing: {:?}\n", self.spacing));
        let _ = std::fs::write(dir.join("report.txt"), report);
        for f in [self.paths.config_file(), self.paths.state_file()] {
            if let Some(name) = f.file_name() {
                let _ = std::fs::copy(&f, dir.join(name));
            }
        }
        if let Ok(rd) = std::fs::read_dir(&self.paths.log_dir) {
            for e in rd.flatten() {
                let _ = std::fs::copy(e.path(), dir.join(e.file_name()));
            }
        }
        bento_win::system::open_in_explorer(&dir.to_string_lossy());
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if self.paused {
            for f in &mut self.fences {
                f.hidden = true;
                f.apply_visibility();
            }
        } else {
            for f in &mut self.fences {
                f.hidden = false;
                f.apply_visibility();
            }
            self.relayout();
        }
        self.update_gestures();
        self.apply_config_side_effects();
    }

    /// Releases everything before exit; leaves icons visible.
    pub fn shutdown(&mut self) {
        self.peek = None;
        self.popup = None;
        if self.quick_hidden {
            self.set_quick_hide(false);
        }
        if let Some(h) = &self.hook {
            h.disable();
        }
        self.save_now();
        self.fences.clear();
        self.edges.clear();
        self.tray = None;
    }
}

/// Default highlight color when the Windows accent is not used.
const NEUTRAL_ACCENT: bento_core::color::Rgba = bento_core::color::Rgba::rgb(0, 120, 212);

/// Light or dark theme of Windows, with its accent color unless the user
/// turned that off.
fn system_theme(config: &AppConfig) -> Theme {
    let accent = if config.appearance.use_accent_color {
        bento_win::dwm::accent_color().unwrap_or(NEUTRAL_ACCENT)
    } else {
        NEUTRAL_ACCENT
    };
    Theme::new(!bento_win::system::apps_use_light_theme(), accent)
}
