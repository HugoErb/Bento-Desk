//! Fence backgrounds (below the icons) and frames (title bar, tabs,
//! buttons, resize ring, above the icons).
//!
//! The frame body is fully transparent (alpha 0) so clicks reach the real
//! icons. The resize ring and hidden title use alpha 1: invisible, but hit
//! testable.

use bento_core::color::Rgba;
use bento_core::geometry::{Point, Rect, dip_to_px};
use bento_core::grid::FrameMetrics;
use bento_core::image::Image;
use bento_core::model::{BackdropEffect, FenceStyle};
use bento_core::snap::Edges;
use bento_win::gfx::{Align, Canvas, RectF, TextStyle};

use crate::icons;
use crate::theme::{ICON_FONT, Theme, scale};

/// Almost invisible color used for hit-testable areas.
const HIT: Rgba = Rgba::new(0, 0, 0, 1);

/// One tab as shown in the title bar.
#[derive(Clone, Debug, PartialEq)]
pub struct TabView {
    pub title: String,
    pub color: Rgba,
}

/// Scroll state of the fence content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollView {
    pub first_row: u32,
    pub max_first_row: u32,
    pub visible_rows: u32,
}

/// Portal specific title buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalHeader {
    pub can_go_up: bool,
    pub show_folder_icon: bool,
}

/// Everything needed to draw a frame.
#[derive(Clone, Debug)]
pub struct FrameView<'a> {
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    pub style: &'a FenceStyle,
    pub tabs: Vec<TabView>,
    pub active_tab: usize,
    pub title_visible: bool,
    pub hovered: bool,
    pub rolled_up: bool,
    pub scroll: Option<ScrollView>,
    pub portal: Option<PortalHeader>,
    pub drop_tab: Option<usize>,
    pub accent: Rgba,
}

/// Region of a frame under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameHit {
    /// Transparent body (only reachable for portals).
    Body,
    Title,
    Tab(usize),
    Menu,
    PortalUp,
    PortalOpen,
    ScrollUp,
    ScrollDown,
    Resize(Edges),
}

/// Positions of the frame parts, local pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLayout {
    pub metrics: FrameMetrics,
    pub title: Rect,
    pub title_text: Rect,
    pub tabs: Vec<Rect>,
    pub menu_btn: Rect,
    pub up_btn: Option<Rect>,
    pub open_btn: Option<Rect>,
    pub scroll_up: Option<Rect>,
    pub scroll_down: Option<Rect>,
    pub body: Rect,
    pub width: i32,
    pub height: i32,
    pub rolled_up: bool,
}

impl FrameLayout {
    pub fn new(v: &FrameView<'_>) -> FrameLayout {
        let m = FrameMetrics::for_dpi(v.dpi);
        let s = |d: i32| dip_to_px(d, v.dpi);
        let full = Rect::from_xywh(0, 0, v.width, v.height);
        let title = m.title(&full);
        let btn = m.title_height;
        let mut right = title.right - s(4);
        let menu_btn = Rect::new(right - btn, title.top, right, title.bottom);
        right = menu_btn.left;
        let (mut up_btn, mut open_btn) = (None, None);
        if let Some(p) = v.portal {
            let o = Rect::new(right - btn, title.top, right, title.bottom);
            open_btn = Some(o);
            right = o.left;
            if p.can_go_up {
                let u = Rect::new(right - btn, title.top, right, title.bottom);
                up_btn = Some(u);
                right = u.left;
            }
        }
        let mut left = title.left + s(10);
        if v.portal.is_some_and(|p| p.show_folder_icon) {
            left += s(22);
        }
        let title_text = Rect::new(left, title.top, right.max(left), title.bottom);
        let tabs = if v.tabs.len() > 1 {
            let n = i32::try_from(v.tabs.len()).unwrap_or(1);
            let avail = (right - title.left - s(6)).max(n);
            let w = (avail / n).min(s(160));
            (0..n)
                .map(|i| {
                    Rect::new(
                        title.left + s(6) + i * w,
                        title.top + s(4),
                        title.left + s(6) + (i + 1) * w - s(2),
                        title.bottom - s(3),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let body = m.body(&full);
        let (mut scroll_up, mut scroll_down) = (None, None);
        if !v.rolled_up && v.scroll.is_some_and(|sc| sc.max_first_row > 0) {
            let bw = s(18);
            scroll_up = Some(Rect::new(
                body.right - bw,
                body.top,
                body.right,
                body.top + bw,
            ));
            scroll_down = Some(Rect::new(
                body.right - bw,
                body.bottom - bw,
                body.right,
                body.bottom,
            ));
        }
        FrameLayout {
            metrics: m,
            title,
            title_text,
            tabs,
            menu_btn,
            up_btn,
            open_btn,
            scroll_up,
            scroll_down,
            body,
            width: v.width,
            height: v.height,
            rolled_up: v.rolled_up,
        }
    }

    /// What is under a local point.
    pub fn hit(&self, p: Point, portal: bool) -> FrameHit {
        let rb = self.metrics.resize_border;
        let edges = Edges {
            left: p.x < rb,
            right: p.x >= self.width - rb,
            top: p.y < rb / 2,
            bottom: !self.rolled_up && p.y >= self.height - rb,
        };
        if edges.any() {
            return FrameHit::Resize(edges);
        }
        if self.menu_btn.contains(p) {
            return FrameHit::Menu;
        }
        if self.up_btn.is_some_and(|r| r.contains(p)) {
            return FrameHit::PortalUp;
        }
        if self.open_btn.is_some_and(|r| r.contains(p)) {
            return FrameHit::PortalOpen;
        }
        if let Some(i) = self.tabs.iter().position(|r| r.contains(p)) {
            return FrameHit::Tab(i);
        }
        if self.title.contains(p) {
            return FrameHit::Title;
        }
        if self.scroll_up.is_some_and(|r| r.contains(p)) {
            return FrameHit::ScrollUp;
        }
        if self.scroll_down.is_some_and(|r| r.contains(p)) {
            return FrameHit::ScrollDown;
        }
        let _ = portal;
        FrameHit::Body
    }
}

/// Color of the title text for a style.
pub fn title_color(style: &FenceStyle, bg: Rgba) -> Rgba {
    style
        .title_color
        .unwrap_or_else(|| bg.with_alpha(255).contrasting_text())
}

/// Opaque color seen under the title text: the band laid over `bg`.
pub fn title_band_seen(style: &FenceStyle, bg: Rgba) -> Rgba {
    let band = style.title_band();
    bg.with_alpha(255)
        .mix(band.with_alpha(255), f32::from(band.a) / 255.0)
}

fn button(c: &mut Canvas<'_>, r: Rect, glyph: &str, fg: Rgba, hovered: bool, dpi: u32) {
    let rf = RectF::from(r).inset(3.0 * scale(dpi));
    if hovered {
        c.fill_round_rect(rf, 4.0 * scale(dpi), fg.with_alpha(0x22));
    } else {
        c.fill_rect(RectF::from(r), HIT);
    }
    let mut st = TextStyle::new(ICON_FONT, 11.0 * scale(dpi), fg);
    st.align = Align::Center;
    c.text(glyph, RectF::from(r), &st);
}

/// Draws a frame. `hover` is the local cursor position when hovered.
pub fn draw_frame(c: &mut Canvas<'_>, v: &FrameView<'_>, l: &FrameLayout, hover: Option<Point>) {
    let s = scale(v.dpi);
    let radius = f32::from(v.style.corner_radius) * s;
    let bg = v
        .tabs
        .get(v.active_tab)
        .map(|t| t.color)
        .unwrap_or_else(|| v.style.tint.to_rgba());
    let fg = title_color(v.style, title_band_seen(v.style, bg));

    // Invisible but hit-testable resize ring.
    let rb = l.metrics.resize_border as f32;
    let (w, h) = (v.width as f32, v.height as f32);
    if !v.rolled_up {
        c.fill_rect(RectF::new(0.0, l.title.bottom as f32, rb, h), HIT);
        c.fill_rect(RectF::new(w - rb, l.title.bottom as f32, w, h), HIT);
        c.fill_rect(RectF::new(rb, h - rb, w - rb, h), HIT);
    }

    // Title area: always hit-testable, drawn when visible.
    let title = RectF::from(l.title);
    if v.title_visible {
        let band = v.style.title_band();
        if v.rolled_up {
            c.fill_round_rect(title, radius, band);
        } else {
            // Rounded top corners, square bottom ones, in a single fill: two
            // overlapping translucent fills would leave a darker strip.
            c.push_clip(title);
            c.fill_round_rect(
                RectF::new(title.left, title.top, title.right, title.bottom + radius),
                radius,
                band,
            );
            c.pop_clip();
        }
    } else {
        c.fill_rect(title, HIT);
    }

    if v.title_visible {
        let mut st = TextStyle::new(&v.style.title_font, f32::from(v.style.title_size) * s, fg);
        st.bold = v.style.title_bold;
        if l.tabs.is_empty() {
            if v.portal.is_some_and(|p| p.show_folder_icon) {
                let mut ic = TextStyle::new(ICON_FONT, 13.0 * s, fg);
                ic.align = Align::Center;
                let r = Rect::new(
                    l.title.left + dip_to_px(8, v.dpi),
                    l.title.top,
                    l.title_text.left,
                    l.title.bottom,
                );
                c.text(icons::FOLDER, RectF::from(r), &ic);
            }
            let title_txt = v
                .tabs
                .get(v.active_tab)
                .map(|t| t.title.as_str())
                .unwrap_or("");
            c.text(title_txt, RectF::from(l.title_text), &st);
        } else {
            for (i, (r, t)) in l.tabs.iter().zip(&v.tabs).enumerate() {
                let rf = RectF::from(*r);
                let active = i == v.active_tab;
                let alpha = if active { 0xE0 } else { 0x70 };
                c.fill_round_rect(rf, 5.0 * s, t.color.with_alpha(alpha));
                if v.drop_tab == Some(i) {
                    c.stroke_round_rect(rf.inset(1.0), 5.0 * s, v.accent, 2.0 * s);
                }
                let mut ts = st.clone();
                ts.color = title_color(v.style, t.color);
                ts.align = Align::Center;
                ts.bold = active;
                c.text(&t.title, rf.inset(4.0 * s), &ts);
            }
        }
        let hov = |r: Rect| hover.is_some_and(|p| r.contains(p));
        if v.hovered {
            button(c, l.menu_btn, icons::MORE, fg, hov(l.menu_btn), v.dpi);
        }
        if let Some(r) = l.up_btn {
            button(c, r, icons::UP, fg, hov(r), v.dpi);
        }
        if let Some(r) = l.open_btn {
            button(c, r, icons::OPEN, fg, hov(r), v.dpi);
        }
    }

    // Scroll buttons and position.
    if let (Some(up), Some(down), Some(sc)) = (l.scroll_up, l.scroll_down, v.scroll) {
        let track = RectF::new(
            up.left as f32 + up.width() as f32 * 0.4,
            up.bottom as f32,
            up.left as f32 + up.width() as f32 * 0.6,
            down.top as f32,
        );
        c.fill_round_rect(track, 2.0 * s, Rgba::new(255, 255, 255, 0x20));
        let total = (sc.max_first_row + sc.visible_rows).max(1) as f32;
        let th = (track.height() * sc.visible_rows as f32 / total).max(8.0 * s);
        let ty = track.top
            + (track.height() - th) * (sc.first_row as f32 / sc.max_first_row.max(1) as f32);
        c.fill_round_rect(
            RectF::new(track.left, ty, track.right, ty + th),
            2.0 * s,
            fg.with_alpha(0x90),
        );
        let can_up = sc.first_row > 0;
        let can_down = sc.first_row < sc.max_first_row;
        for (r, g, on) in [
            (up, icons::CHEVRON_UP, can_up),
            (down, icons::CHEVRON_DOWN, can_down),
        ] {
            let col = if on { fg } else { fg.with_alpha(0x50) };
            c.fill_round_rect(RectF::from(r), 4.0 * s, Rgba::new(0, 0, 0, 0x40));
            let mut st = TextStyle::new(ICON_FONT, 9.0 * s, col);
            st.align = Align::Center;
            c.text(g, RectF::from(r), &st);
        }
    }
}

/// Everything needed to draw a background.
#[derive(Clone, Debug)]
pub struct BackdropView<'a> {
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    pub style: &'a FenceStyle,
    /// Tint replacing the style tint (Chameleon).
    pub tint: Option<Rgba>,
    /// Blurred wallpaper and where its top-left lands, local pixels.
    pub blur: Option<(&'a Image, RectF)>,
    /// Highlight when the fence is a drop target or selected in Peek.
    pub highlight: Option<Rgba>,
}

/// Draws a background: blurred wallpaper (optional), tint, border.
pub fn draw_backdrop(c: &mut Canvas<'_>, v: &BackdropView<'_>) {
    let s = scale(v.dpi);
    let r = RectF::xywh(0.0, 0.0, v.width as f32, v.height as f32).inset(0.5);
    let radius = f32::from(v.style.corner_radius) * s;
    if v.style.effect == BackdropEffect::Blur
        && let Some((img, at)) = v.blur
    {
        c.fill_round_rect_image(r, radius, img, at, 1.0);
    }
    let tint = v.tint.unwrap_or_else(|| v.style.tint.to_rgba());
    c.fill_round_rect(r, radius, tint);
    if v.style.border {
        let border = if tint.with_alpha(255).luminance() > 0.5 {
            Rgba::new(0, 0, 0, 0x30)
        } else {
            Rgba::new(255, 255, 255, 0x26)
        };
        c.stroke_round_rect(r, radius, border, 1.0);
    }
    if let Some(h) = v.highlight {
        c.stroke_round_rect(r.inset(1.0), radius, h, 2.0 * s);
    }
}

/// Theme-independent helper for tests and previews.
pub fn default_theme() -> Theme {
    Theme::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(style: &FenceStyle, tabs: usize, portal: bool) -> FrameView<'_> {
        FrameView {
            width: 300,
            height: 200,
            dpi: 96,
            style,
            tabs: (0..tabs)
                .map(|i| TabView {
                    title: format!("T{i}"),
                    color: Rgba::BLACK,
                })
                .collect(),
            active_tab: 0,
            title_visible: true,
            hovered: true,
            rolled_up: false,
            scroll: Some(ScrollView {
                first_row: 0,
                max_first_row: 2,
                visible_rows: 2,
            }),
            portal: portal.then_some(PortalHeader {
                can_go_up: true,
                show_folder_icon: true,
            }),
            drop_tab: None,
            accent: Rgba::WHITE,
        }
    }

    #[test]
    fn hit_regions() {
        let st = FenceStyle::default();
        let v = view(&st, 1, false);
        let l = FrameLayout::new(&v);
        assert_eq!(
            l.hit(Point::new(1, 100), false),
            FrameHit::Resize(Edges {
                left: true,
                ..Edges::NONE
            })
        );
        assert_eq!(l.hit(Point::new(150, 10), false), FrameHit::Title);
        assert_eq!(l.hit(Point::new(150, 120), false), FrameHit::Body);
        assert_eq!(l.hit(Point::new(285, 10), false), FrameHit::Menu);
        let corner = l.hit(Point::new(299, 199), false);
        assert_eq!(
            corner,
            FrameHit::Resize(Edges {
                right: true,
                bottom: true,
                ..Edges::NONE
            })
        );
        assert!(matches!(
            l.hit(Point::new(285, 40), false),
            FrameHit::ScrollUp
        ));
    }

    #[test]
    fn tabs_and_portal_buttons() {
        let st = FenceStyle::default();
        let v = view(&st, 3, true);
        let l = FrameLayout::new(&v);
        assert_eq!(l.tabs.len(), 3);
        assert!(l.up_btn.is_some() && l.open_btn.is_some());
        let t1 = l.tabs[1].center();
        assert_eq!(l.hit(t1, true), FrameHit::Tab(1));
        let up = l.up_btn.map(|r| r.center()).unwrap();
        assert_eq!(l.hit(up, true), FrameHit::PortalUp);
    }
}
