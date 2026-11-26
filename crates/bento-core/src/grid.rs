//! Placement of desktop icons inside a fence body.
//!
//! All values are physical pixels. Positions are the top-left corner of the
//! icon cell, which is what `IFolderView::GetItemPosition` returns and what
//! `IFolderView::SelectAndPositionItems` expects.

use crate::geometry::{Point, Rect, Size};

/// Metrics of a fence frame, in DIP. Converted with the monitor DPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMetrics {
    pub title_height: i32,
    pub padding: i32,
    pub resize_border: i32,
    pub min_width: i32,
    pub min_height: i32,
}

impl FrameMetrics {
    pub const DIP: FrameMetrics = FrameMetrics {
        title_height: 30,
        padding: 6,
        resize_border: 6,
        min_width: 120,
        min_height: 60,
    };

    /// Scales DIP metrics to a DPI.
    pub fn for_dpi(dpi: u32) -> FrameMetrics {
        let s = |v: i32| crate::geometry::dip_to_px(v, dpi);
        let d = Self::DIP;
        FrameMetrics {
            title_height: s(d.title_height),
            padding: s(d.padding),
            resize_border: s(d.resize_border),
            min_width: s(d.min_width),
            min_height: s(d.min_height),
        }
    }

    /// Body of the fence: area below the title, minus padding.
    pub fn body(&self, fence: &Rect) -> Rect {
        Rect::new(
            fence.left + self.padding,
            fence.top + self.title_height,
            fence.right - self.padding,
            fence.bottom - self.padding,
        )
    }

    /// Title bar rectangle.
    pub fn title(&self, fence: &Rect) -> Rect {
        Rect::new(
            fence.left,
            fence.top,
            fence.right,
            fence.top + self.title_height,
        )
    }

    /// Rectangle of a rolled-up fence (title only).
    pub fn rolled(&self, fence: &Rect) -> Rect {
        self.title(fence)
    }
}

/// Grid computed for a fence body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub origin: Point,
    pub cell: Size,
    pub cols: u32,
    pub visible_rows: u32,
}

impl Grid {
    /// Lays a grid of `cell` sized slots in `body`, centered horizontally.
    pub fn new(body: Rect, cell: Size) -> Grid {
        let cw = cell.width.max(1);
        let ch = cell.height.max(1);
        let cols = u32::try_from((body.width() / cw).max(1)).unwrap_or(1);
        let visible_rows = u32::try_from((body.height() / ch).max(0)).unwrap_or(0);
        let used = i32::try_from(cols).unwrap_or(1) * cw;
        let slack = (body.width() - used).max(0) / 2;
        Grid {
            origin: Point::new(body.left + slack, body.top),
            cell: Size::new(cw, ch),
            cols,
            visible_rows,
        }
    }

    pub fn total_rows(&self, count: usize) -> u32 {
        let cols = self.cols.max(1) as usize;
        u32::try_from(count.div_ceil(cols)).unwrap_or(u32::MAX)
    }

    /// Largest first-row value that still shows the last row.
    pub fn max_scroll(&self, count: usize) -> u32 {
        self.total_rows(count).saturating_sub(self.visible_rows)
    }

    /// Position of the item at `index` when the first visible row is
    /// `scroll`, `None` when the item is scrolled out of view.
    pub fn slot(&self, index: usize, scroll: u32) -> Option<Point> {
        let cols = self.cols.max(1) as usize;
        let row = u32::try_from(index / cols).ok()?;
        let col = i32::try_from(index % cols).ok()?;
        if row < scroll || row >= scroll + self.visible_rows {
            return None;
        }
        let vis_row = i32::try_from(row - scroll).ok()?;
        Some(Point::new(
            self.origin.x + col * self.cell.width,
            self.origin.y + vis_row * self.cell.height,
        ))
    }

    /// Index where an item dropped at `p` (its top-left corner) should go,
    /// clamped to `0..=count`.
    pub fn index_at(&self, p: Point, scroll: u32, count: usize) -> usize {
        let cols = self.cols.max(1) as i64;
        let col = (i64::from(p.x - self.origin.x) + i64::from(self.cell.width) / 2)
            .div_euclid(i64::from(self.cell.width.max(1)))
            .clamp(0, cols - 1);
        let row = (i64::from(p.y - self.origin.y) + i64::from(self.cell.height) / 2)
            .div_euclid(i64::from(self.cell.height.max(1)))
            .max(0)
            + i64::from(scroll);
        let idx = row * cols + col;
        usize::try_from(idx).unwrap_or(0).min(count)
    }

    /// Smallest body size that shows `count` items without scrolling,
    /// keeping the current number of columns.
    pub fn fit_height(&self, count: usize) -> i32 {
        i32::try_from(self.total_rows(count)).unwrap_or(i32::MAX / 2) * self.cell.height
    }
}

/// Cell size used inside fences: desktop spacing scaled by the user
/// spacing preference (percent, 100 = same as the desktop).
pub fn fence_cell(desktop_spacing: Size, spacing_percent: u16) -> Size {
    let p = i32::from(spacing_percent.clamp(50, 200));
    Size::new(
        (desktop_spacing.width * p / 100).max(16),
        (desktop_spacing.height * p / 100).max(16),
    )
}

/// Offset from the top-left corner of a fence cell to the position given
/// to Explorer. Explorer positions an icon by the top-left corner of its
/// image, its label being centered below and as wide as the desktop
/// spacing: the image is centered horizontally in the cell, and the extra
/// height of a cell larger than the desktop spacing is split above and
/// below the icon.
pub fn icon_offset(cell: Size, desktop_spacing: Size, icon_size: i32) -> Point {
    let icon = icon_size.clamp(1, desktop_spacing.width.max(1));
    Point::new(
        (cell.width - icon).max(0) / 2,
        (cell.height - desktop_spacing.height).max(0) / 2,
    )
}

/// Off-screen position used to hide ("park") an icon. Explorer cannot hide
/// a single icon, so hidden icons are moved right of the virtual screen.
pub fn park_slot(virtual_screen: &Rect, cell: Size, n: usize) -> Point {
    let col = i32::try_from(n % 16).unwrap_or(0);
    let row = i32::try_from(n / 16).unwrap_or(0);
    Point::new(
        virtual_screen.right + 4 * cell.width.max(1) + col * cell.width.max(1),
        virtual_screen.top + row * cell.height.max(1),
    )
}

/// True when a position is in the parking area.
pub fn is_parked(virtual_screen: &Rect, p: Point) -> bool {
    p.x >= virtual_screen.right
}

/// Result of laying out one fence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// Target position of each visible item.
    pub visible: Vec<(usize, Point)>,
    /// Items that must be parked.
    pub hidden: Vec<usize>,
    pub max_scroll: u32,
}

/// Computes the placement of `count` items for a fence.
pub fn place(grid: &Grid, count: usize, scroll: u32, rolled_up: bool) -> Placement {
    let mut out = Placement {
        max_scroll: grid.max_scroll(count),
        ..Placement::default()
    };
    let scroll = scroll.min(out.max_scroll);
    for i in 0..count {
        match (!rolled_up).then(|| grid.slot(i, scroll)).flatten() {
            Some(p) => out.visible.push((i, p)),
            None => out.hidden.push(i),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::new(Rect::from_xywh(0, 0, 300, 200), Size::new(100, 100))
    }

    #[test]
    fn grid_dimensions() {
        let g = grid();
        assert_eq!(g.cols, 3);
        assert_eq!(g.visible_rows, 2);
        assert_eq!(g.origin, Point::new(0, 0));
    }

    #[test]
    fn grid_centers_horizontally() {
        let g = Grid::new(Rect::from_xywh(0, 0, 350, 100), Size::new(100, 100));
        assert_eq!(g.origin.x, 25);
    }

    #[test]
    fn slots_and_scroll() {
        let g = grid();
        assert_eq!(g.slot(0, 0), Some(Point::new(0, 0)));
        assert_eq!(g.slot(4, 0), Some(Point::new(100, 100)));
        assert_eq!(g.slot(6, 0), None);
        assert_eq!(g.slot(6, 1), Some(Point::new(0, 100)));
        assert_eq!(g.slot(0, 1), None);
        assert_eq!(g.max_scroll(7), 1);
        assert_eq!(g.max_scroll(3), 0);
    }

    #[test]
    fn index_at_rounds_to_nearest_cell() {
        let g = grid();
        assert_eq!(g.index_at(Point::new(10, 10), 0, 10), 0);
        assert_eq!(g.index_at(Point::new(160, 10), 0, 10), 2);
        assert_eq!(g.index_at(Point::new(10, 160), 0, 10), 6);
        assert_eq!(g.index_at(Point::new(10, 160), 1, 10), 9);
        assert_eq!(g.index_at(Point::new(900, 900), 0, 4), 4);
        assert_eq!(g.index_at(Point::new(-500, -500), 0, 4), 0);
    }

    #[test]
    fn place_hides_overflow_and_rolled_up() {
        let g = grid();
        let p = place(&g, 8, 0, false);
        assert_eq!(p.visible.len(), 6);
        assert_eq!(p.hidden, vec![6, 7]);
        assert_eq!(p.max_scroll, 1);
        let p = place(&g, 8, 5, false);
        assert_eq!(p.visible.len(), 5);
        let p = place(&g, 3, 0, true);
        assert_eq!(p.hidden.len(), 3);
    }

    #[test]
    fn frame_metrics_scale() {
        let m = FrameMetrics::for_dpi(192);
        assert_eq!(m.title_height, 60);
        let body = m.body(&Rect::from_xywh(0, 0, 400, 300));
        assert_eq!(body, Rect::new(12, 60, 388, 288));
    }

    #[test]
    fn parking_is_off_screen() {
        let vs = Rect::new(-1920, 0, 1920, 1080);
        let p = park_slot(&vs, Size::new(75, 100), 20);
        assert!(is_parked(&vs, p));
        assert!(!is_parked(&vs, Point::new(100, 100)));
    }

    #[test]
    fn icon_is_centered_in_its_cell() {
        let spacing = Size::new(96, 101);
        assert_eq!(icon_offset(spacing, spacing, 48), Point::new(24, 0));
        let wide = fence_cell(spacing, 120);
        assert_eq!(icon_offset(wide, spacing, 48), Point::new(33, 10));
        // A cell narrower than the icon never shifts it left.
        assert_eq!(
            icon_offset(Size::new(40, 50), spacing, 48),
            Point::new(0, 0)
        );
    }

    #[test]
    fn cell_spacing_is_clamped() {
        assert_eq!(fence_cell(Size::new(80, 100), 100), Size::new(80, 100));
        assert_eq!(fence_cell(Size::new(80, 100), 10), Size::new(40, 50));
    }
}
