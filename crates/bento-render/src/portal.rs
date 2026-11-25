//! Content of folder portals and of the Peek overlay: icons drawn by Bento
//! Desk itself (they are not desktop items).

use bento_core::color::Rgba;
use bento_core::geometry::{Point, Rect, dip_to_px};
use bento_core::image::Image;
use bento_core::model::ViewMode;
use bento_win::gfx::{Align, Canvas, RectF, TextStyle};

use crate::theme::{UI_FONT, scale};

/// One entry shown in a portal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub icon: Option<Image>,
}

/// Grid of entry cells inside a body rectangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryLayout {
    pub cells: Vec<(usize, Rect)>,
    pub per_row: usize,
    pub visible_rows: usize,
    pub total_rows: usize,
    pub first_row: usize,
}

impl EntryLayout {
    pub fn max_first_row(&self) -> usize {
        self.total_rows.saturating_sub(self.visible_rows)
    }

    pub fn hit(&self, p: Point) -> Option<usize> {
        self.cells
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|(i, _)| *i)
    }
}

/// Cell size of a view mode.
pub fn cell_size(view: ViewMode, dpi: u32, body_width: i32) -> (i32, i32) {
    match view {
        ViewMode::Icons => (dip_to_px(78, dpi), dip_to_px(80, dpi)),
        ViewMode::List => (body_width.max(1), dip_to_px(26, dpi)),
    }
}

pub fn layout(view: ViewMode, body: Rect, dpi: u32, count: usize, first_row: usize) -> EntryLayout {
    let (cw, ch) = cell_size(view, dpi, body.width());
    let per_row = usize::try_from((body.width() / cw.max(1)).max(1)).unwrap_or(1);
    let visible_rows = usize::try_from((body.height() / ch.max(1)).max(1)).unwrap_or(1);
    let total_rows = count.div_ceil(per_row);
    let first_row = first_row.min(total_rows.saturating_sub(visible_rows));
    let slack = (body.width() - i32::try_from(per_row).unwrap_or(1) * cw).max(0) / 2;
    let mut cells = Vec::new();
    for row in first_row..(first_row + visible_rows).min(total_rows) {
        for col in 0..per_row {
            let i = row * per_row + col;
            if i >= count {
                break;
            }
            let x = body.left + slack + i32::try_from(col).unwrap_or(0) * cw;
            let y = body.top + i32::try_from(row - first_row).unwrap_or(0) * ch;
            cells.push((i, Rect::from_xywh(x, y, cw, ch)));
        }
    }
    EntryLayout {
        cells,
        per_row,
        visible_rows,
        total_rows,
        first_row,
    }
}

/// Draws entries; `message` replaces them (unavailable or empty folder).
#[allow(clippy::too_many_arguments)]
pub fn draw_entries(
    c: &mut Canvas<'_>,
    entries: &[Entry],
    l: &EntryLayout,
    view: ViewMode,
    dpi: u32,
    fg: Rgba,
    hover: Option<usize>,
    selected: &[usize],
    message: Option<(&str, &str)>,
    body: Rect,
) {
    let s = scale(dpi);
    if let Some((title, detail)) = message {
        let mut st = TextStyle::new(UI_FONT, 14.0 * s, fg);
        st.align = Align::Center;
        let b = RectF::from(body);
        let mid = (b.top + b.bottom) / 2.0;
        c.text(title, RectF::new(b.left, mid - 22.0 * s, b.right, mid), &st);
        let mut d = st.clone();
        d.size = 12.0 * s;
        d.color = fg.with_alpha(0xA0);
        c.text(detail, RectF::new(b.left, mid, b.right, mid + 20.0 * s), &d);
        return;
    }
    for (i, r) in &l.cells {
        let Some(e) = entries.get(*i) else { continue };
        let rf = RectF::from(*r).inset(2.0 * s);
        if selected.contains(i) {
            c.fill_round_rect(rf, 4.0 * s, Rgba::new(255, 255, 255, 0x30));
        } else if hover == Some(*i) {
            c.fill_round_rect(rf, 4.0 * s, Rgba::new(255, 255, 255, 0x1A));
        }
        match view {
            ViewMode::Icons => {
                let isz = 32.0 * s;
                let ix = (rf.left + rf.right - isz) / 2.0;
                let icon = RectF::xywh(ix, rf.top + 4.0 * s, isz, isz);
                if let Some(img) = &e.icon {
                    c.image(img, icon, 1.0);
                }
                let mut st = TextStyle::new(UI_FONT, 12.0 * s, fg);
                st.align = Align::Center;
                st.vcenter = false;
                st.wrap = true;
                c.text(
                    &e.name,
                    RectF::new(
                        rf.left + 2.0 * s,
                        icon.bottom + 4.0 * s,
                        rf.right - 2.0 * s,
                        rf.bottom,
                    ),
                    &st,
                );
            }
            ViewMode::List => {
                let isz = 16.0 * s;
                let icon = RectF::xywh(
                    rf.left + 4.0 * s,
                    (rf.top + rf.bottom - isz) / 2.0,
                    isz,
                    isz,
                );
                if let Some(img) = &e.icon {
                    c.image(img, icon, 1.0);
                }
                let st = TextStyle::new(UI_FONT, 13.0 * s, fg);
                c.text(
                    &e.name,
                    RectF::new(icon.right + 8.0 * s, rf.top, rf.right, rf.bottom),
                    &st,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_grid_layout_and_hit() {
        let body = Rect::from_xywh(0, 0, 320, 170);
        let l = layout(ViewMode::Icons, body, 96, 10, 0);
        assert_eq!(l.per_row, 4);
        assert_eq!(l.visible_rows, 2);
        assert_eq!(l.total_rows, 3);
        assert_eq!(l.cells.len(), 8);
        assert_eq!(l.max_first_row(), 1);
        let p = l.cells[5].1.center();
        assert_eq!(l.hit(p), Some(5));
        let scrolled = layout(ViewMode::Icons, body, 96, 10, 5);
        assert_eq!(scrolled.first_row, 1);
        assert_eq!(scrolled.cells[0].0, 4);
    }

    #[test]
    fn list_layout() {
        let body = Rect::from_xywh(0, 0, 200, 100);
        let l = layout(ViewMode::List, body, 96, 10, 2);
        assert_eq!(l.per_row, 1);
        assert_eq!(l.visible_rows, 3);
        assert_eq!(l.cells[0].0, 2);
    }
}
