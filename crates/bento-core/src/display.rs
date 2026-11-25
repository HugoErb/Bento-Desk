//! Monitors, display configuration keys and conversion of fence geometry.

use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect, dip_to_px, px_to_dip};
use crate::model::{FenceGeometry, Layout};

/// One monitor as seen by the app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorInfo {
    /// Stable identity: monitor device path when available, otherwise the
    /// GDI device name.
    pub id: String,
    /// Friendly name for the UI.
    pub name: String,
    /// Full monitor rectangle, physical pixels.
    pub rect: Rect,
    /// Work area (without taskbars), physical pixels.
    pub work: Rect,
    pub dpi: u32,
    pub primary: bool,
}

/// Key identifying a display configuration: monitor identities with their
/// resolution, position and DPI. Order independent.
pub fn display_key(monitors: &[MonitorInfo]) -> String {
    let mut parts: Vec<String> = monitors
        .iter()
        .map(|m| {
            format!(
                "{}@{}x{}+{}+{}@{}",
                m.id,
                m.rect.width(),
                m.rect.height(),
                m.rect.left,
                m.rect.top,
                m.dpi
            )
        })
        .collect();
    parts.sort();
    parts.join("|")
}

/// Short human readable description, for example `2 écrans : 2560x1440, 1920x1080`.
pub fn describe(monitors: &[MonitorInfo]) -> String {
    let sizes: Vec<String> = monitors
        .iter()
        .map(|m| format!("{}x{}", m.rect.width(), m.rect.height()))
        .collect();
    format!("{} : {}", monitors.len(), sizes.join(", "))
}

/// Monitor with the given id, or the primary, or the first.
pub fn find_monitor<'a>(monitors: &'a [MonitorInfo], id: &str) -> Option<&'a MonitorInfo> {
    monitors
        .iter()
        .find(|m| m.id == id)
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

/// Monitor that shows most of `r`, falling back to the nearest one.
pub fn monitor_for_rect<'a>(monitors: &'a [MonitorInfo], r: &Rect) -> Option<&'a MonitorInfo> {
    let best = monitors
        .iter()
        .max_by_key(|m| m.rect.overlap_area(r))
        .filter(|m| m.rect.overlap_area(r) > 0);
    best.or_else(|| {
        let c = r.center();
        monitors
            .iter()
            .min_by_key(|m| m.rect.center().distance_sq(c))
    })
}

/// Physical rectangle of a fence, kept visible on its monitor.
pub fn geometry_to_px(g: &FenceGeometry, monitors: &[MonitorInfo]) -> Option<Rect> {
    let m = find_monitor(monitors, &g.monitor)?;
    let r = Rect::new(
        m.rect.left + dip_to_px(g.rect.left, m.dpi),
        m.rect.top + dip_to_px(g.rect.top, m.dpi),
        m.rect.left + dip_to_px(g.rect.right, m.dpi),
        m.rect.top + dip_to_px(g.rect.bottom, m.dpi),
    );
    Some(r.clamp_inside(&m.work))
}

/// Geometry of a physical rectangle, relative to the monitor showing it.
pub fn px_to_geometry(r: &Rect, monitors: &[MonitorInfo]) -> Option<FenceGeometry> {
    let m = monitor_for_rect(monitors, r)?;
    Some(FenceGeometry {
        monitor: m.id.clone(),
        rect: Rect::new(
            px_to_dip(r.left - m.rect.left, m.dpi),
            px_to_dip(r.top - m.rect.top, m.dpi),
            px_to_dip(r.right - m.rect.left, m.dpi),
            px_to_dip(r.bottom - m.rect.top, m.dpi),
        ),
    })
}

/// Bounding rectangle of all monitors.
pub fn virtual_screen(monitors: &[MonitorInfo]) -> Rect {
    monitors
        .iter()
        .fold(Rect::default(), |acc, m| acc.union(&m.rect))
}

/// Monitor under a point.
pub fn monitor_at(monitors: &[MonitorInfo], p: Point) -> Option<&MonitorInfo> {
    monitors.iter().find(|m| m.rect.contains(p))
}

/// Swaps the content of two monitors in a layout ("Échanger le contenu des
/// écrans"). Fences keep their relative position, scaled to the new size.
pub fn swap_monitors(layout: &mut Layout, a: &MonitorInfo, b: &MonitorInfo) {
    let remap = |g: &mut FenceGeometry, from: &MonitorInfo, to: &MonitorInfo| {
        let fw = px_to_dip(from.rect.width(), from.dpi).max(1);
        let fh = px_to_dip(from.rect.height(), from.dpi).max(1);
        let tw = px_to_dip(to.rect.width(), to.dpi).max(1);
        let th = px_to_dip(to.rect.height(), to.dpi).max(1);
        let x = crate::geometry::mul_div(g.rect.left, tw, fw);
        let y = crate::geometry::mul_div(g.rect.top, th, fh);
        g.rect = Rect::from_xywh(x, y, g.rect.width(), g.rect.height());
        g.monitor = to.id.clone();
    };
    for page in &mut layout.pages {
        for fence in &mut page.fences {
            let g = &mut fence.geometry;
            if g.monitor == a.id {
                remap(g, a, b);
            } else if g.monitor == b.id {
                remap(g, b, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fence, Page};

    fn mon(id: &str, rect: Rect, dpi: u32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            id: id.into(),
            name: id.into(),
            rect,
            work: rect,
            dpi,
            primary,
        }
    }

    fn two() -> Vec<MonitorInfo> {
        vec![
            mon("A", Rect::new(0, 0, 1920, 1080), 96, true),
            mon("B", Rect::new(1920, 0, 1920 + 2560, 1440), 144, false),
        ]
    }

    #[test]
    fn key_is_order_independent() {
        let mut m = two();
        let k1 = display_key(&m);
        m.reverse();
        assert_eq!(k1, display_key(&m));
        m[0].dpi = 192;
        assert_ne!(k1, display_key(&m));
    }

    #[test]
    fn geometry_round_trip_with_dpi() {
        let m = two();
        let px = Rect::from_xywh(2000, 100, 300, 150);
        let g = px_to_geometry(&px, &m).unwrap();
        assert_eq!(g.monitor, "B");
        assert_eq!(g.rect, Rect::from_xywh(53, 67, 200, 100));
        let back = geometry_to_px(&g, &m).unwrap();
        assert!((back.left - px.left).abs() <= 1);
        assert_eq!(back.width(), 300);
    }

    #[test]
    fn missing_monitor_falls_back_to_primary() {
        let m = two();
        let g = FenceGeometry {
            monitor: "gone".into(),
            rect: Rect::from_xywh(10, 10, 100, 100),
        };
        let r = geometry_to_px(&g, &m).unwrap();
        assert!(m[0].rect.contains_rect(&r));
    }

    #[test]
    fn off_screen_rect_is_clamped() {
        let m = two();
        let g = FenceGeometry {
            monitor: "A".into(),
            rect: Rect::from_xywh(1900, 1000, 300, 300),
        };
        let r = geometry_to_px(&g, &m).unwrap();
        assert!(m[0].work.contains_rect(&r));
    }

    #[test]
    fn swap_moves_fences() {
        let m = two();
        let mut l = Layout {
            pages: vec![Page {
                fences: vec![Fence {
                    geometry: FenceGeometry {
                        monitor: "A".into(),
                        rect: Rect::from_xywh(960, 540, 100, 100),
                    },
                    ..Fence::default()
                }],
                ..Page::default()
            }],
            ..Layout::default()
        };
        swap_monitors(&mut l, &m[0], &m[1]);
        let g = &l.pages[0].fences[0].geometry;
        assert_eq!(g.monitor, "B");
        assert_eq!(g.rect.width(), 100);
        assert_eq!(virtual_screen(&m), Rect::new(0, 0, 4480, 1440));
    }
}
