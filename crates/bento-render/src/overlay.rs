//! Small overlays: rubber band, page indicator, notification toast, Peek
//! dimming.

use bento_core::color::Rgba;
use bento_win::gfx::{Align, Canvas, RectF, TextStyle};

use crate::theme::{Theme, UI_FONT, scale};

/// Selection rectangle while drawing a new fence.
pub fn draw_rubber_band(c: &mut Canvas<'_>, w: i32, h: i32, theme: &Theme, dpi: u32) {
    let s = scale(dpi);
    let r = RectF::xywh(0.0, 0.0, w as f32, h as f32).inset(1.0);
    c.fill_round_rect(r, 8.0 * s, theme.accent.with_alpha(0x33));
    c.stroke_round_rect(r, 8.0 * s, theme.accent.with_alpha(0xE0), 2.0 * s);
}

/// Pill showing "page n / total" with dots.
#[allow(clippy::too_many_arguments)]
pub fn draw_page_indicator(
    c: &mut Canvas<'_>,
    w: i32,
    h: i32,
    current: usize,
    total: usize,
    label: &str,
    theme: &Theme,
    dpi: u32,
) {
    let s = scale(dpi);
    let r = RectF::xywh(0.0, 0.0, w as f32, h as f32).inset(0.5);
    c.fill_round_rect(r, r.height() / 2.0, theme.surface);
    c.stroke_round_rect(r, r.height() / 2.0, theme.surface_border, 1.0);
    let mut st = TextStyle::new(UI_FONT, 13.0 * s, theme.text);
    st.align = Align::Center;
    c.text(
        label,
        RectF::new(r.left, r.top, r.right, r.top + r.height() * 0.6),
        &st,
    );
    let n = total.max(1) as f32;
    let gap = 12.0 * s;
    let start = (r.left + r.right) / 2.0 - gap * (n - 1.0) / 2.0;
    let y = r.top + r.height() * 0.75;
    for i in 0..total {
        let col = if i == current {
            theme.accent
        } else {
            theme.text_secondary.with_alpha(0x60)
        };
        c.fill_circle(start + gap * i as f32, y, 3.5 * s, col);
    }
}

/// Short message pill.
pub fn draw_toast(c: &mut Canvas<'_>, w: i32, h: i32, text: &str, theme: &Theme, dpi: u32) {
    let s = scale(dpi);
    let r = RectF::xywh(0.0, 0.0, w as f32, h as f32).inset(0.5);
    c.fill_round_rect(r, 8.0 * s, theme.surface);
    c.stroke_round_rect(r, 8.0 * s, theme.surface_border, 1.0);
    let mut st = TextStyle::new(UI_FONT, 14.0 * s, theme.text);
    st.align = Align::Center;
    c.text(text, r.inset(8.0 * s), &st);
}

/// Dimmed full-screen background of Peek with a hint at the bottom.
pub fn draw_peek_background(c: &mut Canvas<'_>, w: i32, h: i32, dim: u8, hint: &str, dpi: u32) {
    let s = scale(dpi);
    let alpha = (u32::from(dim.min(90)) * 255 / 100) as u8;
    c.fill_rect(
        RectF::xywh(0.0, 0.0, w as f32, h as f32),
        Rgba::new(0, 0, 0, alpha),
    );
    let mut st = TextStyle::new(UI_FONT, 13.0 * s, Rgba::new(255, 255, 255, 0xB0));
    st.align = Align::Center;
    c.text(
        hint,
        RectF::new(0.0, h as f32 - 40.0 * s, w as f32, h as f32 - 12.0 * s),
        &st,
    );
}
