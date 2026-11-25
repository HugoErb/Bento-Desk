//! Commands sent to the resident app (by the settings window or a second
//! instance) through `WM_COPYDATA`, encoded as JSON.

use serde::{Deserialize, Serialize};

use crate::model::{FenceId, FenceStyle};

/// Class name of the hidden window of the resident app.
pub const DAEMON_CLASS: &str = "BentoDesk.Daemon";
/// `dwData` value identifying a JSON command.
pub const COPYDATA_KIND: usize = 0xB3_70;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCmd {
    /// `config.json` changed: reload and apply.
    ReloadConfig,
    /// Open (or focus) the settings window.
    OpenSettings,
    /// Style of one fence; `None` = back to the global style.
    SetFenceStyle {
        fence: FenceId,
        style: Option<FenceStyle>,
    },
    /// Same style for every fence.
    SetAllFenceStyles {
        style: Option<FenceStyle>,
    },
    RenameFence {
        fence: FenceId,
        title: String,
    },
    SetFenceOptions {
        fence: FenceId,
        auto_rollup: bool,
        quick_hide_exempt: bool,
    },
    SaveSnapshot {
        name: String,
    },
    RestoreSnapshot {
        name: String,
    },
    /// Apply the rules to every desktop icon now.
    SortNow,
    /// Publish the current desktop items for the rule tester.
    PublishItems,
    RecoverIcons,
    SwapScreens {
        a: String,
        b: String,
    },
    /// Write a troubleshooting folder and open it.
    DebugPackage,
    /// Apply a full import (files already written by the settings window).
    ReloadAll,
    /// Put back Explorer's "auto arrange" and "align to grid" options as
    /// they were before Bento Desk.
    RestoreExplorerOptions,
    /// Quit the resident app.
    Quit,
}

impl IpcCmd {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_bytes(b: &[u8]) -> Option<IpcCmd> {
        serde_json::from_slice(b).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = IpcCmd::RenameFence {
            fence: FenceId(4),
            title: "Jeux".into(),
        };
        assert_eq!(IpcCmd::from_bytes(&c.to_bytes()), Some(c));
        assert_eq!(
            String::from_utf8(IpcCmd::ReloadConfig.to_bytes()).unwrap(),
            r#"{"cmd":"reload_config"}"#
        );
        assert!(IpcCmd::from_bytes(b"nope").is_none());
    }
}
