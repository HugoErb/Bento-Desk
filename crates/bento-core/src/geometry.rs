//! Integer geometry used everywhere in Bento Desk.
//!
//! Screen coordinates are physical pixels in the virtual-screen space, as
//! returned by Win32 APIs for a Per-Monitor V2 aware process. Persisted fence
//! geometry uses device independent pixels (DIP, 96 DPI = 1.0) relative to the
//! top-left corner of a monitor, see [`crate::model::FenceGeometry`].

use serde::{Deserialize, Serialize};

/// A point in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    /// Squared euclidean distance, avoids floating point.
    pub fn distance_sq(self, other: Point) -> i64 {
        let dx = i64::from(self.x) - i64::from(other.x);
        let dy = i64::from(self.y) - i64::from(other.y);
        dx * dx + dy * dy
    }
}

/// A size in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl Size {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

/// A rectangle, `right` and `bottom` are exclusive (Win32 convention).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn from_xywh(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self::new(x, y, x + width, y + height)
    }

    /// Builds the normalized rectangle spanning two arbitrary corners.
    pub fn from_points(a: Point, b: Point) -> Self {
        Self::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    pub const fn origin(&self) -> Point {
        Point::new(self.left, self.top)
    }

    pub const fn center(&self) -> Point {
        Point::new(self.left + self.width() / 2, self.top + self.height() / 2)
    }

    pub const fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    pub const fn contains_rect(&self, other: &Rect) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    pub const fn intersects(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let r = Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!r.is_empty()).then_some(r)
    }

    /// Area of the intersection, 0 when disjoint.
    pub fn overlap_area(&self, other: &Rect) -> i64 {
        self.intersection(other)
            .map(|r| i64::from(r.width()) * i64::from(r.height()))
            .unwrap_or(0)
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// Grows (positive) or shrinks (negative) every side.
    pub const fn inflate(&self, d: i32) -> Rect {
        Rect::new(self.left - d, self.top - d, self.right + d, self.bottom + d)
    }

    pub const fn with_origin(&self, p: Point) -> Rect {
        Rect::from_xywh(p.x, p.y, self.width(), self.height())
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// Moves the rectangle so that it lies inside `bounds` when possible,
    /// keeping its size (it is shrunk only when larger than `bounds`).
    pub fn clamp_inside(&self, bounds: &Rect) -> Rect {
        let w = self.width().min(bounds.width());
        let h = self.height().min(bounds.height());
        let x = self.left.clamp(bounds.left, bounds.right - w);
        let y = self.top.clamp(bounds.top, bounds.bottom - h);
        Rect::from_xywh(x, y, w, h)
    }

    /// Scales every coordinate by `num / den`, rounding to nearest.
    pub fn scale(&self, num: i32, den: i32) -> Rect {
        Rect::new(
            mul_div(self.left, num, den),
            mul_div(self.top, num, den),
            mul_div(self.right, num, den),
            mul_div(self.bottom, num, den),
        )
    }
}

/// `value * num / den` rounded to nearest, computed in 64 bits.
pub fn mul_div(value: i32, num: i32, den: i32) -> i32 {
    if den == 0 {
        return value;
    }
    let v = i64::from(value) * i64::from(num);
    let d = i64::from(den);
    let half = d.abs() / 2;
    let r = if (v >= 0) == (d > 0) {
        (v.abs() + half) / d.abs()
    } else {
        -((v.abs() + half) / d.abs())
    };
    i32::try_from(r).unwrap_or(if r < 0 { i32::MIN } else { i32::MAX })
}

/// Converts DIP to physical pixels for a monitor DPI (96 = 100 %).
pub fn dip_to_px(dip: i32, dpi: u32) -> i32 {
    mul_div(dip, i32::try_from(dpi).unwrap_or(96), 96)
}

/// Converts physical pixels to DIP for a monitor DPI (96 = 100 %).
pub fn px_to_dip(px: i32, dpi: u32) -> i32 {
    mul_div(px, 96, i32::try_from(dpi).unwrap_or(96))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_basics() {
        let r = Rect::from_xywh(10, 20, 100, 50);
        assert_eq!(r.width(), 100);
        assert_eq!(r.height(), 50);
        assert!(r.contains(Point::new(10, 20)));
        assert!(!r.contains(Point::new(110, 20)));
        assert_eq!(r.center(), Point::new(60, 45));
    }

    #[test]
    fn intersection_and_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 15, 15);
        assert_eq!(a.intersection(&b), Some(Rect::new(5, 5, 10, 10)));
        assert_eq!(a.overlap_area(&b), 25);
        assert_eq!(a.intersection(&Rect::new(10, 0, 20, 10)), None);
    }

    #[test]
    fn clamp_inside_moves_and_shrinks() {
        let bounds = Rect::new(0, 0, 100, 100);
        let r = Rect::from_xywh(90, -10, 30, 30);
        assert_eq!(r.clamp_inside(&bounds), Rect::from_xywh(70, 0, 30, 30));
        let big = Rect::from_xywh(-5, -5, 200, 50);
        assert_eq!(big.clamp_inside(&bounds), Rect::from_xywh(0, 0, 100, 50));
    }

    #[test]
    fn dpi_conversions_round_trip() {
        assert_eq!(dip_to_px(100, 144), 150);
        assert_eq!(px_to_dip(150, 144), 100);
        assert_eq!(dip_to_px(-100, 192), -200);
        assert_eq!(dip_to_px(33, 384), 132);
    }

    #[test]
    fn from_points_normalizes() {
        let r = Rect::from_points(Point::new(50, 10), Point::new(10, 40));
        assert_eq!(r, Rect::new(10, 10, 50, 40));
    }

    #[test]
    fn mul_div_rounds_to_nearest() {
        assert_eq!(mul_div(5, 1, 2), 3);
        assert_eq!(mul_div(-5, 1, 2), -3);
        assert_eq!(mul_div(7, 3, 0), 7);
    }
}
