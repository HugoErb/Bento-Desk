//! Persistent domain model: layouts, pages, fences, tabs and styles.
//!
//! Every struct uses `#[serde(default)]` so that files written by an older
//! version keep loading when fields are added. Breaking changes go through
//! the migrations of `bento-config`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::color::{Rgba, Tint};
use crate::geometry::{Point, Rect};

/// Identity of a desktop item: its absolute parsing name, for example
/// `C:\Users\me\Desktop\notes.txt` or `::{645FF040-5081-101B-9F08-00AA002F954E}`.
pub type ItemKey = String;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct FenceId(pub u64);

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct TabId(pub u64);

/// When the title bar is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleMode {
    Never,
    Hover,
    #[default]
    Always,
}

/// Horizontal position of the title in the title band.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// How the fence background is rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackdropEffect {
    /// Plain tinted color.
    Solid,
    /// Blurred wallpaper under the tint (acrylic look, simulated).
    #[default]
    Blur,
}

/// Visual style of a fence (or of one tab of a fence).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FenceStyle {
    pub tint: Tint,
    pub effect: BackdropEffect,
    /// Adapt the tint to the wallpaper under the fence.
    pub chameleon: bool,
    pub title_mode: TitleMode,
    pub title_font: String,
    /// Title font size in DIP.
    pub title_size: u8,
    pub title_bold: bool,
    pub title_align: TitleAlign,
    /// `None` = automatic contrast with the background.
    pub title_color: Option<Rgba>,
    /// Background of the title band. `None` = the fence color, slightly
    /// darkened.
    pub title_background: Option<Tint>,
    /// Corner radius in DIP.
    pub corner_radius: u8,
    pub border: bool,
}

impl Default for FenceStyle {
    fn default() -> Self {
        Self {
            tint: Tint::default(),
            effect: BackdropEffect::Blur,
            chameleon: false,
            title_mode: TitleMode::Always,
            title_font: "Segoe UI Variable Text".to_owned(),
            title_size: 13,
            title_bold: false,
            title_align: TitleAlign::Left,
            title_color: None,
            title_background: None,
            corner_radius: 8,
            border: true,
        }
    }
}

impl FenceStyle {
    /// Color painted over the fence background for the title band.
    pub fn title_band(&self) -> Rgba {
        self.title_background
            .map_or(Rgba::new(0, 0, 0, 0x26), Tint::to_rgba)
    }
}

/// Sort criterion inside a fence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    /// Order chosen by the user (drag and drop).
    #[default]
    Manual,
    Name,
    Type,
    Date,
    Size,
    Opens,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SortMode {
    pub key: SortKey,
    pub descending: bool,
}

/// Display mode of a folder portal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    Icons,
    List,
}

/// What a tab shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabContent {
    /// Real desktop icons, in display order.
    Desktop {
        #[serde(default)]
        items: Vec<ItemKey>,
    },
    /// Content of any folder, drawn by Bento Desk.
    Portal {
        root: String,
        #[serde(default)]
        current: String,
        #[serde(default)]
        view: ViewMode,
    },
}

impl Default for TabContent {
    fn default() -> Self {
        TabContent::Desktop { items: Vec::new() }
    }
}

impl TabContent {
    pub fn desktop_items(&self) -> &[ItemKey] {
        match self {
            TabContent::Desktop { items } => items,
            TabContent::Portal { .. } => &[],
        }
    }

    pub fn desktop_items_mut(&mut self) -> Option<&mut Vec<ItemKey>> {
        match self {
            TabContent::Desktop { items } => Some(items),
            TabContent::Portal { .. } => None,
        }
    }

    pub fn is_portal(&self) -> bool {
        matches!(self, TabContent::Portal { .. })
    }
}

/// One tab of a fence. A plain fence has exactly one tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub content: TabContent,
    /// `None` = use the global default style.
    pub style: Option<FenceStyle>,
    pub sort: SortMode,
    /// First visible row when the content overflows.
    pub scroll_row: u32,
}

/// Position of a fence: monitor identity and rectangle in DIP relative to
/// the monitor top-left corner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FenceGeometry {
    pub monitor: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fence {
    pub id: FenceId,
    pub geometry: FenceGeometry,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub rolled_up: bool,
    /// Roll up automatically when docked to a screen edge.
    pub auto_rollup: bool,
    /// Stays visible during quick-hide.
    pub quick_hide_exempt: bool,
    /// Cannot be moved or resized, and icons cannot be added or removed.
    pub locked: bool,
}

impl Fence {
    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab).or_else(|| self.tabs.first())
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        let idx = if self.active_tab < self.tabs.len() {
            self.active_tab
        } else {
            0
        };
        self.tabs.get_mut(idx)
    }

    pub fn title(&self) -> &str {
        self.active().map(|t| t.title.as_str()).unwrap_or_default()
    }

    /// True if any tab holds the item.
    pub fn contains_item(&self, key: &str) -> bool {
        self.tabs
            .iter()
            .any(|t| t.content.desktop_items().iter().any(|k| k == key))
    }

    /// All desktop items of all tabs.
    pub fn all_items(&self) -> impl Iterator<Item = &ItemKey> {
        self.tabs.iter().flat_map(|t| t.content.desktop_items())
    }

    /// Removes the item from every tab, returns true if it was present.
    pub fn remove_item(&mut self, key: &str) -> bool {
        let mut removed = false;
        for tab in &mut self.tabs {
            if let Some(items) = tab.content.desktop_items_mut() {
                let before = items.len();
                items.retain(|k| k != key);
                removed |= items.len() != before;
            }
        }
        removed
    }

    /// Renames an item key in every tab.
    pub fn rename_item(&mut self, old: &str, new: &str) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if let Some(items) = tab.content.desktop_items_mut() {
                for k in items.iter_mut() {
                    if k == old {
                        *k = new.to_owned();
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub name: String,
    pub fences: Vec<Fence>,
}

/// Everything that is shown for one display configuration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Key built by [`crate::display::display_key`].
    pub key: String,
    /// Human readable summary of the monitors.
    pub description: String,
    pub pages: Vec<Page>,
    pub current_page: usize,
}

impl Layout {
    pub fn page(&self) -> Option<&Page> {
        self.pages
            .get(self.current_page)
            .or_else(|| self.pages.first())
    }

    pub fn page_mut(&mut self) -> Option<&mut Page> {
        let idx = if self.current_page < self.pages.len() {
            self.current_page
        } else {
            0
        };
        self.pages.get_mut(idx)
    }

    /// Ensures at least one page exists and the current index is valid.
    pub fn normalize(&mut self) {
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        if self.current_page >= self.pages.len() {
            self.current_page = 0;
        }
        for page in &mut self.pages {
            for fence in &mut page.fences {
                if fence.tabs.is_empty() {
                    fence.tabs.push(Tab::default());
                }
                if fence.active_tab >= fence.tabs.len() {
                    fence.active_tab = 0;
                }
            }
        }
    }

    /// Fence lookup across all pages.
    pub fn fence(&self, id: FenceId) -> Option<&Fence> {
        self.pages
            .iter()
            .flat_map(|p| p.fences.iter())
            .find(|f| f.id == id)
    }

    pub fn fence_mut(&mut self, id: FenceId) -> Option<&mut Fence> {
        self.pages
            .iter_mut()
            .flat_map(|p| p.fences.iter_mut())
            .find(|f| f.id == id)
    }

    /// Page index holding the fence.
    pub fn page_of(&self, id: FenceId) -> Option<usize> {
        self.pages
            .iter()
            .position(|p| p.fences.iter().any(|f| f.id == id))
    }

    /// Every item that belongs to a fence on any page.
    pub fn fenced_items(&self) -> impl Iterator<Item = &ItemKey> {
        self.pages
            .iter()
            .flat_map(|p| p.fences.iter())
            .flat_map(|f| f.all_items())
    }
}

/// Whole persisted desktop state (`layouts.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopState {
    pub layouts: Vec<Layout>,
    /// Launch counter per item, fed by double-clicks on icons.
    pub open_counts: BTreeMap<ItemKey, u32>,
    /// Next value for fence and tab identifiers.
    pub next_id: u64,
    /// Explorer folder flags found before Bento Desk changed them.
    pub original_folder_flags: Option<u32>,
    /// Positions of free icons hidden by quick-hide, to put them back after
    /// a crash.
    pub parked_free_icons: BTreeMap<ItemKey, Point>,
    /// Key of the layout shown by the resident app. It can differ from the
    /// key of the connected monitors when automatic restore is off.
    pub current_layout: String,
}

impl DesktopState {
    /// Returns a fresh identifier.
    pub fn new_id(&mut self) -> u64 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn layout(&self, key: &str) -> Option<&Layout> {
        self.layouts.iter().find(|l| l.key == key)
    }

    pub fn layout_mut(&mut self, key: &str) -> Option<&mut Layout> {
        self.layouts.iter_mut().find(|l| l.key == key)
    }

    /// Makes `next_id` larger than every id in use (repairs hand-edited files).
    pub fn repair_ids(&mut self) {
        let mut max = 0;
        for l in &self.layouts {
            for p in &l.pages {
                for f in &p.fences {
                    max = max.max(f.id.0);
                    for t in &f.tabs {
                        max = max.max(t.id.0);
                    }
                }
            }
        }
        self.next_id = self.next_id.max(max + 1);
    }
}

/// A named copy of a layout taken by the user.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub name: String,
    /// Seconds since the Unix epoch.
    pub created: u64,
    pub layout: Layout,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fence_with(items: &[&str]) -> Fence {
        Fence {
            id: FenceId(1),
            tabs: vec![Tab {
                id: TabId(2),
                content: TabContent::Desktop {
                    items: items.iter().map(|s| (*s).to_owned()).collect(),
                },
                ..Tab::default()
            }],
            ..Fence::default()
        }
    }

    #[test]
    fn title_band_default_and_custom() {
        let mut st = FenceStyle::default();
        assert_eq!(st.title_band(), Rgba::new(0, 0, 0, 0x26));
        st.title_background = Some(Tint {
            hue: 0,
            saturation: 0,
            intensity: 0,
            transparency: 0,
        });
        assert_eq!(st.title_band(), Rgba::BLACK);
        // Older files without the field keep loading.
        let old: FenceStyle = serde_json::from_str(r#"{"title_bold":true}"#).unwrap();
        assert_eq!(old.title_background, None);
        assert_eq!(old.title_align, TitleAlign::Left);
    }

    #[test]
    fn remove_and_rename_items() {
        let mut f = fence_with(&["a", "b"]);
        assert!(f.contains_item("a"));
        assert!(f.rename_item("a", "c"));
        assert!(f.contains_item("c"));
        assert!(f.remove_item("b"));
        assert!(!f.remove_item("b"));
        assert_eq!(f.all_items().count(), 1);
    }

    #[test]
    fn layout_normalize_adds_page_and_tab() {
        let mut l = Layout {
            pages: vec![Page {
                fences: vec![Fence::default()],
                ..Page::default()
            }],
            current_page: 4,
            ..Layout::default()
        };
        l.normalize();
        assert_eq!(l.current_page, 0);
        assert_eq!(l.pages[0].fences[0].tabs.len(), 1);
        let mut empty = Layout::default();
        empty.normalize();
        assert_eq!(empty.pages.len(), 1);
    }

    #[test]
    fn ids_are_unique_and_repaired() {
        let mut s = DesktopState::default();
        let a = s.new_id();
        let b = s.new_id();
        assert_ne!(a, b);
        s.layouts.push(Layout {
            pages: vec![Page {
                fences: vec![Fence {
                    id: FenceId(50),
                    ..Fence::default()
                }],
                ..Page::default()
            }],
            ..Layout::default()
        });
        s.repair_ids();
        assert!(s.new_id() > 50);
    }

    #[test]
    fn serde_defaults_accept_minimal_json() {
        let f: Fence = serde_json::from_str(r#"{"id":3}"#).unwrap();
        assert_eq!(f.id, FenceId(3));
        let t: TabContent = serde_json::from_str(r#"{"type":"portal","root":"C:\\Data"}"#).unwrap();
        assert!(t.is_portal());
    }
}
