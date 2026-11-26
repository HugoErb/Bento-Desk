//! Fluent style popup menus. Submenus expand in place (accordion) so a
//! menu is always a single window.

use bento_core::color::Rgba;
use bento_core::geometry::{Point, Rect, dip_to_px};
use bento_win::gfx::{Align, Canvas, Gfx, RectF, TextStyle};

use crate::icons;
use crate::theme::{ICON_FONT, Theme, UI_FONT, scale};

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Action,
    Check(bool),
    Radio(bool),
    Separator,
    Header,
    Submenu(Vec<MenuItem>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    pub kind: ItemKind,
    pub enabled: bool,
    pub danger: bool,
}

impl MenuItem {
    pub fn action(id: u32, label: impl Into<String>) -> MenuItem {
        MenuItem {
            id,
            label: label.into(),
            kind: ItemKind::Action,
            enabled: true,
            danger: false,
        }
    }

    pub fn check(id: u32, label: impl Into<String>, on: bool) -> MenuItem {
        MenuItem {
            kind: ItemKind::Check(on),
            ..MenuItem::action(id, label)
        }
    }

    pub fn radio(id: u32, label: impl Into<String>, on: bool) -> MenuItem {
        MenuItem {
            kind: ItemKind::Radio(on),
            ..MenuItem::action(id, label)
        }
    }

    pub fn separator() -> MenuItem {
        MenuItem {
            kind: ItemKind::Separator,
            ..MenuItem::action(0, "")
        }
    }

    pub fn header(label: impl Into<String>) -> MenuItem {
        MenuItem {
            kind: ItemKind::Header,
            enabled: false,
            ..MenuItem::action(0, label)
        }
    }

    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> MenuItem {
        MenuItem {
            kind: ItemKind::Submenu(items),
            ..MenuItem::action(0, label)
        }
    }

    pub fn enabled(mut self, on: bool) -> MenuItem {
        self.enabled = on;
        self
    }

    pub fn danger(mut self) -> MenuItem {
        self.danger = true;
        self
    }

    fn selectable(&self) -> bool {
        self.enabled && !matches!(self.kind, ItemKind::Separator | ItemKind::Header)
    }
}

/// A laid out row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: Vec<usize>,
    pub depth: u32,
    pub rect: Rect,
}

/// Menu content plus interaction state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub expanded: Vec<Vec<usize>>,
    pub hover: Option<usize>,
}

/// What a click on a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Run the command and close.
    Invoke(u32),
    /// Submenu toggled, redraw.
    Toggled,
    None,
}

fn metrics(dpi: u32) -> (i32, i32, i32, i32) {
    // Row height, separator height, outer padding, indent per level.
    (
        dip_to_px(32, dpi),
        dip_to_px(9, dpi),
        dip_to_px(4, dpi),
        dip_to_px(14, dpi),
    )
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Menu {
        Menu {
            items,
            ..Menu::default()
        }
    }

    pub fn item(&self, path: &[usize]) -> Option<&MenuItem> {
        let (first, rest) = path.split_first()?;
        let mut it = self.items.get(*first)?;
        for i in rest {
            match &it.kind {
                ItemKind::Submenu(children) => it = children.get(*i)?,
                _ => return None,
            }
        }
        Some(it)
    }

    fn is_expanded(&self, path: &[usize]) -> bool {
        self.expanded.iter().any(|p| p == path)
    }

    fn flatten(
        &self,
        items: &[MenuItem],
        prefix: &mut Vec<usize>,
        depth: u32,
        out: &mut Vec<(Vec<usize>, u32)>,
    ) {
        for (i, it) in items.iter().enumerate() {
            prefix.push(i);
            out.push((prefix.clone(), depth));
            if let ItemKind::Submenu(children) = &it.kind
                && self.is_expanded(prefix)
            {
                self.flatten(children, prefix, depth + 1, out);
            }
            prefix.pop();
        }
    }

    /// Rows with their rectangles for a menu of `width` pixels.
    pub fn rows(&self, dpi: u32, width: i32) -> Vec<Row> {
        let (rh, sh, pad, _) = metrics(dpi);
        let mut flat = Vec::new();
        self.flatten(&self.items, &mut Vec::new(), 0, &mut flat);
        let mut y = pad;
        flat.into_iter()
            .map(|(path, depth)| {
                let h = match self.item(&path).map(|i| &i.kind) {
                    Some(ItemKind::Separator) => sh,
                    Some(ItemKind::Header) => rh * 3 / 4,
                    _ => rh,
                };
                let r = Rect::new(pad, y, width - pad, y + h);
                y += h;
                Row {
                    path,
                    depth,
                    rect: r,
                }
            })
            .collect()
    }

    /// Size of the menu: width fits every label, submenus included (stable
    /// while expanding), height fits the rows currently shown.
    pub fn size(&self, gfx: &Gfx, dpi: u32) -> (i32, i32) {
        let (w, _) = self.full_size(gfx, dpi);
        (w, self.height(dpi))
    }

    /// Height of the rows currently shown, padding included.
    pub fn height(&self, dpi: u32) -> i32 {
        let (_, _, pad, _) = metrics(dpi);
        self.rows(dpi, 0).last().map_or(0, |r| r.rect.bottom) + pad
    }

    fn full_size(&self, gfx: &Gfx, dpi: u32) -> (i32, i32) {
        let (rh, sh, pad, indent) = metrics(dpi);
        let s = scale(dpi);
        let st = TextStyle::new(UI_FONT, 14.0 * s, Rgba::BLACK);
        fn walk(
            items: &[MenuItem],
            depth: i32,
            gfx: &Gfx,
            st: &TextStyle,
            acc: &mut (f32, i32),
            m: (i32, i32, i32),
        ) {
            for it in items {
                let (w, _) = gfx.measure(&it.label, st, 2000.0);
                acc.0 = acc.0.max(w + (depth * m.2) as f32);
                acc.1 += match it.kind {
                    ItemKind::Separator => m.1,
                    ItemKind::Header => m.0 * 3 / 4,
                    _ => m.0,
                };
                if let ItemKind::Submenu(c) = &it.kind {
                    walk(c, depth + 1, gfx, st, acc, m);
                }
            }
        }
        let mut acc = (0.0f32, 0i32);
        walk(&self.items, 0, gfx, &st, &mut acc, (rh, sh, indent));
        let w = acc.0 as i32 + dip_to_px(84, dpi);
        (
            w.clamp(dip_to_px(200, dpi), dip_to_px(460, dpi)),
            acc.1 + 2 * pad,
        )
    }

    pub fn row_at(rows: &[Row], p: Point) -> Option<usize> {
        rows.iter().position(|r| r.rect.contains(p))
    }

    /// Activates a row (click or Enter).
    pub fn activate(&mut self, rows: &[Row], row: usize) -> MenuAction {
        let Some(r) = rows.get(row) else {
            return MenuAction::None;
        };
        let Some(item) = self.item(&r.path) else {
            return MenuAction::None;
        };
        if !item.selectable() {
            return MenuAction::None;
        }
        match item.kind {
            ItemKind::Submenu(_) => {
                let path = r.path.clone();
                if let Some(i) = self.expanded.iter().position(|p| *p == path) {
                    self.expanded.remove(i);
                    // Collapse children too.
                    self.expanded.retain(|p| !p.starts_with(&path));
                } else {
                    // Keep a single open branch per level.
                    let parent = &path[..path.len() - 1];
                    self.expanded
                        .retain(|p| !(p.len() >= path.len() && p.starts_with(parent)));
                    self.expanded.push(path);
                }
                MenuAction::Toggled
            }
            _ => MenuAction::Invoke(item.id),
        }
    }

    /// Next selectable row in a direction (keyboard navigation).
    pub fn step(&self, rows: &[Row], from: Option<usize>, down: bool) -> Option<usize> {
        let n = rows.len();
        if n == 0 {
            return None;
        }
        let mut i = from.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if self.item(&rows[i].path).is_some_and(MenuItem::selectable) {
                return Some(i);
            }
        }
        None
    }
}

/// Draws a menu filling the whole surface.
pub fn draw_menu(
    c: &mut Canvas<'_>,
    m: &Menu,
    rows: &[Row],
    theme: &Theme,
    dpi: u32,
    w: i32,
    h: i32,
) {
    let s = scale(dpi);
    let (_, _, _, indent) = metrics(dpi);
    let bg = RectF::xywh(0.0, 0.0, w as f32, h as f32).inset(0.5);
    c.fill_round_rect(bg, 8.0 * s, theme.surface);
    c.stroke_round_rect(bg, 8.0 * s, theme.surface_border, 1.0);
    for (i, row) in rows.iter().enumerate() {
        let Some(item) = m.item(&row.path) else {
            continue;
        };
        let r = RectF::from(row.rect);
        let x0 = r.left + (row.depth as i32 * indent) as f32;
        match &item.kind {
            ItemKind::Separator => {
                let y = (r.top + r.bottom) / 2.0;
                c.line(x0 + 8.0 * s, y, r.right - 8.0 * s, y, theme.separator, 1.0);
                continue;
            }
            ItemKind::Header => {
                let mut st = TextStyle::new(UI_FONT, 12.0 * s, theme.text_secondary);
                st.bold = true;
                c.text(
                    &item.label,
                    RectF::new(x0 + 12.0 * s, r.top, r.right, r.bottom),
                    &st,
                );
                continue;
            }
            _ => {}
        }
        if m.hover == Some(i) && item.enabled {
            c.fill_round_rect(r.inset(1.0), 4.0 * s, theme.hover);
            c.fill_round_rect(
                RectF::new(
                    r.left + 1.0,
                    r.top + r.height() * 0.3,
                    r.left + 4.0 * s,
                    r.bottom - r.height() * 0.3,
                ),
                1.5 * s,
                theme.accent,
            );
        }
        let fg = if !item.enabled {
            theme.text_secondary.with_alpha(0x60)
        } else if item.danger {
            theme.danger
        } else {
            theme.text
        };
        let mut gl = TextStyle::new(ICON_FONT, 12.0 * s, fg);
        gl.align = Align::Center;
        let glyph_rect = RectF::new(x0 + 6.0 * s, r.top, x0 + 30.0 * s, r.bottom);
        match item.kind {
            ItemKind::Check(true) => c.text(icons::CHECK, glyph_rect, &gl),
            ItemKind::Radio(true) => {
                c.fill_circle(
                    (glyph_rect.left + glyph_rect.right) / 2.0,
                    (r.top + r.bottom) / 2.0,
                    3.5 * s,
                    fg,
                );
            }
            _ => {}
        }
        let st = TextStyle::new(UI_FONT, 14.0 * s, fg);
        c.text(
            &item.label,
            RectF::new(x0 + 34.0 * s, r.top, r.right - 30.0 * s, r.bottom),
            &st,
        );
        if let ItemKind::Submenu(_) = item.kind {
            let open = m.expanded.contains(&row.path);
            let g = if open {
                icons::CHEVRON_DOWN
            } else {
                icons::CHEVRON_RIGHT
            };
            let mut cs = TextStyle::new(ICON_FONT, 10.0 * s, theme.text_secondary);
            cs.align = Align::Center;
            c.text(
                g,
                RectF::new(r.right - 30.0 * s, r.top, r.right - 6.0 * s, r.bottom),
                &cs,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Menu {
        Menu::new(vec![
            MenuItem::action(1, "A"),
            MenuItem::separator(),
            MenuItem::submenu(
                "Sub",
                vec![
                    MenuItem::radio(2, "x", true),
                    MenuItem::radio(3, "y", false),
                ],
            ),
            MenuItem::action(4, "Disabled").enabled(false),
        ])
    }

    #[test]
    fn expand_and_invoke() {
        let mut m = sample();
        let rows = m.rows(96, 200);
        assert_eq!(rows.len(), 4);
        assert_eq!(m.activate(&rows, 0), MenuAction::Invoke(1));
        assert_eq!(m.activate(&rows, 1), MenuAction::None);
        assert_eq!(m.activate(&rows, 2), MenuAction::Toggled);
        let rows = m.rows(96, 200);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3].depth, 1);
        assert_eq!(m.activate(&rows, 4), MenuAction::Invoke(3));
        assert_eq!(m.activate(&rows, 2), MenuAction::Toggled);
        assert_eq!(m.rows(96, 200).len(), 4);
    }

    #[test]
    fn height_follows_expanded_rows() {
        let mut m = sample();
        let collapsed = m.height(96);
        // 3 rows of 32 + separator 9 + padding 4 on both sides.
        assert_eq!(collapsed, 3 * 32 + 9 + 2 * 4);
        let rows = m.rows(96, 200);
        m.activate(&rows, 2);
        assert_eq!(m.height(96), collapsed + 2 * 32);
    }

    #[test]
    fn keyboard_skips_unselectable() {
        let m = sample();
        let rows = m.rows(96, 200);
        assert_eq!(m.step(&rows, Some(0), true), Some(2));
        assert_eq!(m.step(&rows, Some(2), true), Some(0));
        assert_eq!(m.step(&rows, None, true), Some(0));
    }

    #[test]
    fn row_hit() {
        let m = sample();
        let rows = m.rows(96, 200);
        let p = rows[2].rect.center();
        assert_eq!(Menu::row_at(&rows, p), Some(2));
        assert_eq!(Menu::row_at(&rows, Point::new(-5, -5)), None);
    }
}
