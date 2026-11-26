//! Folder portals: a fence that shows any folder, drawn by Bento Desk.

use std::path::Path;

use bento_core::geometry::{Point, Rect};
use bento_core::i18n::Msg;
use bento_core::model::{FenceId, TabContent, ViewMode};
use bento_render::fence::FrameLayout;
use bento_render::portal::{Entry, EntryLayout, draw_entries, layout};
use bento_win::dirwatch::DirWatcher;
use bento_win::gfx::Canvas;

use crate::app::{App, WM_LISTING};
use crate::worker::{Listing, list_folder_async};

/// Content of an open portal.
pub struct PortalState {
    pub path: String,
    pub token: u64,
    pub entries: Option<Result<Vec<Entry>, String>>,
    pub first_row: usize,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// Left button pressed on an entry: screen point and entry index, to
    /// start a drag once the mouse moves far enough.
    pub press: Option<(Point, usize)>,
    _watch: Option<DirWatcher>,
}

/// What `render_frame` needs to draw the portal body.
pub struct PortalDraw {
    entries: Vec<Entry>,
    layout: EntryLayout,
    view: ViewMode,
    dpi: u32,
    fg: bento_core::color::Rgba,
    hover: Option<usize>,
    selected: Vec<usize>,
    message: Option<(String, String)>,
    body: Rect,
}

impl PortalDraw {
    pub fn draw(&self, c: &mut Canvas<'_>) {
        let msg = self.message.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
        draw_entries(
            c,
            &self.entries,
            &self.layout,
            self.view,
            self.dpi,
            self.fg,
            self.hover,
            &self.selected,
            msg,
            self.body,
        );
    }
}

fn portal_paths(content: &TabContent) -> Option<(String, String, ViewMode)> {
    match content {
        TabContent::Portal {
            root,
            current,
            view,
        } => {
            let cur = if current.is_empty() {
                root.clone()
            } else {
                current.clone()
            };
            Some((root.clone(), cur, *view))
        }
        TabContent::Desktop { .. } => None,
    }
}

impl App {
    fn portal_content(&self, id: FenceId) -> Option<(String, String, ViewMode)> {
        let f = self.layout()?.fence(id)?;
        portal_paths(&f.active()?.content)
    }

    /// Starts listing the portal folder if needed.
    pub fn ensure_portal(&mut self, i: usize) {
        let Some(ui) = self.fences.get(i) else { return };
        let Some((_, current, _)) = self.portal_content(ui.id) else {
            self.fences[i].portal_state = None;
            return;
        };
        if ui.portal_state.as_ref().is_some_and(|p| p.path == current) {
            return;
        }
        self.list_portal(i, current);
    }

    fn list_portal(&mut self, i: usize, path: String) {
        self.listing_token += 1;
        let token = self.listing_token;
        let icon_px = (32 * self.fences[i].dpi / 96) as i32;
        // Re-use the watcher when the folder did not change.
        let watch = match self.fences[i].portal_state.take() {
            Some(old) if old.path == path => old._watch,
            _ => DirWatcher::spawn(
                std::path::PathBuf::from(&path),
                self.fences[i].id.0,
                self.fs_tx.clone(),
                self.hwnd(),
                crate::app::WM_SHELL_CHANGE,
            )
            .ok(),
        };
        self.fences[i].portal_state = Some(PortalState {
            path: path.clone(),
            token,
            entries: None,
            first_row: 0,
            hover: None,
            selected: None,
            press: None,
            _watch: watch,
        });
        list_folder_async(
            token,
            path,
            self.config.portals.show_hidden_files,
            icon_px,
            self.listing_tx.clone(),
            self.hwnd(),
            WM_LISTING,
        );
        self.render_frame(i);
    }

    pub fn on_listing(&mut self, l: Listing) {
        let Some(i) = self
            .fences
            .iter()
            .position(|f| f.portal_state.as_ref().is_some_and(|p| p.token == l.token))
        else {
            return;
        };
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.entries = Some(l.entries);
        }
        self.render_frame(i);
    }

    /// Refresh after a change notification in the watched folder.
    pub fn refresh_portal(&mut self, i: usize) {
        if let Some(path) = self.fences[i].portal_state.as_ref().map(|p| p.path.clone()) {
            let first = self.fences[i]
                .portal_state
                .as_ref()
                .map(|p| p.first_row)
                .unwrap_or(0);
            self.list_portal(i, path);
            if let Some(p) = self.fences[i].portal_state.as_mut() {
                p.first_row = first;
            }
        }
    }

    pub fn portal_draw_data(&self, i: usize, frame: &FrameLayout) -> Option<PortalDraw> {
        let ui = self.fences.get(i)?;
        let (_, _, view) = self.portal_content(ui.id)?;
        let st = ui.portal_state.as_ref()?;
        let style = self
            .layout()
            .and_then(|l| l.fence(ui.id))
            .and_then(|f| f.active())
            .and_then(|t| t.style.clone())
            .unwrap_or_else(|| self.config.appearance.default_style.clone());
        let bg = ui.tint.unwrap_or_else(|| style.tint.to_rgba());
        let fg = bento_render::fence::title_color(&style, bg);
        let body = frame.body;
        let lang = self.lang;
        let (entries, message) = match &st.entries {
            None => (Vec::new(), Some((String::new(), String::new()))),
            Some(Err(e)) => (
                Vec::new(),
                Some((
                    Msg::PortalUnavailable.text(lang).to_owned(),
                    format!("{} - {e}", Msg::PortalRetry.text(lang)),
                )),
            ),
            Some(Ok(v)) if v.is_empty() => (
                Vec::new(),
                Some((Msg::PortalEmpty.text(lang).to_owned(), String::new())),
            ),
            Some(Ok(v)) => (v.clone(), None),
        };
        let l = layout(view, body, ui.dpi, entries.len(), st.first_row);
        Some(PortalDraw {
            entries,
            layout: l,
            view,
            dpi: ui.dpi,
            fg,
            hover: st.hover,
            selected: st.selected.into_iter().collect(),
            message,
            body,
        })
    }

    fn portal_layout(&self, i: usize) -> Option<(EntryLayout, Vec<Entry>)> {
        let ui = self.fences.get(i)?;
        let (_, _, view) = self.portal_content(ui.id)?;
        let st = ui.portal_state.as_ref()?;
        let entries = match &st.entries {
            Some(Ok(v)) => v.clone(),
            _ => Vec::new(),
        };
        let m = bento_core::grid::FrameMetrics::for_dpi(ui.dpi);
        let body = m.body(&Rect::from_xywh(0, 0, ui.rect.width(), ui.rect.height()));
        Some((
            layout(view, body, ui.dpi, entries.len(), st.first_row),
            entries,
        ))
    }

    pub fn portal_hover(&mut self, i: usize, local: Point) {
        let hit = self.portal_layout(i).and_then(|(l, _)| l.hit(local));
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.hover = hit;
        }
    }

    pub fn portal_click(&mut self, i: usize, local: Point, double: bool) {
        let Some((l, entries)) = self.portal_layout(i) else {
            return;
        };
        let failed = self.fences[i]
            .portal_state
            .as_ref()
            .is_some_and(|p| matches!(p.entries, Some(Err(_))));
        if failed {
            self.refresh_portal(i);
            return;
        }
        let Some(idx) = l.hit(local) else { return };
        let Some(e) = entries.get(idx).cloned() else {
            return;
        };
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.selected = Some(idx);
        }
        if double {
            if e.is_dir {
                self.portal_navigate(i, e.path);
            } else if !bento_win::system::shell_open(&e.path) {
                tracing::warn!("cannot open {}", e.path);
            }
        }
        self.render_frame(i);
    }

    pub fn portal_context(&mut self, i: usize, local: Point, screen: Point) {
        let Some((l, entries)) = self.portal_layout(i) else {
            return;
        };
        let target = match l.hit(local).and_then(|idx| entries.get(idx)) {
            Some(e) => e.path.clone(),
            None => match self.fences[i].portal_state.as_ref() {
                Some(p) => p.path.clone(),
                None => return,
            },
        };
        bento_win::shell::show_context_menu(self.hwnd(), &target, screen);
        self.refresh_portal(i);
    }

    fn portal_navigate(&mut self, i: usize, path: String) {
        let id = self.fences[i].id;
        if let Some(t) = self
            .layout_mut()
            .and_then(|l| l.fence_mut(id))
            .and_then(|f| f.active_mut())
            && let TabContent::Portal { current, .. } = &mut t.content
        {
            *current = path.clone();
        }
        self.mark_dirty();
        self.list_portal(i, path);
    }

    pub fn portal_up(&mut self, id: FenceId) {
        let Some(i) = self.fences.iter().position(|f| f.id == id) else {
            return;
        };
        let Some((root, current, _)) = self.portal_content(id) else {
            return;
        };
        if current == root {
            return;
        }
        let parent = Path::new(&current)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| p.len() >= root.len())
            .unwrap_or(root);
        self.portal_navigate(i, parent);
    }

    pub fn portal_open(&mut self, id: FenceId) {
        if let Some((_, current, _)) = self.portal_content(id)
            && !bento_win::system::open_in_explorer(&current)
        {
            tracing::warn!("cannot open {current}");
        }
    }

    /// Wheel in a portal: one row (icon view) or one entry (list view).
    pub fn portal_scroll(&mut self, i: usize, steps: i32) {
        let Some((l, _)) = self.portal_layout(i) else {
            return;
        };
        let max = l.max_first_row();
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            let next = (p.first_row as i64 + i64::from(steps)).clamp(0, max as i64) as usize;
            if next == p.first_row {
                return;
            }
            p.first_row = next;
        }
        self.render_frame(i);
    }

    /// Left button down on a portal body: selects the entry and arms a drag.
    pub fn portal_press(&mut self, i: usize, local: Point, screen: Point) {
        self.portal_click(i, local, false);
        let hit = self.portal_layout(i).and_then(|(l, _)| l.hit(local));
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.press = hit.map(|idx| (screen, idx));
        }
    }

    /// Mouse moved with the button down: starts an OLE drag of the entry
    /// once the system drag threshold is passed. Returns true if a drag ran.
    pub fn portal_drag(&mut self, i: usize, screen: Point) -> bool {
        let Some((start, idx)) = self.fences[i].portal_state.as_ref().and_then(|p| p.press) else {
            return false;
        };
        if !bento_win::input::left_button_down() {
            if let Some(p) = self.fences[i].portal_state.as_mut() {
                p.press = None;
            }
            return false;
        }
        let (tw, th) = bento_win::input::double_click_size();
        if (screen.x - start.x).abs() <= tw && (screen.y - start.y).abs() <= th {
            return false;
        }
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.press = None;
        }
        let Some(path) = self
            .portal_layout(i)
            .and_then(|(_, entries)| entries.get(idx).map(|e| e.path.clone()))
        else {
            return false;
        };
        let hwnd = self.fences[i].frame.hwnd();
        bento_win::dragdrop::drag_out(hwnd, &[path]);
        self.refresh_portal(i);
        true
    }

    pub fn portal_release(&mut self, i: usize) {
        if let Some(p) = self.fences[i].portal_state.as_mut() {
            p.press = None;
        }
    }

    /// Files dropped on a portal: moved into its current folder (copied
    /// across volumes, or with Ctrl), like a drop in Explorer.
    pub fn portal_drop(&mut self, i: usize, paths: Vec<String>, ctrl: bool, shift: bool) {
        let Some(dest) = self.fences[i].portal_state.as_ref().map(|p| p.path.clone()) else {
            return;
        };
        let files: Vec<String> = paths
            .into_iter()
            .filter(|p| !p.starts_with("::") && Path::new(p).exists())
            .filter(|p| Path::new(p).parent() != Some(Path::new(&dest)) && *p != dest)
            .collect();
        if files.is_empty() {
            return;
        }
        let same_volume = files.iter().all(|f| volume(f) == volume(&dest));
        let copy = ctrl || (!shift && !same_volume);
        tracing::info!(
            "{} {} item(s) into {dest}",
            if copy { "copying" } else { "moving" },
            files.len()
        );
        if !bento_win::shell::transfer(files, dest, copy) {
            tracing::warn!("cannot start the file operation");
        }
    }
}

/// Drive or share of a path, lower case (`c:`, `\\server\share`).
fn volume(p: &str) -> String {
    match Path::new(p).components().next() {
        Some(std::path::Component::Prefix(pre)) => pre.as_os_str().to_string_lossy().to_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::volume;

    #[test]
    fn volumes() {
        assert_eq!(volume(r"C:\Users\a.txt"), "c:");
        assert_eq!(volume(r"c:\x"), volume(r"C:\y"));
        assert_ne!(volume(r"D:\x"), volume(r"C:\y"));
        assert_eq!(volume(r"\\srv\share\f"), r"\\srv\share");
    }
}
