//! Placement of the real desktop icons from the fence model, and
//! interpretation of the moves made by the user. Pure logic, unit tested.

use std::collections::{BTreeMap, HashSet};

use bento_core::geometry::{Point, Rect, Size};
use bento_core::grid::{Grid, fence_cell, icon_offset, park_slot, place};
use bento_core::items::ItemInfo;
use bento_core::model::{FenceId, Layout};
use bento_core::rules::{Rule, first_match};

/// A desktop icon as last read from Explorer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeskItem {
    pub info: ItemInfo,
    pub pos: Point,
}

/// Geometry of a fence on the current page, physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: FenceId,
    /// Rectangle as displayed (title only when rolled up).
    pub rect: Rect,
    /// Area for icons (empty when rolled up).
    pub body: Rect,
    pub rolled: bool,
    /// Hidden by quick-hide.
    pub hidden: bool,
    /// Active tab is a folder portal (no desktop icons shown).
    pub portal: bool,
    /// During a roll animation: icons reaching below this line are parked.
    pub clip_bottom: Option<i32>,
}

/// Inputs of a layout pass.
pub struct LayoutInput<'a> {
    pub layout: &'a Layout,
    pub placed: &'a [Placed],
    pub items: &'a BTreeMap<String, DeskItem>,
    pub desktop_spacing: Size,
    pub spacing_percent: u16,
    /// Size of the icon image, physical pixels.
    pub icon_size: i32,
    pub virtual_screen: Rect,
    /// Park free icons too (quick-hide without "keep free icons").
    pub park_free: bool,
}

/// Where every fenced (or parked) icon must be. Free icons are absent
/// unless `park_free` is set.
pub fn targets(inp: &LayoutInput<'_>) -> BTreeMap<String, Point> {
    let cell = fence_cell(inp.desktop_spacing, inp.spacing_percent);
    let shift = icon_offset(cell, inp.desktop_spacing, inp.icon_size);
    let mut out = BTreeMap::new();
    let mut parked = 0usize;
    let mut park = |key: &str, out: &mut BTreeMap<String, Point>| {
        out.insert(
            key.to_owned(),
            park_slot(&inp.virtual_screen, inp.desktop_spacing, parked),
        );
        parked += 1;
    };
    let mut seen: HashSet<&str> = HashSet::new();
    for page in &inp.layout.pages {
        for fence in &page.fences {
            let placed = inp.placed.iter().find(|p| p.id == fence.id);
            for (ti, tab) in fence.tabs.iter().enumerate() {
                let present: Vec<&String> = tab
                    .content
                    .desktop_items()
                    .iter()
                    .filter(|k| inp.items.contains_key(k.as_str()) && !seen.contains(k.as_str()))
                    .collect();
                for k in &present {
                    seen.insert(k.as_str());
                }
                let active = ti == fence.active_tab.min(fence.tabs.len().saturating_sub(1));
                match placed {
                    Some(p) if active && !p.hidden && !p.portal => {
                        let grid = Grid::new(p.body, cell);
                        let pl = place(&grid, present.len(), tab.scroll_row, p.rolled);
                        for (i, slot) in pl.visible {
                            let pos = slot.offset(shift.x, shift.y);
                            let cut = p
                                .clip_bottom
                                .is_some_and(|c| pos.y + inp.desktop_spacing.height > c);
                            if cut {
                                park(present[i], &mut out);
                            } else {
                                out.insert(present[i].clone(), pos);
                            }
                        }
                        for i in pl.hidden {
                            park(present[i], &mut out);
                        }
                    }
                    _ => {
                        for k in present {
                            park(k, &mut out);
                        }
                    }
                }
            }
        }
    }
    if inp.park_free {
        for key in inp.items.keys() {
            if !seen.contains(key.as_str()) {
                park(key, &mut out);
            }
        }
    }
    out
}

/// Moves needed to reach `targets` from the current positions.
pub fn moves(
    targets: &BTreeMap<String, Point>,
    items: &BTreeMap<String, DeskItem>,
) -> Vec<(String, Point)> {
    targets
        .iter()
        .filter(|(k, p)| items.get(k.as_str()).is_some_and(|it| it.pos != **p))
        .map(|(k, p)| (k.clone(), *p))
        .collect()
}

/// Items whose position differs from what Bento Desk expected: they were
/// moved by the user (drag and drop) or by Explorer.
pub fn moved_by_user(
    expected: &BTreeMap<String, Point>,
    items: &BTreeMap<String, DeskItem>,
) -> Vec<(String, Point)> {
    items
        .iter()
        .filter(|(k, it)| {
            expected
                .get(k.as_str())
                .is_some_and(|e| (e.x - it.pos.x).abs() > 2 || (e.y - it.pos.y).abs() > 2)
        })
        .map(|(k, it)| (k.clone(), it.pos))
        .collect()
}

/// Fence whose icon area contains the center of an icon cell at `pos`.
pub fn fence_at(placed: &[Placed], pos: Point, cell: Size) -> Option<FenceId> {
    let c = pos.offset(cell.width / 2, cell.height / 2);
    placed
        .iter()
        .find(|p| !p.rolled && !p.hidden && !p.portal && p.body.contains(c))
        .map(|p| p.id)
}

/// Applies a user move: the icon joins, leaves or moves inside a fence.
/// Returns true when the model changed.
pub fn apply_user_move(
    layout: &mut Layout,
    placed: &[Placed],
    key: &str,
    pos: Point,
    desktop_spacing: Size,
    spacing_percent: u16,
    icon_size: i32,
) -> bool {
    let cell = fence_cell(desktop_spacing, spacing_percent);
    let shift = icon_offset(cell, desktop_spacing, icon_size);
    let target = fence_at(placed, pos, desktop_spacing);
    let source = layout
        .pages
        .iter()
        .flat_map(|p| p.fences.iter())
        .find(|f| f.contains_item(key))
        .map(|f| f.id);
    match (source, target) {
        (None, None) => false,
        (Some(s), None) => {
            // Dragged out of a visible fence onto the free desktop. Ignore
            // moves of hidden icons (parked by us): a rolled-up, quick-hidden
            // or portal fence shows no icon the user could have dragged, so
            // the move comes from Explorer (stale position, grid snap).
            if placed
                .iter()
                .any(|p| p.id == s && !p.rolled && !p.hidden && !p.portal)
            {
                if let Some(f) = layout.fence_mut(s) {
                    f.remove_item(key);
                }
                true
            } else {
                false
            }
        }
        (src, Some(t)) => {
            let Some(p) = placed.iter().find(|p| p.id == t) else {
                return false;
            };
            if let Some(s) = src
                && let Some(f) = layout.fence_mut(s)
            {
                f.remove_item(key);
            }
            let Some(f) = layout.fence_mut(t) else {
                return false;
            };
            let Some(tab) = f.active_mut() else {
                return false;
            };
            let grid = Grid::new(p.body, cell);
            let scroll = tab.scroll_row;
            if let Some(items) = tab.content.desktop_items_mut() {
                let cell_pos = pos.offset(-shift.x, -shift.y);
                let idx = grid.index_at(cell_pos, scroll, items.len());
                items.insert(idx, key.to_owned());
            }
            true
        }
    }
}

/// False when a user move would add an icon to, remove it from or reorder
/// it inside a locked fence. The icon must then go back where it was.
pub fn move_allowed(
    layout: &Layout,
    placed: &[Placed],
    key: &str,
    pos: Point,
    desktop_spacing: Size,
) -> bool {
    let locked = |id: FenceId| layout.fence(id).is_some_and(|f| f.locked);
    let source = layout
        .pages
        .iter()
        .flat_map(|p| p.fences.iter())
        .find(|f| f.contains_item(key));
    if source.is_some_and(|f| f.locked) {
        return false;
    }
    !fence_at(placed, pos, desktop_spacing).is_some_and(locked)
}

/// Fence that should receive a new desktop item: first matching rule,
/// otherwise the fence under the place where Explorer put it. Locked
/// fences receive nothing.
pub fn fence_for_new_item(
    layout: &Layout,
    rules: &[Rule],
    auto_rules: bool,
    item: &DeskItem,
    placed: &[Placed],
    desktop_spacing: Size,
    now: u64,
) -> Option<FenceId> {
    let open = |id: FenceId| layout.fence(id).is_some_and(|f| !f.locked);
    if auto_rules
        && let Some(r) = first_match(rules, &item.info, now)
        && open(r.target)
    {
        return Some(r.target);
    }
    fence_at(placed, item.pos, desktop_spacing).filter(|id| open(*id))
}

/// Free grid slots on a work area, column by column from the top-left
/// corner like Explorer, avoiding occupied rectangles.
pub fn free_slots(work: Rect, cell: Size, occupied: &[Rect], n: usize) -> Vec<Point> {
    let mut out = Vec::new();
    if n == 0 || cell.width <= 0 || cell.height <= 0 {
        return out;
    }
    let mut taken: Vec<Rect> = occupied.to_vec();
    let mut x = work.left;
    while x + cell.width <= work.right {
        let mut y = work.top;
        while y + cell.height <= work.bottom {
            let r = Rect::from_xywh(x, y, cell.width, cell.height);
            if !taken.iter().any(|o| o.intersects(&r)) {
                out.push(Point::new(x, y));
                taken.push(r);
                if out.len() == n {
                    return out;
                }
            }
            y += cell.height;
        }
        x += cell.width;
    }
    out
}

/// Items inside a rectangle, in reading order (rows then columns).
pub fn items_in_rect(items: &BTreeMap<String, DeskItem>, r: &Rect, cell: Size) -> Vec<String> {
    let mut inside: Vec<(&String, Point)> = items
        .iter()
        .filter(|(_, it)| r.contains(it.pos.offset(cell.width / 2, cell.height / 2)))
        .map(|(k, it)| (k, it.pos))
        .collect();
    let row_h = cell.height.max(1);
    inside.sort_by_key(|(_, p)| ((p.y - r.top) / row_h, p.x));
    inside.into_iter().map(|(k, _)| k.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bento_core::model::{Fence, Page, Tab, TabContent, TabId};

    const CELL: Size = Size::new(80, 100);

    fn item(key: &str, x: i32, y: i32) -> (String, DeskItem) {
        (
            key.to_owned(),
            DeskItem {
                info: ItemInfo::from_key(key, false),
                pos: Point::new(x, y),
            },
        )
    }

    fn fence(id: u64, items: &[&str]) -> Fence {
        Fence {
            id: FenceId(id),
            tabs: vec![Tab {
                id: TabId(id * 10),
                content: TabContent::Desktop {
                    items: items.iter().map(|s| (*s).to_owned()).collect(),
                },
                ..Tab::default()
            }],
            ..Fence::default()
        }
    }

    fn placed(id: u64, body: Rect) -> Placed {
        Placed {
            id: FenceId(id),
            rect: body,
            body,
            rolled: false,
            hidden: false,
            portal: false,
            clip_bottom: None,
        }
    }

    fn layout(fences: Vec<Fence>) -> Layout {
        Layout {
            pages: vec![Page {
                fences,
                ..Page::default()
            }],
            ..Layout::default()
        }
    }

    fn vs() -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }

    #[test]
    fn targets_fill_grid_and_park_overflow() {
        let l = layout(vec![fence(1, &["a", "b", "c"])]);
        let items: BTreeMap<_, _> = [
            item("a", 0, 0),
            item("b", 0, 0),
            item("c", 0, 0),
            item("free", 5, 5),
        ]
        .into_iter()
        .collect();
        let pl = [placed(1, Rect::from_xywh(100, 100, 160, 100))];
        let inp = LayoutInput {
            layout: &l,
            placed: &pl,
            items: &items,
            desktop_spacing: CELL,
            spacing_percent: 100,
            icon_size: CELL.width,
            virtual_screen: vs(),
            park_free: false,
        };
        let t = targets(&inp);
        assert_eq!(t["a"], Point::new(100, 100));
        assert_eq!(t["b"], Point::new(180, 100));
        assert!(t["c"].x >= 1920, "overflow parked");
        assert!(!t.contains_key("free"));
        let m = moves(&t, &items);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn icons_are_centered_in_wide_cells() {
        let l = layout(vec![fence(1, &["a", "b"])]);
        let items: BTreeMap<_, _> = [item("a", 0, 0), item("b", 0, 0)].into_iter().collect();
        let pl = [placed(1, Rect::from_xywh(100, 100, 400, 200))];
        let spacing = Size::new(96, 101);
        let inp = LayoutInput {
            layout: &l,
            placed: &pl,
            items: &items,
            desktop_spacing: spacing,
            spacing_percent: 120,
            icon_size: 48,
            virtual_screen: vs(),
            park_free: false,
        };
        let t = targets(&inp);
        // 3 cells of 115 px in 400 px: 27 px of slack each side, then the
        // 48 px image centered in its cell (+33) and the extra height split.
        assert_eq!(t["a"], Point::new(100 + 27 + 33, 110));
        assert_eq!(t["b"], Point::new(100 + 27 + 115 + 33, 110));
        // A drop where Explorer reports the icon maps back to its cell.
        let mut l2 = l.clone();
        assert!(apply_user_move(&mut l2, &pl, "a", t["b"], spacing, 120, 48));
        assert_eq!(
            l2.fence(FenceId(1)).unwrap().tabs[0]
                .content
                .desktop_items(),
            ["b", "a"]
        );
    }

    #[test]
    fn hidden_and_other_pages_are_parked() {
        let mut l = layout(vec![fence(1, &["a"])]);
        l.pages.push(Page {
            fences: vec![fence(2, &["b"])],
            ..Page::default()
        });
        let items: BTreeMap<_, _> = [item("a", 0, 0), item("b", 0, 0), item("free", 1, 1)]
            .into_iter()
            .collect();
        let mut p = placed(1, Rect::from_xywh(0, 0, 400, 400));
        p.hidden = true;
        let pl = [p];
        let inp = LayoutInput {
            layout: &l,
            placed: &pl,
            items: &items,
            desktop_spacing: CELL,
            spacing_percent: 100,
            icon_size: CELL.width,
            virtual_screen: vs(),
            park_free: true,
        };
        let t = targets(&inp);
        assert!(t.values().all(|p| p.x >= 1920));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn locked_fence_keeps_its_icons() {
        let mut locked = fence(1, &["a"]);
        locked.locked = true;
        let l = layout(vec![locked, fence(2, &["b"])]);
        let pl = [
            placed(1, Rect::from_xywh(0, 0, 400, 200)),
            placed(2, Rect::from_xywh(500, 0, 400, 200)),
        ];
        // Out of the locked fence: refused.
        assert!(!move_allowed(&l, &pl, "a", Point::new(900, 900), CELL));
        assert!(!move_allowed(&l, &pl, "a", Point::new(510, 10), CELL));
        // Into the locked fence: refused, from a fence or from the desktop.
        assert!(!move_allowed(&l, &pl, "b", Point::new(10, 10), CELL));
        assert!(!move_allowed(&l, &pl, "free", Point::new(10, 10), CELL));
        // Elsewhere: allowed.
        assert!(move_allowed(&l, &pl, "b", Point::new(900, 900), CELL));
        assert!(move_allowed(&l, &pl, "free", Point::new(510, 10), CELL));
        // New items never land in the locked fence.
        let (_, it) = item("new", 10, 10);
        assert_eq!(fence_for_new_item(&l, &[], false, &it, &pl, CELL, 0), None);
    }

    #[test]
    fn user_moves_between_fences_and_desktop() {
        let mut l = layout(vec![fence(1, &["a", "b"]), fence(2, &[])]);
        let pl = [
            placed(1, Rect::from_xywh(0, 0, 400, 200)),
            placed(2, Rect::from_xywh(500, 0, 400, 200)),
        ];
        // a dropped into fence 2.
        assert!(apply_user_move(
            &mut l,
            &pl,
            "a",
            Point::new(510, 10),
            CELL,
            100,
            CELL.width
        ));
        assert!(l.fence(FenceId(2)).unwrap().contains_item("a"));
        assert!(!l.fence(FenceId(1)).unwrap().contains_item("a"));
        // b reordered inside fence 1? stays.
        assert!(apply_user_move(
            &mut l,
            &pl,
            "b",
            Point::new(90, 10),
            CELL,
            100,
            CELL.width
        ));
        assert!(l.fence(FenceId(1)).unwrap().contains_item("b"));
        // a dragged out to the desktop.
        assert!(apply_user_move(
            &mut l,
            &pl,
            "a",
            Point::new(1500, 600),
            CELL,
            100,
            CELL.width
        ));
        assert!(!l.fence(FenceId(2)).unwrap().contains_item("a"));
        // free icon moved on the desktop: nothing.
        assert!(!apply_user_move(
            &mut l,
            &pl,
            "z",
            Point::new(1500, 600),
            CELL,
            100,
            CELL.width
        ));
        // free icon dropped into fence 1 at the start.
        assert!(apply_user_move(
            &mut l,
            &pl,
            "z",
            Point::new(0, 0),
            CELL,
            100,
            CELL.width
        ));
        assert_eq!(
            l.fence(FenceId(1)).unwrap().tabs[0].content.desktop_items()[0],
            "z"
        );
    }

    #[test]
    fn rolled_or_hidden_fence_keeps_its_parked_icons() {
        let mut l = layout(vec![fence(1, &["a"]), fence(2, &["b"])]);
        let mut rolled = placed(1, Rect::from_xywh(0, 0, 400, 30));
        rolled.rolled = true;
        let mut hidden = placed(2, Rect::from_xywh(500, 0, 400, 200));
        hidden.hidden = true;
        let pl = [rolled, hidden];
        // Explorer reports the icons elsewhere (stale position or snap):
        // they stay in their fence.
        for (key, x) in [("a", 50), ("b", 3000)] {
            assert!(!apply_user_move(
                &mut l,
                &pl,
                key,
                Point::new(x, 600),
                CELL,
                100,
                CELL.width
            ));
        }
        assert!(l.fence(FenceId(1)).unwrap().contains_item("a"));
        assert!(l.fence(FenceId(2)).unwrap().contains_item("b"));
    }

    #[test]
    fn detect_user_moves() {
        let items: BTreeMap<_, _> = [item("a", 10, 10), item("b", 200, 200)]
            .into_iter()
            .collect();
        let mut expected = BTreeMap::new();
        expected.insert("a".to_owned(), Point::new(11, 10));
        expected.insert("b".to_owned(), Point::new(0, 0));
        assert_eq!(
            moved_by_user(&expected, &items),
            vec![("b".to_owned(), Point::new(200, 200))]
        );
    }

    #[test]
    fn free_slots_avoid_occupied() {
        let work = Rect::new(0, 0, 240, 300);
        let occ = [Rect::from_xywh(0, 0, 80, 100)];
        let s = free_slots(work, CELL, &occ, 3);
        assert_eq!(
            s,
            vec![Point::new(0, 100), Point::new(0, 200), Point::new(80, 0)]
        );
    }

    #[test]
    fn items_in_rect_reading_order() {
        let items: BTreeMap<_, _> = [
            item("c", 0, 100),
            item("b", 80, 0),
            item("a", 0, 0),
            item("out", 900, 900),
        ]
        .into_iter()
        .collect();
        let r = Rect::new(0, 0, 200, 220);
        assert_eq!(items_in_rect(&items, &r, CELL), vec!["a", "b", "c"]);
    }

    #[test]
    fn new_item_rules_then_position() {
        use bento_core::rules::{Condition, TextOp};
        let l = layout(vec![fence(1, &[]), fence(2, &[])]);
        let rules = vec![Rule {
            id: 1,
            name: "r".into(),
            enabled: true,
            match_all: true,
            conditions: vec![Condition::Name {
                op: TextOp::Contains,
                value: "game".into(),
            }],
            target: FenceId(2),
        }];
        let pl = [placed(1, Rect::from_xywh(0, 0, 400, 400))];
        let (_, game) = item("Game.lnk", 1500, 900);
        assert_eq!(
            fence_for_new_item(&l, &rules, true, &game, &pl, CELL, 0),
            Some(FenceId(2))
        );
        let (_, other) = item("doc.txt", 10, 10);
        assert_eq!(
            fence_for_new_item(&l, &rules, true, &other, &pl, CELL, 0),
            Some(FenceId(1))
        );
        assert_eq!(
            fence_for_new_item(&l, &rules, false, &game, &pl, CELL, 0),
            None
        );
    }
}
