//! Menu contents (tray, fence, creation) and command execution.

use bento_core::geometry::{Point, Rect};
use bento_core::i18n::Msg;
use bento_core::model::{FenceId, SortKey, TabContent, ViewMode};
use bento_render::menu::MenuItem;

use crate::app::App;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    OpenSettings,
    NewFence,
    NewPortal,
    Peek,
    ToggleQuickHide,
    SaveSnapshot,
    RestoreSnapshot(String),
    TogglePause,
    Quit,
    ToggleAutostart,
    SortNow,
    RecoverIcons,
    NextPage,
    PrevPage,
    NewPage,
    DeletePage,
    SwapScreens(String, String),
    CreateFence(Rect),
    CreatePortal(Rect),
    Fence(FenceId, FenceCmd),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FenceCmd {
    Rename,
    DeleteRestore,
    DeleteKeep,
    ToggleRoll,
    ToggleAutoRollup,
    Sort(SortKey),
    ToggleDescending,
    Spacing(u16),
    ToggleFitHeight,
    /// Width of `span` columns out of `count` splitting the screen.
    Width(u8, u8),
    CopyStyle,
    PasteStyle,
    PasteStyleAll,
    ResetStyle,
    Appearance,
    ToggleQuickHideExempt,
    ToggleLock,
    MoveToPage(usize),
    MergeInto(FenceId),
    DetachTab,
    PortalView(ViewMode),
    PortalChangeFolder,
    PortalOpen,
}

/// Screen widths offered in the fence menu: `span` columns out of `count`.
const WIDTHS: [(u8, u8); 6] = [(1, 1), (1, 2), (1, 3), (2, 3), (1, 4), (3, 4)];

/// Builds items and their commands in parallel (item id = index + 1).
struct Builder {
    commands: Vec<Command>,
}

impl Builder {
    fn new() -> Builder {
        Builder {
            commands: Vec::new(),
        }
    }

    fn id(&mut self, c: Command) -> u32 {
        self.commands.push(c);
        u32::try_from(self.commands.len()).unwrap_or(0)
    }

    fn action(&mut self, label: &str, c: Command) -> MenuItem {
        let id = self.id(c);
        MenuItem::action(id, label)
    }

    fn check(&mut self, label: &str, on: bool, c: Command) -> MenuItem {
        let id = self.id(c);
        MenuItem::check(id, label, on)
    }

    fn radio(&mut self, label: &str, on: bool, c: Command) -> MenuItem {
        let id = self.id(c);
        MenuItem::radio(id, label, on)
    }
}

impl App {
    pub fn show_tray_menu(&mut self) {
        let lang = self.lang;
        let t = |m: Msg| m.text(lang);
        let mut b = Builder::new();
        let mut items = vec![
            b.action(t(Msg::TraySettings), Command::OpenSettings),
            MenuItem::separator(),
            b.action(t(Msg::TrayNewFence), Command::NewFence),
            b.action(t(Msg::TrayNewPortal), Command::NewPortal),
            b.action(t(Msg::TrayPeek), Command::Peek),
            b.check(
                t(Msg::TrayQuickHide),
                self.quick_hidden,
                Command::ToggleQuickHide,
            ),
            b.action(t(Msg::TraySortNow), Command::SortNow),
        ];
        // Snapshots.
        let mut snaps = vec![b.action(t(Msg::TraySaveSnapshot), Command::SaveSnapshot)];
        let list = bento_config::list_snapshots(&self.paths);
        if list.is_empty() {
            snaps.push(MenuItem::action(0, t(Msg::TrayNoSnapshot)).enabled(false));
        } else {
            snaps.push(MenuItem::separator());
            for s in list.into_iter().take(12) {
                snaps.push(b.action(&s.name, Command::RestoreSnapshot(s.name.clone())));
            }
        }
        items.push(MenuItem::submenu(t(Msg::TraySnapshots), snaps));
        // Pages.
        if self.config.pages.enabled {
            let mut pages = vec![
                b.action(t(Msg::PageNext), Command::NextPage),
                b.action(t(Msg::PagePrev), Command::PrevPage),
                b.action(t(Msg::PageNew), Command::NewPage),
            ];
            let n = self.layout().map(|l| l.pages.len()).unwrap_or(1);
            if n > 1 {
                pages.push(b.action(t(Msg::PageDelete), Command::DeletePage).danger());
            }
            items.push(MenuItem::submenu(t(Msg::TrayPages), pages));
        }
        if self.monitors.len() == 2 {
            let (a, c) = (self.monitors[0].id.clone(), self.monitors[1].id.clone());
            items.push(b.action(t(Msg::SwapScreens), Command::SwapScreens(a, c)));
        }
        items.push(MenuItem::separator());
        items.push(b.check(
            t(Msg::TrayStartWithWindows),
            self.config.general.start_with_windows,
            Command::ToggleAutostart,
        ));
        items.push(b.action(t(Msg::TrayRecoverIcons), Command::RecoverIcons));
        let pause = if self.paused {
            t(Msg::TrayResume)
        } else {
            t(Msg::TrayPause)
        };
        items.push(b.action(pause, Command::TogglePause));
        items.push(b.action(t(Msg::TrayQuit), Command::Quit));
        let at = bento_win::input::cursor_pos();
        // The tray requires the owner to be in the foreground for the menu
        // to close correctly.
        self.hwnd().set_foreground();
        self.show_menu(items, b.commands, at);
    }

    pub fn show_fence_menu(&mut self, id: FenceId, at: Point) {
        let lang = self.lang;
        let t = |m: Msg| m.text(lang);
        let Some(fence) = self.layout().and_then(|l| l.fence(id)).cloned() else {
            return;
        };
        let tab = fence.active().cloned().unwrap_or_default();
        let f = |c: FenceCmd| Command::Fence(id, c);
        let mut b = Builder::new();
        let mut items = vec![MenuItem::header(fence.title())];
        items.push(b.action(t(Msg::FenceRename), f(FenceCmd::Rename)));
        let roll = if fence.rolled_up {
            t(Msg::FenceUnroll)
        } else {
            t(Msg::FenceRollUp)
        };
        items.push(b.action(roll, f(FenceCmd::ToggleRoll)));
        match &tab.content {
            TabContent::Desktop { .. } => {
                let s = tab.sort;
                let sorts = vec![
                    b.radio(
                        t(Msg::SortManual),
                        s.key == SortKey::Manual,
                        f(FenceCmd::Sort(SortKey::Manual)),
                    ),
                    b.radio(
                        t(Msg::SortName),
                        s.key == SortKey::Name,
                        f(FenceCmd::Sort(SortKey::Name)),
                    ),
                    b.radio(
                        t(Msg::SortType),
                        s.key == SortKey::Type,
                        f(FenceCmd::Sort(SortKey::Type)),
                    ),
                    b.radio(
                        t(Msg::SortDate),
                        s.key == SortKey::Date,
                        f(FenceCmd::Sort(SortKey::Date)),
                    ),
                    b.radio(
                        t(Msg::SortSize),
                        s.key == SortKey::Size,
                        f(FenceCmd::Sort(SortKey::Size)),
                    ),
                    b.radio(
                        t(Msg::SortOpens),
                        s.key == SortKey::Opens,
                        f(FenceCmd::Sort(SortKey::Opens)),
                    ),
                    MenuItem::separator(),
                    b.check(
                        t(Msg::SortDescending),
                        s.descending,
                        f(FenceCmd::ToggleDescending),
                    ),
                ];
                items.push(MenuItem::submenu(t(Msg::FenceSortBy), sorts));
                let sp = self.config.layout.icon_spacing;
                let spacing = vec![
                    b.radio(t(Msg::SpacingCompact), sp < 95, f(FenceCmd::Spacing(85))),
                    b.radio(
                        t(Msg::SpacingNormal),
                        (95..=105).contains(&sp),
                        f(FenceCmd::Spacing(100)),
                    ),
                    b.radio(t(Msg::SpacingWide), sp > 105, f(FenceCmd::Spacing(120))),
                ];
                items.push(MenuItem::submenu(t(Msg::FenceSpacing), spacing));
                items.push(
                    b.check(
                        t(Msg::FenceFitContent),
                        fence.fit_height,
                        f(FenceCmd::ToggleFitHeight),
                    )
                    .enabled(!fence.locked),
                );
            }
            TabContent::Portal { view, .. } => {
                items.push(b.radio(
                    t(Msg::PortalViewIcons),
                    *view == ViewMode::Icons,
                    f(FenceCmd::PortalView(ViewMode::Icons)),
                ));
                items.push(b.radio(
                    t(Msg::PortalViewList),
                    *view == ViewMode::List,
                    f(FenceCmd::PortalView(ViewMode::List)),
                ));
                items.push(b.action(t(Msg::PortalOpen), f(FenceCmd::PortalOpen)));
                items.push(b.action(t(Msg::PortalChangeFolder), f(FenceCmd::PortalChangeFolder)));
            }
        }
        if !fence.locked {
            let current = self.fences.iter().find(|u| u.id == id).map(|u| u.full_rect);
            let widths = WIDTHS
                .iter()
                .map(|&(span, count)| {
                    let label = if span == count {
                        t(Msg::WidthFull).to_owned()
                    } else {
                        Msg::WidthFraction.fmt(lang, &format!("{span}/{count}"))
                    };
                    let on = current.is_some()
                        && self.column_rect(id, span, count).map(|(_, r)| r) == current;
                    b.radio(&label, on, f(FenceCmd::Width(span, count)))
                })
                .collect();
            items.push(MenuItem::submenu(t(Msg::FenceWidth), widths));
        }
        items.push(MenuItem::separator());
        let mut style = vec![
            b.action(t(Msg::FenceAppearance), f(FenceCmd::Appearance)),
            b.action(t(Msg::FenceCopyStyle), f(FenceCmd::CopyStyle)),
        ];
        if self.style_clip.is_some() {
            style.push(b.action(t(Msg::FencePasteStyle), f(FenceCmd::PasteStyle)));
            style.push(b.action(t(Msg::FencePasteStyleAll), f(FenceCmd::PasteStyleAll)));
        }
        style.push(MenuItem::submenu(
            t(Msg::FenceResetStyle),
            vec![b.action(t(Msg::Reset), f(FenceCmd::ResetStyle)).danger()],
        ));
        items.push(MenuItem::submenu(
            t(Msg::FenceAppearance).trim_end_matches('.'),
            style,
        ));
        items.push(b.check(
            t(Msg::FenceAutoRollup),
            fence.auto_rollup,
            f(FenceCmd::ToggleAutoRollup),
        ));
        items.push(b.check(
            t(Msg::FenceQuickHideExempt),
            fence.quick_hide_exempt,
            f(FenceCmd::ToggleQuickHideExempt),
        ));
        items.push(b.check(t(Msg::FenceLock), fence.locked, f(FenceCmd::ToggleLock)));
        // Tabs.
        let others: Vec<(FenceId, String)> = self
            .layout()
            .and_then(|l| l.page())
            .map(|p| {
                p.fences
                    .iter()
                    .filter(|o| o.id != id && !o.locked)
                    .map(|o| (o.id, o.title().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        // A locked fence keeps its content: no merge, no detach.
        if !fence.locked && !others.is_empty() {
            let merge = others
                .into_iter()
                .map(|(oid, title)| b.action(&title, f(FenceCmd::MergeInto(oid))))
                .collect();
            items.push(MenuItem::submenu(t(Msg::TabMergeInto), merge));
        }
        if !fence.locked && fence.tabs.len() > 1 {
            items.push(b.action(t(Msg::TabDetach), f(FenceCmd::DetachTab)));
        }
        // Pages.
        let n = self.layout().map(|l| l.pages.len()).unwrap_or(1);
        if self.config.pages.enabled && n > 1 {
            let cur = self.layout().map(|l| l.current_page).unwrap_or(0);
            let pages = (0..n)
                .filter(|p| *p != cur)
                .map(|p| {
                    b.action(
                        &Msg::PageName.fmt(lang, &(p + 1).to_string()),
                        f(FenceCmd::MoveToPage(p)),
                    )
                })
                .collect();
            items.push(MenuItem::submenu(t(Msg::FenceMoveToPage), pages));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(
            t(Msg::FenceDelete),
            vec![
                b.action(t(Msg::DeleteFenceRestore), f(FenceCmd::DeleteRestore))
                    .danger(),
                b.action(t(Msg::DeleteFenceKeep), f(FenceCmd::DeleteKeep))
                    .danger(),
            ],
        ));
        items.push(MenuItem::separator());
        items.push(b.action(t(Msg::FenceOpenSettings), Command::OpenSettings));
        self.show_menu(items, b.commands, at);
    }

    pub fn show_create_menu(&mut self, rect: Rect, at: Point) {
        let lang = self.lang;
        let mut b = Builder::new();
        let items = vec![
            b.action(Msg::CreateFenceHere.text(lang), Command::CreateFence(rect)),
            b.action(
                Msg::CreatePortalHere.text(lang),
                Command::CreatePortal(rect),
            ),
        ];
        self.show_menu(items, b.commands, at);
    }

    pub fn execute(&mut self, c: Command) {
        tracing::debug!("command {c:?}");
        match c {
            Command::OpenSettings => self.open_settings(None),
            Command::NewFence => {
                self.create_fence(None, None);
            }
            Command::NewPortal => {
                if let Some(path) = bento_win::shell::pick_folder(self.hwnd()) {
                    self.create_fence(None, Some(path));
                }
            }
            Command::Peek => self.toggle_peek(),
            Command::ToggleQuickHide => {
                let h = !self.quick_hidden;
                self.set_quick_hide(h);
            }
            Command::SaveSnapshot => self.save_snapshot(None),
            Command::RestoreSnapshot(n) => self.restore_snapshot(&n),
            Command::TogglePause => self.toggle_pause(),
            Command::Quit => bento_win::window::post_quit(0),
            Command::ToggleAutostart => {
                self.config.general.start_with_windows = !self.config.general.start_with_windows;
                if let Err(e) = bento_config::save_config(&self.paths, &self.config) {
                    tracing::error!("save config: {e}");
                }
                self.apply_config_side_effects();
            }
            Command::SortNow => self.sort_now(),
            Command::RecoverIcons => self.recover_all_icons(),
            Command::NextPage => self.switch_page(1),
            Command::PrevPage => self.switch_page(-1),
            Command::NewPage => self.new_page(),
            Command::DeletePage => self.delete_page(),
            Command::SwapScreens(a, b) => self.swap_screens(&a, &b),
            Command::CreateFence(r) => {
                self.create_fence(Some(r), None);
            }
            Command::CreatePortal(r) => {
                if let Some(path) = bento_win::shell::pick_folder(self.hwnd()) {
                    self.create_fence(Some(r), Some(path));
                }
            }
            Command::Fence(id, fc) => self.execute_fence(id, fc),
        }
    }

    fn execute_fence(&mut self, id: FenceId, c: FenceCmd) {
        match c {
            FenceCmd::Rename => self.begin_rename(id),
            FenceCmd::DeleteRestore => self.delete_fence(id, true),
            FenceCmd::DeleteKeep => self.delete_fence(id, false),
            FenceCmd::ToggleRoll => self.toggle_rollup(id),
            FenceCmd::ToggleAutoRollup => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
                    f.auto_rollup = !f.auto_rollup;
                }
                self.mark_dirty();
                self.render_all();
                self.relayout();
            }
            FenceCmd::Sort(key) => {
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(id))
                    .and_then(|f| f.active_mut())
                {
                    t.sort.key = key;
                }
                self.sort_all_tabs();
                self.mark_dirty();
                self.relayout();
            }
            FenceCmd::ToggleDescending => {
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(id))
                    .and_then(|f| f.active_mut())
                {
                    t.sort.descending = !t.sort.descending;
                }
                self.sort_all_tabs();
                self.mark_dirty();
                self.relayout();
            }
            FenceCmd::Spacing(p) => {
                self.config.layout.icon_spacing = p;
                if let Err(e) = bento_config::save_config(&self.paths, &self.config) {
                    tracing::error!("save config: {e}");
                }
                self.render_all();
                self.relayout();
            }
            FenceCmd::ToggleFitHeight => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
                    f.fit_height = !f.fit_height;
                }
                self.mark_dirty();
                self.relayout();
            }
            FenceCmd::Width(span, count) => self.fit_width(id, span, count),
            FenceCmd::CopyStyle => {
                self.style_clip = Some(
                    self.layout()
                        .and_then(|l| l.fence(id))
                        .and_then(|f| f.active())
                        .and_then(|t| t.style.clone())
                        .unwrap_or_else(|| self.config.appearance.default_style.clone()),
                );
            }
            FenceCmd::PasteStyle => {
                let s = self.style_clip.clone();
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(id))
                    .and_then(|f| f.active_mut())
                {
                    t.style = s;
                }
                self.mark_dirty();
                self.apply_chameleon();
                self.render_all();
            }
            FenceCmd::PasteStyleAll => {
                let s = self.style_clip.clone();
                if let Some(l) = self.layout_mut() {
                    for p in &mut l.pages {
                        for f in &mut p.fences {
                            for t in &mut f.tabs {
                                t.style = s.clone();
                            }
                        }
                    }
                }
                self.mark_dirty();
                self.apply_chameleon();
                self.render_all();
            }
            FenceCmd::ResetStyle => {
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(id))
                    .and_then(|f| f.active_mut())
                {
                    t.style = None;
                }
                self.mark_dirty();
                self.apply_chameleon();
                self.render_all();
            }
            FenceCmd::Appearance => {
                self.open_settings(Some(&format!("appearance --fence {}", id.0)));
            }
            FenceCmd::ToggleQuickHideExempt => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
                    f.quick_hide_exempt = !f.quick_hide_exempt;
                }
                self.mark_dirty();
            }
            FenceCmd::ToggleLock => {
                if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
                    f.locked = !f.locked;
                }
                self.mark_dirty();
                if let Some(i) = self.fences.iter().position(|f| f.id == id) {
                    self.render_frame(i);
                }
            }
            FenceCmd::MoveToPage(p) => {
                if let Some(l) = self.layout_mut()
                    && let Some(src) = l.page_of(id)
                    && p < l.pages.len()
                    && let Some(pos) = l.pages[src].fences.iter().position(|f| f.id == id)
                {
                    let f = l.pages[src].fences.remove(pos);
                    l.pages[p].fences.push(f);
                }
                self.mark_dirty();
                self.rebuild_fences();
                self.relayout();
            }
            FenceCmd::MergeInto(target) => {
                if let Some(l) = self.layout_mut()
                    && let Some(src_page) = l.page_of(id)
                    && let Some(pos) = l.pages[src_page].fences.iter().position(|f| f.id == id)
                {
                    let f = l.pages[src_page].fences.remove(pos);
                    if let Some(t) = l.fence_mut(target) {
                        t.tabs.extend(f.tabs);
                    }
                }
                self.mark_dirty();
                self.rebuild_fences();
                self.relayout();
            }
            FenceCmd::DetachTab => self.detach_tab(id),
            FenceCmd::PortalView(v) => {
                if let Some(t) = self
                    .layout_mut()
                    .and_then(|l| l.fence_mut(id))
                    .and_then(|f| f.active_mut())
                    && let TabContent::Portal { view, .. } = &mut t.content
                {
                    *view = v;
                }
                self.mark_dirty();
                if let Some(i) = self.fences.iter().position(|f| f.id == id) {
                    self.render_frame(i);
                }
            }
            FenceCmd::PortalChangeFolder => {
                if let Some(path) = bento_win::shell::pick_folder(self.hwnd()) {
                    if let Some(t) = self
                        .layout_mut()
                        .and_then(|l| l.fence_mut(id))
                        .and_then(|f| f.active_mut())
                        && let TabContent::Portal { root, current, .. } = &mut t.content
                    {
                        *root = path.clone();
                        *current = path;
                    }
                    self.mark_dirty();
                    if let Some(i) = self.fences.iter().position(|f| f.id == id) {
                        self.fences[i].portal_state = None;
                        self.ensure_portal(i);
                    }
                }
            }
            FenceCmd::PortalOpen => self.portal_open(id),
        }
    }

    /// Geometry of fence `i` with the height that shows every icon without
    /// scrolling, `None` when it already has it or cannot be fitted.
    fn fitted_geometry(&self, i: usize) -> Option<bento_core::model::FenceGeometry> {
        use bento_core::display::{geometry_to_px, monitor_for_rect, px_to_geometry};
        use bento_core::grid::{FrameMetrics, Grid, fence_cell};
        let ui = self.fences.get(i)?;
        let fence = self.layout()?.fence(ui.id)?;
        let tab = fence.active()?;
        if tab.content.is_portal() {
            return None;
        }
        let count = tab
            .content
            .desktop_items()
            .iter()
            .filter(|k| self.items.contains_key(k.as_str()))
            .count()
            .max(1);
        let full = geometry_to_px(&fence.geometry, &self.monitors)?;
        let work = monitor_for_rect(&self.monitors, &full)?.work;
        let m = FrameMetrics::for_dpi(ui.dpi);
        let cell = fence_cell(self.spacing, self.config.layout.icon_spacing);
        let rows = Grid::new(m.body(&full), cell).total_rows(count);
        let wanted = Grid::new(m.body(&full), cell).fit_height(count) + m.title_height + m.padding;
        let max_h = (work.bottom - full.top).max(m.min_height);
        // The geometry is stored in DIP: rounding can take a pixel or two
        // off, which would hide the last row. Grow until it is shown.
        let mut h = wanted.clamp(m.min_height, max_h);
        let mut g = px_to_geometry(
            &Rect::from_xywh(full.left, full.top, full.width(), h),
            &self.monitors,
        )?;
        for _ in 0..4 {
            let back = geometry_to_px(&g, &self.monitors)?;
            if h >= max_h || Grid::new(m.body(&back), cell).visible_rows >= rows {
                break;
            }
            h += 1;
            g = px_to_geometry(
                &Rect::from_xywh(full.left, full.top, full.width(), h),
                &self.monitors,
            )?;
        }
        (g != fence.geometry).then_some(g)
    }

    /// Applies "fit height to content" to every fence that has it. Called
    /// before each layout pass, so the height follows added or removed
    /// icons, the width and the icon spacing.
    pub fn refit_heights(&mut self) {
        for i in 0..self.fences.len() {
            let id = self.fences[i].id;
            let fit = self
                .layout()
                .and_then(|l| l.fence(id))
                .is_some_and(|f| f.fit_height);
            if !fit || self.fences[i].drag.is_some() {
                continue;
            }
            let Some(g) = self.fitted_geometry(i) else {
                continue;
            };
            if let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id)) {
                f.geometry = g;
                if let Some(t) = f.active_mut() {
                    t.scroll_row = 0;
                }
            }
            self.mark_dirty();
            self.update_fence_geometry(i);
            self.render_backdrop(i);
            self.render_frame(i);
        }
    }

    /// Full rectangle of a fence resized to `span` columns out of `count`
    /// splitting the work area of its monitor, with the gap used by
    /// magnetism between two columns.
    fn column_rect(&self, id: FenceId, span: u8, count: u8) -> Option<(usize, Rect)> {
        let i = self.fences.iter().position(|f| f.id == id)?;
        let ui = &self.fences[i];
        let full = ui.full_rect;
        let work = bento_core::display::monitor_for_rect(&self.monitors, &full)?.work;
        let cfg = self.snap_config(i, ui.dpi);
        let r = bento_core::snap::fit_columns(
            full,
            &work,
            i32::from(span),
            i32::from(count),
            cfg.gap_to(cfg.border),
        );
        Some((i, r))
    }

    /// Sets the fence width to a fraction of its screen (see `WIDTHS`).
    fn fit_width(&mut self, id: FenceId, span: u8, count: u8) {
        let Some((i, r)) = self.column_rect(id, span, count) else {
            return;
        };
        if let Some(g) = bento_core::display::px_to_geometry(&r, &self.monitors)
            && let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id))
        {
            f.geometry = g;
        }
        self.mark_dirty();
        self.update_fence_geometry(i);
        self.render_backdrop(i);
        self.render_frame(i);
        self.relayout();
    }

    /// Moves the active tab into a new fence next to the current one.
    fn detach_tab(&mut self, id: FenceId) {
        let new_id = FenceId(self.state.new_id());
        let Some(l) = self.layout_mut() else { return };
        let Some(page) = l.page_of(id) else { return };
        let Some(f) = l.fence_mut(id) else { return };
        if f.tabs.len() < 2 {
            return;
        }
        let idx = f.active_tab.min(f.tabs.len() - 1);
        let tab = f.tabs.remove(idx);
        f.active_tab = 0;
        let mut g = f.geometry.clone();
        g.rect = g.rect.offset(32, 32);
        let nf = bento_core::model::Fence {
            id: new_id,
            geometry: g,
            tabs: vec![tab],
            ..bento_core::model::Fence::default()
        };
        l.pages[page].fences.push(nf);
        self.mark_dirty();
        self.rebuild_fences();
        self.relayout();
    }
}
