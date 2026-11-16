//! Vikui's settings window.
//!
//! Shows what the app is (its name, version and a link to its source) and lets the user
//! update it. The app supplies [`About`] and the [`UpdateStatus`], and does the checking
//! and installing when the view asks through [`SettingsEvent`]; this crate never fetches.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowOptions, div,
    prelude::FluentBuilder as _,
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

/// Where checking for and installing updates stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Not checked yet.
    #[default]
    Unchecked,
    Checking,
    UpToDate,
    /// A newer version can be installed.
    Available {
        version: SharedString,
    },
    /// Downloading and installing; the app restarts when done.
    Installing {
        version: SharedString,
    },
    Failed {
        message: SharedString,
    },
}

impl UpdateStatus {
    /// The line describing the status.
    fn message(&self) -> SharedString {
        match self {
            Self::Unchecked => "Check whether a newer version is available.".into(),
            Self::Checking => "Checking for updates…".into(),
            Self::UpToDate => "Vikui is up to date.".into(),
            Self::Available { version } => format!("Version {version} is available.").into(),
            Self::Installing { version } => {
                format!("Installing version {version}; Vikui will restart…").into()
            }
            Self::Failed { message } => format!("Couldn't update: {message}").into(),
        }
    }

    fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Installing { .. })
    }
}

/// What the user asked for, for the app to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    /// Check for a newer version, then call [`SettingsView::set_update_status`].
    CheckForUpdates,
    /// Install the available version and restart.
    InstallUpdate,
}

/// The settings view.
pub struct SettingsView {
    about: About,
    version_label: SharedString,
    update: UpdateStatus,
    update_message: SharedString,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(about: About) -> Self {
        let version_label = format!("Version {}", about.version).into();
        let update = UpdateStatus::default();
        Self {
            about,
            version_label,
            update_message: update.message(),
            update,
        }
    }

    pub fn set_update_status(&mut self, status: UpdateStatus, cx: &mut Context<Self>) {
        if self.update != status {
            self.update_message = status.message();
            self.update = status;
            cx.notify();
        }
    }

    #[must_use]
    pub fn update_status(&self) -> &UpdateStatus {
        &self.update
    }

    fn render_updates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, label, event) = match &self.update {
            UpdateStatus::Available { version } | UpdateStatus::Installing { version } => (
                "install-update",
                SharedString::from(format!("Update to {version} and Restart")),
                SettingsEvent::InstallUpdate,
            ),
            _ => (
                "check-for-updates",
                SharedString::from("Check for Updates"),
                SettingsEvent::CheckForUpdates,
            ),
        };
        let busy = self.update.busy();
        v_flex()
            .id("settings-updates")
            .role(Role::Group)
            .aria_label("Updates")
            .test_support()
            .gap_2()
            .pt_4()
            .child(
                div()
                    .id("update-status")
                    .role(Role::Status)
                    .aria_label(self.update_message.clone())
                    .test_support()
                    .when(matches!(self.update, UpdateStatus::Failed { .. }), |line| {
                        line.text_color(cx.theme().danger)
                    })
                    .child(self.update_message.clone()),
            )
            .child(
                h_flex().child(
                    Button::new(id)
                        .primary()
                        .label(label.clone())
                        .accessibility_label(label)
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.update.busy() {
                                cx.emit(event);
                            }
                        })),
                ),
            )
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
            .child(self.render_updates(cx))
    }
}

/// Open the settings window.
pub fn open(
    options: WindowOptions,
    about: About,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<SettingsView>)> {
    gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| SettingsView::new(about)))
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
            open(WindowOptions::default(), about, cx).unwrap().0
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

    #[gpui_kit::test]
    fn checks_for_and_installs_updates(cx: &mut TestAppContext) {
        let about = About {
            name: "Vikui".into(),
            version: "1.2.3".into(),
            repository: "https://example.com/vikui".into(),
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(WindowOptions::default(), about, cx).unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| events.borrow_mut().push(*event))
                .detach();
        });
        let click = |cx: &mut TestAppContext, id: &'static str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(id, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };
        let set = |cx: &mut TestAppContext, status: UpdateStatus| {
            cx.update(|cx| view.update(cx, |view, cx| view.set_update_status(status, cx)));
        };

        assert!(has_label(cx, "Check for Updates"));
        click(cx, "check-for-updates");
        assert_eq!(*events.borrow(), [SettingsEvent::CheckForUpdates]);

        set(cx, UpdateStatus::Checking);
        assert!(has_label(cx, "Checking for updates…"));

        set(
            cx,
            UpdateStatus::Available {
                version: "1.3.0".into(),
            },
        );
        assert!(has_label(cx, "Version 1.3.0 is available."));
        assert!(has_label(cx, "Update to 1.3.0 and Restart"));
        click(cx, "install-update");
        assert_eq!(
            *events.borrow(),
            [SettingsEvent::CheckForUpdates, SettingsEvent::InstallUpdate]
        );

        // Clicking again while installing does nothing.
        set(
            cx,
            UpdateStatus::Installing {
                version: "1.3.0".into(),
            },
        );
        click(cx, "install-update");
        assert_eq!(events.borrow().len(), 2);

        set(
            cx,
            UpdateStatus::Failed {
                message: "no network".into(),
            },
        );
        assert!(has_label(cx, "Couldn't update: no network"));
        assert!(has_label(cx, "Check for Updates"));
    }
}
