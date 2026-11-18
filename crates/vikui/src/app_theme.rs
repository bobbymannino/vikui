//! Colour themes: the themes GPUI Kit publishes, bundled into the app, and the names the
//! user can pick from.
//!
//! The kit's crates build in only its default light and dark themes; the rest live as
//! JSON in its repository (`themes/` at the release in use) and are copied into
//! `crates/vikui/themes`, under the kit's Apache 2.0 licence.

use gpui_kit::App;
use gpui_kit::component::ThemeRegistry;

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

/// Register the bundled themes. Call once at startup, after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for (file, json) in BUNDLED {
        if let Err(err) = registry.load_themes_from_str(json) {
            log::error!("skipping bundled theme {file}: {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

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
