//! Colour themes: the themes GPUI Kit publishes, bundled into the app; the user's light
//! and dark theme; and whether the app is light, dark or follows the system.
//!
//! The kit's crates build in only its default light and dark themes; the rest live as
//! JSON in its repository (`themes/` at the release in use) and are copied into
//! `crates/vikui/themes`, under the kit's Apache 2.0 licence.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString, Window};
use preferences::{Appearance, ThemePreferences};

/// Each bundled theme file, by name.
const BUNDLED: [(&str, &str); 21] = [
    ("adventure.json", include_str!("../themes/adventure.json")),
    ("alduin.json", include_str!("../themes/alduin.json")),
    ("asciinema.json", include_str!("../themes/asciinema.json")),
    ("aurora.json", include_str!("../themes/aurora.json")),
    ("ayu.json", include_str!("../themes/ayu.json")),
    ("catppuccin.json", include_str!("../themes/catppuccin.json")),
    ("everforest.json", include_str!("../themes/everforest.json")),
    ("fahrenheit.json", include_str!("../themes/fahrenheit.json")),
    ("flexoki.json", include_str!("../themes/flexoki.json")),
    ("gruvbox.json", include_str!("../themes/gruvbox.json")),
    ("harper.json", include_str!("../themes/harper.json")),
    ("hybrid.json", include_str!("../themes/hybrid.json")),
    ("jellybeans.json", include_str!("../themes/jellybeans.json")),
    ("kibble.json", include_str!("../themes/kibble.json")),
    (
        "macos-classic.json",
        include_str!("../themes/macos-classic.json"),
    ),
    (
        "mellifluous.json",
        include_str!("../themes/mellifluous.json"),
    ),
    ("molokai.json", include_str!("../themes/molokai.json")),
    ("solarized.json", include_str!("../themes/solarized.json")),
    ("spaceduck.json", include_str!("../themes/spaceduck.json")),
    ("tokyonight.json", include_str!("../themes/tokyonight.json")),
    ("twilight.json", include_str!("../themes/twilight.json")),
];

/// Register the bundled themes and follow the system's appearance while the preference
/// is [`Appearance::Auto`]. Call once at startup, after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for (file, json) in BUNDLED {
        if let Err(err) = registry.load_themes_from_str(json) {
            log::error!("skipping bundled theme {file}: {err:#}");
        }
    }
    // Every window is wrapped in a kit `Root`; the system reports appearance changes to
    // each window, not to the app.
    cx.observe_new(|_: &mut gpui_kit::base::Root, window, _| {
        if let Some(window) = window {
            window
                .observe_window_appearance(|window, cx| {
                    if crate::app_preferences::get(cx).theme.appearance == Appearance::Auto {
                        Theme::sync_system_appearance(Some(window), cx);
                    }
                })
                .detach();
        }
    })
    .detach();
}

/// Show the themes and appearance `preferences` name. A theme that isn't registered
/// (removed since it was saved) falls back to the kit's default for its mode.
pub fn apply(preferences: &ThemePreferences, cx: &mut App) {
    let light = find(preferences.light.as_deref(), ThemeMode::Light, cx);
    let dark = find(preferences.dark.as_deref(), ThemeMode::Dark, cx);
    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
    let mode = match preferences.appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Auto => cx.window_appearance().into(),
    };
    // Reloads the theme for `mode` even if it is current, and refreshes every window.
    Theme::change(mode, None::<&mut Window>, cx);
}

/// Save a change to the theme preferences, show it, and check it in the menu bar.
pub fn change(cx: &mut App, change: impl FnOnce(&mut ThemePreferences)) {
    crate::app_preferences::update(cx, |preferences| change(&mut preferences.theme));
    let theme = crate::app_preferences::get(cx).theme;
    apply(&theme, cx);
    crate::app_menu::refresh(cx);
}

/// The light and dark themes currently shown, by name.
pub fn current(cx: &App) -> (SharedString, SharedString) {
    let theme = Theme::global(cx);
    (
        theme.light_theme.name.clone(),
        theme.dark_theme.name.clone(),
    )
}

fn find(name: Option<&str>, mode: ThemeMode, cx: &App) -> Rc<ThemeConfig> {
    let registry = ThemeRegistry::global(cx);
    let theme = name
        .and_then(|name| registry.themes().get(name))
        .filter(|theme| theme.mode == mode)
        .unwrap_or_else(|| match mode {
            ThemeMode::Light => registry.default_light_theme(),
            ThemeMode::Dark => registry.default_dark_theme(),
        });
    Rc::clone(theme)
}

/// The registered themes for `mode`, the kit's default first and the rest by name.
pub fn names(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    let registry = ThemeRegistry::global(cx);
    let default = match mode {
        ThemeMode::Light => registry.default_light_theme(),
        ThemeMode::Dark => registry.default_dark_theme(),
    }
    .name
    .clone();
    let mut others: Vec<SharedString> = registry
        .themes()
        .values()
        .filter(|theme| theme.mode == mode && theme.name != default)
        .map(|theme| theme.name.clone())
        .collect();
    others.sort_by_key(|name| name.to_lowercase());
    std::iter::once(default).chain(others).collect()
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    fn preferences(appearance: Appearance, light: &str, dark: &str) -> ThemePreferences {
        ThemePreferences {
            appearance,
            light: Some(light.to_owned()),
            dark: Some(dark.to_owned()),
        }
    }

    #[gpui_kit::test]
    fn applies_the_chosen_themes_and_appearance(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            apply(
                &preferences(Appearance::Dark, "Solarized Light", "Gruvbox Dark"),
                cx,
            );
            assert!(Theme::global(cx).is_dark());
            assert_eq!(Theme::global(cx).theme_name(), "Gruvbox Dark");
            assert_eq!(
                current(cx),
                ("Solarized Light".into(), "Gruvbox Dark".into())
            );

            apply(
                &preferences(Appearance::Light, "Solarized Light", "Gruvbox Dark"),
                cx,
            );
            assert!(!Theme::global(cx).is_dark());
            assert_eq!(Theme::global(cx).theme_name(), "Solarized Light");

            // Unknown names, and a dark theme named for light, fall back to the defaults.
            apply(&preferences(Appearance::Light, "Gone", "Gone"), cx);
            assert_eq!(current(cx), ("Default Light".into(), "Default Dark".into()));
            apply(&preferences(Appearance::Light, "Gruvbox Dark", "Gone"), cx);
            assert_eq!(Theme::global(cx).theme_name(), "Default Light");
        });
    }

    #[gpui_kit::test]
    fn lists_themes_by_mode_default_first(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let light = names(ThemeMode::Light, cx);
            let dark = names(ThemeMode::Dark, cx);
            assert_eq!(light.first().map(AsRef::as_ref), Some("Default Light"));
            assert_eq!(dark.first().map(AsRef::as_ref), Some("Default Dark"));
            assert!(light.iter().any(|name| name == "Solarized Light"));
            assert!(dark.iter().any(|name| name == "Catppuccin Mocha"));
            assert!(!light.iter().any(|name| name == "Catppuccin Mocha"));
            let rest: Vec<_> = dark
                .iter()
                .skip(1)
                .map(|name| name.to_lowercase())
                .collect();
            assert!(rest.is_sorted(), "{rest:?}");
        });
    }

    #[gpui_kit::test]
    fn registers_every_bundled_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for (file, json) in BUNDLED {
                let mut registry = ThemeRegistry::default();
                assert!(registry.load_themes_from_str(json).is_ok(), "{file} parses");
            }
            init(cx);
            let themes = ThemeRegistry::global(cx).themes();
            for name in [
                "Default Light",
                "Solarized Light",
                "Catppuccin Mocha",
                "Tokyo Night",
            ] {
                assert!(themes.contains_key(name), "{name} is registered");
            }
        });
    }
}
