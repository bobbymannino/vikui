//! The application menu bar.
//!
//! Items dispatch the app-wide actions from [`crate::app_actions`] or the text-editing
//! actions of the focused input, so the menus show the same shortcuts the keys use.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, Menu, MenuItem, OsAction};

use crate::app_actions::{
    CheckForUpdates, CloseWindow, Minimize, NewReminder, OpenProjects, OpenSettings, Quit, Refresh,
    ReportBug, RequestFeature, SearchAllTasks, Zoom,
};
#[cfg(target_os = "macos")]
use crate::app_actions::{Hide, HideOthers, ShowAll};

/// Install the menu bar. Call after [`crate::app_actions::init`] and
/// [`crate::projects_menu::init`], so the menus pick up their key bindings, and after
/// [`crate::app_theme::init`], so the Theme submenu lists every theme.
pub fn init(cx: &mut App) {
    refresh(cx);
}

/// Rebuild the menu bar, e.g. after the projects listed in it change.
pub fn refresh(cx: &mut App) {
    let projects = crate::projects_menu::items(cx);
    let theme = crate::theme_menu::submenu(cx);
    cx.set_menus(menus(projects, theme));
}

fn menus(projects: Vec<MenuItem>, theme: MenuItem) -> Vec<Menu> {
    vec![
        app_menu(),
        Menu::new("File").items([
            MenuItem::action("New Reminder…", NewReminder),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Search All Tasks…", SearchAllTasks),
        ]),
        Menu::new("View").items([
            MenuItem::action("Refresh", Refresh),
            MenuItem::separator(),
            theme,
        ]),
        Menu::new("Projects").items(projects),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::separator(),
            MenuItem::action("Open Projects Window", OpenProjects),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Report a Bug…", ReportBug),
            MenuItem::action("Request a Feature…", RequestFeature),
        ]),
    ]
}

/// The menu named after the app; macOS adds its own Services and hiding conventions.
#[cfg(target_os = "macos")]
fn app_menu() -> Menu {
    Menu::new("Vikui").items([
        MenuItem::action("Settings…", OpenSettings),
        MenuItem::action("Check for Updates…", CheckForUpdates),
        MenuItem::separator(),
        MenuItem::os_submenu("Services", gpui_kit::SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("Hide Vikui", Hide),
        MenuItem::action("Hide Others", HideOthers),
        MenuItem::action("Show All", ShowAll),
        MenuItem::separator(),
        MenuItem::action("Quit Vikui", Quit),
    ])
}

#[cfg(not(target_os = "macos"))]
fn app_menu() -> Menu {
    Menu::new("Vikui").items([
        MenuItem::action("Settings…", OpenSettings),
        MenuItem::action("Check for Updates…", CheckForUpdates),
        MenuItem::separator(),
        MenuItem::action("Quit", Quit),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(menu: &Menu) -> Vec<&str> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.as_ref()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn has_the_standard_menus() {
        let menus = menus(Vec::new(), MenuItem::submenu(Menu::new("Theme")));
        let titles: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(
            titles,
            [
                "Vikui", "File", "Edit", "View", "Projects", "Window", "Help"
            ]
        );
        assert!(menus.iter().any(|menu| names(menu).contains(&"Refresh")));
        assert!(
            menus
                .iter()
                .find(|menu| menu.name.as_ref() == "Edit")
                .is_some_and(|menu| names(menu).contains(&"Search All Tasks…"))
        );
        let menu = |title: &str| menus.iter().find(|menu| menu.name.as_ref() == title);
        assert!(menu("Window").is_some_and(|menu| names(menu).contains(&"Open Projects Window")));
        assert!(
            menu("Help")
                .is_some_and(|menu| { names(menu) == ["Report a Bug…", "Request a Feature…"] })
        );
        assert!(
            menus
                .first()
                .is_some_and(|menu| names(menu).contains(&"Settings…"))
        );
        assert!(
            menus
                .first()
                .is_some_and(|menu| names(menu).contains(&"Check for Updates…"))
        );
        assert!(
            menus
                .first()
                .is_some_and(|menu| names(menu).iter().any(|name| name.starts_with("Quit")))
        );
    }
}
