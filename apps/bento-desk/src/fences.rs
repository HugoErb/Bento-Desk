//! Fence windows: creation inside the desktop layer, drawing, and mouse
//! interaction (move, resize, roll-up, tabs, scrolling, renaming).

use bento_core::color::{Rgba, Tint};
use bento_core::display::{geometry_to_px, px_to_geometry};
use bento_core::geometry::{Point, Rect, dip_to_px};
use bento_core::grid::{FrameMetrics, Grid, fence_cell};
use bento_core::i18n::Msg;
use bento_core::model::{Fence, FenceId, FenceStyle, Tab, TabContent, TabId, TitleMode};
use bento_core::settings::OpenTrigger;
use bento_core::snap::{self, Edges, SnapConfig};
use bento_render::fence::{
    BackdropView, FrameHit, FrameLayout, FrameView, PortalHeader, ScrollView, TabView,
    draw_backdrop, draw_frame,
};
use bento_win::Hwnd;
use bento_win::desktop::DesktopLayer;
use bento_win::dragdrop::{DragEvent, DropRegistration, take_dropped};
use bento_win::gfx::{RectF, Surface};
use bento_win::hooks::unpack_point;
use bento_win::window::{self, Cursor, Font, Kind, Message, OwnedWindow, msg};

use crate::app::{App, CLASS_FENCE, Timer, WM_DRAG, WM_EDIT};
use crate::sync::Placed;

/// Mouse operation in progress on a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    Pending(FrameHit),
    Move,
    Resize(Edges),
}

#[derive(Clone, Copy, Debug)]
pub struct FrameDrag {
    pub kind: DragKind,
    pub start_mouse: Point,
    pub start_rect: Rect,
}

/// Duration of the roll-up and unroll animation.
const ROLL_ANIM_MS: f32 = 180.0;
/// Delay between two animation frames.
const ROLL_FRAME_MS: u32 = 15;
/// While a fence is temporarily open, how often the pointer is checked.
const COLLAPSE_POLL_MS: u32 = 40;

/// Height animation between the rolled-up and the open rectangle. The top
/// edge stays in place, the bottom edge moves.
#[derive(Clone, Copy, Debug)]
pub struct RollAnim {
    from_bottom: i32,
    start: std::time::Instant,
}

impl RollAnim {
    fn progress(&self, now: std::time::Instant) -> f32 {
        let ms = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        (ms / ROLL_ANIM_MS).clamp(0.0, 1.0)
    }

    fn done(&self, now: std::time::Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// Bottom edge at `now`, ease-out cubic towards `to`.
    fn bottom(&self, to: i32, now: std::time::Instant) -> i32 {
        let t = self.progress(now);
        let e = 1.0 - (1.0 - t).powi(3);
        self.from_bottom + ((to - self.from_bottom) as f32 * e).round() as i32
    }
}

/// Style, tabs, active tab, title visibility and portal header of a frame.
type FrameData = (FenceStyle, Vec<TabView>, usize, bool, Option<PortalHeader>);

/// In-place rename box.
pub struct InlineEdit {
    pub fence: FenceId,
    pub window: OwnedWindow,
    _font: Font,
}

/// Windows and display state of one fence of the current page.
pub struct FenceUi {
    pub id: FenceId,
    pub backdrop: OwnedWindow,
    pub frame: OwnedWindow,
    back_surface: Option<Surface>,
    frame_surface: Option<Surface>,
    _drop: Option<DropRegistration>,
    layer: DesktopLayer,
    /// Displayed rectangle (title only when rolled up), screen pixels.
    pub rect: Rect,
    /// Rectangle when unrolled.
    pub full_rect: Rect,
    pub dpi: u32,
    pub monitor: String,
    pub rolled: bool,
    pub temp_open: bool,
    /// Open state last shown, `None` before the first placement (no
    /// animation when a fence appears).
    shown_open: Option<bool>,
    anim: Option<RollAnim>,
    /// When the pointer was first seen outside the temporarily opened
    /// fence (delay before rolling up again).
    left_at: Option<std::time::Instant>,
    pub hovered_frame: bool,
    pub hovered_body: bool,
    pub mouse: Option<Point>,
    pub drag: Option<FrameDrag>,
    pub drop_tab: Option<usize>,
    pub hidden: bool,
    pub visible: bool,
    pub portal: bool,
    pub max_scroll: u32,
    pub visible_rows: u32,
    pub tint: Option<Rgba>,
    pub portal_state: Option<crate::portals::PortalState>,
}

impl FenceUi {
    pub fn owns(&self, h: Hwnd) -> bool {
        self.backdrop.hwnd() == h || self.frame.hwnd() == h
    }

    pub fn placed(&self) -> Placed {
        let m = FrameMetrics::for_dpi(self.dpi);
        if self.anim.is_some() {
            // While the height animates, icons keep their final grid (full
            // body) and only those inside the visible part are shown, so
            // they appear and disappear with the moving bottom edge.
            return Placed {
                id: self.id,
                rect: self.rect,
                body: m.body(&self.full_rect),
                rolled: false,
                hidden: self.hidden,
                portal: self.portal,
                clip_bottom: Some(m.body(&self.rect).bottom),
            };
        }
        let open = !self.rolled || self.temp_open;
        Placed {
            id: self.id,
            rect: self.rect,
            body: if open {
                m.body(&self.rect)
            } else {
                Rect::default()
            },
            rolled: !open,
            hidden: self.hidden,
            portal: self.portal,
            clip_bottom: None,
        }
    }

    /// Shows or hides both windows.
    pub fn apply_visibility(&mut self) {
        let show = self.visible && !self.hidden;
        self.backdrop.hwnd().show(show && !self.portal);
        self.frame.hwnd().show(show);
    }

    fn local(&self, parent: Hwnd) -> Rect {
        let o = DesktopLayer::origin_of(parent);
        self.rect.offset(-o.x, -o.y)
    }

    /// Moves both windows to `self.rect`.
    pub fn apply_rect(&self) {
        self.backdrop
            .hwnd()
            .set_rect(self.local(self.layer.backdrop_parent));
        self.frame
            .hwnd()
            .set_rect(self.local(self.layer.frame_parent));
    }

    fn local_mouse(&self, screen: Point) -> Point {
        Point::new(screen.x - self.rect.left, screen.y - self.rect.top)
    }
}

fn effective_style(app: &App, tab: Option<&Tab>) -> FenceStyle {
    tab.and_then(|t| t.style.clone())
        .unwrap_or_else(|| app.config.appearance.default_style.clone())
}

impl App {
    fn snap_config(&self, dpi: u32) -> SnapConfig {
        SnapConfig {
            to_edges: self.config.layout.snap_to_edges,
            to_fences: self.config.layout.snap_to_fences,
            threshold: dip_to_px(i32::from(self.config.layout.snap_distance), dpi),
            gap: dip_to_px(i32::from(self.config.layout.snap_gap), dpi),
        }
    }

    /// Recreates the windows of every fence of the current page.
    pub fn rebuild_fences(&mut self) {
        let mut prev: Vec<(FenceId, bool, Option<crate::portals::PortalState>)> = self
            .fences
            .drain(..)
            .map(|mut f| (f.id, f.temp_open, f.portal_state.take()))
            .collect();
        let Some(layer) = self.layer else { return };
        if !layer.is_alive() || self.paused {
            return;
        }
        let fences: Vec<Fence> = self
            .layout()
            .and_then(|l| l.page())
            .map(|p| p.fences.clone())
            .unwrap_or_default();
        for f in fences {
            match self.create_fence_ui(&layer, &f) {
                Ok(mut ui) => {
                    if let Some((_, temp, portal)) = prev.iter_mut().find(|(id, _, _)| *id == f.id)
                    {
                        ui.temp_open = *temp && ui.rolled;
                        if let Some(p) = portal.take() {
                            ui.portal_state = Some(p);
                        }
                    }
                    self.fences.push(ui);
                }
                Err(e) => tracing::error!("fence window: {e}"),
            }
        }
        self.apply_chameleon();
        for i in 0..self.fences.len() {
            self.update_fence_geometry(i);
            self.render_backdrop(i);
            self.render_frame(i);
            self.ensure_portal(i);
            let f = &mut self.fences[i];
            f.apply_visibility();
            tracing::debug!(
                "fence {:?}: rect {:?}, backdrop {:?} in {:?} visible={}, frame {:?} in {:?} visible={}",
                f.id,
                f.rect,
                f.backdrop.hwnd(),
                f.backdrop.hwnd().parent(),
                f.backdrop.hwnd().is_visible(),
                f.frame.hwnd(),
                f.frame.hwnd().parent(),
                f.frame.hwnd().is_visible()
            );
        }
        self.update_hit_data();
    }

    fn create_fence_ui(&self, layer: &DesktopLayer, f: &Fence) -> anyhow::Result<FenceUi> {
        let rect = geometry_to_px(&f.geometry, &self.monitors)
            .unwrap_or(Rect::from_xywh(100, 100, 300, 200));
        let mon = bento_core::display::monitor_for_rect(&self.monitors, &rect)
            .cloned()
            .unwrap_or_default();
        let backdrop = window::create(
            CLASS_FENCE,
            "",
            Kind::LayeredChild {
                parent: layer.backdrop_parent,
            },
            Rect::from_xywh(0, 0, 10, 10),
        )?;
        let frame = window::create(
            CLASS_FENCE,
            f.title(),
            Kind::LayeredChild {
                parent: layer.frame_parent,
            },
            Rect::from_xywh(0, 0, 10, 10),
        )?;
        layer.place_backdrop(backdrop.hwnd());
        layer.place_frame(frame.hwnd());
        let drop = match DropRegistration::register(frame.hwnd(), self.hwnd(), WM_DRAG) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::debug!("drop target: {e}");
                None
            }
        };
        let portal = f.active().is_some_and(|t| t.content.is_portal());
        Ok(FenceUi {
            id: f.id,
            backdrop,
            frame,
            back_surface: None,
            frame_surface: None,
            _drop: drop,
            layer: *layer,
            rect,
            full_rect: rect,
            dpi: mon.dpi.max(96),
            monitor: mon.id,
            rolled: f.rolled_up,
            temp_open: false,
            shown_open: None,
            anim: None,
            left_at: None,
            hovered_frame: false,
            hovered_body: false,
            mouse: None,
            drag: None,
            drop_tab: None,
            hidden: self.quick_hidden && !f.quick_hide_exempt,
            visible: true,
            portal,
            max_scroll: 0,
            visible_rows: 0,
            tint: None,
            portal_state: None,
        })
    }

    /// Recomputes displayed rectangle (roll-up, docking) and scroll data.
    pub fn update_fence_geometry(&mut self, i: usize) {
        let Some(id) = self.fences.get(i).map(|f| f.id) else {
            return;
        };
        let Some(fence) = self.layout().and_then(|l| l.fence(id)).cloned() else {
            return;
        };
        let Some(full) = geometry_to_px(&fence.geometry, &self.monitors) else {
            return;
        };
        let mon = bento_core::display::monitor_for_rect(&self.monitors, &full).cloned();
        let cell = fence_cell(self.spacing, self.config.layout.icon_spacing);
        let present = fence
            .active()
            .map(|t| {
                t.content
                    .desktop_items()
                    .iter()
                    .filter(|k| self.items.contains_key(k.as_str()))
                    .count()
            })
            .unwrap_or(0);
        let animate = self.config.rollup.animate;
        let ui = &mut self.fences[i];
        if let Some(m) = &mon {
            ui.dpi = m.dpi.max(96);
            ui.monitor = m.id.clone();
        }
        let docked = mon
            .as_ref()
            .and_then(|m| snap::docked_edge(&full, &m.work, 2))
            .is_some();
        ui.rolled = fence.rolled_up || (fence.auto_rollup && docked);
        ui.portal = fence.active().is_some_and(|t| t.content.is_portal());
        ui.full_rect = full;
        let m = FrameMetrics::for_dpi(ui.dpi);
        let open = !ui.rolled || ui.temp_open;
        let target = if open { full } else { m.rolled(&full) };
        let now = std::time::Instant::now();
        let mut start_anim = false;
        if !animate {
            ui.anim = None;
        } else if ui.shown_open.is_some_and(|o| o != open) {
            // Starts from the height shown now, so a reversal mid-way is
            // smooth too.
            ui.anim = Some(RollAnim {
                from_bottom: ui.rect.bottom,
                start: now,
            });
            start_anim = true;
        }
        ui.shown_open = Some(open);
        ui.rect = match ui.anim {
            Some(a) => {
                let bottom = a
                    .bottom(target.bottom, now)
                    .max(m.rolled(&full).bottom)
                    .min(full.bottom);
                Rect::new(target.left, target.top, target.right, bottom)
            }
            None => target,
        };
        let grid = Grid::new(m.body(&full), cell);
        ui.max_scroll = grid.max_scroll(present);
        ui.visible_rows = grid.visible_rows;
        ui.apply_rect();
        if start_anim {
            self.set_timer(Timer::RollAnim, ROLL_FRAME_MS);
        }
    }

    /// Advances the roll-up animations by one frame.
    pub fn step_roll_anims(&mut self) {
        let now = std::time::Instant::now();
        let (mut running, mut finished) = (false, false);
        for i in 0..self.fences.len() {
            let Some(a) = self.fences[i].anim else {
                continue;
            };
            if a.done(now) {
                self.fences[i].anim = None;
                finished = true;
            } else {
                running = true;
            }
            self.update_fence_geometry(i);
            self.render_backdrop(i);
            self.render_frame(i);
        }
        if running {
            self.set_timer(Timer::RollAnim, ROLL_FRAME_MS);
            // Icons follow the moving bottom edge.
            self.move_icons();
        }
        if finished {
            self.update_hit_data();
            self.relayout();
        }
    }

    fn blur_for(&self, i: usize) -> Option<(&bento_core::image::Image, RectF)> {
        let ui = self.fences.get(i)?;
        let img = self.blur.get(&ui.monitor)?;
        let m = self.monitors.iter().find(|m| m.id == ui.monitor)?;
        let at = RectF::xywh(
            (m.rect.left - ui.rect.left) as f32,
            (m.rect.top - ui.rect.top) as f32,
            m.rect.width() as f32,
            m.rect.height() as f32,
        );
        Some((img, at))
    }

    /// Chameleon: tint from the wallpaper under each fence.
    pub fn apply_chameleon(&mut self) {
        for i in 0..self.fences.len() {
            let id = self.fences[i].id;
            let style = effective_style(
                self,
                self.layout()
                    .and_then(|l| l.fence(id))
                    .and_then(Fence::active),
            );
            let tint = if style.chameleon {
                self.blur_for(i).and_then(|(img, at)| {
                    let ui = &self.fences[i];
                    let sx = img.width as f32 / at.width().max(1.0);
                    let sy = img.height as f32 / at.height().max(1.0);
                    let x = ((-at.left) * sx).max(0.0) as u32;
                    let y = ((-at.top) * sy).max(0.0) as u32;
                    let w = (ui.rect.width() as f32 * sx).max(1.0) as u32;
                    let h = (ui.rect.height() as f32 * sy).max(1.0) as u32;
                    let crop = img.crop(x, y, w, h);
                    bento_core::color::average_bgra(&crop.pixels).map(|avg| {
                        let mut t = Tint::from_rgba(avg, style.tint.transparency);
                        t.intensity = (f32::from(t.intensity) * 0.55) as u8;
                        t.saturation = t.saturation.min(60);
                        t.to_rgba()
                    })
                })
            } else {
                None
            };
            self.fences[i].tint = tint;
        }
    }

    pub fn render_all(&mut self) {
        for i in 0..self.fences.len() {
            self.update_fence_geometry(i);
            self.render_backdrop(i);
            self.render_frame(i);
        }
    }

    pub fn render_backdrop(&mut self, i: usize) {
        let Some(ui) = self.fences.get(i) else { return };
        if ui.portal {
            return;
        }
        let id = ui.id;
        let (w, h, dpi, tint) = (ui.rect.width(), ui.rect.height(), ui.dpi, ui.tint);
        let style = effective_style(
            self,
            self.layout()
                .and_then(|l| l.fence(id))
                .and_then(Fence::active),
        );
        let blur = self.blur_for(i).map(|(img, at)| (img.clone(), at));
        let highlight = self.fences[i].drop_tab.map(|_| self.theme.accent);
        let gfx = &self.gfx;
        let ui = &mut self.fences[i];
        if ui.back_surface.as_ref().map(Surface::size) != Some((w, h)) {
            ui.back_surface = Surface::new(gfx, w, h).ok();
        }
        let Some(s) = ui.back_surface.as_mut() else {
            return;
        };
        let view = BackdropView {
            width: w,
            height: h,
            dpi,
            style: &style,
            tint,
            blur: blur.as_ref().map(|(img, at)| (img, *at)),
            highlight,
        };
        if s.draw(gfx, |c| draw_backdrop(c, &view)).is_ok()
            && let Err(e) = s.present(ui.backdrop.hwnd())
        {
            tracing::debug!("present backdrop: {e}");
        }
    }

    fn frame_view_data(&self, i: usize) -> Option<FrameData> {
        let ui = self.fences.get(i)?;
        let fence = self.layout()?.fence(ui.id)?;
        let style = effective_style(self, fence.active());
        let tabs: Vec<TabView> = fence
            .tabs
            .iter()
            .map(|t| TabView {
                title: t.title.clone(),
                color: ui
                    .tint
                    .unwrap_or_else(|| effective_style(self, Some(t)).tint.to_rgba()),
            })
            .collect();
        let hovered = ui.hovered_frame || ui.hovered_body;
        let title_visible = match style.title_mode {
            TitleMode::Always => true,
            TitleMode::Hover => hovered || ui.rolled,
            TitleMode::Never => ui.rolled,
        };
        let portal = match fence.active().map(|t| &t.content) {
            Some(TabContent::Portal { root, current, .. }) => Some(PortalHeader {
                can_go_up: !current.is_empty() && current != root,
                show_folder_icon: !self.config.portals.hide_title_icon,
            }),
            _ => None,
        };
        Some((style, tabs, fence.active_tab, title_visible, portal))
    }

    pub fn render_frame(&mut self, i: usize) {
        let Some((style, tabs, active, title_visible, portal)) = self.frame_view_data(i) else {
            return;
        };
        let scroll_row = self
            .layout()
            .and_then(|l| l.fence(self.fences[i].id))
            .and_then(Fence::active)
            .map(|t| t.scroll_row)
            .unwrap_or(0);
        let blur = if portal.is_some() {
            self.blur_for(i).map(|(img, at)| (img.clone(), at))
        } else {
            None
        };
        let accent = self.theme.accent;
        let ui = &self.fences[i];
        let rolled = ui.rolled && !ui.temp_open;
        let view = FrameView {
            width: ui.rect.width(),
            height: ui.rect.height(),
            dpi: ui.dpi,
            style: &style,
            tabs,
            active_tab: active,
            title_visible,
            hovered: ui.hovered_frame || ui.hovered_body,
            rolled_up: rolled,
            scroll: (!ui.portal).then_some(ScrollView {
                first_row: scroll_row.min(ui.max_scroll),
                max_first_row: ui.max_scroll,
                visible_rows: ui.visible_rows,
            }),
            portal,
            drop_tab: ui.drop_tab,
            accent,
        };
        let layout = FrameLayout::new(&view);
        let mouse = ui.mouse;
        let tint = ui.tint;
        let portal_draw = self.portal_draw_data(i, &layout);
        let gfx = &self.gfx;
        let ui = &mut self.fences[i];
        let (w, h) = (view.width, view.height);
        if ui.frame_surface.as_ref().map(Surface::size) != Some((w, h)) {
            ui.frame_surface = Surface::new(gfx, w, h).ok();
        }
        let Some(s) = ui.frame_surface.as_mut() else {
            return;
        };
        let res = s.draw(gfx, |c| {
            if view.portal.is_some() {
                let bv = BackdropView {
                    width: w,
                    height: h,
                    dpi: view.dpi,
                    style: view.style,
                    tint,
                    blur: blur.as_ref().map(|(img, at)| (img, *at)),
                    highlight: None,
                };
                draw_backdrop(c, &bv);
                if !rolled && let Some(pd) = &portal_draw {
                    pd.draw(c);
                }
            }
            draw_frame(c, &view, &layout, mouse);
        });
        if res.is_ok()
            && let Err(e) = s.present(ui.frame.hwnd())
        {
            tracing::debug!("present frame: {e}");
        }
    }

    fn frame_layout(&self, i: usize) -> Option<FrameLayout> {
        let (style, tabs, active, title_visible, portal) = self.frame_view_data(i)?;
        let ui = &self.fences[i];
        let view = FrameView {
            width: ui.rect.width(),
            height: ui.rect.height(),
            dpi: ui.dpi,
            style: &style,
            tabs,
            active_tab: active,
            title_visible,
            hovered: true,
            rolled_up: ui.rolled && !ui.temp_open,
            scroll: (!ui.portal).then_some(ScrollView {
                first_row: 0,
                max_first_row: ui.max_scroll,
                visible_rows: ui.visible_rows,
            }),
            portal,
            drop_tab: None,
            accent: self.theme.accent,
        };
        Some(FrameLayout::new(&view))
    }

    // ----- interaction ----------------------------------------------------

    pub fn on_fence_message(&mut self, i: usize, m: &Message) -> Option<isize> {
        if self.fences[i].backdrop.hwnd() == m.hwnd {
            return None;
        }
        let local = m.point();
        let screen = Point::new(
            local.x + self.fences[i].rect.left,
            local.y + self.fences[i].rect.top,
        );
        match m.msg {
            msg::WM_MOUSEACTIVATE => return Some(msg::MA_NOACTIVATE),
            msg::WM_SETCURSOR => {
                let p = self.fences[i].local_mouse(bento_win::input::cursor_pos());
                let locked = self.fence_locked(i);
                let cur = match self
                    .frame_layout(i)
                    .map(|l| l.hit(p, self.fences[i].portal))
                {
                    Some(FrameHit::Resize(e)) if !locked => {
                        match (e.left || e.right, e.top || e.bottom) {
                            (true, true) => {
                                if (e.left && e.top) || (e.right && e.bottom) {
                                    Cursor::SizeNwse
                                } else {
                                    Cursor::SizeNesw
                                }
                            }
                            (true, false) => Cursor::SizeWe,
                            _ => Cursor::SizeNs,
                        }
                    }
                    _ => Cursor::Arrow,
                };
                window::set_cursor(cur);
                return Some(1);
            }
            msg::WM_MOUSEMOVE => self.on_frame_move(i, local, screen),
            msg::WM_MOUSELEAVE => {
                let f = &mut self.fences[i];
                f.hovered_frame = false;
                f.mouse = None;
                if self.fences[i].temp_open {
                    self.check_collapse();
                }
                if i < self.fences.len() {
                    self.render_frame(i);
                }
            }
            msg::WM_LBUTTONDOWN => {
                let Some(layout) = self.frame_layout(i) else {
                    return Some(0);
                };
                let hit = layout.hit(local, self.fences[i].portal);
                if hit == FrameHit::Body && self.fences[i].portal {
                    self.portal_press(i, local, screen);
                    return Some(0);
                }
                let locked = self.fence_locked(i);
                let f = &mut self.fences[i];
                f.drag = Some(FrameDrag {
                    kind: match hit {
                        FrameHit::Resize(e) if !locked => DragKind::Resize(e),
                        other => DragKind::Pending(other),
                    },
                    start_mouse: screen,
                    start_rect: f.rect,
                });
                f.frame.hwnd().set_capture();
            }
            msg::WM_LBUTTONUP => {
                if self.fences[i].portal {
                    self.portal_release(i);
                }
                self.on_frame_up(i, local, screen);
            }
            msg::WM_LBUTTONDBLCLK => {
                let Some(layout) = self.frame_layout(i) else {
                    return Some(0);
                };
                match layout.hit(local, self.fences[i].portal) {
                    FrameHit::Title | FrameHit::Tab(_) if self.config.rollup.double_click_title => {
                        self.toggle_rollup(self.fences[i].id);
                    }
                    FrameHit::Body if self.fences[i].portal => self.portal_click(i, local, true),
                    _ => {}
                }
            }
            msg::WM_RBUTTONUP => {
                let Some(layout) = self.frame_layout(i) else {
                    return Some(0);
                };
                match layout.hit(local, self.fences[i].portal) {
                    FrameHit::Body if self.fences[i].portal => {
                        self.portal_context(i, local, screen)
                    }
                    _ => self.show_fence_menu(self.fences[i].id, screen),
                }
            }
            msg::WM_MOUSEWHEEL => {
                let p = bento_win::window::lparam_point(m.lparam);
                self.on_body_wheel(p, m.wheel_delta());
            }
            msg::WM_CAPTURECHANGED => {
                self.fences[i].drag = None;
            }
            _ => return None,
        }
        Some(0)
    }

    /// True when the item belongs to a locked fence of the current layout.
    pub fn in_locked_fence(&self, key: &str) -> bool {
        self.layout().is_some_and(|l| {
            l.pages
                .iter()
                .flat_map(|p| p.fences.iter())
                .any(|f| f.locked && f.contains_item(key))
        })
    }

    /// Locked fences cannot be moved or resized.
    fn fence_locked(&self, i: usize) -> bool {
        let id = self.fences[i].id;
        self.layout()
            .and_then(|l| l.fence(id))
            .is_some_and(|f| f.locked)
    }

    fn on_frame_move(&mut self, i: usize, local: Point, screen: Point) {
        let f = &mut self.fences[i];
        if !f.hovered_frame {
            f.hovered_frame = true;
            f.frame.hwnd().track_mouse_leave();
            if f.rolled && !f.temp_open && self.config.rollup.open_on == OpenTrigger::Hover {
                self.set_temp_open(i, true);
                return;
            }
        }
        self.fences[i].mouse = Some(local);
        let Some(drag) = self.fences[i].drag else {
            if self.fences[i].portal {
                if self.portal_drag(i, screen) {
                    return;
                }
                self.portal_hover(i, local);
            }
            self.render_frame(i);
            return;
        };
        let dx = screen.x - drag.start_mouse.x;
        let dy = screen.y - drag.start_mouse.y;
        let dpi = self.fences[i].dpi;
        let locked = self.fence_locked(i);
        let kind = match drag.kind {
            DragKind::Pending(FrameHit::Title | FrameHit::Tab(_))
                if !locked && dx.abs() + dy.abs() > 4 =>
            {
                DragKind::Move
            }
            k => k,
        };
        let others: Vec<Rect> = self
            .fences
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, f)| f.rect)
            .collect();
        let works: Vec<Rect> = self.monitors.iter().map(|m| m.work).collect();
        let cfg = self.snap_config(dpi);
        let m = FrameMetrics::for_dpi(dpi);
        let new_rect = match kind {
            DragKind::Move => Some(snap::snap_move(
                drag.start_rect.offset(dx, dy),
                &others,
                &works,
                &cfg,
            )),
            DragKind::Resize(e) => {
                let mut e = e;
                if self.fences[i].rolled && !self.fences[i].temp_open {
                    e.top = false;
                    e.bottom = false;
                }
                let r = snap::resize_rect(
                    drag.start_rect,
                    e,
                    dx,
                    dy,
                    m.min_width,
                    m.min_height.min(drag.start_rect.height()),
                );
                Some(snap::snap_resize(r, e, &others, &works, &cfg))
            }
            DragKind::Pending(_) => None,
        };
        let f = &mut self.fences[i];
        if let Some(d) = f.drag.as_mut() {
            d.kind = kind;
        }
        if let Some(r) = new_rect {
            let resized = r.size() != f.rect.size();
            let moved = r != f.rect;
            f.rect = r;
            f.apply_rect();
            if resized {
                self.render_backdrop(i);
                self.render_frame(i);
            }
            if moved {
                // Icons follow the fence during the drag.
                self.move_icons();
            }
        }
    }

    fn on_frame_up(&mut self, i: usize, local: Point, _screen: Point) {
        window::release_capture();
        let Some(drag) = self.fences[i].drag.take() else {
            return;
        };
        let id = self.fences[i].id;
        match drag.kind {
            DragKind::Move | DragKind::Resize(_) => {
                let f = &self.fences[i];
                let mut r = f.rect;
                if f.rolled && !f.temp_open {
                    // Keep the unrolled height when moving a rolled fence.
                    r = Rect::from_xywh(r.left, r.top, r.width(), f.full_rect.height());
                }
                if let Some(g) = px_to_geometry(&r, &self.monitors)
                    && let Some(fence) = self.layout_mut().and_then(|l| l.fence_mut(id))
                {
                    fence.geometry = g;
                }
                self.mark_dirty();
                self.update_fence_geometry(i);
                self.apply_chameleon();
                self.render_backdrop(i);
                self.render_frame(i);
                self.relayout();
            }
            DragKind::Pending(hit) => match hit {
                FrameHit::Menu => {
                    let p = Point::new(
                        self.fences[i].rect.left + local.x,
                        self.fences[i].rect.top + local.y,
                    );
                    self.show_fence_menu(id, p);
                }
                FrameHit::Tab(t) => self.set_active_tab(id, t),
                FrameHit::Title => {
                    let f = &self.fences[i];
                    if f.rolled && self.config.rollup.open_on == OpenTrigger::Click {
                        let open = !f.temp_open;
                        self.set_temp_open(i, open);
                    }
                }
                FrameHit::ScrollUp => self.scroll_fence(id, -1),
                FrameHit::ScrollDown => self.scroll_fence(id, 1),
                FrameHit::PortalUp => self.portal_up(id),
                FrameHit::PortalOpen => self.portal_open(id),
                FrameHit::Body | FrameHit::Resize(_) => {}
            },
        }
    }

    pub fn set_temp_open(&mut self, i: usize, open: bool) {
        self.fences[i].temp_open = open;
        self.fences[i].left_at = None;
        self.update_fence_geometry(i);
        self.render_backdrop(i);
        self.render_frame(i);
        self.relayout();
        if open {
            self.set_timer(Timer::Collapse, COLLAPSE_POLL_MS);
        }
    }

    /// True when the pointer is over the fence and not over a window
    /// covering the desktop at that place.
    fn pointer_over(&self, i: usize, p: Point) -> bool {
        let f = &self.fences[i];
        if !f.rect.contains(p) {
            return false;
        }
        let under = window::window_from_point(p);
        // Icon tooltips of Explorer float over the desktop.
        if under.class_name() == "tooltips_class32" {
            return true;
        }
        let root = under.root();
        [
            f.frame.hwnd().root(),
            f.backdrop.hwnd().root(),
            f.layer.listview.root(),
        ]
        .contains(&root)
    }

    /// Rolls temporarily opened fences up again once the pointer left them
    /// for the configured delay. They stay open during a drag or while a
    /// menu is shown. Polls while any fence is temporarily open, so a
    /// missed mouse-leave notification cannot keep one open.
    pub fn check_collapse(&mut self) {
        let p = bento_win::input::cursor_pos();
        let busy =
            self.popup.is_some() || self.edit.is_some() || bento_win::input::left_button_down();
        let delay =
            std::time::Duration::from_millis(u64::from(self.config.rollup.collapse_delay_ms));
        let now = std::time::Instant::now();
        let mut pending = false;
        for i in 0..self.fences.len() {
            if !self.fences[i].temp_open {
                continue;
            }
            if busy || self.fences[i].drag.is_some() || self.pointer_over(i, p) {
                self.fences[i].left_at = None;
                pending = true;
                continue;
            }
            let since = *self.fences[i].left_at.get_or_insert(now);
            if now.saturating_duration_since(since) < delay {
                pending = true;
                continue;
            }
            tracing::debug!("fence {:?}: rolls up again", self.fences[i].id);
            self.set_temp_open(i, false);
        }
        if pending {
            self.set_timer(Timer::Collapse, COLLAPSE_POLL_MS);
        }
    }

    pub fn on_body_hover(&mut self, zone: Option<usize>) {
        let visible: Vec<usize> = (0..self.fences.len())
            .filter(|&i| self.fences[i].visible)
            .collect();
        let target = zone.and_then(|z| visible.get(z).copied());
        for i in 0..self.fences.len() {
            let h = Some(i) == target;
            if self.fences[i].hovered_body != h {
                self.fences[i].hovered_body = h;
                if !h && self.fences[i].temp_open {
                    self.set_timer(Timer::Collapse, 0);
                }
                self.render_frame(i);
            }
        }
    }

    pub fn on_body_wheel(&mut self, p: Point, delta: i16) {
        let Some(i) = self
            .fences
            .iter()
            .position(|f| f.visible && f.rect.contains(p))
        else {
            return;
        };
        let steps = -i32::from(delta) / 120;
        let steps = if steps == 0 {
            -i32::from(delta.signum())
        } else {
            steps
        };
        if self.fences[i].portal {
            self.portal_scroll(i, steps);
        } else {
            let id = self.fences[i].id;
            self.scroll_fence(id, steps);
        }
    }

    pub fn scroll_fence(&mut self, id: FenceId, delta: i32) {
        let Some(i) = self.fences.iter().position(|f| f.id == id) else {
            return;
        };
        let max = self.fences[i].max_scroll;
        if let Some(t) = self
            .layout_mut()
            .and_then(|l| l.fence_mut(id))
            .and_then(Fence::active_mut)
        {
            let next = (i64::from(t.scroll_row) + i64::from(delta)).clamp(0, i64::from(max)) as u32;
            if next == t.scroll_row {
                return;
            }
            t.scroll_row = next;
        }
        self.mark_dirty();
        self.relayout();
    }

    pub fn set_active_tab(&mut self, id: FenceId, tab: usize) {
        if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
            if tab >= f.tabs.len() || f.active_tab == tab {
                return;
            }
            f.active_tab = tab;
        }
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
    }

    pub fn toggle_rollup(&mut self, id: FenceId) {
        if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
            f.rolled_up = !f.rolled_up;
        }
        if let Some(i) = self.fences.iter().position(|f| f.id == id) {
            self.fences[i].temp_open = false;
            self.update_fence_geometry(i);
            self.render_backdrop(i);
            self.render_frame(i);
        }
        self.mark_dirty();
        self.relayout();
    }

    // ----- creation and deletion -------------------------------------------

    /// Creates a fence on `rect` (screen pixels), capturing icons inside.
    pub fn create_fence(&mut self, rect: Option<Rect>, portal: Option<String>) -> Option<FenceId> {
        let rect = rect.unwrap_or_else(|| {
            let p = bento_win::input::cursor_pos();
            let m = bento_core::display::monitor_at(&self.monitors, p)
                .or_else(|| self.monitors.iter().find(|m| m.primary))
                .cloned()
                .unwrap_or_default();
            let w = dip_to_px(340, m.dpi);
            let h = dip_to_px(240, m.dpi);
            Rect::from_xywh(
                m.work.left + (m.work.width() - w) / 2,
                m.work.top + (m.work.height() - h) / 3,
                w,
                h,
            )
        });
        let g = px_to_geometry(&rect, &self.monitors)?;
        let members = if portal.is_none() {
            crate::sync::items_in_rect(&self.items, &rect, self.spacing)
                .into_iter()
                .filter(|k| !self.is_fenced(k))
                .collect()
        } else {
            Vec::new()
        };
        // Exact icon positions need "auto arrange" and "align to grid" off;
        // this is only done once the user has a fence.
        self.worker.send(crate::worker::Cmd::FreePositioning);
        let fid = FenceId(self.state.new_id());
        let tid = TabId(self.state.new_id());
        let title = match &portal {
            Some(p) => std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone()),
            None => self.t(Msg::NewFenceTitle).to_owned(),
        };
        let content = match portal {
            Some(root) => TabContent::Portal {
                current: root.clone(),
                root,
                view: self.config.portals.default_view,
            },
            None => TabContent::Desktop { items: members },
        };
        let fence = Fence {
            id: fid,
            geometry: g,
            tabs: vec![Tab {
                id: tid,
                title,
                content,
                ..Tab::default()
            }],
            auto_rollup: self.config.rollup.auto_rollup_docked,
            ..Fence::default()
        };
        self.layout_mut()?.page_mut()?.fences.push(fence);
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
        Some(fid)
    }

    /// Deletes a fence; `restore` moves its icons to free desktop slots.
    pub fn delete_fence(&mut self, id: FenceId, restore: bool) {
        let Some(l) = self.layout_mut() else { return };
        let Some(page) = l.page_of(id) else { return };
        let Some(pos) = l.pages[page].fences.iter().position(|f| f.id == id) else {
            return;
        };
        let fence = l.pages[page].fences.remove(pos);
        if restore {
            let keys: Vec<String> = fence.all_items().cloned().collect();
            let rect = geometry_to_px(&fence.geometry, &self.monitors).unwrap_or_default();
            let work = bento_core::display::monitor_for_rect(&self.monitors, &rect)
                .map(|m| m.work)
                .unwrap_or_default();
            let vs = self.virtual_screen();
            let occupied: Vec<Rect> = self
                .items
                .iter()
                .filter(|(k, it)| !keys.contains(k) && vs.contains(it.pos))
                .map(|(_, it)| {
                    Rect::from_xywh(it.pos.x, it.pos.y, self.spacing.width, self.spacing.height)
                })
                .chain(self.fences.iter().filter(|f| f.id != id).map(|f| f.rect))
                .collect();
            let slots = crate::sync::free_slots(work, self.spacing, &occupied, keys.len());
            let moves: Vec<(String, Point)> = keys.into_iter().zip(slots).collect();
            for (k, p) in &moves {
                if let Some(it) = self.items.get_mut(k) {
                    it.pos = *p;
                }
            }
            if !moves.is_empty() {
                self.worker.send(crate::worker::Cmd::Move(moves));
            }
        } else {
            // Icons stay where they are; parked ones come back in view.
            let vs = self.virtual_screen();
            let parked: Vec<String> = fence
                .all_items()
                .filter(|k| {
                    self.items
                        .get(k.as_str())
                        .is_some_and(|it| !vs.contains(it.pos))
                })
                .cloned()
                .collect();
            if !parked.is_empty() {
                let work = self
                    .monitors
                    .iter()
                    .find(|m| m.primary)
                    .map(|m| m.work)
                    .unwrap_or_default();
                let occupied: Vec<Rect> = self
                    .items
                    .values()
                    .filter(|it| vs.contains(it.pos))
                    .map(|it| {
                        Rect::from_xywh(it.pos.x, it.pos.y, self.spacing.width, self.spacing.height)
                    })
                    .collect();
                let slots = crate::sync::free_slots(work, self.spacing, &occupied, parked.len());
                self.worker.send(crate::worker::Cmd::Move(
                    parked.into_iter().zip(slots).collect(),
                ));
            }
        }
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
        self.worker.send(crate::worker::Cmd::Snapshot);
    }

    // ----- rubber band (right-drag on the desktop) --------------------------

    pub fn begin_rubber(&mut self, p: Point) {
        self.close_popup();
        self.rubber = crate::popup::RubberBand::new(self, p).ok();
    }

    pub fn update_rubber(&mut self, p: Point) {
        if let Some(mut r) = self.rubber.take() {
            r.update(self, p);
            self.rubber = Some(r);
        }
    }

    pub fn end_rubber(&mut self, p: Point) {
        let Some(r) = self.rubber.take() else { return };
        let rect = Rect::from_points(r.start, p);
        drop(r);
        let min = dip_to_px(60, 96);
        if rect.width() < min || rect.height() < min {
            // Plain right click: give it back to the desktop so that the
            // native context menu opens.
            if let Some(layer) = self.layer {
                let o = layer.listview.client_origin();
                let lp = (((p.y - o.y) as u16 as isize) << 16) | ((p.x - o.x) as u16 as isize);
                layer.listview.post(msg::WM_RBUTTONDOWN, 0x0002, lp);
                layer.listview.post(msg::WM_RBUTTONUP, 0, lp);
            }
            return;
        }
        self.show_create_menu(rect, p);
    }

    // ----- rename -----------------------------------------------------------

    pub fn begin_rename(&mut self, id: FenceId) {
        let Some(i) = self.fences.iter().position(|f| f.id == id) else {
            return;
        };
        let title = self
            .layout()
            .and_then(|l| l.fence(id))
            .map(|f| f.title().to_owned())
            .unwrap_or_default();
        let ui = &self.fences[i];
        let m = FrameMetrics::for_dpi(ui.dpi);
        let r = m.title(&ui.rect).inflate(-dip_to_px(3, ui.dpi));
        let Ok(font) = Font::new("Segoe UI", dip_to_px(14, ui.dpi)) else {
            return;
        };
        match window::create_inline_edit(self.hwnd(), r, &title, &font, self.hwnd(), WM_EDIT) {
            Ok(w) => {
                w.hwnd().show_active();
                w.hwnd().set_foreground();
                w.hwnd().set_focus();
                self.edit = Some(InlineEdit {
                    fence: id,
                    window: w,
                    _font: font,
                });
            }
            Err(e) => tracing::warn!("rename box: {e}"),
        }
    }

    /// Enter (1), Escape (2) or focus lost (3) in the rename box.
    pub fn on_edit_key(&mut self, code: usize) {
        let Some(edit) = self.edit.take() else { return };
        if code == 1 || code == 3 {
            let text = edit.window.hwnd().text().trim().to_owned();
            if !text.is_empty() {
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(edit.fence))
                    .and_then(Fence::active_mut)
                {
                    t.title = text;
                }
                self.mark_dirty();
            }
        }
        drop(edit);
        for i in 0..self.fences.len() {
            self.render_frame(i);
        }
    }

    // ----- drag and drop of desktop icons onto titles and tabs ----------------

    pub fn on_drag_event(&mut self, wparam: usize, lparam: isize) {
        let Some(ev) = DragEvent::decode(wparam) else {
            return;
        };
        let p = unpack_point(lparam);
        // A locked fence accepts no drop.
        let target = self
            .fences
            .iter()
            .position(|f| f.visible && f.rect.contains(p))
            .filter(|&i| !self.fence_locked(i));
        let tab_at = |app: &App, i: usize| -> Option<usize> {
            let l = app.frame_layout(i)?;
            let local = app.fences[i].local_mouse(p);
            match l.hit(local, false) {
                FrameHit::Tab(t) => Some(t),
                _ => app
                    .layout()
                    .and_then(|lay| lay.fence(app.fences[i].id))
                    .map(|f| f.active_tab),
            }
        };
        match ev {
            DragEvent::Enter | DragEvent::Over => {
                for i in 0..self.fences.len() {
                    let want = if Some(i) == target {
                        tab_at(self, i)
                    } else {
                        None
                    };
                    if self.fences[i].drop_tab != want {
                        self.fences[i].drop_tab = want;
                        self.render_frame(i);
                    }
                }
                // Hovering a rolled fence during a drag opens it.
                if let Some(i) = target
                    && self.fences[i].rolled
                    && !self.fences[i].temp_open
                {
                    self.set_temp_open(i, true);
                }
            }
            DragEvent::Leave => {
                for i in 0..self.fences.len() {
                    if self.fences[i].drop_tab.take().is_some() {
                        self.render_frame(i);
                    }
                }
            }
            DragEvent::Drop => {
                let keys = take_dropped();
                let Some(i) = target else { return };
                let tab = tab_at(self, i).unwrap_or(0);
                self.fences[i].drop_tab = None;
                let on_portal_body = self.fences[i].portal
                    && self.frame_layout(i).is_some_and(|l| {
                        l.hit(self.fences[i].local_mouse(p), true) == FrameHit::Body
                    });
                if on_portal_body {
                    let (ctrl, shift) = bento_win::dragdrop::dropped_keys();
                    self.portal_drop(i, keys, ctrl, shift);
                    self.render_frame(i);
                    return;
                }
                let id = self.fences[i].id;
                // Icons of a locked fence stay in it.
                let known: Vec<String> = keys
                    .into_iter()
                    .filter(|k| self.items.contains_key(k) && !self.in_locked_fence(k))
                    .collect();
                if known.is_empty() {
                    self.render_frame(i);
                    return;
                }
                for l in &mut self.state.layouts {
                    for pg in &mut l.pages {
                        for f in &mut pg.fences {
                            for k in &known {
                                f.remove_item(k);
                            }
                        }
                    }
                }
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id))
                    && let Some(t) = f.tabs.get_mut(tab)
                    && let Some(list) = t.content.desktop_items_mut()
                {
                    list.extend(known);
                }
                self.sort_all_tabs();
                self.mark_dirty();
                self.render_frame(i);
                self.relayout();
            }
        }
    }
}

#[cfg(test)]
mod roll_anim_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn eases_from_start_to_target() {
        let start = Instant::now();
        let a = RollAnim {
            from_bottom: 130,
            start,
        };
        assert_eq!(a.bottom(330, start), 130);
        let mid = a.bottom(330, start + Duration::from_millis(90));
        // Ease-out: past the halfway point at half time.
        assert!(mid > 230 && mid < 330, "{mid}");
        assert!(!a.done(start + Duration::from_millis(90)));
        let end = start + Duration::from_millis(ROLL_ANIM_MS as u64);
        assert_eq!(a.bottom(330, end), 330);
        assert!(a.done(end));
        // Rolling up goes the other way.
        assert_eq!(a.bottom(30, end), 30);
    }
}
