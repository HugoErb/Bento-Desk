//! Magnetism: snapping fences to screen edges and to other fences.

use crate::geometry::{Rect, mul_div};

/// Which edges a resize operation moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Edges {
    pub const NONE: Edges = Edges {
        left: false,
        top: false,
        right: false,
        bottom: false,
    };

    pub fn any(&self) -> bool {
        self.left || self.top || self.right || self.bottom
    }
}

/// Snapping parameters, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapConfig {
    pub to_edges: bool,
    pub to_fences: bool,
    pub threshold: i32,
    /// Gap kept between two fences snapped side by side.
    pub gap: i32,
    /// Border width of the fence being moved or resized.
    pub border: i32,
}

/// Another fence the moving one can snap to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub rect: Rect,
    /// Border width of that fence.
    pub border: i32,
}

impl SnapConfig {
    /// Distance between two fences snapped side by side. With no gap and a
    /// border on both, they overlap by the thinner border so that a single
    /// line separates them instead of two.
    pub fn gap_to(&self, other_border: i32) -> i32 {
        if self.gap > 0 {
            self.gap
        } else {
            -self.border.min(other_border).max(0)
        }
    }
}

/// Best correction for one axis: the smallest delta under the threshold.
fn best(current: Option<i32>, delta: i32, threshold: i32) -> Option<i32> {
    if delta.abs() > threshold {
        return current;
    }
    match current {
        Some(c) if c.abs() <= delta.abs() => Some(c),
        _ => Some(delta),
    }
}

fn overlaps_range(a0: i32, a1: i32, b0: i32, b1: i32, slack: i32) -> bool {
    a0 < b1 + slack && b0 < a1 + slack
}

/// Candidate x and y targets for the moving edges of `r`.
fn candidates(
    r: &Rect,
    others: &[Target],
    work_areas: &[Rect],
    cfg: &SnapConfig,
) -> (Vec<i32>, Vec<i32>, Vec<i32>, Vec<i32>) {
    // Targets for left, right, top and bottom edges.
    let (mut l, mut rr, mut t, mut b) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    if cfg.to_edges {
        for w in work_areas {
            l.push(w.left);
            rr.push(w.right);
            t.push(w.top);
            b.push(w.bottom);
        }
    }
    if cfg.to_fences {
        for target in others {
            let o = &target.rect;
            let gap = cfg.gap_to(target.border);
            // Only snap to fences that are close on the other axis.
            if overlaps_range(r.top, r.bottom, o.top, o.bottom, cfg.threshold) {
                l.push(o.right + gap);
                l.push(o.left);
                rr.push(o.left - gap);
                rr.push(o.right);
            }
            if overlaps_range(r.left, r.right, o.left, o.right, cfg.threshold) {
                t.push(o.bottom + gap);
                t.push(o.top);
                b.push(o.top - gap);
                b.push(o.bottom);
            }
        }
    }
    (l, rr, t, b)
}

/// Snaps a rectangle being moved. Size is preserved.
pub fn snap_move(r: Rect, others: &[Target], work_areas: &[Rect], cfg: &SnapConfig) -> Rect {
    let (l, rr, t, b) = candidates(&r, others, work_areas, cfg);
    let mut dx = None;
    for x in l {
        dx = best(dx, x - r.left, cfg.threshold);
    }
    for x in rr {
        dx = best(dx, x - r.right, cfg.threshold);
    }
    let mut dy = None;
    for y in t {
        dy = best(dy, y - r.top, cfg.threshold);
    }
    for y in b {
        dy = best(dy, y - r.bottom, cfg.threshold);
    }
    r.offset(dx.unwrap_or(0), dy.unwrap_or(0))
}

/// Snaps the moving edges of a rectangle being resized.
pub fn snap_resize(
    r: Rect,
    edges: Edges,
    others: &[Target],
    work_areas: &[Rect],
    cfg: &SnapConfig,
) -> Rect {
    let (l, rr, t, b) = candidates(&r, others, work_areas, cfg);
    let pick = |value: i32, targets: &[i32]| -> i32 {
        let mut d = None;
        for &x in targets {
            d = best(d, x - value, cfg.threshold);
        }
        value + d.unwrap_or(0)
    };
    let mut out = r;
    if edges.left {
        out.left = pick(r.left, &l);
    }
    if edges.right {
        out.right = pick(r.right, &rr);
    }
    if edges.top {
        out.top = pick(r.top, &t);
    }
    if edges.bottom {
        out.bottom = pick(r.bottom, &b);
    }
    out
}

/// Applies a resize drag to `start`, keeping a minimum size.
pub fn resize_rect(start: Rect, edges: Edges, dx: i32, dy: i32, min_w: i32, min_h: i32) -> Rect {
    let mut r = start;
    if edges.left {
        r.left = (start.left + dx).min(start.right - min_w);
    }
    if edges.right {
        r.right = (start.right + dx).max(start.left + min_w);
    }
    if edges.top {
        r.top = (start.top + dy).min(start.bottom - min_h);
    }
    if edges.bottom {
        r.bottom = (start.bottom + dy).max(start.top + min_h);
    }
    r
}

/// Gives `r` the width of `span` columns out of `count` equal columns
/// splitting `work`, with `gap` pixels between two columns (negative when
/// neighbours share a border). The rectangle moves horizontally to the
/// nearest column so that `count / span` such fences tile the work area
/// exactly; rounding leaves at most one pixel of difference between widths.
pub fn fit_columns(r: Rect, work: &Rect, span: i32, count: i32, gap: i32) -> Rect {
    let count = count.max(1);
    let span = span.clamp(1, count);
    let usable = work.width() - (count - 1) * gap;
    let left = |k: i32| work.left + mul_div(usable, k, count) + k * gap;
    let right = |k: i32| work.left + mul_div(usable, k + 1, count) + k * gap;
    let center = r.left + r.width() / 2;
    let first = (0..=count - span)
        .min_by_key(|&k| ((left(k) + right(k + span - 1)) / 2 - center).abs())
        .unwrap_or(0);
    Rect {
        left: left(first),
        right: right(first + span - 1),
        ..r
    }
}

/// Screen edge a fence is docked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockEdge {
    Left,
    Top,
    Right,
    Bottom,
}

/// Returns the work-area edge touched by the rectangle, if any.
pub fn docked_edge(r: &Rect, work: &Rect, tolerance: i32) -> Option<DockEdge> {
    if (r.top - work.top).abs() <= tolerance {
        Some(DockEdge::Top)
    } else if (r.bottom - work.bottom).abs() <= tolerance {
        Some(DockEdge::Bottom)
    } else if (r.left - work.left).abs() <= tolerance {
        Some(DockEdge::Left)
    } else if (r.right - work.right).abs() <= tolerance {
        Some(DockEdge::Right)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: SnapConfig = SnapConfig {
        to_edges: true,
        to_fences: true,
        threshold: 10,
        gap: 8,
        border: 1,
    };

    fn targets(rects: &[Rect], border: i32) -> Vec<Target> {
        rects.iter().map(|&rect| Target { rect, border }).collect()
    }

    #[test]
    fn snaps_to_work_area_edge() {
        let work = [Rect::new(0, 0, 1920, 1040)];
        let r = Rect::from_xywh(6, 300, 200, 100);
        assert_eq!(snap_move(r, &[], &work, &CFG).left, 0);
        let r = Rect::from_xywh(1715, 935, 200, 100);
        let s = snap_move(r, &[], &work, &CFG);
        assert_eq!((s.right, s.bottom), (1920, 1040));
    }

    #[test]
    fn snaps_next_to_fence_with_gap() {
        let other = targets(&[Rect::from_xywh(100, 100, 200, 200)], 1);
        let r = Rect::from_xywh(305, 120, 100, 100);
        let s = snap_move(r, &other, &[], &CFG);
        assert_eq!(s.left, 308);
    }

    #[test]
    fn zero_gap_shares_the_border() {
        let flush = SnapConfig { gap: 0, ..CFG };
        let r = Rect::from_xywh(305, 120, 100, 100);
        // Both bordered: one line between them.
        let other = targets(&[Rect::from_xywh(100, 100, 200, 200)], 1);
        assert_eq!(snap_move(r, &other, &[], &flush).left, 299);
        // Other one without border: edges simply touch.
        let other = targets(&[Rect::from_xywh(100, 100, 200, 200)], 0);
        assert_eq!(snap_move(r, &other, &[], &flush).left, 300);
        // Moving one without border.
        let plain = SnapConfig { border: 0, ..flush };
        let other = targets(&[Rect::from_xywh(100, 100, 200, 200)], 1);
        assert_eq!(snap_move(r, &other, &[], &plain).left, 300);
        // Screen edges are never overlapped.
        let work = [Rect::new(0, 0, 1920, 1040)];
        assert_eq!(
            snap_move(Rect::from_xywh(4, 300, 100, 100), &[], &work, &flush).left,
            0
        );
    }

    #[test]
    fn ignores_far_targets() {
        let other = targets(&[Rect::from_xywh(100, 100, 200, 200)], 1);
        let r = Rect::from_xywh(400, 120, 100, 100);
        assert_eq!(snap_move(r, &other, &[], &CFG), r);
        let disabled = SnapConfig {
            to_edges: false,
            to_fences: false,
            ..CFG
        };
        let r = Rect::from_xywh(305, 120, 100, 100);
        assert_eq!(snap_move(r, &other, &[], &disabled), r);
    }

    #[test]
    fn resize_respects_minimum() {
        let start = Rect::from_xywh(0, 0, 200, 200);
        let edges = Edges {
            right: true,
            bottom: true,
            ..Edges::NONE
        };
        let r = resize_rect(start, edges, -500, 50, 100, 60);
        assert_eq!(r, Rect::new(0, 0, 100, 250));
        let left = Edges {
            left: true,
            ..Edges::NONE
        };
        assert_eq!(resize_rect(start, left, 150, 0, 100, 60).left, 100);
    }

    #[test]
    fn resize_snaps_only_moving_edges() {
        let work = [Rect::new(0, 0, 1000, 1000)];
        let r = Rect::new(5, 5, 995, 500);
        let edges = Edges {
            right: true,
            ..Edges::NONE
        };
        let s = snap_resize(r, edges, &[], &work, &CFG);
        assert_eq!(s, Rect::new(5, 5, 1000, 500));
    }

    #[test]
    fn dock_detection() {
        let work = Rect::new(0, 0, 1920, 1040);
        assert_eq!(
            docked_edge(&Rect::from_xywh(100, 0, 50, 50), &work, 2),
            Some(DockEdge::Top)
        );
        assert_eq!(
            docked_edge(&Rect::from_xywh(1870, 500, 50, 50), &work, 2),
            Some(DockEdge::Right)
        );
        assert_eq!(
            docked_edge(&Rect::from_xywh(100, 100, 50, 50), &work, 2),
            None
        );
    }

    #[test]
    fn three_thirds_tile_the_work_area() {
        // 2560 px does not divide by 3: widths differ by one pixel at most.
        let work = Rect::new(0, 40, 2560, 1440);
        let r = Rect::from_xywh(10, 100, 300, 400);
        let a = fit_columns(r, &work, 1, 3, 8);
        assert_eq!((a.left, a.top, a.bottom), (0, 100, 500));
        let b = fit_columns(a.offset(a.width(), 0), &work, 1, 3, 8);
        let c = fit_columns(Rect::from_xywh(2400, 100, 100, 400), &work, 1, 3, 8);
        assert_eq!(b.left, a.right + 8);
        assert_eq!(c.left, b.right + 8);
        assert_eq!(c.right, work.right);
        assert!((a.width() - c.width()).abs() <= 1);
    }

    #[test]
    fn shared_border_and_spans() {
        let work = Rect::new(0, 0, 1920, 1040);
        // Gap 0 with a 1 px border: neighbours overlap by that border.
        let a = fit_columns(Rect::from_xywh(0, 0, 50, 50), &work, 1, 2, -1);
        let b = fit_columns(Rect::from_xywh(1800, 0, 50, 50), &work, 1, 2, -1);
        assert_eq!(b.left, a.right - 1);
        assert_eq!(b.right, 1920);
        // Two thirds starting at the nearest column, full width.
        let t = fit_columns(Rect::from_xywh(1500, 0, 50, 50), &work, 2, 3, 0);
        assert_eq!((t.left, t.right), (640, 1920));
        let f = fit_columns(Rect::from_xywh(700, 0, 50, 50), &work, 1, 1, 8);
        assert_eq!((f.left, f.right), (0, 1920));
    }
}
