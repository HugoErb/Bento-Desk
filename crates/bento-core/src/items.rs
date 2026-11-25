//! Description of a desktop or folder item, used by rules and sorting.

use serde::{Deserialize, Serialize};

use crate::model::ItemKey;

/// Coarse item type, as offered in the rule editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Shortcut,
    Folder,
    #[default]
    File,
    Url,
    /// Virtual shell items such as the Recycle Bin.
    Special,
}

impl ItemKind {
    /// Sort rank used by "sort by type": folders first, then shortcuts.
    pub fn rank(self) -> u8 {
        match self {
            ItemKind::Special => 0,
            ItemKind::Folder => 1,
            ItemKind::Shortcut => 2,
            ItemKind::Url => 3,
            ItemKind::File => 4,
        }
    }
}

/// Everything rules and sorting need to know about an item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemInfo {
    pub key: ItemKey,
    /// Name shown under the icon.
    pub name: String,
    /// Lower-case extension without the dot, empty for folders.
    pub extension: String,
    pub kind: ItemKind,
    /// Target path of a `.lnk` shortcut or URL of a `.url` file.
    pub target: Option<String>,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
    pub created: Option<u64>,
    pub size: u64,
}

impl ItemInfo {
    /// Builds the parts that can be derived from the key alone.
    pub fn from_key(key: &str, is_dir: bool) -> ItemInfo {
        if key.starts_with("::") {
            return ItemInfo {
                key: key.to_owned(),
                name: key.to_owned(),
                kind: ItemKind::Special,
                ..ItemInfo::default()
            };
        }
        let file_name = key.rsplit(['\\', '/']).next().unwrap_or(key);
        let (stem, ext) = match file_name.rsplit_once('.') {
            Some((s, e)) if !is_dir && !s.is_empty() => (s, e.to_ascii_lowercase()),
            _ => (file_name, String::new()),
        };
        let kind = if is_dir {
            ItemKind::Folder
        } else {
            match ext.as_str() {
                "lnk" => ItemKind::Shortcut,
                "url" => ItemKind::Url,
                _ => ItemKind::File,
            }
        };
        let name = match kind {
            ItemKind::Shortcut | ItemKind::Url => stem.to_owned(),
            _ => file_name.to_owned(),
        };
        ItemInfo {
            key: key.to_owned(),
            name,
            extension: ext,
            kind,
            ..ItemInfo::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_kind_and_name() {
        let i = ItemInfo::from_key(r"C:\Users\a\Desktop\Steam.lnk", false);
        assert_eq!(i.kind, ItemKind::Shortcut);
        assert_eq!(i.name, "Steam");
        assert_eq!(i.extension, "lnk");
        let f = ItemInfo::from_key(r"C:\Users\a\Desktop\Notes.TXT", false);
        assert_eq!(f.kind, ItemKind::File);
        assert_eq!(f.extension, "txt");
        assert_eq!(f.name, "Notes.TXT");
        let d = ItemInfo::from_key(r"C:\Users\a\Desktop\Projects.v2", true);
        assert_eq!(d.kind, ItemKind::Folder);
        assert_eq!(d.extension, "");
        let s = ItemInfo::from_key("::{645FF040-5081-101B-9F08-00AA002F954E}", false);
        assert_eq!(s.kind, ItemKind::Special);
        let dot = ItemInfo::from_key(r"C:\x\.gitignore", false);
        assert_eq!(dot.extension, "");
    }
}
