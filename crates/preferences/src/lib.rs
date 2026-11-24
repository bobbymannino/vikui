//! Vikui's preferences, saved as JSON in the platform's config folder:
//! `~/Library/Application Support/vikui` on macOS, `$XDG_CONFIG_HOME/vikui` (usually
//! `~/.config/vikui`) on Linux and `%APPDATA%\vikui` on Windows.
//!
//! Loading never fails: a missing or unreadable file gives the defaults, so a damaged
//! file can't stop the app from starting. Reading and writing block; run them on a
//! background task. No UI.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use table_layout::TableLayout;

const FOLDER: &str = "vikui";
const FILE: &str = "preferences.json";

/// Everything the user can set. Fields missing from the file take their defaults, and
/// unknown fields are ignored, so older and newer versions can share one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Look for a newer release when the app opens.
    pub check_for_updates_on_open: bool,
    /// The colour themes and whether to follow the system's light or dark appearance.
    pub theme: ThemePreferences,
    /// What double-clicking a reminder does.
    pub reminder_double_click: DoubleClickAction,
    /// How the user last arranged each table.
    pub tables: Tables,
    /// Where each kind of window was last, by a stable name such as `"main"`.
    pub windows: BTreeMap<String, WindowPlacement>,
    /// The reminders shown from the menu bar (macOS) or notification area (Windows).
    pub tray: TrayPreferences,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            check_for_updates_on_open: true,
            theme: ThemePreferences::default(),
            reminder_double_click: DoubleClickAction::default(),
            tables: Tables::default(),
            windows: BTreeMap::new(),
            tray: TrayPreferences::default(),
        }
    }
}

/// Which colour theme to show.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemePreferences {
    pub appearance: Appearance,
    /// The theme shown while the appearance is light, by name; `None` for the default.
    pub light: Option<String>,
    /// The theme shown while the appearance is dark, by name; `None` for the default.
    pub dark: Option<String>,
}

/// Whether the app looks light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
    /// Follow the system's appearance as it changes.
    #[default]
    Auto,
}

/// The tray icon: whether it shows, and which reminders its menu lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayPreferences {
    pub enabled: bool,
    /// Where its reminders come from; `None` until the user picks one.
    pub source: Option<TraySource>,
    /// List at most this many reminders; `None` for all of them.
    pub limit: Option<u32>,
    /// Show how many upcoming reminders there are beside the icon (macOS) or in its
    /// tooltip (Windows).
    pub show_count: bool,
}

/// A project or saved filter the tray lists reminders from. The title is kept to name
/// it before the projects load (and if it is gone from the server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TraySource {
    /// A project, with its subprojects.
    Project {
        id: i64,
        title: String,
    },
    SavedFilter {
        filter_id: i64,
        title: String,
    },
}

impl TraySource {
    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Self::Project { title, .. } | Self::SavedFilter { title, .. } => title,
        }
    }
}

/// What double-clicking a reminder does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoubleClickAction {
    /// Open the reminder's task in a window of its own.
    #[default]
    OpenDetails,
    /// Edit the task.
    Edit,
    /// Mark the task done, or not done if it is.
    ToggleDone,
    /// Open the task's page in the web browser.
    OpenInBrowser,
}

/// A window's frame, in logical pixels in the desktop's coordinates (which can be
/// negative on a display left of or above the main one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Maximized; the frame is where it goes when restored.
    pub maximized: bool,
}

/// The saved layout of each table: its column order, widths and sort.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tables {
    /// The project list in the main window.
    pub projects: TableLayout,
    /// Every reminders and search window; they share one layout.
    pub reminders: TableLayout,
}

impl Preferences {
    /// The preferences file, or `None` if the platform has no config folder.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join(FOLDER).join(FILE))
    }

    /// The saved preferences, or the defaults if there are none or they can't be read.
    #[must_use]
    pub fn load() -> Self {
        Self::path().map_or_else(Self::default, |path| Self::load_from(&path))
    }

    /// Save to [`Self::path`].
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config folder"))?;
        self.save_to(&path)
    }

    fn load_from(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                log::warn!("couldn't read {}: {err}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|err| {
            log::warn!("ignoring damaged {}: {err}", path.display());
            Self::default()
        })
    }

    /// Write to a temporary file beside `path`, then rename it into place, so a crash
    /// mid-write leaves the old file intact.
    fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        fs::write(&temporary, json + "\n")?;
        fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_damaged_files_give_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(Preferences::load_from(&path), Preferences::default());
        assert!(Preferences::default().check_for_updates_on_open);

        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());
    }

    #[test]
    fn saves_and_loads_creating_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FOLDER).join(FILE);
        let off = Preferences {
            check_for_updates_on_open: false,
            ..Preferences::default()
        };
        off.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), off);
        assert!(!path.with_extension("json.tmp").exists());
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("\"check_for_updates_on_open\": false")
        );
    }

    #[test]
    fn missing_fields_default_and_unknown_fields_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, r#"{"from_a_newer_version": 1}"#).unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());
    }

    #[test]
    fn saves_and_loads_table_layouts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let mut preferences = Preferences::default();
        preferences.tables.reminders = TableLayout {
            columns: vec![table_layout::ColumnLayout {
                key: "due".into(),
                width: 180,
            }],
            sort: Some(table_layout::Sort {
                column: "due".into(),
                direction: table_layout::SortDirection::Descending,
            }),
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);
    }

    #[test]
    fn saves_and_loads_the_tray() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let mut preferences = Preferences::default();
        assert!(!preferences.tray.enabled, "off until the user turns it on");
        preferences.tray = TrayPreferences {
            enabled: true,
            source: Some(TraySource::SavedFilter {
                filter_id: 3,
                title: "This week".into(),
            }),
            limit: Some(5),
            show_count: true,
        };
        preferences.save_to(&path).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(r#""kind": "saved-filter""#), "{saved}");
        assert_eq!(Preferences::load_from(&path), preferences);
    }

    #[test]
    fn files_from_before_table_layouts_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, r#"{"check_for_updates_on_open": false}"#).unwrap();
        let loaded = Preferences::load_from(&path);
        assert!(!loaded.check_for_updates_on_open);
        assert_eq!(loaded.tables, Tables::default());
    }

    #[test]
    fn saves_and_loads_the_theme() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(Preferences::default().theme.appearance, Appearance::Auto);
        let preferences = Preferences {
            theme: ThemePreferences {
                appearance: Appearance::Dark,
                light: Some("Solarized Light".into()),
                dark: None,
            },
            ..Preferences::default()
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("\"appearance\": \"dark\"")
        );
    }

    #[test]
    fn saves_and_loads_window_placements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let preferences = Preferences {
            windows: BTreeMap::from([(
                "main".to_owned(),
                WindowPlacement {
                    x: -1200,
                    y: 40,
                    width: 960,
                    height: 640,
                    maximized: true,
                },
            )]),
            ..Preferences::default()
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);
        assert!(Preferences::default().windows.is_empty());
    }

    #[test]
    fn lives_in_the_config_folder() {
        if let Some(path) = Preferences::path() {
            assert!(path.ends_with(Path::new(FOLDER).join(FILE)));
        }
    }

    #[test]
    fn saves_and_loads_the_double_click_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(
            Preferences::default().reminder_double_click,
            DoubleClickAction::OpenDetails
        );
        let preferences = Preferences {
            reminder_double_click: DoubleClickAction::ToggleDone,
            ..Preferences::default()
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("\"reminder_double_click\": \"toggle-done\"")
        );
        fs::write(&path, r#"{"reminder_double_click": "open-in-browser"}"#).unwrap();
        assert_eq!(
            Preferences::load_from(&path).reminder_double_click,
            DoubleClickAction::OpenInBrowser
        );
    }
}
