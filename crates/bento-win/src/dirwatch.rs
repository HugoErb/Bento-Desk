//! Folder change notifications with `ReadDirectoryChangesW`.
//!
//! One thread per watched folder blocks in the kernel until something
//! changes: no polling. Used for the desktop folders (user and public) and
//! for folder portals.
//! <https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-readdirectorychangesw>

use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadDirectoryChangesW,
};
use windows::Win32::System::IO::CancelSynchronousIo;

use crate::error::{Result, WinError};
use crate::util::{pcwstr, wide};
use crate::window::Hwnd;

/// One change in a watched folder (full paths).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsChange {
    Created(String),
    Deleted(String),
    Renamed {
        from: String,
        to: String,
    },
    Modified(String),
    /// Too many changes at once: list everything again.
    Overflow,
}

/// A running watcher; stops on drop.
#[derive(Debug)]
pub struct DirWatcher {
    join: Option<JoinHandle<()>>,
}

struct Dir(HANDLE);

// SAFETY: a directory handle can be used from any thread.
unsafe impl Send for Dir {}

impl Drop for Dir {
    fn drop(&mut self) {
        // SAFETY: handle opened by CreateFileW and owned by this struct.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open_dir(path: &Path) -> Result<Dir> {
    let w = wide(&path.to_string_lossy());
    // SAFETY: NUL-terminated path; FILE_FLAG_BACKUP_SEMANTICS is required
    // to open a directory handle.
    let h = unsafe {
        CreateFileW(
            pcwstr(&w),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| WinError::Api {
        context: "CreateFileW(directory)",
        source: e,
    })?;
    Ok(Dir(h))
}

/// Decodes a buffer of `FILE_NOTIFY_INFORMATION` records.
fn parse(dir: &Path, buf: &[u32], len: usize) -> Vec<FsChange> {
    let bytes_len = len.min(buf.len() * 4);
    let base = buf.as_ptr() as *const u8;
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut old_name: Option<String> = None;
    loop {
        let header = size_of::<FILE_NOTIFY_INFORMATION>();
        if offset + header > bytes_len {
            break;
        }
        // SAFETY: the record starts at a DWORD-aligned offset inside the
        // part of the buffer written by ReadDirectoryChangesW; the name
        // length is checked against the buffer length below.
        let (next, action, name) = unsafe {
            let rec = &*(base.add(offset) as *const FILE_NOTIFY_INFORMATION);
            let name_len = rec.FileNameLength as usize / 2;
            let name_ptr = rec.FileName.as_ptr();
            let name_end = offset + header - 4 + name_len * 2;
            if name_end > bytes_len {
                break;
            }
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(name_ptr, name_len));
            (rec.NextEntryOffset as usize, rec.Action, name)
        };
        let full = dir.join(&name).to_string_lossy().into_owned();
        match action {
            a if a == FILE_ACTION_ADDED => out.push(FsChange::Created(full)),
            a if a == FILE_ACTION_REMOVED => out.push(FsChange::Deleted(full)),
            a if a == FILE_ACTION_RENAMED_OLD_NAME => old_name = Some(full),
            a if a == FILE_ACTION_RENAMED_NEW_NAME => match old_name.take() {
                Some(from) => out.push(FsChange::Renamed { from, to: full }),
                None => out.push(FsChange::Created(full)),
            },
            _ => out.push(FsChange::Modified(full)),
        }
        if next == 0 {
            break;
        }
        offset += next;
    }
    out
}

impl DirWatcher {
    /// Watches `dir` (not recursive). Every batch of changes is sent with
    /// `tag` on `tx` and `msg` is posted to `notify`.
    pub fn spawn(
        dir: PathBuf,
        tag: u64,
        tx: Sender<(u64, Vec<FsChange>)>,
        notify: Hwnd,
        msg: u32,
    ) -> Result<DirWatcher> {
        let handle = open_dir(&dir)?;
        let join = std::thread::Builder::new()
            .name("dir-watch".into())
            .spawn(move || {
                let h = handle;
                let mut buf = vec![0u32; 16 * 1024];
                loop {
                    let mut got = 0u32;
                    // SAFETY: synchronous call; the buffer is DWORD aligned
                    // and its length in bytes is given.
                    let r = unsafe {
                        ReadDirectoryChangesW(
                            h.0,
                            buf.as_mut_ptr() as *mut _,
                            (buf.len() * 4) as u32,
                            false,
                            FILE_NOTIFY_CHANGE_FILE_NAME
                                | FILE_NOTIFY_CHANGE_DIR_NAME
                                | FILE_NOTIFY_CHANGE_LAST_WRITE,
                            Some(&mut got),
                            None,
                            None,
                        )
                    };
                    if r.is_err() {
                        // Cancelled (shutdown) or folder gone.
                        break;
                    }
                    let changes = if got == 0 {
                        vec![FsChange::Overflow]
                    } else {
                        parse(&dir, &buf, got as usize)
                    };
                    if tx.send((tag, changes)).is_err() {
                        break;
                    }
                    notify.post(msg, 0, 0);
                }
            })
            .map_err(|e| WinError::Other(format!("spawn watcher: {e}")))?;
        Ok(DirWatcher { join: Some(join) })
    }
}

impl Drop for DirWatcher {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            let raw = j.as_raw_handle();
            // SAFETY: the thread handle is valid while `j` is alive; this
            // cancels the blocking ReadDirectoryChangesW of that thread.
            // https://learn.microsoft.com/windows/win32/fileio/cancelsynchronousio-func
            let _ = unsafe { CancelSynchronousIo(HANDLE(raw)) };
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn reports_create_rename_delete() {
        let dir = std::env::temp_dir().join(format!("bento-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let w = DirWatcher::spawn(dir.clone(), 7, tx, Hwnd::NULL, 0).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "x").unwrap();
        std::fs::rename(&a, &b).unwrap();
        std::fs::remove_file(&b).unwrap();
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok((tag, ch)) = rx.recv_timeout(Duration::from_millis(200)) {
                assert_eq!(tag, 7);
                all.extend(ch);
            }
            if all.iter().any(|c| matches!(c, FsChange::Deleted(_))) {
                break;
            }
        }
        drop(w);
        let a = a.to_string_lossy().into_owned();
        let b = b.to_string_lossy().into_owned();
        assert!(all.contains(&FsChange::Created(a.clone())), "{all:?}");
        assert!(
            all.contains(&FsChange::Renamed {
                from: a,
                to: b.clone()
            }),
            "{all:?}"
        );
        assert!(all.contains(&FsChange::Deleted(b)), "{all:?}");
    }
}
