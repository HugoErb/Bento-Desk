//! Background threads that talk to Explorer and to the file system.
//!
//! - The shell worker owns the `IFolderView` connection. Every COM call into
//!   Explorer happens there, so a hung Explorer never blocks the UI.
//! - Folder listings (portals) and icon extraction run on short-lived
//!   threads, because a network folder may block for a long time.
//!
//! Results come back through a channel; the UI thread is woken with a
//! posted message.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::UNIX_EPOCH;

use bento_core::geometry::{Point, Size};
use bento_core::image::Image;
use bento_core::items::{ItemInfo, ItemKind};
use bento_win::Hwnd;
use bento_win::com::ComApartment;
use bento_win::folderview::{ChildPidl, DesktopFolderView};

/// One desktop icon with its metadata.
#[derive(Clone, Debug)]
pub struct ItemSnap {
    pub info: ItemInfo,
    pub pos: Point,
}

pub enum Cmd {
    Connect(Hwnd),
    Snapshot,
    Move(Vec<(String, Point)>),
    FreePositioning,
    RestorePositioning(u32),
    SetIconsHidden(bool),
    Icons(Vec<String>, i32),
}

#[derive(Debug)]
pub enum Reply {
    Connected {
        spacing: Size,
        icon_size: i32,
        flags: u32,
    },
    ConnectFailed(String),
    Snapshot(Vec<ItemSnap>),
    /// Free positioning is active.
    Positioning,
    Icons(Vec<(String, Option<Image>)>),
    Error(String),
}

/// Handle to the shell worker thread.
pub struct ShellWorker {
    tx: Option<Sender<Cmd>>,
    rx: Receiver<Reply>,
    join: Option<JoinHandle<()>>,
}

impl ShellWorker {
    /// Starts the thread. Each reply posts `msg` to `notify`.
    pub fn start(notify: Hwnd, msg: u32) -> std::io::Result<ShellWorker> {
        let (tx, crx) = mpsc::channel::<Cmd>();
        let (rtx, rx) = mpsc::channel::<Reply>();
        let join = std::thread::Builder::new()
            .name("shell-worker".into())
            .spawn(move || run(crx, rtx, notify, msg))?;
        Ok(ShellWorker {
            tx: Some(tx),
            rx,
            join: Some(join),
        })
    }

    pub fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx
            && tx.send(cmd).is_err()
        {
            tracing::error!("shell worker stopped");
        }
    }

    /// Pending replies.
    pub fn drain(&self) -> Vec<Reply> {
        self.rx.try_iter().collect()
    }
}

impl Drop for ShellWorker {
    fn drop(&mut self) {
        self.tx = None; // closes the channel, the thread exits
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    t.ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Target URL of a `.url` file (simple INI parsing).
fn url_target(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("URL="))
        .map(str::to_owned)
}

/// Builds the metadata of an item. `targets` caches shortcut targets by
/// (path, modification time).
pub fn item_info(
    key: &str,
    targets: &mut HashMap<String, (Option<u64>, Option<String>)>,
) -> ItemInfo {
    let path = Path::new(key);
    let meta = if key.starts_with("::") {
        None
    } else {
        std::fs::metadata(path).ok()
    };
    let is_dir = meta.as_ref().is_some_and(std::fs::Metadata::is_dir);
    let mut info = ItemInfo::from_key(key, is_dir);
    if let Some(m) = &meta {
        info.size = if is_dir { 0 } else { m.len() };
        info.modified = secs(m.modified());
        info.created = secs(m.created());
    }
    info.target = match info.kind {
        ItemKind::Shortcut => {
            let cached = targets.get(key).filter(|(t, _)| *t == info.modified);
            match cached {
                Some((_, t)) => t.clone(),
                None => {
                    let t = bento_win::shell::shortcut_target(key);
                    targets.insert(key.to_owned(), (info.modified, t.clone()));
                    t
                }
            }
        }
        ItemKind::Url => url_target(path),
        _ => None,
    };
    info
}

struct State {
    view: Option<DesktopFolderView>,
    pidls: HashMap<String, ChildPidl>,
    targets: HashMap<String, (Option<u64>, Option<String>)>,
}

impl State {
    fn snapshot(&mut self) -> Result<Vec<ItemSnap>, String> {
        let view = self.view.as_ref().ok_or("not connected")?;
        let items = view.items().map_err(|e| e.to_string())?;
        self.pidls.clear();
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let mut info = item_info(&it.key, &mut self.targets);
            if !it.display.is_empty() {
                info.name = it.display.clone();
            }
            out.push(ItemSnap { info, pos: it.pos });
            self.pidls.insert(it.key, it.pidl);
        }
        Ok(out)
    }

    fn move_items(&mut self, moves: &[(String, Point)]) -> Result<(), String> {
        if moves.iter().any(|(k, _)| !self.pidls.contains_key(k)) {
            self.snapshot()?;
        }
        let view = self.view.as_ref().ok_or("not connected")?;
        let list: Vec<(&ChildPidl, Point)> = moves
            .iter()
            .filter_map(|(k, p)| self.pidls.get(k).map(|pidl| (pidl, *p)))
            .collect();
        view.set_positions(&list).map_err(|e| e.to_string())
    }
}

fn run(rx: Receiver<Cmd>, tx: Sender<Reply>, notify: Hwnd, msg: u32) {
    let _com = match ComApartment::mta() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("shell worker COM init: {e}");
            return;
        }
    };
    let mut st = State {
        view: None,
        pidls: HashMap::new(),
        targets: HashMap::new(),
    };
    let reply = |r: Reply| {
        if tx.send(r).is_ok() {
            notify.post(msg, 0, 0);
        }
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Connect(listview) => {
                st.view = None;
                st.pidls.clear();
                match DesktopFolderView::connect(listview) {
                    Ok(v) => {
                        let spacing = v.spacing().unwrap_or(Size::new(75, 100));
                        let icon_size = v.icon_size().unwrap_or(48);
                        let flags = v.flags().unwrap_or(0);
                        st.view = Some(v);
                        reply(Reply::Connected {
                            spacing,
                            icon_size,
                            flags,
                        });
                    }
                    Err(e) => reply(Reply::ConnectFailed(e.to_string())),
                }
            }
            Cmd::Snapshot => match st.snapshot() {
                Ok(items) => reply(Reply::Snapshot(items)),
                Err(e) => reply(Reply::Error(format!("snapshot: {e}"))),
            },
            Cmd::Move(moves) => {
                if let Err(e) = st.move_items(&moves) {
                    reply(Reply::Error(format!("move: {e}")));
                }
            }
            Cmd::FreePositioning => {
                if let Some(v) = &st.view {
                    match v.free_positioning() {
                        Ok(_) => reply(Reply::Positioning),
                        Err(e) => reply(Reply::Error(format!("flags: {e}"))),
                    }
                }
            }
            Cmd::RestorePositioning(f) => {
                if let Some(v) = &st.view
                    && let Err(e) = v.restore_positioning(f)
                {
                    reply(Reply::Error(format!("flags: {e}")));
                }
            }
            Cmd::SetIconsHidden(h) => {
                if let Some(v) = &st.view
                    && let Err(e) = v.set_icons_hidden(h)
                {
                    reply(Reply::Error(format!("hide icons: {e}")));
                }
            }
            Cmd::Icons(keys, size) => {
                let icons = keys
                    .into_iter()
                    .map(|k| {
                        let img = bento_win::shell::item_icon(&k, size);
                        (k, img)
                    })
                    .collect();
                reply(Reply::Icons(icons));
            }
        }
    }
}

/// Result of listing a folder for a portal.
#[derive(Debug)]
pub struct Listing {
    pub token: u64,
    pub entries: Result<Vec<bento_render::portal::Entry>, String>,
}

/// Lists a folder on a new thread (folders may be slow network shares).
/// The result is sent on `tx` and `msg` is posted to `notify`.
pub fn list_folder_async(
    token: u64,
    path: String,
    show_hidden: bool,
    icon_px: i32,
    tx: Sender<Listing>,
    notify: Hwnd,
    msg: u32,
) {
    let spawn = std::thread::Builder::new()
        .name("portal-list".into())
        .spawn(move || {
            let _com = ComApartment::mta().ok();
            let entries = list_folder(&path, show_hidden, icon_px);
            if tx.send(Listing { token, entries }).is_ok() {
                notify.post(msg, 0, 0);
            }
        });
    if let Err(e) = spawn {
        tracing::error!("cannot start listing thread: {e}");
    }
}

const MAX_ICONS: usize = 400;

fn list_folder(
    path: &str,
    show_hidden: bool,
    icon_px: i32,
) -> Result<Vec<bento_render::portal::Entry>, String> {
    use std::os::windows::fs::MetadataExt;
    let hide_ext = bento_win::system::hide_known_extensions();
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut entries: Vec<bento_render::portal::Entry> = Vec::new();
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let attrs = meta.file_attributes();
        // FILE_ATTRIBUTE_HIDDEN = 2, FILE_ATTRIBUTE_SYSTEM = 4
        if !show_hidden && attrs & 0x6 != 0 {
            continue;
        }
        let file_name = e.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        let name = if is_dir {
            file_name.clone()
        } else {
            match file_name.rsplit_once('.') {
                Some((stem, ext))
                    if !stem.is_empty()
                        && (ext.eq_ignore_ascii_case("lnk")
                            || ext.eq_ignore_ascii_case("url")
                            || hide_ext) =>
                {
                    stem.to_owned()
                }
                _ => file_name.clone(),
            }
        };
        entries.push(bento_render::portal::Entry {
            name,
            path: e.path().to_string_lossy().into_owned(),
            is_dir,
            icon: None,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| bento_core::sort::natural_cmp(&a.name, &b.name))
    });
    for e in entries.iter_mut().take(MAX_ICONS) {
        e.icon = bento_win::shell::item_icon(&e.path, icon_px);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_file_target() {
        let dir = std::env::temp_dir().join(format!("bento-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("site.url");
        std::fs::write(&f, "[InternetShortcut]\r\nURL=https://example.org/\r\n").unwrap();
        assert_eq!(url_target(&f).as_deref(), Some("https://example.org/"));
        let mut cache = HashMap::new();
        let info = item_info(&f.to_string_lossy(), &mut cache);
        assert_eq!(info.kind, ItemKind::Url);
        assert_eq!(info.target.as_deref(), Some("https://example.org/"));
        assert!(info.modified.is_some());
    }
}
