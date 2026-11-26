//! Popup windows: Fluent menus, the rubber band used to draw a fence, and
//! short-lived overlays (toast, page indicator).

use bento_core::geometry::{Point, Rect, dip_to_px};
use bento_core::i18n::Msg;
use bento_render::menu::{Menu, MenuAction, MenuItem, Row, draw_menu};
use bento_render::overlay::{draw_page_indicator, draw_rubber_band, draw_toast};
use bento_win::Hwnd;
use bento_win::gfx::Surface;
use bento_win::window::{self, Kind, Message, OwnedWindow, msg};

use crate::app::{App, CLASS_POPUP, Timer};
use crate::menus::Command;

/// An open menu.
pub struct Popup {
    window: OwnedWindow,
    surface: Option<Surface>,
    pub menu: Menu,
    rows: Vec<Row>,
    commands: Vec<Command>,
    dpi: u32,
    size: (i32, i32),
    /// Point the menu was opened at and work area of its monitor, kept to
    /// place the window again when a submenu changes its height.
    at: Point,
    work: Rect,
}

/// Places a `w` x `h` menu at `at`, flipped left or up when it would leave
/// the work area.
fn place_menu(at: Point, w: i32, h: i32, work: Rect) -> Rect {
    let h = h.min(work.height());
    let mut x = at.x;
    let mut y = at.y;
    if x + w > work.right {
        x = (at.x - w).max(work.left);
    }
    if y + h > work.bottom {
        y = (at.y - h).max(work.top);
    }
    Rect::from_xywh(x, y, w, h)
}

impl Popup {
    pub fn hwnd(&self) -> Hwnd {
        self.window.hwnd()
    }
}

/// Rectangle drawn while right-dragging on the desktop.
pub struct RubberBand {
    window: OwnedWindow,
    surface: Option<Surface>,
    pub start: Point,
    dpi: u32,
}

impl RubberBand {
    pub fn new(app: &App, start: Point) -> anyhow::Result<RubberBand> {
        let w = window::create(
            CLASS_POPUP,
            "",
            Kind::LayeredPopup {
                owner: Some(app.hwnd()),
                topmost: true,
                activate: false,
            },
            Rect::from_xywh(start.x, start.y, 1, 1),
        )?;
        let dpi = bento_core::display::monitor_at(&app.monitors, start)
            .map(|m| m.dpi)
            .unwrap_or(96);
        Ok(RubberBand {
            window: w,
            surface: None,
            start,
            dpi,
        })
    }

    pub fn update(&mut self, app: &App, p: Point) {
        let r = Rect::from_points(self.start, p);
        if r.width() < 4 || r.height() < 4 {
            return;
        }
        let (w, h) = (r.width(), r.height());
        if self.surface.as_ref().map(Surface::size) != Some((w, h)) {
            self.surface = Surface::new(&app.gfx, w, h).ok();
        }
        let theme = app.theme;
        let dpi = self.dpi;
        if let Some(s) = self.surface.as_mut()
            && s.draw(&app.gfx, |c| draw_rubber_band(c, w, h, &theme, dpi))
                .is_ok()
        {
            self.window.hwnd().set_rect(r);
            let _ = s.present(self.window.hwnd());
            self.window.hwnd().show(true);
        }
    }
}

/// Toast or page indicator.
pub struct Overlay {
    _window: OwnedWindow,
}

impl App {
    fn primary_dpi_and_work(&self) -> (u32, Rect) {
        self.monitors
            .iter()
            .find(|m| m.primary)
            .map(|m| (m.dpi, m.work))
            .unwrap_or((96, Rect::from_xywh(0, 0, 1280, 720)))
    }

    fn show_overlay(&mut self, w: i32, h: i32, draw: impl FnOnce(&mut bento_win::gfx::Canvas<'_>)) {
        let (_, work) = self.primary_dpi_and_work();
        let r = Rect::from_xywh(
            work.left + (work.width() - w) / 2,
            work.bottom - h - dip_to_px(48, 96),
            w,
            h,
        );
        self.overlay = None;
        let Ok(win) = window::create(
            CLASS_POPUP,
            "",
            Kind::LayeredPopup {
                owner: Some(self.hwnd()),
                topmost: true,
                activate: false,
            },
            r,
        ) else {
            return;
        };
        if let Ok(mut s) = Surface::new(&self.gfx, w, h)
            && s.draw(&self.gfx, draw).is_ok()
            && s.present(win.hwnd()).is_ok()
        {
            win.hwnd().set_rect(r);
            win.hwnd().show(true);
            self.overlay = Some(Overlay { _window: win });
            self.set_timer(Timer::Overlay, 1400);
        }
    }

    pub fn show_toast(&mut self, text: &str) {
        let (dpi, _) = self.primary_dpi_and_work();
        let st = bento_win::gfx::TextStyle::new(
            bento_render::theme::UI_FONT,
            14.0 * dpi as f32 / 96.0,
            self.theme.text,
        );
        let (tw, _) = self.gfx.measure(text, &st, 2000.0);
        let w = tw as i32 + dip_to_px(48, dpi);
        let h = dip_to_px(44, dpi);
        let theme = self.theme;
        let text = text.to_owned();
        self.show_overlay(w, h, |c| draw_toast(c, w, h, &text, &theme, dpi));
    }

    pub fn show_page_indicator(&mut self) {
        if !self.config.pages.show_indicator {
            return;
        }
        let Some(l) = self.layout() else { return };
        let total = l.pages.len();
        let cur = l.current_page;
        let name = l
            .page()
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| Msg::PageName.fmt(self.lang, &(cur + 1).to_string()));
        let (dpi, _) = self.primary_dpi_and_work();
        let w = dip_to_px(180, dpi);
        let h = dip_to_px(56, dpi);
        let theme = self.theme;
        self.show_overlay(w, h, |c| {
            draw_page_indicator(c, w, h, cur, total, &name, &theme, dpi)
        });
    }

    /// Opens a menu at a screen point.
    pub fn show_menu(&mut self, items: Vec<MenuItem>, commands: Vec<Command>, at: Point) {
        self.close_popup();
        let dpi = bento_core::display::monitor_at(&self.monitors, at)
            .map(|m| m.dpi)
            .unwrap_or(96);
        let work = bento_core::display::monitor_at(&self.monitors, at)
            .map(|m| m.work)
            .unwrap_or(Rect::from_xywh(0, 0, 1920, 1080));
        let menu = Menu::new(items);
        let (w, h) = menu.size(&self.gfx, dpi);
        let rect = place_menu(at, w, h, work);
        let h = rect.height();
        let Ok(win) = window::create(
            CLASS_POPUP,
            "",
            Kind::LayeredPopup {
                owner: Some(self.hwnd()),
                topmost: true,
                activate: true,
            },
            rect,
        ) else {
            return;
        };
        win.hwnd().set_rect(rect);
        let rows = menu.rows(dpi, w);
        self.popup = Some(Popup {
            window: win,
            surface: None,
            menu,
            rows,
            commands,
            dpi,
            size: (w, h),
            at,
            work,
        });
        self.render_popup();
        if let Some(p) = &self.popup {
            p.hwnd().show_active();
            p.hwnd().set_foreground();
            p.hwnd().set_focus();
        }
    }

    pub fn close_popup(&mut self) {
        self.popup = None;
    }

    fn render_popup(&mut self) {
        let theme = self.theme;
        let gfx = &self.gfx;
        let Some(p) = self.popup.as_mut() else { return };
        let (w, h) = p.size;
        if p.surface.is_none() {
            p.surface = Surface::new(gfx, w, h).ok();
        }
        let (menu, rows, dpi) = (&p.menu, &p.rows, p.dpi);
        if let Some(s) = p.surface.as_mut()
            && s.draw(gfx, |c| draw_menu(c, menu, rows, &theme, dpi, w, h))
                .is_ok()
        {
            let _ = s.present(p.window.hwnd());
        }
    }

    fn popup_activate(&mut self, row: usize) {
        let Some(p) = self.popup.as_mut() else { return };
        match p.menu.activate(&p.rows, row) {
            MenuAction::Invoke(id) => {
                let cmd = p.commands.get(id.saturating_sub(1) as usize).cloned();
                self.close_popup();
                if let Some(c) = cmd {
                    self.execute(c);
                }
            }
            MenuAction::Toggled => {
                let w = p.size.0;
                p.rows = p.menu.rows(p.dpi, w);
                let rect = place_menu(p.at, w, p.menu.height(p.dpi), p.work);
                if rect.height() != p.size.1 {
                    p.size = (w, rect.height());
                    p.surface = None;
                    p.window.hwnd().set_rect(rect);
                }
                self.render_popup();
            }
            MenuAction::None => {}
        }
    }

    pub fn on_popup_message(&mut self, m: &Message) -> Option<isize> {
        match m.msg {
            msg::WM_MOUSEMOVE => {
                let Some(p) = self.popup.as_mut() else {
                    return Some(0);
                };
                let row = Menu::row_at(&p.rows, m.point());
                if row != p.menu.hover {
                    p.menu.hover = row;
                    p.window.hwnd().track_mouse_leave();
                    self.render_popup();
                }
            }
            msg::WM_MOUSELEAVE => {
                if let Some(p) = self.popup.as_mut() {
                    p.menu.hover = None;
                }
                self.render_popup();
            }
            msg::WM_LBUTTONUP => {
                let row = self
                    .popup
                    .as_ref()
                    .and_then(|p| Menu::row_at(&p.rows, m.point()));
                if let Some(r) = row {
                    self.popup_activate(r);
                }
            }
            msg::WM_KEYDOWN => {
                let Some(p) = self.popup.as_mut() else {
                    return Some(0);
                };
                match m.wparam {
                    0x1B => self.close_popup(), // Escape
                    0x26 | 0x28 => {
                        // Up / Down
                        p.menu.hover = p.menu.step(&p.rows, p.menu.hover, m.wparam == 0x28);
                        self.render_popup();
                    }
                    0x0D | 0x20 | 0x27 => {
                        // Enter, Space, Right
                        if let Some(r) = p.menu.hover {
                            self.popup_activate(r);
                        }
                    }
                    _ => {}
                }
            }
            msg::WM_ACTIVATE if m.wparam & 0xFFFF == msg::WA_INACTIVE => {
                self.close_popup();
            }
            _ => return None,
        }
        Some(0)
    }
}
