//! Vikui's tray icon: in the menu bar on macOS and the notification area on Windows,
//! with a menu the app fills. No GPUI, no network.
//!
//! The app builds the menu as a list of [`Entry`]s and reacts to the [`Command`]s that
//! picking them sends on [`Tray::commands`]. Linux would need GTK running on a thread of
//! its own, so there [`is_supported`] is false and [`Tray::new`] fails.
//!
//! The icon must be created, changed and dropped on the main thread, after the app's
//! event loop has started.

pub mod icon;

use std::fmt;

/// Whether this platform has a tray icon.
#[must_use]
pub const fn is_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// What picking a menu entry asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show this task.
    OpenTask(i64),
    /// Open the list the tray's reminders come from.
    OpenList,
    /// Bring the app's main window to the front.
    OpenApp,
    Refresh,
    OpenSettings,
    Quit,
}

// Only the tray's menu reads ids, so other platforms need these just for the tests.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl Command {
    /// The menu item id that carries the command.
    fn id(self) -> String {
        match self {
            Self::OpenTask(task_id) => format!("task:{task_id}"),
            Self::OpenList => "open-list".into(),
            Self::OpenApp => "open-app".into(),
            Self::Refresh => "refresh".into(),
            Self::OpenSettings => "settings".into(),
            Self::Quit => "quit".into(),
        }
    }

    /// The command a menu item id carries, if any.
    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "open-list" => Self::OpenList,
            "open-app" => Self::OpenApp,
            "refresh" => Self::Refresh,
            "settings" => Self::OpenSettings,
            "quit" => Self::Quit,
            _ => Self::OpenTask(id.strip_prefix("task:")?.parse().ok()?),
        })
    }
}

/// One line of the tray's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Text that can't be picked, such as a heading or "No reminders".
    Text(String),
    Item {
        label: String,
        command: Command,
    },
    Separator,
}

/// Why the tray icon couldn't be shown.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The tray icon; dropping it removes it.
pub struct Tray {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    icon: tray_icon::TrayIcon,
    commands: async_channel::Receiver<Command>,
}

impl Tray {
    /// Show the icon with an empty menu, hovering `tooltip`.
    ///
    /// # Errors
    /// If the platform has no tray (see [`is_supported`]) or refuses the icon.
    pub fn new(tooltip: &str) -> Result<Self, Error> {
        let (sender, commands) = async_channel::unbounded();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let icon = imp::new(tooltip, sender)?;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        imp::new(tooltip, sender)?;
        Ok(Self {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            icon,
            commands,
        })
    }

    /// What the user picks from the menu.
    #[must_use]
    pub fn commands(&self) -> async_channel::Receiver<Command> {
        self.commands.clone()
    }

    /// Replace the menu with `entries`.
    #[cfg_attr(
        not(any(target_os = "macos", target_os = "windows")),
        expect(clippy::unused_self, reason = "there is no tray icon to change here")
    )]
    pub fn set_menu(&self, entries: &[Entry]) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        imp::set_menu(&self.icon, entries);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = entries;
    }

    /// Show `count` beside the icon (macOS) and `tooltip` when hovering it; `None` shows
    /// no count.
    #[cfg_attr(
        not(any(target_os = "macos", target_os = "windows")),
        expect(clippy::unused_self, reason = "there is no tray icon to change here")
    )]
    pub fn set_count(&self, count: Option<usize>, tooltip: &str) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        imp::set_count(&self.icon, count, tooltip);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (count, tooltip);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::{Command, Entry, Error, icon};

    /// Pixels: the menu bar is 22pt high, drawn at 2x on Retina displays; Windows scales
    /// the 32px icon to the notification area.
    #[cfg(target_os = "macos")]
    const SIZE: u32 = 36;
    #[cfg(target_os = "windows")]
    const SIZE: u32 = 32;

    /// macOS tints a template image to the menu bar; Windows shows the app's colours.
    #[cfg(target_os = "macos")]
    const STYLE: icon::Style = icon::Style::Template;
    #[cfg(target_os = "windows")]
    const STYLE: icon::Style = icon::Style::Colour;

    pub fn new(tooltip: &str, sender: async_channel::Sender<Command>) -> Result<TrayIcon, Error> {
        let image = Icon::from_rgba(icon::draw(STYLE, SIZE), SIZE, SIZE)
            .map_err(|err| Error(format!("bad tray icon: {err}")))?;
        // Menu events come on another thread on some platforms; the channel brings them
        // to the app's.
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = Command::from_id(&event.id.0) {
                // Only fails once the tray is gone, when nobody is listening.
                let _ = sender.try_send(command);
            }
        }));
        // Only macOS has template images.
        #[cfg(target_os = "macos")]
        let builder = TrayIconBuilder::new().with_icon_templated(image);
        #[cfg(target_os = "windows")]
        let builder = TrayIconBuilder::new().with_icon(image);
        builder
            .with_tooltip(tooltip)
            .with_menu(Box::new(Menu::new()))
            .with_menu_on_left_click(true)
            .build()
            .map_err(|err| Error(format!("couldn't show the tray icon: {err}")))
    }

    pub fn set_menu(icon: &TrayIcon, entries: &[Entry]) {
        let menu = Menu::new();
        for entry in entries {
            let appended = match entry {
                Entry::Text(text) => menu.append(&MenuItem::new(text, false, None)),
                Entry::Item { label, command } => {
                    menu.append(&MenuItem::with_id(command.id(), label, true, None))
                }
                Entry::Separator => menu.append(&PredefinedMenuItem::separator()),
            };
            if let Err(err) = appended {
                log::warn!("tray menu entry {entry:?} not added: {err}");
            }
        }
        icon.set_menu(Some(Box::new(menu)));
    }

    pub fn set_count(icon: &TrayIcon, count: Option<usize>, tooltip: &str) {
        // Only macOS shows a title beside the icon; elsewhere this does nothing.
        icon.set_title(count.map(|count| count.to_string()));
        if let Err(err) = icon.set_tooltip(Some(tooltip)) {
            log::warn!("tray tooltip not set: {err}");
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::{Command, Error};

    pub fn new(_: &str, _: async_channel::Sender<Command>) -> Result<(), Error> {
        Err(Error("this platform has no tray icon".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_through_menu_ids() {
        for command in [
            Command::OpenTask(42),
            Command::OpenList,
            Command::OpenApp,
            Command::Refresh,
            Command::OpenSettings,
            Command::Quit,
        ] {
            assert_eq!(Command::from_id(&command.id()), Some(command));
        }
        assert_eq!(Command::from_id("task:x"), None);
        assert_eq!(Command::from_id("something-else"), None);
    }

    #[test]
    fn supported_on_macos_and_windows_only() {
        assert_eq!(
            is_supported(),
            cfg!(target_os = "macos") || cfg!(target_os = "windows")
        );
    }
}
