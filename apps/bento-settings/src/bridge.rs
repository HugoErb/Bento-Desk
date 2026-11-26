//! Link between the Slint globals and the files of Bento Desk.
//!
//! Global preferences are written to `config.json` (debounced) and the
//! resident app is told to reload them. Changes to one fence go through
//! IPC commands, because the resident app owns `state.json`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use bento_config::Paths;
use bento_core::color::{Rgba, Tint};
use bento_core::display::{MonitorInfo, display_key};
use bento_core::hotkey::Hotkey;
use bento_core::i18n::{Lang, LangPref};
use bento_core::ipc::{COPYDATA_KIND, DAEMON_CLASS, IpcCmd};
use bento_core::items::{ItemInfo, ItemKind};
use bento_core::model::{
    BackdropEffect, DesktopState, Fence, FenceId, FenceStyle, Layout, Snapshot, TitleAlign,
    TitleMode, ViewMode,
};
use bento_core::rules::{Condition, DateField, DateOp, Rule, TextOp};
use bento_core::settings::{AppConfig, LogLevel, OpenTrigger};
use bento_win::gfx::FontFamily;
use bento_win::window::Hwnd;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};

use crate::text::T;
use crate::{
    App, Cfg, ColorUtil, CondRow, Desk, FenceRow, FontItem, Hsl, MainWindow, RuleRow, Rules,
    SnapRow, Snaps, Style, Sys,
};

const SAVE_DELAY: Duration = Duration::from_millis(300);
const STYLE_DELAY: Duration = Duration::from_millis(120);
const TOAST_DELAY: Duration = Duration::from_millis(2600);
const PUBLISH_DELAY: Duration = Duration::from_millis(700);
const REFRESH_DELAY: Duration = Duration::from_millis(700);

/// State of the settings window.
pub struct Ctl {
    pub paths: Paths,
    pub config: AppConfig,
    pub state: DesktopState,
    pub lang: Lang,
    pub owner: Hwnd,
    monitors: Vec<MonitorInfo>,
    /// Fence ids in the order of `Desk.fences`.
    fence_ids: Vec<FenceId>,
    snapshots: Vec<Snapshot>,
    /// Installed font families, read on the first use of the font list.
    fonts: Option<Vec<FontFamily>>,
    save_timer: Timer,
    style_timer: Timer,
    toast_timer: Timer,
    test_timer: Timer,
    refresh_timer: Timer,
}

pub type Shared = Rc<RefCell<Ctl>>;

impl Ctl {
    pub fn new(paths: Paths, config: AppConfig, state: DesktopState, lang: Lang) -> Ctl {
        Ctl {
            paths,
            config,
            state,
            lang,
            owner: Hwnd(0),
            monitors: bento_win::display::monitors(),
            fence_ids: Vec::new(),
            snapshots: Vec::new(),
            fonts: None,
            save_timer: Timer::default(),
            style_timer: Timer::default(),
            toast_timer: Timer::default(),
            test_timer: Timer::default(),
            refresh_timer: Timer::default(),
        }
    }

    fn t(&self, t: T) -> &'static str {
        t.get(self.lang)
    }

    /// Layout of the current display configuration.
    /// Index of the layout shown by the resident app, or of the one of the
    /// connected monitors.
    fn layout_index(&self) -> usize {
        let key = display_key(&self.monitors);
        let find = |k: &str| self.state.layouts.iter().position(|l| l.key == k);
        find(&self.state.current_layout)
            .or_else(|| find(&key))
            .unwrap_or(0)
    }

    fn layout(&self) -> Option<&Layout> {
        self.state.layouts.get(self.layout_index())
    }

    fn layout_mut(&mut self) -> Option<&mut Layout> {
        let idx = self.layout_index();
        self.state.layouts.get_mut(idx)
    }

    fn fence(&self, id: FenceId) -> Option<&Fence> {
        self.layout().and_then(|l| l.fence(id))
    }

    fn fence_mut(&mut self, id: FenceId) -> Option<&mut Fence> {
        self.layout_mut().and_then(|l| l.fence_mut(id))
    }

    fn fence_title(&self, id: FenceId) -> String {
        match self.fence(id).map(Fence::title) {
            Some("") => self.t(T::Untitled).to_owned(),
            Some(t) => t.to_owned(),
            None => self.t(T::NoFence).to_owned(),
        }
    }
}

// ----- helpers -------------------------------------------------------------

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn strings(v: Vec<String>) -> ModelRc<SharedString> {
    model(v.into_iter().map(SharedString::from).collect())
}

pub fn daemon_running() -> bool {
    bento_win::window::find_window(DAEMON_CLASS).is_some()
}

/// Sends a command to the resident app. False if it is not running.
pub fn send(cmd: &IpcCmd) -> bool {
    match bento_win::window::find_window(DAEMON_CLASS) {
        Some(h) => bento_win::system::send_copydata(h, COPYDATA_KIND, &cmd.to_bytes()),
        None => false,
    }
}

pub fn apply_language(lang: Lang) {
    let code = match lang {
        Lang::Fr => "fr",
        Lang::En => "en",
    };
    if let Err(e) = slint::select_bundled_translation(code) {
        eprintln!("translation {code}: {e}");
    }
}

fn toast(ctl: &Ctl, ui: &MainWindow, text: &str) {
    ui.global::<App>().set_toast(text.into());
    let weak = ui.as_weak();
    ctl.toast_timer
        .start(TimerMode::SingleShot, TOAST_DELAY, move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<App>().set_toast(SharedString::new());
            }
        });
}

fn daemon_exe() -> Option<std::path::PathBuf> {
    bento_win::system::exe_path().and_then(|p| p.parent().map(|d| d.join("bento-desk.exe")))
}

// ----- configuration -------------------------------------------------------

fn log_index(l: LogLevel) -> i32 {
    match l {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
        LogLevel::Trace => 4,
    }
}

fn log_level(i: i32) -> LogLevel {
    match i {
        0 => LogLevel::Error,
        1 => LogLevel::Warn,
        3 => LogLevel::Debug,
        4 => LogLevel::Trace,
        _ => LogLevel::Info,
    }
}

fn hk_text(h: Option<Hotkey>) -> SharedString {
    h.map(|h| h.to_string()).unwrap_or_default().into()
}

fn hk_parse(s: &str) -> Option<Hotkey> {
    Hotkey::parse(s).filter(Hotkey::is_valid)
}

fn push_config(ui: &MainWindow, c: &AppConfig) {
    let g = ui.global::<Cfg>();
    g.set_language(match c.general.language {
        LangPref::Auto => 0,
        LangPref::Fr => 1,
        LangPref::En => 2,
    });
    g.set_start_with_windows(c.general.start_with_windows);
    g.set_show_in_taskbar(c.general.show_in_taskbar);
    g.set_pause_on_fullscreen(c.general.pause_on_fullscreen);
    g.set_desktop_gestures(c.general.desktop_gestures);
    g.set_exclusions(c.general.game_mode_exclusions.join("\n").into());
    g.set_log_level(log_index(c.general.log_level));
    g.set_use_accent_color(c.appearance.use_accent_color);
    g.set_snap_to_edges(c.layout.snap_to_edges);
    g.set_snap_to_fences(c.layout.snap_to_fences);
    g.set_snap_distance(f32::from(c.layout.snap_distance));
    g.set_snap_gap(f32::from(c.layout.snap_gap));
    g.set_icon_spacing(f32::from(c.layout.icon_spacing));
    g.set_rollup_double_click(c.rollup.double_click_title);
    g.set_rollup_open_on(match c.rollup.open_on {
        OpenTrigger::Click => 0,
        OpenTrigger::Hover => 1,
    });
    g.set_rollup_auto_docked(c.rollup.auto_rollup_docked);
    g.set_rollup_animate(c.rollup.animate);
    g.set_qh_keep_free_icons(c.quick_hide.keep_free_icons);
    g.set_qh_idle_seconds(c.quick_hide.idle_seconds.min(3600) as i32);
    g.set_pages_enabled(c.pages.enabled);
    g.set_pages_edge_switch(c.pages.edge_switch);
    g.set_pages_indicator(c.pages.show_indicator);
    g.set_portal_view(match c.portals.default_view {
        ViewMode::Icons => 0,
        ViewMode::List => 1,
    });
    g.set_portal_hide_title_icon(c.portals.hide_title_icon);
    g.set_portal_show_hidden(c.portals.show_hidden_files);
    g.set_peek_dim(f32::from(c.peek.dim));
    g.set_layouts_auto_restore(c.layouts.auto_restore);
    g.set_rules_auto_apply(c.sorting.auto_apply);
    g.set_hk_peek(hk_text(c.hotkeys.peek));
    g.set_hk_quick_hide(hk_text(c.hotkeys.quick_hide));
    g.set_hk_next_page(hk_text(c.hotkeys.next_page));
    g.set_hk_prev_page(hk_text(c.hotkeys.prev_page));
}

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn pull_config(ui: &MainWindow, c: &mut AppConfig) {
    let g = ui.global::<Cfg>();
    c.general.language = match g.get_language() {
        1 => LangPref::Fr,
        2 => LangPref::En,
        _ => LangPref::Auto,
    };
    c.general.start_with_windows = g.get_start_with_windows();
    c.general.show_in_taskbar = g.get_show_in_taskbar();
    c.general.pause_on_fullscreen = g.get_pause_on_fullscreen();
    c.general.desktop_gestures = g.get_desktop_gestures();
    c.general.game_mode_exclusions = g
        .get_exclusions()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    c.general.log_level = log_level(g.get_log_level());
    c.appearance.use_accent_color = g.get_use_accent_color();
    c.layout.snap_to_edges = g.get_snap_to_edges();
    c.layout.snap_to_fences = g.get_snap_to_fences();
    c.layout.snap_distance = clamp_u8(g.get_snap_distance());
    c.layout.snap_gap = clamp_u8(g.get_snap_gap());
    c.layout.icon_spacing = g.get_icon_spacing().round().clamp(50.0, 300.0) as u16;
    c.rollup.double_click_title = g.get_rollup_double_click();
    c.rollup.open_on = if g.get_rollup_open_on() == 1 {
        OpenTrigger::Hover
    } else {
        OpenTrigger::Click
    };
    c.rollup.auto_rollup_docked = g.get_rollup_auto_docked();
    c.rollup.animate = g.get_rollup_animate();
    c.quick_hide.keep_free_icons = g.get_qh_keep_free_icons();
    c.quick_hide.idle_seconds = g.get_qh_idle_seconds().clamp(0, 3600) as u32;
    c.pages.enabled = g.get_pages_enabled();
    c.pages.edge_switch = g.get_pages_edge_switch();
    c.pages.show_indicator = g.get_pages_indicator();
    c.portals.default_view = if g.get_portal_view() == 1 {
        ViewMode::List
    } else {
        ViewMode::Icons
    };
    c.portals.hide_title_icon = g.get_portal_hide_title_icon();
    c.portals.show_hidden_files = g.get_portal_show_hidden();
    c.peek.dim = clamp_u8(g.get_peek_dim()).min(90);
    c.layouts.auto_restore = g.get_layouts_auto_restore();
    c.sorting.auto_apply = g.get_rules_auto_apply();
    c.hotkeys.peek = hk_parse(&g.get_hk_peek());
    c.hotkeys.quick_hide = hk_parse(&g.get_hk_quick_hide());
    c.hotkeys.next_page = hk_parse(&g.get_hk_next_page());
    c.hotkeys.prev_page = hk_parse(&g.get_hk_prev_page());
}

/// Writes `config.json`, applies the Run key and tells the resident app.
fn save_now(ctl: &Ctl, ui: &MainWindow) {
    if let Err(e) = bento_config::save_config(&ctl.paths, &ctl.config) {
        toast(ctl, ui, &T::SaveFailed.fmt(ctl.lang, &[&e.to_string()]));
        return;
    }
    if !ctl.paths.portable {
        let cmd = daemon_exe().map(|p| format!("\"{}\"", p.display()));
        let res = match (ctl.config.general.start_with_windows, cmd) {
            (true, Some(c)) => bento_win::system::set_autostart("BentoDesk", Some(&c)),
            (true, None) => Ok(()),
            (false, _) => bento_win::system::set_autostart("BentoDesk", None),
        };
        if let Err(e) = res {
            eprintln!("autostart: {e}");
        }
    }
    send(&IpcCmd::ReloadConfig);
}

fn schedule_save(ctl: &Ctl, weak: Weak<MainWindow>, shared: Shared) {
    ctl.save_timer
        .start(TimerMode::SingleShot, SAVE_DELAY, move || {
            if let (Some(ui), Ok(ctl)) = (weak.upgrade(), shared.try_borrow()) {
                save_now(&ctl, &ui);
            }
        });
}

// ----- fences ----------------------------------------------------------------

fn push_desk(ctl: &mut Ctl, ui: &MainWindow) {
    let lang = ctl.lang;
    let mut rows = Vec::new();
    let mut ids = Vec::new();
    let mut titles = Vec::new();
    let mut pages = 0;
    let mut description = String::new();
    if let Some(layout) = ctl.layout() {
        pages = layout.pages.len();
        description = layout.description.clone();
        for (pi, page) in layout.pages.iter().enumerate() {
            for f in &page.fences {
                let title = match f.title() {
                    "" => T::Untitled.get(lang).to_owned(),
                    t => t.to_owned(),
                };
                let page_no = (pi + 1).to_string();
                let active = f.active();
                let portal = active.is_some_and(|t| t.content.is_portal());
                let detail = match active.map(|t| &t.content) {
                    Some(bento_core::model::TabContent::Portal { root, .. }) => {
                        T::Portal.fmt(lang, &[&page_no, root])
                    }
                    _ => {
                        let n = f.all_items().count().to_string();
                        if f.tabs.len() > 1 {
                            T::PageTabs.fmt(lang, &[&page_no, &n, &f.tabs.len().to_string()])
                        } else {
                            T::PageIcons.fmt(lang, &[&page_no, &n])
                        }
                    }
                };
                rows.push(FenceRow {
                    id: f.id.0 as i32,
                    title: title.clone().into(),
                    detail: detail.into(),
                    auto_rollup: f.auto_rollup,
                    exempt: f.quick_hide_exempt,
                    custom_style: active.is_some_and(|t| t.style.is_some()),
                    portal,
                });
                ids.push(f.id);
                titles.push(title);
            }
        }
    }
    let d = ui.global::<Desk>();
    let previous = usize::try_from(d.get_selected())
        .ok()
        .and_then(|i| ctl.fence_ids.get(i).copied());
    let selected = previous
        .and_then(|id| ids.iter().position(|x| *x == id))
        .map_or(-1, |i| i as i32);
    ctl.fence_ids = ids;
    d.set_daemon_running(daemon_running());
    d.set_fences(model(rows));
    d.set_fence_titles(strings(titles.clone()));
    d.set_page_count(pages as i32);
    d.set_layout_name(description.into());
    d.set_selected(selected);
    let s = ui.global::<Style>();
    let mut targets = vec![ctl.t(T::DefaultStyle).to_owned()];
    targets.extend(titles);
    let target = s.get_target();
    s.set_targets(strings(targets));
    if target as usize > ctl.fence_ids.len() {
        s.set_target(0);
    }
}

fn select_fence(ctl: &Ctl, ui: &MainWindow, i: i32) {
    let d = ui.global::<Desk>();
    d.set_selected(i);
    let Some(f) = usize::try_from(i)
        .ok()
        .and_then(|i| ctl.fence_ids.get(i))
        .and_then(|id| ctl.fence(*id))
    else {
        return;
    };
    d.set_edit_title(f.title().into());
    d.set_edit_auto_rollup(f.auto_rollup);
    d.set_edit_exempt(f.quick_hide_exempt);
}

fn selected_fence(ctl: &Ctl, ui: &MainWindow) -> Option<FenceId> {
    usize::try_from(ui.global::<Desk>().get_selected())
        .ok()
        .and_then(|i| ctl.fence_ids.get(i).copied())
}

// ----- style -----------------------------------------------------------------

/// Colors offered as presets: (hue, saturation, intensity).
const PRESETS: &[(u16, u8, u8)] = &[
    (0, 0, 0),
    (215, 12, 19),
    (213, 51, 25),
    (180, 67, 18),
    (103, 37, 21),
    (40, 64, 22),
    (355, 50, 24),
    (263, 37, 26),
    (0, 0, 91),
];

fn target_fence(ctl: &Ctl, ui: &MainWindow) -> Option<FenceId> {
    let t = ui.global::<Style>().get_target();
    usize::try_from(t - 1)
        .ok()
        .and_then(|i| ctl.fence_ids.get(i).copied())
}

/// Style of the current target, and whether a fence uses the default.
fn target_style(ctl: &Ctl, ui: &MainWindow) -> (FenceStyle, bool) {
    let default = ctl.config.appearance.default_style.clone();
    match target_fence(ctl, ui) {
        None => (default, false),
        Some(id) => match ctl
            .fence(id)
            .and_then(Fence::active)
            .and_then(|t| t.style.clone())
        {
            Some(s) => (s, false),
            None => (default, true),
        },
    }
}

fn luminance(c: Rgba) -> u32 {
    (u32::from(c.r) * 299 + u32::from(c.g) * 587 + u32::from(c.b) * 114) / 1000
}

fn to_color(c: Rgba) -> slint::Color {
    slint::Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

const TITLE_WHITE: Rgba = Rgba::rgb(255, 255, 255);
const TITLE_BLACK: Rgba = Rgba::rgb(0, 0, 0);

fn title_color_index(c: Option<Rgba>) -> i32 {
    match c {
        None => 0,
        Some(TITLE_WHITE) => 1,
        Some(TITLE_BLACK) => 2,
        Some(_) => 3,
    }
}

/// Color from the picker values (hue in degrees, percentages).
fn hsl_rgba(h: f32, s: f32, l: f32) -> Rgba {
    Rgba::from_hsl(h, s / 100.0, l / 100.0, 255)
}

/// Picker values (hue in degrees, percentages) of a color.
fn rgba_hsl(c: Rgba) -> (f32, f32, f32) {
    let (h, s, l) = c.to_hsl();
    let hue = (h.round() as i32).rem_euclid(360) as f32;
    (hue, (s * 100.0).round(), (l * 100.0).round())
}

/// Default custom title band: the fence color, darker and a bit more opaque.
fn default_title_background(st: &FenceStyle) -> Tint {
    Tint {
        intensity: st.tint.intensity.saturating_sub(8),
        transparency: st.tint.transparency.saturating_sub(10),
        ..st.tint
    }
}

fn push_style(ctl: &Ctl, ui: &MainWindow) {
    let (st, use_default) = target_style(ctl, ui);
    let s = ui.global::<Style>();
    s.set_use_default(use_default);
    s.set_hue(f32::from(st.tint.hue));
    s.set_saturation(f32::from(st.tint.saturation));
    s.set_intensity(f32::from(st.tint.intensity));
    s.set_transparency(f32::from(st.tint.transparency));
    s.set_effect(match st.effect {
        BackdropEffect::Solid => 0,
        BackdropEffect::Blur => 1,
    });
    s.set_chameleon(st.chameleon);
    s.set_title_mode(match st.title_mode {
        TitleMode::Never => 0,
        TitleMode::Hover => 1,
        TitleMode::Always => 2,
    });
    s.set_title_font(st.title_font.clone().into());
    s.set_title_size(f32::from(st.title_size));
    s.set_title_bold(st.title_bold);
    s.set_title_align(match st.title_align {
        TitleAlign::Left => 0,
        TitleAlign::Center => 1,
        TitleAlign::Right => 2,
    });
    s.set_title_color(title_color_index(st.title_color));
    let (th, ts, tl) = rgba_hsl(st.title_color.unwrap_or(TITLE_WHITE));
    s.set_title_text_hue(th);
    s.set_title_text_saturation(ts);
    s.set_title_text_lightness(tl);
    s.set_title_bg_mode(i32::from(st.title_background.is_some()));
    let tb = st
        .title_background
        .unwrap_or_else(|| default_title_background(&st));
    s.set_title_bg_hue(f32::from(tb.hue));
    s.set_title_bg_saturation(f32::from(tb.saturation));
    s.set_title_bg_lightness(f32::from(tb.intensity));
    s.set_title_bg_transparency(f32::from(tb.transparency));
    s.set_corner_radius(f32::from(st.corner_radius));
    s.set_border(st.border);
    let title = match target_fence(ctl, ui) {
        Some(id) => ctl.fence_title(id),
        None => "Bureau".to_owned(),
    };
    s.set_preview_title(title.into());
    update_preview(ui, &st);
}

fn pull_style(ui: &MainWindow, base: &FenceStyle) -> FenceStyle {
    let s = ui.global::<Style>();
    let mut st = base.clone();
    st.tint = Tint {
        hue: (s.get_hue().round() as u16).min(359),
        saturation: clamp_u8(s.get_saturation()).min(100),
        intensity: clamp_u8(s.get_intensity()).min(100),
        transparency: clamp_u8(s.get_transparency()).min(100),
    };
    st.effect = if s.get_effect() == 0 {
        BackdropEffect::Solid
    } else {
        BackdropEffect::Blur
    };
    st.chameleon = s.get_chameleon();
    st.title_mode = match s.get_title_mode() {
        0 => TitleMode::Never,
        1 => TitleMode::Hover,
        _ => TitleMode::Always,
    };
    let font = s.get_title_font().trim().to_owned();
    if !font.is_empty() {
        st.title_font = font;
    }
    st.title_size = clamp_u8(s.get_title_size()).clamp(8, 32);
    st.title_bold = s.get_title_bold();
    st.title_align = match s.get_title_align() {
        1 => TitleAlign::Center,
        2 => TitleAlign::Right,
        _ => TitleAlign::Left,
    };
    st.title_color = match s.get_title_color() {
        1 => Some(TITLE_WHITE),
        2 => Some(TITLE_BLACK),
        3 => {
            let c = hsl_rgba(
                s.get_title_text_hue(),
                s.get_title_text_saturation(),
                s.get_title_text_lightness(),
            );
            // Pure white or black would read back as the fixed choices.
            Some(match c {
                TITLE_WHITE => Rgba::rgb(254, 254, 254),
                TITLE_BLACK => Rgba::rgb(1, 1, 1),
                c => c,
            })
        }
        _ => None,
    };
    st.title_background = (s.get_title_bg_mode() == 1).then(|| Tint {
        hue: (s.get_title_bg_hue().round() as u16).min(359),
        saturation: clamp_u8(s.get_title_bg_saturation()).min(100),
        intensity: clamp_u8(s.get_title_bg_lightness()).min(100),
        transparency: clamp_u8(s.get_title_bg_transparency()).min(100),
    });
    st.corner_radius = clamp_u8(s.get_corner_radius()).min(32);
    st.border = s.get_border();
    st
}

fn update_preview(ui: &MainWindow, st: &FenceStyle) {
    let s = ui.global::<Style>();
    let fill = st.tint.to_rgba();
    s.set_preview_fill(to_color(fill));
    s.set_preview_title_fill(to_color(st.title_band()));
    let text = st.title_color.unwrap_or_else(|| {
        // Contrast against the tint laid over a mid-tone wallpaper, then
        // the title band laid over both.
        let a = u32::from(fill.a);
        let mix = |c: u8| ((u32::from(c) * a + 110 * (255 - a)) / 255) as u8;
        let body = Rgba::rgb(mix(fill.r), mix(fill.g), mix(fill.b));
        let band = st.title_band();
        let seen = body.mix(band.with_alpha(255), f32::from(band.a) / 255.0);
        if luminance(seen) >= 150 {
            Rgba::rgb(20, 20, 20)
        } else {
            Rgba::rgb(255, 255, 255)
        }
    });
    s.set_preview_text(to_color(text));
}

/// Applies the edited style to its target.
fn style_changed(ctl: &mut Ctl, ui: &MainWindow, weak: Weak<MainWindow>, shared: Shared) {
    let (base, _) = target_style(ctl, ui);
    let st = pull_style(ui, &base);
    let use_default = ui.global::<Style>().get_use_default();
    match target_fence(ctl, ui) {
        None => {
            update_preview(ui, &st);
            ctl.config.appearance.default_style = st;
            schedule_save(ctl, weak, shared);
        }
        Some(id) => {
            let style = (!use_default).then_some(st);
            update_preview(
                ui,
                style
                    .as_ref()
                    .unwrap_or(&ctl.config.appearance.default_style),
            );
            if let Some(t) = ctl.fence_mut(id).and_then(Fence::active_mut) {
                t.style = style.clone();
            }
            let cmd = IpcCmd::SetFenceStyle { fence: id, style };
            ctl.style_timer
                .start(TimerMode::SingleShot, STYLE_DELAY, move || {
                    send(&cmd);
                });
        }
    }
}

/// Shows the installed fonts whose name contains `query` (any case).
fn search_fonts(ctl: &mut Ctl, ui: &MainWindow, query: &str) {
    let fonts = ctl.fonts.get_or_insert_with(|| {
        bento_win::gfx::system_font_families().unwrap_or_else(|e| {
            eprintln!("fonts: {e}");
            Vec::new()
        })
    });
    let q = query.trim().to_lowercase();
    let items: Vec<FontItem> = fonts
        .iter()
        .filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q))
        .map(|f| FontItem {
            name: f.name.as_str().into(),
            symbol: f.symbol,
        })
        .collect();
    let s = ui.global::<Style>();
    let current = s.get_title_font();
    let index = items
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case(&current))
        .map_or(-1, |i| i as i32);
    s.set_fonts(model(items));
    s.set_font_index(index);
}

// ----- defaults --------------------------------------------------------------

/// Puts back the default values of the settings shown on one page (index
/// of `App.page`). Data such as rules, layouts or the styles of single
/// fences is kept. Returns false for a page without settings.
fn reset_page(c: &mut AppConfig, page: i32) -> bool {
    let d = AppConfig::default();
    match page {
        2 => c.appearance = d.appearance,
        3 => c.layout = d.layout,
        4 => {
            c.rollup = d.rollup;
            c.quick_hide = d.quick_hide;
            c.hotkeys.quick_hide = d.hotkeys.quick_hide;
        }
        5 => {
            c.pages = d.pages;
            c.hotkeys.next_page = d.hotkeys.next_page;
            c.hotkeys.prev_page = d.hotkeys.prev_page;
        }
        6 => c.sorting.auto_apply = d.sorting.auto_apply,
        7 => c.portals = d.portals,
        8 => {
            c.peek = d.peek;
            c.hotkeys.peek = d.hotkeys.peek;
        }
        9 => c.layouts = d.layouts,
        10 => {
            let log_level = c.general.log_level;
            c.general = d.general;
            c.general.log_level = log_level;
        }
        12 => c.general.log_level = d.general.log_level,
        _ => return false,
    }
    true
}

// ----- rules -----------------------------------------------------------------

fn text_op_index(op: TextOp) -> i32 {
    match op {
        TextOp::Contains => 0,
        TextOp::StartsWith => 1,
        TextOp::Equals => 2,
    }
}

fn text_op(i: i32) -> TextOp {
    match i {
        1 => TextOp::StartsWith,
        2 => TextOp::Equals,
        _ => TextOp::Contains,
    }
}

const KINDS: [ItemKind; 5] = [
    ItemKind::Shortcut,
    ItemKind::Folder,
    ItemKind::File,
    ItemKind::Url,
    ItemKind::Special,
];

fn kind_index(k: ItemKind) -> i32 {
    KINDS.iter().position(|x| *x == k).unwrap_or(2) as i32
}

fn cond_row(c: &Condition) -> CondRow {
    let mut row = CondRow {
        field: 0,
        op: 0,
        value: SharedString::new(),
        kind: 2,
        date_field: 0,
        days: 7,
    };
    match c {
        Condition::Name { op, value } => {
            row.op = text_op_index(*op);
            row.value = value.as_str().into();
        }
        Condition::Extension { values } => {
            row.field = 1;
            row.value = values.join(", ").into();
        }
        Condition::Kind { kind } => {
            row.field = 2;
            row.kind = kind_index(*kind);
        }
        Condition::Target { op, value } => {
            row.field = 3;
            row.op = text_op_index(*op);
            row.value = value.as_str().into();
        }
        Condition::Date { date, op, days } => {
            row.field = 4;
            row.date_field = i32::from(*date == DateField::Created);
            row.op = i32::from(*op == DateOp::OlderThanDays);
            row.days = (*days).min(3650) as i32;
        }
    }
    row
}

fn extensions(s: &str) -> Vec<String> {
    s.split([',', ';', ' '])
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

/// Condition after one edit. `key` names the edited part.
fn edit_condition(c: &Condition, key: &str, ival: i32, sval: &str) -> Condition {
    let row = cond_row(c);
    let text = match c {
        Condition::Name { value, .. } | Condition::Target { value, .. } => value.clone(),
        Condition::Extension { values } => values.join(", "),
        _ => String::new(),
    };
    let field = if key == "field" { ival } else { row.field };
    let op = if key == "op" { ival } else { row.op };
    let value = if key == "value" {
        sval.to_owned()
    } else {
        text
    };
    let kind = if key == "kind" { ival } else { row.kind };
    let date_field = if key == "date" { ival } else { row.date_field };
    let days = if key == "days" { ival } else { row.days };
    let changed_field = key == "field" && field != row.field;
    match field {
        1 => Condition::Extension {
            values: extensions(&value),
        },
        2 => Condition::Kind {
            kind: KINDS
                .get(usize::try_from(kind).unwrap_or(2))
                .copied()
                .unwrap_or(ItemKind::File),
        },
        3 => Condition::Target {
            op: text_op(if changed_field { 0 } else { op }),
            value,
        },
        4 => Condition::Date {
            date: if date_field == 1 {
                DateField::Created
            } else {
                DateField::Modified
            },
            op: if !changed_field && op == 1 {
                DateOp::OlderThanDays
            } else {
                DateOp::NewerThanDays
            },
            days: days.clamp(0, 3650) as u32,
        },
        _ => Condition::Name {
            op: text_op(if changed_field { 0 } else { op }),
            value,
        },
    }
}

fn op_text(lang: Lang, op: TextOp) -> &'static str {
    match op {
        TextOp::Contains => T::OpContains.get(lang),
        TextOp::StartsWith => T::OpStarts.get(lang),
        TextOp::Equals => T::OpEquals.get(lang),
    }
}

fn kind_text(lang: Lang, k: ItemKind) -> &'static str {
    match k {
        ItemKind::Shortcut => T::KindShortcut.get(lang),
        ItemKind::Folder => T::KindFolder.get(lang),
        ItemKind::File => T::KindFile.get(lang),
        ItemKind::Url => T::KindUrl.get(lang),
        ItemKind::Special => T::KindSpecial.get(lang),
    }
}

fn summary(lang: Lang, r: &Rule) -> String {
    if r.conditions.is_empty() {
        return T::NoCondition.get(lang).to_owned();
    }
    let parts: Vec<String> = r
        .conditions
        .iter()
        .map(|c| match c {
            Condition::Name { op, value } => T::CondName.fmt(lang, &[op_text(lang, *op), value]),
            Condition::Extension { values } => T::CondExt.fmt(lang, &[&values.join(", ")]),
            Condition::Kind { kind } => T::CondKind.fmt(lang, &[kind_text(lang, *kind)]),
            Condition::Target { op, value } => {
                T::CondTarget.fmt(lang, &[op_text(lang, *op), value])
            }
            Condition::Date { date, op, days } => {
                let d = match date {
                    DateField::Modified => T::Modified.get(lang),
                    DateField::Created => T::Created.get(lang),
                };
                let o = match op {
                    DateOp::NewerThanDays => T::LessThan.get(lang),
                    DateOp::OlderThanDays => T::MoreThan.get(lang),
                };
                T::CondDate.fmt(lang, &[d, o, &days.to_string()])
            }
        })
        .collect();
    let sep = if r.match_all {
        T::And.get(lang)
    } else {
        T::Or.get(lang)
    };
    parts.join(sep)
}

fn selected_rule(ui: &MainWindow) -> Option<usize> {
    usize::try_from(ui.global::<Rules>().get_selected()).ok()
}

fn push_rule_rows(ctl: &Ctl, ui: &MainWindow) {
    let rows: Vec<RuleRow> = ctl
        .config
        .sorting
        .rules
        .iter()
        .map(|r| RuleRow {
            name: r.name.as_str().into(),
            enabled: r.enabled,
            target: ctl.fence_title(r.target).into(),
            summary: summary(ctl.lang, r).into(),
        })
        .collect();
    ui.global::<Rules>().set_rules(model(rows));
}

fn push_conditions(ctl: &Ctl, ui: &MainWindow) {
    let rows: Vec<CondRow> = selected_rule(ui)
        .and_then(|i| ctl.config.sorting.rules.get(i))
        .map(|r| r.conditions.iter().map(cond_row).collect())
        .unwrap_or_default();
    ui.global::<Rules>().set_conditions(model(rows));
}

fn push_rule_editor(ctl: &Ctl, ui: &MainWindow) {
    let g = ui.global::<Rules>();
    let Some(r) = selected_rule(ui).and_then(|i| ctl.config.sorting.rules.get(i)) else {
        g.set_selected(-1);
        return;
    };
    g.set_name(r.name.as_str().into());
    g.set_enabled(r.enabled);
    g.set_match_mode(i32::from(!r.match_all));
    g.set_target(
        ctl.fence_ids
            .iter()
            .position(|id| *id == r.target)
            .map_or(-1, |i| i as i32),
    );
    push_conditions(ctl, ui);
    g.set_test_results(strings(Vec::new()));
    g.set_test_status(ctl.t(T::TestHint).into());
}

fn push_rules(ctl: &Ctl, ui: &MainWindow) {
    let g = ui.global::<Rules>();
    if selected_rule(ui).is_some_and(|i| i >= ctl.config.sorting.rules.len()) {
        g.set_selected(-1);
    }
    push_rule_rows(ctl, ui);
    push_rule_editor(ctl, ui);
}

fn run_test(ctl: &Ctl, ui: &MainWindow) {
    let g = ui.global::<Rules>();
    let Some(rule) = selected_rule(ui).and_then(|i| ctl.config.sorting.rules.get(i)) else {
        return;
    };
    let items: Option<Vec<ItemInfo>> = std::fs::read(ctl.paths.items_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(items) = items else {
        g.set_test_status(ctl.t(T::TestNoData).into());
        g.set_test_results(strings(Vec::new()));
        return;
    };
    let mut names: Vec<String> = bento_core::rules::test_rule(rule, &items, bento_core::now_secs())
        .into_iter()
        .map(|i| i.name.clone())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    let status = if names.is_empty() {
        ctl.t(T::TestNone).to_owned()
    } else {
        T::TestCount.fmt(ctl.lang, &[&names.len().to_string()])
    };
    names.truncate(200);
    g.set_test_status(status.into());
    g.set_test_results(strings(names));
}

// ----- snapshots ---------------------------------------------------------------

fn date_text(lang: Lang, secs: u64) -> String {
    match bento_win::system::local_time(secs) {
        Some((y, mo, d, h, mi)) => match lang {
            Lang::Fr => format!("{d:02}/{mo:02}/{y} {h:02}:{mi:02}"),
            Lang::En => format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}"),
        },
        None => String::new(),
    }
}

fn push_snaps(ctl: &mut Ctl, ui: &MainWindow) {
    ctl.snapshots = bento_config::list_snapshots(&ctl.paths);
    let rows: Vec<SnapRow> = ctl
        .snapshots
        .iter()
        .map(|s| {
            let n: usize = s.layout.pages.iter().map(|p| p.fences.len()).sum();
            SnapRow {
                name: s.name.as_str().into(),
                date: date_text(ctl.lang, s.created).into(),
                fences: T::SnapFences.fmt(ctl.lang, &[&n.to_string()]).into(),
            }
        })
        .collect();
    let g = ui.global::<Snaps>();
    g.set_items(model(rows));
    let names: Vec<String> = ctl
        .monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            format!(
                "{}. {} ({}x{})",
                i + 1,
                m.name,
                m.rect.width(),
                m.rect.height()
            )
        })
        .collect();
    g.set_monitors(strings(names));
}

// ----- installation ------------------------------------------------------------

/// Fills every page.
pub fn push_all(ctl: &mut Ctl, ui: &MainWindow) {
    ui.set_window_title(ctl.t(T::WindowTitle).into());
    let app = ui.global::<App>();
    app.set_version(env!("CARGO_PKG_VERSION").into());
    app.set_portable(ctl.paths.portable);
    app.set_data_dir(ctl.paths.data_dir.display().to_string().into());
    push_config(ui, &ctl.config);
    push_desk(ctl, ui);
    push_style(ctl, ui);
    push_rules(ctl, ui);
    push_snaps(ctl, ui);
}

/// Opens a section given on the command line.
pub fn open_section(ctl: &Ctl, ui: &MainWindow, section: &str, fence: Option<u64>) {
    let page = match section {
        "fences" => 1,
        "appearance" => 2,
        "layout" | "magnetism" => 3,
        "rollup" | "quickhide" | "behavior" => 4,
        "pages" => 5,
        "rules" | "sorting" => 6,
        "portals" => 7,
        "peek" | "hotkeys" => 8,
        "layouts" | "snapshots" => 9,
        "general" => 10,
        "backup" => 11,
        "troubleshooting" | "debug" => 12,
        "about" => 13,
        _ => 0,
    };
    ui.global::<App>().set_page(page);
    if let Some(id) = fence
        && let Some(i) = ctl.fence_ids.iter().position(|f| f.0 == id)
    {
        ui.global::<Style>().set_target(i as i32 + 1);
        ui.global::<Desk>().set_selected(i as i32);
        select_fence(ctl, ui, i as i32);
        push_style(ctl, ui);
    }
}

/// Connects every callback of the globals.
/// Color conversions used by the color pickers. Installed right after the
/// window is created, before any page binding evaluates them.
pub fn install_color_util(ui: &MainWindow) {
    let cu = ui.global::<ColorUtil>();
    cu.on_hsl(|h, s, l| to_color(hsl_rgba(h, s, l)));
    cu.on_to_hex(|h, s, l| hsl_rgba(h, s, l).to_hex().into());
    cu.on_parse_hex(|t| match Rgba::from_hex(&t) {
        Some(c) => {
            let (hue, saturation, lightness) = rgba_hsl(c);
            Hsl {
                ok: true,
                hue,
                saturation,
                lightness,
            }
        }
        None => Hsl::default(),
    });
}

/// Applies the language of the configuration to the whole window.
fn language_changed(ctl: &mut Ctl, ui: &MainWindow) {
    ctl.lang = ctl
        .config
        .general
        .language
        .resolve(bento_win::system::ui_language());
    apply_language(ctl.lang);
    ui.set_window_title(ctl.t(T::WindowTitle).into());
    push_desk(ctl, ui);
    push_style(ctl, ui);
    push_rules(ctl, ui);
    push_snaps(ctl, ui);
}

pub fn install(ui: &MainWindow, shared: &Shared) {
    // Runs `f` with the window and the state, if both are available.
    macro_rules! on {
        ($global:ident, $setter:ident, |$ui:ident, $ctl:ident, $weak:ident, $sh:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            let weak = ui.as_weak();
            let sh = shared.clone();
            ui.global::<$global>().$setter(move |$($arg: $ty),*| {
                let Some($ui) = weak.upgrade() else { return };
                let $sh = sh.clone();
                let $weak = weak.clone();
                #[allow(unused_mut)]
                let Ok(mut $ctl) = sh.try_borrow_mut() else { return };
                let _ = (&$ui, &$weak, &$sh);
                $body
            });
        }};
    }

    // Configuration.
    on!(Cfg, on_changed, |ui, ctl, weak, sh| {
        let mut c = ctl.config.clone();
        pull_config(&ui, &mut c);
        ctl.config = c;
        schedule_save(&ctl, weak, sh);
    });
    on!(Cfg, on_language_changed, |ui, ctl, weak, sh| {
        language_changed(&mut ctl, &ui);
    });
    on!(Cfg, on_reset_page, |ui, ctl, weak, sh, page: i32| {
        let language = ctl.config.general.language;
        if !reset_page(&mut ctl.config, page) {
            return;
        }
        push_config(&ui, &ctl.config);
        if page == 2 {
            // Show the default style that was just put back.
            ui.global::<Style>().set_target(0);
            push_style(&ctl, &ui);
        }
        if ctl.config.general.language != language {
            language_changed(&mut ctl, &ui);
        }
        schedule_save(&ctl, weak, sh);
        toast(&ctl, &ui, ctl.t(T::DefaultsRestored));
    });
    ui.global::<Cfg>()
        .on_capture(|text, ctrl, alt, shift, meta| {
            crate::keys::capture(&text, ctrl, alt, shift, meta).into()
        });
    ui.global::<App>().on_open_url(|url| {
        bento_win::system::shell_open(&url);
    });

    // Fences.
    on!(Desk, on_select, |ui, ctl, weak, sh, i: i32| {
        select_fence(&ctl, &ui, i);
    });
    on!(Desk, on_rename, |ui, ctl, weak, sh| {
        let Some(id) = selected_fence(&ctl, &ui) else {
            return;
        };
        let title = ui.global::<Desk>().get_edit_title().trim().to_owned();
        if let Some(t) = ctl.fence_mut(id).and_then(Fence::active_mut) {
            t.title = title.clone();
        }
        send(&IpcCmd::RenameFence { fence: id, title });
        push_desk(&mut ctl, &ui);
        push_rule_rows(&ctl, &ui);
    });
    on!(Desk, on_options_changed, |ui, ctl, weak, sh| {
        let Some(id) = selected_fence(&ctl, &ui) else {
            return;
        };
        let d = ui.global::<Desk>();
        let (auto_rollup, quick_hide_exempt) = (d.get_edit_auto_rollup(), d.get_edit_exempt());
        if let Some(f) = ctl.fence_mut(id) {
            f.auto_rollup = auto_rollup;
            f.quick_hide_exempt = quick_hide_exempt;
        }
        send(&IpcCmd::SetFenceOptions {
            fence: id,
            auto_rollup,
            quick_hide_exempt,
        });
    });
    on!(Desk, on_customize, |ui, ctl, weak, sh| {
        let sel = ui.global::<Desk>().get_selected();
        if sel >= 0 {
            ui.global::<Style>().set_target(sel + 1);
            push_style(&ctl, &ui);
            ui.global::<App>().set_page(2);
        }
    });
    on!(Desk, on_refresh, |ui, ctl, weak, sh| {
        ctl.state = bento_config::load_state(&ctl.paths);
        ctl.monitors = bento_win::display::monitors();
        push_desk(&mut ctl, &ui);
        let sel = ui.global::<Desk>().get_selected();
        if sel >= 0 {
            select_fence(&ctl, &ui, sel);
        }
        push_style(&ctl, &ui);
        push_rule_rows(&ctl, &ui);
        push_snaps(&mut ctl, &ui);
    });

    // Style.
    on!(Style, on_changed, |ui, ctl, weak, sh| {
        style_changed(&mut ctl, &ui, weak, sh);
    });
    on!(Style, on_target_changed, |ui, ctl, weak, sh| {
        push_style(&ctl, &ui);
    });
    on!(Style, on_preset, |ui, ctl, weak, sh, i: i32| {
        if let Some((h, s, l)) = usize::try_from(i).ok().and_then(|i| PRESETS.get(i)) {
            let g = ui.global::<Style>();
            g.set_hue(f32::from(*h));
            g.set_saturation(f32::from(*s));
            g.set_intensity(f32::from(*l));
            style_changed(&mut ctl, &ui, weak, sh);
        }
    });
    on!(Style, on_apply_to_all, |ui, ctl, weak, sh| {
        let (base, _) = target_style(&ctl, &ui);
        let st = pull_style(&ui, &base);
        ctl.config.appearance.default_style = st;
        for l in &mut ctl.state.layouts {
            for p in &mut l.pages {
                for f in &mut p.fences {
                    for t in &mut f.tabs {
                        t.style = None;
                    }
                }
            }
        }
        save_now(&ctl, &ui);
        send(&IpcCmd::SetAllFenceStyles { style: None });
        push_desk(&mut ctl, &ui);
        push_style(&ctl, &ui);
        toast(&ctl, &ui, ctl.t(T::StyleAll));
    });
    on!(
        Style,
        on_search_fonts,
        |ui, ctl, weak, sh, query: SharedString| {
            search_fonts(&mut ctl, &ui, &query);
        }
    );
    on!(Style, on_reset, |ui, ctl, weak, sh| {
        match target_fence(&ctl, &ui) {
            None => {
                ctl.config.appearance.default_style = FenceStyle::default();
                schedule_save(&ctl, weak, sh);
            }
            Some(id) => {
                if let Some(t) = ctl.fence_mut(id).and_then(Fence::active_mut) {
                    t.style = None;
                }
                send(&IpcCmd::SetFenceStyle {
                    fence: id,
                    style: None,
                });
            }
        }
        push_style(&ctl, &ui);
    });

    // Rules.
    on!(Rules, on_select, |ui, ctl, weak, sh, i: i32| {
        ui.global::<Rules>().set_selected(i);
        push_rule_editor(&ctl, &ui);
    });
    on!(Rules, on_add, |ui, ctl, weak, sh| {
        let id = ctl
            .config
            .sorting
            .rules
            .iter()
            .map(|r| r.id)
            .max()
            .unwrap_or(0)
            + 1;
        let target = ctl.fence_ids.first().copied().unwrap_or_default();
        let name = ctl.t(T::NewRule).to_owned();
        ctl.config.sorting.rules.push(Rule {
            id,
            name,
            enabled: true,
            match_all: true,
            conditions: vec![Condition::Extension { values: Vec::new() }],
            target,
        });
        let n = ctl.config.sorting.rules.len() as i32;
        ui.global::<Rules>().set_selected(n - 1);
        push_rules(&ctl, &ui);
        schedule_save(&ctl, weak, sh);
    });
    on!(Rules, on_remove, |ui, ctl, weak, sh| {
        if let Some(i) = selected_rule(&ui).filter(|i| *i < ctl.config.sorting.rules.len()) {
            ctl.config.sorting.rules.remove(i);
            ui.global::<Rules>().set_selected(-1);
            push_rules(&ctl, &ui);
            schedule_save(&ctl, weak, sh);
        }
    });
    on!(Rules, on_move, |ui, ctl, weak, sh, delta: i32| {
        let Some(i) = selected_rule(&ui) else { return };
        let j = i as i64 + i64::from(delta);
        let n = ctl.config.sorting.rules.len() as i64;
        if i as i64 >= n || j < 0 || j >= n {
            return;
        }
        ctl.config.sorting.rules.swap(i, j as usize);
        ui.global::<Rules>().set_selected(j as i32);
        push_rules(&ctl, &ui);
        schedule_save(&ctl, weak, sh);
    });
    on!(
        Rules,
        on_set_enabled,
        |ui, ctl, weak, sh, i: i32, on: bool| {
            if let Some(r) = usize::try_from(i)
                .ok()
                .and_then(|i| ctl.config.sorting.rules.get_mut(i))
            {
                r.enabled = on;
                if selected_rule(&ui) == usize::try_from(i).ok() {
                    ui.global::<Rules>().set_enabled(on);
                }
                schedule_save(&ctl, weak, sh);
            }
        }
    );
    on!(Rules, on_edited, |ui, ctl, weak, sh| {
        let g = ui.global::<Rules>();
        let target = usize::try_from(g.get_target())
            .ok()
            .and_then(|i| ctl.fence_ids.get(i).copied());
        let Some(i) = selected_rule(&ui) else { return };
        let Some(r) = ctl.config.sorting.rules.get_mut(i) else {
            return;
        };
        r.name = g.get_name().trim().to_owned();
        r.match_all = g.get_match_mode() == 0;
        if let Some(t) = target {
            r.target = t;
        }
        push_rule_rows(&ctl, &ui);
        schedule_save(&ctl, weak, sh);
    });
    on!(Rules, on_cond_add, |ui, ctl, weak, sh| {
        let Some(i) = selected_rule(&ui) else { return };
        if let Some(r) = ctl.config.sorting.rules.get_mut(i) {
            r.conditions.push(Condition::Name {
                op: TextOp::Contains,
                value: String::new(),
            });
            push_conditions(&ctl, &ui);
            push_rule_rows(&ctl, &ui);
            schedule_save(&ctl, weak, sh);
        }
    });
    on!(Rules, on_cond_remove, |ui, ctl, weak, sh, ci: i32| {
        let Some(i) = selected_rule(&ui) else { return };
        let Ok(ci) = usize::try_from(ci) else { return };
        if let Some(r) = ctl.config.sorting.rules.get_mut(i)
            && ci < r.conditions.len()
        {
            r.conditions.remove(ci);
            push_conditions(&ctl, &ui);
            push_rule_rows(&ctl, &ui);
            schedule_save(&ctl, weak, sh);
        }
    });
    on!(
        Rules,
        on_cond_set,
        |ui, ctl, weak, sh, ci: i32, key: SharedString, ival: i32, sval: SharedString| {
            let Some(i) = selected_rule(&ui) else { return };
            let Ok(ci) = usize::try_from(ci) else { return };
            let Some(c) = ctl
                .config
                .sorting
                .rules
                .get_mut(i)
                .and_then(|r| r.conditions.get_mut(ci))
            else {
                return;
            };
            *c = edit_condition(c, &key, ival, &sval);
            // Rebuilding the rows while typing would drop the keyboard focus.
            if key != "value" && key != "days" {
                push_conditions(&ctl, &ui);
            }
            push_rule_rows(&ctl, &ui);
            schedule_save(&ctl, weak, sh);
        }
    );
    on!(Rules, on_test, |ui, ctl, weak, sh| {
        // The resident app publishes the current desktop items on request.
        if send(&IpcCmd::PublishItems) {
            ui.global::<Rules>()
                .set_test_status(ctl.t(T::TestRunning).into());
            let (weak, sh) = (weak.clone(), sh.clone());
            ctl.test_timer
                .start(TimerMode::SingleShot, PUBLISH_DELAY, move || {
                    if let (Some(ui), Ok(ctl)) = (weak.upgrade(), sh.try_borrow()) {
                        run_test(&ctl, &ui);
                    }
                });
        } else {
            run_test(&ctl, &ui);
        }
    });
    on!(Rules, on_sort_now, |ui, ctl, weak, sh| {
        ctl.save_timer.stop();
        save_now(&ctl, &ui);
        if send(&IpcCmd::SortNow) {
            toast(&ctl, &ui, ctl.t(T::Sorted));
        } else {
            toast(&ctl, &ui, ctl.t(T::NotRunning));
        }
    });

    // Snapshots.
    on!(Snaps, on_save, |ui, ctl, weak, sh| {
        let g = ui.global::<Snaps>();
        let name = g.get_new_name().trim().to_owned();
        if !send(&IpcCmd::SaveSnapshot { name }) {
            toast(&ctl, &ui, ctl.t(T::NotRunning));
            return;
        }
        g.set_new_name(SharedString::new());
        toast(&ctl, &ui, ctl.t(T::SnapSaved));
        let (weak, sh) = (weak.clone(), sh.clone());
        ctl.refresh_timer
            .start(TimerMode::SingleShot, REFRESH_DELAY, move || {
                if let (Some(ui), Ok(mut ctl)) = (weak.upgrade(), sh.try_borrow_mut()) {
                    push_snaps(&mut ctl, &ui);
                }
            });
    });
    on!(Snaps, on_restore, |ui, ctl, weak, sh, i: i32| {
        let Some(name) = usize::try_from(i)
            .ok()
            .and_then(|i| ctl.snapshots.get(i))
            .map(|s| s.name.clone())
        else {
            return;
        };
        if send(&IpcCmd::RestoreSnapshot { name: name.clone() }) {
            toast(&ctl, &ui, &T::SnapRestored.fmt(ctl.lang, &[&name]));
        } else {
            toast(&ctl, &ui, ctl.t(T::NotRunning));
        }
    });
    on!(Snaps, on_delete, |ui, ctl, weak, sh, i: i32| {
        let Some(name) = usize::try_from(i)
            .ok()
            .and_then(|i| ctl.snapshots.get(i))
            .map(|s| s.name.clone())
        else {
            return;
        };
        match bento_config::delete_snapshot(&ctl.paths, &name) {
            Ok(()) => toast(&ctl, &ui, ctl.t(T::SnapDeleted)),
            Err(e) => toast(&ctl, &ui, &T::SaveFailed.fmt(ctl.lang, &[&e.to_string()])),
        }
        push_snaps(&mut ctl, &ui);
    });
    on!(Snaps, on_swap, |ui, ctl, weak, sh| {
        let g = ui.global::<Snaps>();
        let pick = |i: i32| {
            usize::try_from(i)
                .ok()
                .and_then(|i| ctl.monitors.get(i))
                .map(|m| m.id.clone())
        };
        if let (Some(a), Some(b)) = (pick(g.get_swap_a()), pick(g.get_swap_b()))
            && a != b
        {
            if send(&IpcCmd::SwapScreens { a, b }) {
                toast(&ctl, &ui, ctl.t(T::Swapped));
            } else {
                toast(&ctl, &ui, ctl.t(T::NotRunning));
            }
        }
    });
    on!(Snaps, on_refresh, |ui, ctl, weak, sh| {
        push_snaps(&mut ctl, &ui);
    });

    // System actions.
    {
        let weak = ui.as_weak();
        let sh = shared.clone();
        ui.global::<Sys>().on_export(move || {
            let Some(ui) = weak.upgrade() else { return };
            // The dialog runs a modal loop: no borrow is held meanwhile.
            let (owner, lang) = match sh.try_borrow() {
                Ok(c) => (c.owner, c.lang),
                Err(_) => return,
            };
            let filter = (T::ExportFilter.get(lang), "*.json");
            let Some(path) =
                bento_win::shell::pick_save_file(owner, filter, "bento-desk-export.json", "json")
            else {
                return;
            };
            let Ok(ctl) = sh.try_borrow() else { return };
            match bento_config::export_all(&ctl.paths, std::path::Path::new(&path)) {
                Ok(()) => toast(&ctl, &ui, ctl.t(T::Exported)),
                Err(e) => toast(&ctl, &ui, &T::ExportFailed.fmt(ctl.lang, &[&e.to_string()])),
            }
        });
    }
    {
        let weak = ui.as_weak();
        let sh = shared.clone();
        ui.global::<Sys>().on_import(move || {
            let Some(ui) = weak.upgrade() else { return };
            let (owner, lang) = match sh.try_borrow() {
                Ok(c) => (c.owner, c.lang),
                Err(_) => return,
            };
            let filter = (T::ExportFilter.get(lang), "*.json");
            let Some(path) = bento_win::shell::pick_open_file(owner, filter) else {
                return;
            };
            let Ok(mut ctl) = sh.try_borrow_mut() else {
                return;
            };
            let res = bento_config::read_export(std::path::Path::new(&path))
                .and_then(|b| bento_config::import_all(&ctl.paths, &b).map(|()| b));
            match res {
                Ok(bundle) => {
                    ctl.config = bundle.config;
                    ctl.state = bundle.state;
                    send(&IpcCmd::ReloadAll);
                    ctl.lang = ctl
                        .config
                        .general
                        .language
                        .resolve(bento_win::system::ui_language());
                    apply_language(ctl.lang);
                    push_all(&mut ctl, &ui);
                    toast(&ctl, &ui, ctl.t(T::Imported));
                }
                Err(e) => toast(&ctl, &ui, &T::ImportFailed.fmt(ctl.lang, &[&e.to_string()])),
            }
        });
    }
    on!(Sys, on_debug_package, |ui, ctl, weak, sh| {
        let msg = if send(&IpcCmd::DebugPackage) {
            T::DebugCreated
        } else {
            T::NotRunning
        };
        toast(&ctl, &ui, ctl.t(msg));
    });
    on!(Sys, on_recover_icons, |ui, ctl, weak, sh| {
        let msg = if send(&IpcCmd::RecoverIcons) {
            T::Recovered
        } else {
            T::NotRunning
        };
        toast(&ctl, &ui, ctl.t(msg));
    });
    on!(Sys, on_restore_explorer, |ui, ctl, weak, sh| {
        let msg = if send(&IpcCmd::RestoreExplorerOptions) {
            T::ExplorerRestored
        } else {
            T::NotRunning
        };
        toast(&ctl, &ui, ctl.t(msg));
    });
    on!(Sys, on_open_logs, |ui, ctl, weak, sh| {
        bento_win::system::open_in_explorer(&ctl.paths.log_dir.to_string_lossy());
    });
    on!(Sys, on_open_data, |ui, ctl, weak, sh| {
        bento_win::system::open_in_explorer(&ctl.paths.data_dir.to_string_lossy());
    });
    on!(Sys, on_quit_daemon, |ui, ctl, weak, sh| {
        send(&IpcCmd::Quit);
        toast(&ctl, &ui, ctl.t(T::Stopped));
        refresh_status_later(&ctl, weak, sh);
    });
    on!(Sys, on_start_daemon, |ui, ctl, weak, sh| {
        let ok =
            daemon_exe().is_some_and(|exe| bento_win::system::spawn(&exe.to_string_lossy(), ""));
        toast(
            &ctl,
            &ui,
            ctl.t(if ok { T::Started } else { T::StartFailed }),
        );
        refresh_status_later(&ctl, weak, sh);
    });
}

fn refresh_status_later(ctl: &Ctl, weak: Weak<MainWindow>, sh: Shared) {
    ctl.refresh_timer.start(
        TimerMode::SingleShot,
        Duration::from_millis(1500),
        move || {
            if let (Some(ui), Ok(mut ctl)) = (weak.upgrade(), sh.try_borrow_mut()) {
                ctl.state = bento_config::load_state(&ctl.paths);
                push_desk(&mut ctl, &ui);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_reset_keeps_data_and_other_pages() {
        let mut c = AppConfig::default();
        c.sorting.auto_apply = !c.sorting.auto_apply;
        c.sorting.rules.push(Rule {
            id: 1,
            name: "Images".into(),
            enabled: true,
            match_all: true,
            conditions: Vec::new(),
            target: FenceId(3),
        });
        c.general.log_level = LogLevel::Debug;
        c.general.show_in_taskbar = !c.general.show_in_taskbar;
        c.layout.snap_gap = 42;
        assert!(reset_page(&mut c, 6));
        assert_eq!(
            c.sorting.auto_apply,
            AppConfig::default().sorting.auto_apply
        );
        assert_eq!(c.sorting.rules.len(), 1);
        assert!(reset_page(&mut c, 10));
        assert_eq!(
            c.general.show_in_taskbar,
            AppConfig::default().general.show_in_taskbar
        );
        assert_eq!(c.general.log_level, LogLevel::Debug);
        assert_eq!(c.layout.snap_gap, 42);
        assert!(reset_page(&mut c, 12));
        assert_eq!(c.general.log_level, AppConfig::default().general.log_level);
        assert!(!reset_page(&mut c, 0));
        assert!(!reset_page(&mut c, 13));
    }

    #[test]
    fn title_color_choices_round_trip() {
        assert_eq!(title_color_index(None), 0);
        assert_eq!(title_color_index(Some(TITLE_WHITE)), 1);
        assert_eq!(title_color_index(Some(TITLE_BLACK)), 2);
        assert_eq!(title_color_index(Some(Rgba::rgb(200, 30, 30))), 3);
        assert_eq!(hsl_rgba(0.0, 0.0, 0.0), TITLE_BLACK);
        let (h, s, l) = rgba_hsl(Rgba::rgb(255, 0, 0));
        assert_eq!((h, s, l), (0.0, 100.0, 50.0));
        assert_eq!(hsl_rgba(h, s, l), Rgba::rgb(255, 0, 0));
    }

    #[test]
    fn condition_edits() {
        let c = Condition::Name {
            op: TextOp::StartsWith,
            value: "rapport".into(),
        };
        // Changing the field keeps the text where it makes sense.
        let e = edit_condition(&c, "field", 1, "");
        assert_eq!(
            e,
            Condition::Extension {
                values: vec!["rapport".into()]
            }
        );
        let e = edit_condition(&e, "value", 0, ".PDF, docx;txt");
        assert_eq!(
            e,
            Condition::Extension {
                values: vec!["pdf".into(), "docx".into(), "txt".into()]
            }
        );
        let d = edit_condition(&e, "field", 4, "");
        let d = edit_condition(&d, "days", 30, "");
        let d = edit_condition(&d, "op", 1, "");
        assert_eq!(
            d,
            Condition::Date {
                date: DateField::Modified,
                op: DateOp::OlderThanDays,
                days: 30
            }
        );
        let k = edit_condition(&d, "field", 2, "");
        let k = edit_condition(&k, "kind", 0, "");
        assert_eq!(
            k,
            Condition::Kind {
                kind: ItemKind::Shortcut
            }
        );
    }

    #[test]
    fn rule_summary() {
        let r = Rule {
            name: "Docs".into(),
            enabled: true,
            match_all: false,
            conditions: vec![
                Condition::Extension {
                    values: vec!["pdf".into()],
                },
                Condition::Kind {
                    kind: ItemKind::Folder,
                },
            ],
            ..Rule::default()
        };
        assert_eq!(summary(Lang::Fr, &r), "extension pdf ou type dossier");
        assert_eq!(summary(Lang::En, &r), "extension pdf or type folder");
    }

    #[test]
    fn config_indices() {
        for l in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            assert_eq!(log_level(log_index(l)), l);
        }
        for k in KINDS {
            assert_eq!(KINDS[kind_index(k) as usize], k);
        }
        assert_eq!(
            hk_parse("Ctrl+Alt+D").map(|h| h.to_string()).as_deref(),
            Some("Ctrl+Alt+D")
        );
        assert_eq!(hk_parse("D"), None);
    }
}
