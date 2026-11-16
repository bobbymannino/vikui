//! Vikui's settings window.
//!
//! For now it only shows what the app is: its name, version and a link to its source.
//! The app supplies those through [`About`], so this crate knows nothing about the build.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowOptions, div,
};

/// What the settings window shows about the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct About {
    /// The app's display name, e.g. "Vikui".
    pub name: SharedString,
    /// The app's version, e.g. "0.1.0".
    pub version: SharedString,
    /// Where the app's source lives.
    pub repository: SharedString,
}

/// The settings view.
pub struct SettingsView {
    about: About,
    version_label: SharedString,
}

impl SettingsView {
    pub fn new(about: About) -> Self {
        let version_label = format!("Version {}", about.version).into();
        Self {
            about,
            version_label,
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let repository = self.about.repository.clone();
        v_flex()
            .id("settings")
            .role(Role::Region)
            .aria_label("Settings")
            .test_support()
            .size_full()
            .gap_2()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .id("settings-app-name")
                    .role(Role::Heading)
                    .aria_label(self.about.name.clone())
                    .test_support()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.about.name.clone()),
            )
            .child(
                div()
                    .id("settings-app-version")
                    .aria_label(self.version_label.clone())
                    .test_support()
                    .text_color(theme.muted_foreground)
                    .child(self.version_label.clone()),
            )
            .child(
                h_flex().child(
                    Button::new("settings-repository")
                        .link()
                        .label(self.about.repository.clone())
                        .accessibility_label("Open the source repository")
                        .on_click(move |_, _, cx| cx.open_url(&repository)),
                ),
            )
    }
}

/// Open the settings window.
pub fn open(
    options: WindowOptions,
    about: About,
    cx: &mut App,
) -> gpui_kit::Result<AnyWindowHandle> {
    let (handle, _) =
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| SettingsView::new(about)))?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    use super::*;

    #[gpui_kit::test]
    fn shows_the_app_and_opens_the_repository(cx: &mut TestAppContext) {
        let about = About {
            name: "Vikui".into(),
            version: "1.2.3".into(),
            repository: "https://example.com/vikui".into(),
        };
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            open(WindowOptions::default(), about, cx).unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Settings");
            window.find_by_label("Vikui");
            window.find_by_label("Version 1.2.3");
            window.find_by_label("Open the source repository");
            window.click("settings-repository", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.com/vikui")
        );
    }
}
