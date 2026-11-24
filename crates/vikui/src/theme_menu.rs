//! The View menu's Theme submenu: whether the app looks light, dark or follows the
//! system, and the light and dark themes, the same choices the settings window offers.
//! The current choices are checked.

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Menu, MenuItem, SharedString};
use preferences::Appearance;

// The `Action` derive names GPUI as `gpui`; this resolves it through the Kit.
use gpui_kit as gpui;

/// Look light, dark or follow the system.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = vikui, no_json)]
pub struct SetAppearance {
    pub appearance: Appearance,
}

/// Show the named theme while the app looks light.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = vikui, no_json)]
pub struct SetLightTheme {
    pub name: SharedString,
}

/// Show the named theme while the app looks dark.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = vikui, no_json)]
pub struct SetDarkTheme {
    pub name: SharedString,
}

/// Each appearance, in the settings window's order, with its label there.
const APPEARANCES: [(Appearance, &str); 3] = [
    (Appearance::Light, "Light"),
    (Appearance::Dark, "Dark"),
    (Appearance::Auto, "Auto"),
];

/// Register the actions. Call once at startup.
pub fn init(cx: &mut App) {
    cx.on_action(|action: &SetAppearance, cx| {
        let appearance = action.appearance;
        change(cx, |theme| theme.appearance = appearance);
    });
    cx.on_action(|action: &SetLightTheme, cx| {
        let name = action.name.to_string();
        change(cx, |theme| theme.light = Some(name));
    });
    cx.on_action(|action: &SetDarkTheme, cx| {
        let name = action.name.to_string();
        change(cx, |theme| theme.dark = Some(name));
    });
}

/// Change the theme from the menu, and show the change in the settings window if open.
fn change(cx: &mut App, change: impl FnOnce(&mut preferences::ThemePreferences)) {
    crate::app_theme::change(cx, change);
    crate::settings_window::show_theme(cx);
}

/// The Theme submenu, checking the current appearance and themes.
pub fn submenu(cx: &App) -> MenuItem {
    let current = crate::app_preferences::get(cx).theme.appearance;
    let (light, dark) = crate::app_theme::current(cx);
    let appearances = APPEARANCES.map(|(appearance, label)| {
        MenuItem::action(label, SetAppearance { appearance }).checked(appearance == current)
    });
    let themes = |mode: ThemeMode, current: &SharedString| -> Vec<MenuItem> {
        crate::app_theme::names(mode, cx)
            .into_iter()
            .map(|name| {
                let checked = name == *current;
                let label = name.clone();
                match mode {
                    ThemeMode::Light => MenuItem::action(label, SetLightTheme { name }),
                    ThemeMode::Dark => MenuItem::action(label, SetDarkTheme { name }),
                }
                .checked(checked)
            })
            .collect()
    };
    MenuItem::submenu(Menu::new("Theme").items(appearances.into_iter().chain([
        MenuItem::separator(),
        MenuItem::submenu(Menu::new("Light Theme").items(themes(ThemeMode::Light, &light))),
        MenuItem::submenu(Menu::new("Dark Theme").items(themes(ThemeMode::Dark, &dark))),
    ])))
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::component::Theme;

    use super::*;

    /// The submenu's entries: `(name, checked)`, nested submenus by their name.
    fn entries(menu: &Menu) -> Vec<(&str, bool)> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, checked, .. } => Some((name.as_ref(), *checked)),
                MenuItem::Submenu(menu) => Some((menu.name.as_ref(), false)),
                _ => None,
            })
            .collect()
    }

    fn submenu_named<'a>(menu: &'a Menu, name: &str) -> &'a Menu {
        menu.items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(menu) if menu.name.as_ref() == name => Some(menu),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name} submenu"))
    }

    #[gpui_kit::test]
    fn checks_and_changes_the_appearance_and_themes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::app_theme::init(cx);
            init(cx);
            let MenuItem::Submenu(menu) = submenu(cx) else {
                panic!("not a submenu");
            };
            assert_eq!(menu.name.as_ref(), "Theme");
            assert_eq!(
                entries(&menu),
                [
                    ("Light", false),
                    ("Dark", false),
                    ("Auto", true),
                    ("Light Theme", false),
                    ("Dark Theme", false),
                ]
            );
            let light = entries(submenu_named(&menu, "Light Theme"));
            assert_eq!(light.first(), Some(&("Default Light", true)));
            assert!(light.contains(&("Solarized Light", false)));
            let dark = entries(submenu_named(&menu, "Dark Theme"));
            assert_eq!(dark.first(), Some(&("Default Dark", true)));
            assert!(!dark.iter().any(|(name, _)| *name == "Solarized Light"));

            cx.dispatch_action(&SetAppearance {
                appearance: Appearance::Dark,
            });
            cx.dispatch_action(&SetDarkTheme {
                name: "Gruvbox Dark".into(),
            });
            cx.dispatch_action(&SetLightTheme {
                name: "Solarized Light".into(),
            });
            assert_eq!(
                crate::app_preferences::get(cx).theme,
                preferences::ThemePreferences {
                    appearance: Appearance::Dark,
                    light: Some("Solarized Light".into()),
                    dark: Some("Gruvbox Dark".into()),
                }
            );
            assert_eq!(Theme::global(cx).theme_name(), "Gruvbox Dark");

            let MenuItem::Submenu(menu) = submenu(cx) else {
                panic!("not a submenu");
            };
            assert!(entries(&menu).contains(&("Dark", true)));
            assert!(entries(&menu).contains(&("Auto", false)));
            assert!(entries(submenu_named(&menu, "Dark Theme")).contains(&("Gruvbox Dark", true)));
            assert!(
                entries(submenu_named(&menu, "Light Theme")).contains(&("Solarized Light", true))
            );
        });
    }
}
