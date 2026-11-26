//! Peek: shows the fences of the current page above every window.
//!
//! Real desktop icons cannot be raised above other windows, so Peek draws
//! the fences and their icons itself in a top-most overlay.

use std::collections::HashSet;

use bento_core::display::geometry_to_px;
use bento_core::geometry::{Point, Rect};
use bento_core::grid::FrameMetrics;
use bento_core::i18n::Msg;
use bento_core::model::{Fence, TitleAlign, ViewMode};
use bento_render::fence::title_color;
use bento_render::overlay::draw_peek_background;
use bento_render::portal::{Entry, EntryLayout, draw_entries, layout};
use bento_win::Hwnd;
use bento_win::gfx::{Align, RectF, Surface, TextStyle};
use bento_win::window::{self, Kind, Message, OwnedWindow, msg};

use crate::app::{App, CLASS_POPUP};
use crate::worker::Cmd;

struct PeekFence {
    rect: Rect,
    title: String,
    entries: Vec<Entry>,
    layout: EntryLayout,
    style: bento_core::model::FenceStyle,
}

pub struct Peek {
    window: OwnedWindow,
    surface: Option<Surface>,
    origin: Point,
    fences: Vec<PeekFence>,
    selected: HashSet<(usize, usize)>,
    hover: Option<(usize, usize)>,
}

impl Peek {
    pub fn hwnd(&self) -> Hwnd {
        self.window.hwnd()
    }

    fn hit(&self, screen: Point) -> Option<(usize, usize)> {
        let local = Point::new(screen.x - self.origin.x, screen.y - self.origin.y);
        self.fences
            .iter()
            .enumerate()
            .find_map(|(fi, f)| f.layout.hit(local).map(|e| (fi, e)))
    }
}

impl App {
    pub fn toggle_peek(&mut self) {
        if self.peek.is_some() {
            self.peek = None;
            return;
        }
        self.close_popup();
        let vs = self.virtual_screen();
        let Ok(win) = window::create(
            CLASS_POPUP,
            "Bento Desk Peek",
            Kind::LayeredPopup {
                owner: Some(self.hwnd()),
                topmost: true,
                activate: true,
            },
            vs,
        ) else {
            return;
        };
        win.hwnd().set_rect(vs);
        let mut missing = Vec::new();
        let fences: Vec<Fence> = self
            .layout()
            .and_then(|l| l.page())
            .map(|p| p.fences.clone())
            .unwrap_or_default();
        let mut list = Vec::new();
        for f in fences {
            let Some(r) = geometry_to_px(&f.geometry, &self.monitors) else {
                continue;
            };
            let dpi = bento_core::display::monitor_for_rect(&self.monitors, &r)
                .map(|m| m.dpi)
                .unwrap_or(96);
            let m = FrameMetrics::for_dpi(dpi);
            let tab = f.active().cloned().unwrap_or_default();
            let entries: Vec<Entry> = tab
                .content
                .desktop_items()
                .iter()
                .filter_map(|k| {
                    let it = self.items.get(k)?;
                    if !self.icon_cache.contains_key(k) {
                        missing.push(k.clone());
                    }
                    Some(Entry {
                        name: it.info.name.clone(),
                        path: k.clone(),
                        is_dir: false,
                        icon: self.icon_cache.get(k).cloned(),
                    })
                })
                .collect();
            let body = m.body(&r).offset(-vs.left, -vs.top);
            let l = layout(ViewMode::Icons, body, dpi, entries.len(), 0);
            list.push(PeekFence {
                rect: r.offset(-vs.left, -vs.top),
                title: f.title().to_owned(),
                entries,
                layout: l,
                style: tab
                    .style
                    .clone()
                    .unwrap_or_else(|| self.config.appearance.default_style.clone()),
            });
        }
        if !missing.is_empty() {
            let px = 32 * self.monitors.iter().map(|m| m.dpi).max().unwrap_or(96) / 96;
            self.worker.send(Cmd::Icons(missing, px as i32));
        }
        self.peek = Some(Peek {
            window: win,
            surface: None,
            origin: vs.origin(),
            fences: list,
            selected: HashSet::new(),
            hover: None,
        });
        self.render_peek();
        if let Some(p) = &self.peek {
            p.hwnd().show_active();
            p.hwnd().set_foreground();
            p.hwnd().set_focus();
        }
    }

    pub fn render_peek(&mut self) {
        let vs = self.virtual_screen();
        let dpi = self
            .monitors
            .iter()
            .find(|m| m.primary)
            .map(|m| m.dpi)
            .unwrap_or(96);
        let Some(peek) = self.peek.as_mut() else {
            return;
        };
        for f in &mut peek.fences {
            for e in &mut f.entries {
                if e.icon.is_none() {
                    e.icon = self.icon_cache.get(&e.path).cloned();
                }
            }
        }
        let (w, h) = (vs.width(), vs.height());
        let dim = self.config.peek.dim;
        let hint = Msg::PeekHint.text(self.lang);
        let gfx = &self.gfx;
        if peek.surface.as_ref().map(Surface::size) != Some((w, h)) {
            peek.surface = Surface::new(gfx, w, h).ok();
        }
        let Some(s) = peek.surface.as_mut() else {
            return;
        };
        let fences = &peek.fences;
        let selected = &peek.selected;
        let hover = peek.hover;
        let res = s.draw(gfx, |c| {
            draw_peek_background(c, w, h, dim, hint, dpi);
            for (fi, f) in fences.iter().enumerate() {
                let sc = dpi as f32 / 96.0;
                let mut opaque = f.style.clone();
                opaque.tint.transparency = opaque.tint.transparency.min(15);
                c.push_clip(RectF::from(f.rect));
                // Backdrop drawn in a translated way: offset all coordinates.
                let r = RectF::from(f.rect);
                let tint = opaque.tint.to_rgba();
                c.fill_round_rect(r, f32::from(opaque.corner_radius) * sc, tint);
                let fg = title_color(&opaque, tint);
                let mut st =
                    TextStyle::new(&opaque.title_font, f32::from(opaque.title_size) * sc, fg);
                st.align = match opaque.title_align {
                    TitleAlign::Left => Align::Left,
                    TitleAlign::Center => Align::Center,
                    TitleAlign::Right => Align::Right,
                };
                c.text(
                    &f.title,
                    RectF::new(
                        r.left + 10.0 * sc,
                        r.top,
                        r.right - 10.0 * sc,
                        r.top + 30.0 * sc,
                    ),
                    &st,
                );
                let sel: Vec<usize> = selected
                    .iter()
                    .filter(|(a, _)| *a == fi)
                    .map(|(_, b)| *b)
                    .collect();
                let hv = hover.filter(|(a, _)| *a == fi).map(|(_, b)| b);
                let body = f.rect;
                draw_entries(
                    c,
                    &f.entries,
                    &f.layout,
                    ViewMode::Icons,
                    dpi,
                    fg,
                    hv,
                    &sel,
                    None,
                    body,
                );
                c.pop_clip();
            }
        });
        if res.is_ok() {
            let _ = s.present(peek.window.hwnd());
        }
    }

    fn peek_open(&mut self, keys: Vec<String>) {
        self.peek = None;
        for k in keys {
            if !bento_win::system::shell_open(&k) {
                tracing::warn!("cannot open {k}");
            } else {
                *self.state.open_counts.entry(k).or_insert(0) += 1;
                self.mark_dirty();
            }
        }
    }

    pub fn on_peek_message(&mut self, m: &Message) -> Option<isize> {
        let screen = {
            let o = self.peek.as_ref().map(|p| p.origin).unwrap_or_default();
            let l = m.point();
            Point::new(l.x + o.x, l.y + o.y)
        };
        match m.msg {
            msg::WM_MOUSEMOVE => {
                let hit = self.peek.as_ref().and_then(|p| p.hit(screen));
                if self.peek.as_ref().is_some_and(|p| p.hover != hit) {
                    if let Some(p) = self.peek.as_mut() {
                        p.hover = hit;
                    }
                    self.render_peek();
                }
            }
            msg::WM_LBUTTONUP => {
                let Some(p) = self.peek.as_mut() else {
                    return Some(0);
                };
                match p.hit(screen) {
                    Some(h) if m.wparam & msg::MK_CONTROL != 0 => {
                        if !p.selected.remove(&h) {
                            p.selected.insert(h);
                        }
                        self.render_peek();
                    }
                    Some((fi, ei)) => {
                        let key = p.fences[fi].entries[ei].path.clone();
                        self.peek_open(vec![key]);
                    }
                    None => self.peek = None,
                }
            }
            msg::WM_RBUTTONUP => {
                let target = self.peek.as_ref().and_then(|p| {
                    p.hit(screen)
                        .map(|(f, e)| p.fences[f].entries[e].path.clone())
                });
                if let Some(t) = target {
                    let owner = self.peek.as_ref().map(Peek::hwnd).unwrap_or_default();
                    bento_win::shell::show_context_menu(owner, &t, screen);
                }
            }
            msg::WM_KEYDOWN => match m.wparam {
                0x1B => self.peek = None,
                0x0D => {
                    let keys: Vec<String> = self
                        .peek
                        .as_ref()
                        .map(|p| {
                            p.selected
                                .iter()
                                .filter_map(|(f, e)| {
                                    p.fences.get(*f)?.entries.get(*e).map(|x| x.path.clone())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.peek_open(keys);
                }
                _ => {}
            },
            msg::WM_ACTIVATE if m.wparam & 0xFFFF == msg::WA_INACTIVE => {
                self.peek = None;
            }
            _ => return None,
        }
        Some(0)
    }
}
