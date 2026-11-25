//! Sorting of the items of a fence.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use crate::items::ItemInfo;
use crate::model::{ItemKey, SortKey, SortMode};

/// Case-insensitive comparison where digit runs compare by value
/// ("file2" before "file10"), like Explorer.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().flat_map(char::to_lowercase).peekable();
    let mut bi = b.chars().flat_map(char::to_lowercase).peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Sorts `keys` in place. Unknown items keep their relative order at the end.
/// `Manual` leaves the order untouched.
pub fn sort_items(
    keys: &mut [ItemKey],
    infos: &HashMap<ItemKey, ItemInfo>,
    open_counts: &BTreeMap<ItemKey, u32>,
    mode: SortMode,
) {
    if mode.key == SortKey::Manual {
        return;
    }
    keys.sort_by(|a, b| {
        let (Some(ia), Some(ib)) = (infos.get(a), infos.get(b)) else {
            return infos.contains_key(b).cmp(&infos.contains_key(a));
        };
        let by_name = || natural_cmp(&ia.name, &ib.name);
        let ord = match mode.key {
            SortKey::Manual | SortKey::Name => by_name(),
            SortKey::Type => ia
                .kind
                .rank()
                .cmp(&ib.kind.rank())
                .then_with(|| ia.extension.cmp(&ib.extension))
                .then_with(by_name),
            SortKey::Date => ia.modified.cmp(&ib.modified).then_with(by_name),
            SortKey::Size => ia.size.cmp(&ib.size).then_with(by_name),
            SortKey::Opens => {
                let ca = open_counts.get(a).copied().unwrap_or(0);
                let cb = open_counts.get(b).copied().unwrap_or(0);
                // Most opened first by default.
                cb.cmp(&ca).then_with(by_name)
            }
        };
        if mode.descending { ord.reverse() } else { ord }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn infos(names: &[&str]) -> HashMap<ItemKey, ItemInfo> {
        names
            .iter()
            .map(|n| {
                let i = ItemInfo::from_key(n, false);
                (i.key.clone(), i)
            })
            .collect()
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("File", "file"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
        assert_eq!(natural_cmp("x010", "x9"), Ordering::Greater);
    }

    #[test]
    fn sort_by_name_and_type() {
        let names = ["b.txt", "a.lnk", "c10.txt", "c9.txt"];
        let map = infos(&names);
        let mut keys: Vec<ItemKey> = names.iter().map(|s| (*s).to_owned()).collect();
        sort_items(
            &mut keys,
            &map,
            &BTreeMap::new(),
            SortMode {
                key: SortKey::Name,
                descending: false,
            },
        );
        assert_eq!(keys, vec!["a.lnk", "b.txt", "c9.txt", "c10.txt"]);
        sort_items(
            &mut keys,
            &map,
            &BTreeMap::new(),
            SortMode {
                key: SortKey::Type,
                descending: false,
            },
        );
        assert_eq!(keys[0], "a.lnk");
    }

    #[test]
    fn sort_by_opens_most_first() {
        let names = ["a.txt", "b.txt"];
        let map = infos(&names);
        let mut counts = BTreeMap::new();
        counts.insert("b.txt".to_owned(), 5);
        let mut keys: Vec<ItemKey> = names.iter().map(|s| (*s).to_owned()).collect();
        sort_items(
            &mut keys,
            &map,
            &counts,
            SortMode {
                key: SortKey::Opens,
                descending: false,
            },
        );
        assert_eq!(keys, vec!["b.txt", "a.txt"]);
    }

    #[test]
    fn manual_keeps_order_and_unknown_go_last() {
        let map = infos(&["b.txt"]);
        let mut keys = vec!["zzz".to_owned(), "b.txt".to_owned()];
        sort_items(&mut keys, &map, &BTreeMap::new(), SortMode::default());
        assert_eq!(keys, vec!["zzz", "b.txt"]);
        sort_items(
            &mut keys,
            &map,
            &BTreeMap::new(),
            SortMode {
                key: SortKey::Name,
                descending: false,
            },
        );
        assert_eq!(keys, vec!["b.txt", "zzz"]);
    }
}
