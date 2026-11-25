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
    FitContent,
    CopyStyle,
    PasteStyle,
    PasteStyleAll,
    ResetStyle,
    Appearance,
    ToggleQuickHideExempt,
    MoveToPage(usize),
    MergeInto(FenceId),
    DetachTab,
    PortalView(ViewMode),
    PortalChangeFolder,
    PortalOpen,
}

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
                items.push(b.action(t(Msg::FenceFitContent), f(FenceCmd::FitContent)));
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
        // Tabs.
        let others: Vec<(FenceId, String)> = self
            .layout()
            .and_then(|l| l.page())
            .map(|p| {
                p.fences
                    .iter()
                    .filter(|o| o.id != id)
                    .map(|o| (o.id, o.title().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        if !others.is_empty() {
            let merge = others
                .into_iter()
                .map(|(oid, title)| b.action(&title, f(FenceCmd::MergeInto(oid))))
                .collect();
            items.push(MenuItem::submenu(t(Msg::TabMergeInto), merge));
        }
        if fence.tabs.len() > 1 {
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
            FenceCmd::FitContent => self.fit_content(id),
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

    /// Resizes the fence height to show every icon without scrolling.
    fn fit_content(&mut self, id: FenceId) {
        let Some(i) = self.fences.iter().position(|f| f.id == id) else {
            return;
        };
        let ui = &self.fences[i];
        let m = bento_core::grid::FrameMetrics::for_dpi(ui.dpi);
        let cell = bento_core::grid::fence_cell(self.spacing, self.config.layout.icon_spacing);
        let full = ui.full_rect;
        let grid = bento_core::grid::Grid::new(m.body(&full), cell);
        let count = self
            .layout()
            .and_then(|l| l.fence(id))
            .and_then(|f| f.active())
            .map(|t| {
                t.content
                    .desktop_items()
                    .iter()
                    .filter(|k| self.items.contains_key(k.as_str()))
                    .count()
            })
            .unwrap_or(0);
        let body_h = grid.fit_height(count.max(1));
        let h = body_h + m.title_height + m.padding;
        let r = Rect::from_xywh(full.left, full.top, full.width(), h.max(m.min_height));
        if let Some(g) = bento_core::display::px_to_geometry(&r, &self.monitors)
            && let Some(f) = self.layout_mut().and_then(|l| l.fence_mut(id))
        {
            f.geometry = g;
            if let Some(t) = f.active_mut() {
                t.scroll_row = 0;
            }
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
