//! User preferences (`config.json`), shared by the daemon and the settings
//! window.

use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;
use crate::i18n::LangPref;
use crate::model::{FenceStyle, ViewMode};
use crate::rules::Rule;

/// Log verbosity selectable in the Troubleshooting section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub language: LangPref,
    pub start_with_windows: bool,
    /// Show a taskbar button ("Afficher comme tâche en cours").
    pub show_in_taskbar: bool,
    pub log_level: LogLevel,
    /// Executables that pause Bento Desk while in the foreground.
    pub game_mode_exclusions: Vec<String>,
    /// Pause when a full-screen application is in the foreground.
    pub pause_on_fullscreen: bool,
    /// Enable the mouse gestures that need the low-level mouse hook
    /// (draw a fence, double-click to hide, wheel to scroll).
    pub desktop_gestures: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: LangPref::Auto,
            start_with_windows: true,
            show_in_taskbar: false,
            log_level: LogLevel::Info,
            game_mode_exclusions: vec![
                "LeagueClient.exe".into(),
                "League of Legends.exe".into(),
                "VALORANT-Win64-Shipping.exe".into(),
                "RiotClientServices.exe".into(),
            ],
            pause_on_fullscreen: true,
            desktop_gestures: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Style used by fences without their own style.
    pub default_style: FenceStyle,
    /// Follow the Windows accent color for highlights.
    pub use_accent_color: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            default_style: FenceStyle::default(),
            use_accent_color: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenTrigger {
    #[default]
    Click,
    Hover,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RollUp {
    /// Double-click on the title rolls up or unrolls.
    pub double_click_title: bool,
    /// How a rolled-up fence opens temporarily.
    pub open_on: OpenTrigger,
    /// Default value of the per-fence auto roll-up when docked.
    pub auto_rollup_docked: bool,
    /// Smooth height animation when rolling up or unrolling, instead of an
    /// instant change.
    pub animate: bool,
}

impl Default for RollUp {
    fn default() -> Self {
        Self {
            double_click_title: true,
            open_on: OpenTrigger::Click,
            auto_rollup_docked: false,
            animate: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuickHide {
    /// Icons outside fences stay visible.
    pub keep_free_icons: bool,
    /// Hide after this many seconds without desktop activity, 0 = off.
    pub idle_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pages {
    pub enabled: bool,
    /// Switch page when the mouse is pushed against a screen edge.
    pub edge_switch: bool,
    pub show_indicator: bool,
}

impl Default for Pages {
    fn default() -> Self {
        Self {
            enabled: true,
            edge_switch: false,
            show_indicator: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutPrefs {
    pub snap_to_edges: bool,
    pub snap_to_fences: bool,
    /// Snap distance in DIP.
    pub snap_distance: u8,
    /// Gap between snapped fences in DIP.
    pub snap_gap: u8,
    /// Icon spacing inside fences, percent of the desktop spacing.
    pub icon_spacing: u16,
}

impl Default for LayoutPrefs {
    fn default() -> Self {
        Self {
            snap_to_edges: true,
            snap_to_fences: true,
            snap_distance: 12,
            snap_gap: 8,
            icon_spacing: 100,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub peek: Option<Hotkey>,
    pub quick_hide: Option<Hotkey>,
    pub next_page: Option<Hotkey>,
    pub prev_page: Option<Hotkey>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            // Win+Space switches the keyboard layout and Ctrl+Win+Space is
            // taken by Windows; Ctrl+Alt+D ("desktop") is free by default.
            peek: Hotkey::parse("Ctrl+Alt+D"),
            quick_hide: None,
            next_page: Hotkey::parse("Ctrl+Win+PageDown"),
            prev_page: Hotkey::parse("Ctrl+Win+PageUp"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sorting {
    /// Rules in priority order.
    pub rules: Vec<Rule>,
    /// Apply rules to new desktop icons automatically.
    pub auto_apply: bool,
}

impl Default for Sorting {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            auto_apply: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Portals {
    pub default_view: ViewMode,
    /// Hide the folder icon in the title bar of portals.
    pub hide_title_icon: bool,
    pub show_hidden_files: bool,
}

impl Default for Portals {
    fn default() -> Self {
        Self {
            default_view: ViewMode::Icons,
            hide_title_icon: false,
            show_hidden_files: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Peek {
    /// Dim the windows behind the Peek overlay, percent.
    pub dim: u8,
}

impl Default for Peek {
    fn default() -> Self {
        Self { dim: 35 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layouts {
    /// Restore the layout matching the display configuration automatically.
    pub auto_restore: bool,
}

impl Default for Layouts {
    fn default() -> Self {
        Self { auto_restore: true }
    }
}

/// Root of `config.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: General,
    pub appearance: Appearance,
    pub rollup: RollUp,
    pub quick_hide: QuickHide,
    pub pages: Pages,
    pub layout: LayoutPrefs,
    pub hotkeys: Hotkeys,
    pub sorting: Sorting,
    pub portals: Portals,
    pub peek: Peek,
    pub layouts: Layouts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_gives_defaults() {
        let c: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, AppConfig::default());
        assert!(c.general.desktop_gestures);
        assert_eq!(
            c.hotkeys.peek.map(|h| h.to_string()).as_deref(),
            Some("Ctrl+Alt+D")
        );
    }

    #[test]
    fn round_trip() {
        let mut c = AppConfig::default();
        c.quick_hide.idle_seconds = 30;
        let s = serde_json::to_string(&c).unwrap();
        let back: AppConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }
}
