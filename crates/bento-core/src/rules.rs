//! Ordered rules that file new desktop icons into fences automatically.

use serde::{Deserialize, Serialize};

use crate::items::{ItemInfo, ItemKind};
use crate::model::FenceId;

/// Text comparison, always case-insensitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    #[default]
    Contains,
    StartsWith,
    Equals,
}

impl TextOp {
    pub fn matches(self, haystack: &str, needle: &str) -> bool {
        let h = haystack.to_lowercase();
        let n = needle.trim().to_lowercase();
        if n.is_empty() {
            return false;
        }
        match self {
            TextOp::Contains => h.contains(&n),
            TextOp::StartsWith => h.starts_with(&n),
            TextOp::Equals => h == n,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateField {
    #[default]
    Modified,
    Created,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOp {
    /// Date is within the last N days.
    #[default]
    NewerThanDays,
    /// Date is older than N days.
    OlderThanDays,
}

/// One condition of a rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum Condition {
    Name {
        op: TextOp,
        value: String,
    },
    /// Matches any of the listed extensions (without dot).
    Extension {
        values: Vec<String>,
    },
    Kind {
        kind: ItemKind,
    },
    /// Target of a shortcut or URL file.
    Target {
        op: TextOp,
        value: String,
    },
    Date {
        date: DateField,
        op: DateOp,
        days: u32,
    },
}

impl Condition {
    pub fn matches(&self, item: &ItemInfo, now_secs: u64) -> bool {
        match self {
            Condition::Name { op, value } => op.matches(&item.name, value),
            Condition::Extension { values } => values.iter().any(|v| {
                let v = v.trim().trim_start_matches('.').to_lowercase();
                !v.is_empty() && v == item.extension
            }),
            Condition::Kind { kind } => item.kind == *kind,
            Condition::Target { op, value } => {
                item.target.as_deref().is_some_and(|t| op.matches(t, value))
            }
            Condition::Date { date, op, days } => {
                let ts = match date {
                    DateField::Modified => item.modified,
                    DateField::Created => item.created,
                };
                let Some(ts) = ts else { return false };
                let age_days = now_secs.saturating_sub(ts) / 86_400;
                match op {
                    DateOp::NewerThanDays => age_days < u64::from(*days),
                    DateOp::OlderThanDays => age_days >= u64::from(*days),
                }
            }
        }
    }
}

/// A rule: conditions plus the fence that receives matching items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    /// `true` = all conditions must match, `false` = any.
    pub match_all: bool,
    pub conditions: Vec<Condition>,
    pub target: FenceId,
}

impl Rule {
    pub fn matches(&self, item: &ItemInfo, now_secs: u64) -> bool {
        if !self.enabled || self.conditions.is_empty() {
            return false;
        }
        if self.match_all {
            self.conditions.iter().all(|c| c.matches(item, now_secs))
        } else {
            self.conditions.iter().any(|c| c.matches(item, now_secs))
        }
    }
}

/// First matching rule in priority order (list order).
pub fn first_match<'a>(rules: &'a [Rule], item: &ItemInfo, now_secs: u64) -> Option<&'a Rule> {
    rules.iter().find(|r| r.matches(item, now_secs))
}

/// Items matched by one rule, for the "test this rule" button.
pub fn test_rule<'a>(rule: &Rule, items: &'a [ItemInfo], now_secs: u64) -> Vec<&'a ItemInfo> {
    let mut probe = rule.clone();
    probe.enabled = true;
    items
        .iter()
        .filter(|i| probe.matches(i, now_secs))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> ItemInfo {
        ItemInfo::from_key(&format!(r"C:\D\{name}"), false)
    }

    fn rule(conds: Vec<Condition>, all: bool, target: u64) -> Rule {
        Rule {
            id: target,
            name: String::new(),
            enabled: true,
            match_all: all,
            conditions: conds,
            target: FenceId(target),
        }
    }

    #[test]
    fn text_ops() {
        assert!(TextOp::Contains.matches("Visual Studio", "studio"));
        assert!(TextOp::StartsWith.matches("Visual Studio", "VIS"));
        assert!(!TextOp::StartsWith.matches("Visual Studio", "studio"));
        assert!(TextOp::Equals.matches("Steam", " steam "));
        assert!(!TextOp::Contains.matches("Steam", ""));
    }

    #[test]
    fn extension_and_kind() {
        let i = item("report.PDF");
        let c = Condition::Extension {
            values: vec![".pdf".into(), "docx".into()],
        };
        assert!(c.matches(&i, 0));
        assert!(
            Condition::Kind {
                kind: ItemKind::File
            }
            .matches(&i, 0)
        );
        assert!(
            Condition::Kind {
                kind: ItemKind::Shortcut
            }
            .matches(&item("Game.lnk"), 0)
        );
    }

    #[test]
    fn target_condition() {
        let mut i = item("Game.lnk");
        i.target = Some(r"D:\Games\Riot\launcher.exe".into());
        let c = Condition::Target {
            op: TextOp::Contains,
            value: "riot".into(),
        };
        assert!(c.matches(&i, 0));
        assert!(!c.matches(&item("Other.lnk"), 0));
    }

    #[test]
    fn date_condition() {
        let mut i = item("a.txt");
        let now = 100 * 86_400;
        i.modified = Some(now - 3 * 86_400);
        let newer = Condition::Date {
            date: DateField::Modified,
            op: DateOp::NewerThanDays,
            days: 7,
        };
        let older = Condition::Date {
            date: DateField::Modified,
            op: DateOp::OlderThanDays,
            days: 7,
        };
        assert!(newer.matches(&i, now));
        assert!(!older.matches(&i, now));
        i.created = None;
        let created = Condition::Date {
            date: DateField::Created,
            op: DateOp::NewerThanDays,
            days: 7,
        };
        assert!(!created.matches(&i, now));
    }

    #[test]
    fn priority_and_logic() {
        let rules = vec![
            rule(
                vec![
                    Condition::Name {
                        op: TextOp::Contains,
                        value: "zz".into(),
                    },
                    Condition::Extension {
                        values: vec!["txt".into()],
                    },
                ],
                true,
                1,
            ),
            rule(
                vec![Condition::Extension {
                    values: vec!["txt".into()],
                }],
                false,
                2,
            ),
        ];
        assert_eq!(
            first_match(&rules, &item("zz.txt"), 0).map(|r| r.target),
            Some(FenceId(1))
        );
        assert_eq!(
            first_match(&rules, &item("a.txt"), 0).map(|r| r.target),
            Some(FenceId(2))
        );
        assert!(first_match(&rules, &item("a.doc"), 0).is_none());
    }

    #[test]
    fn disabled_rules_skip_but_test_runs() {
        let mut r = rule(
            vec![Condition::Kind {
                kind: ItemKind::File,
            }],
            true,
            1,
        );
        r.enabled = false;
        let items = vec![item("a.txt"), item("b.lnk")];
        assert!(first_match(std::slice::from_ref(&r), &items[0], 0).is_none());
        assert_eq!(test_rule(&r, &items, 0).len(), 1);
    }

    #[test]
    fn json_shape_is_stable() {
        let c = Condition::Name {
            op: TextOp::StartsWith,
            value: "a".into(),
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"field":"name","op":"starts_with","value":"a"}"#);
    }
}
