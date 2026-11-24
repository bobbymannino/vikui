//! Vikui's settings window.
//!
//! One tab per [`SettingsTab`]: General lets the user pick what happens at login, Appearance
//! pick the appearance and colour
//! themes, Reminders what double-clicking a reminder does, Updates lets them update the
//! app, and About shows what the app is (its name, version and a link to its source) and
//! links to report a bug or request a feature. The
//! app supplies [`About`], the [`ThemeChoices`], the [`DoubleClickAction`] and the
//! [`UpdateStatus`], what it does at login ([`StartAtLogin`]) (and, for the Tray tab, [`TraySettingsView`]'s choices), and applies
//! and saves changes, checks and installs when the view asks
//! through [`SettingsEvent`]; this crate never fetches or saves. The app can bring a tab
//! forward with [`SettingsView::set_tab`], e.g. Updates when one is available.

mod tray_settings;

pub use tray_settings::{
    TraySettings, TraySettingsEvent, TraySettingsView, TraySourceChoice, TraySourceId,
};

use gpui_kit::base::Selectable as _;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowOptions, div,
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
    /// A new bug report, with what the report needs about this build filled in.
    pub bug_report: SharedString,
    /// A new feature request.
    pub feature_request: SharedString,
}

/// Whether the app looks light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
    /// Follow the system's appearance.
    #[default]
    Auto,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::Auto];

    fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Auto => "Auto",
        }
    }
}

/// The appearance and themes the user picked, and the themes they can pick from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeChoices {
    pub appearance: Appearance,
    /// The theme shown while light; one of `light_themes`.
    pub light: SharedString,
    /// The theme shown while dark; one of `dark_themes`.
    pub dark: SharedString,
    pub light_themes: Vec<SharedString>,
    pub dark_themes: Vec<SharedString>,
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

/// What double-clicking a reminder does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoubleClickAction {
    /// Open its task in a window of its own.
    #[default]
    OpenDetails,
    Edit,
    /// Mark its task done, or not done if it is.
    ToggleDone,
    /// Open its task's page in the web browser.
    OpenInBrowser,
}

impl DoubleClickAction {
    pub const ALL: [Self; 4] = [
        Self::OpenDetails,
        Self::Edit,
        Self::ToggleDone,
        Self::OpenInBrowser,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::OpenDetails => "Open details in a new window",
            Self::Edit => "Edit the task",
            Self::ToggleDone => "Mark as done or not done",
            Self::OpenInBrowser => "Open in browser",
        }
    }
}

/// What the app does when the user logs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartupAction {
    /// Nothing: the app doesn't start at login.
    #[default]
    DoNotOpen,
    /// Start and show the main window.
    OpenWindow,
    /// Start with only the tray icon showing.
    OpenTray,
}

impl StartupAction {
    /// The choices this platform has, in the order shown; the tray icon needs macOS or
    /// Windows (see the `tray` crate).
    #[must_use]
    pub fn all() -> &'static [Self] {
        if tray_settings::SUPPORTED {
            &[Self::DoNotOpen, Self::OpenWindow, Self::OpenTray]
        } else {
            &[Self::DoNotOpen, Self::OpenWindow]
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::DoNotOpen => "Do not open at login",
            Self::OpenWindow => "Open the main window",
            Self::OpenTray => "Open only the tray icon",
        }
    }
}

/// What the app does at login, as far as is known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StartAtLogin {
    /// Not read from the system yet, or being changed.
    #[default]
    Unknown,
    Known(StartupAction),
    /// Reading or changing it failed; it stands at `action` as far as is known.
    Failed {
        action: StartupAction,
        message: SharedString,
    },
}

impl StartAtLogin {
    fn action(&self) -> Option<StartupAction> {
        match self {
            Self::Unknown => None,
            Self::Known(action) | Self::Failed { action, .. } => Some(*action),
        }
    }
}

/// A tab of the settings window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Reminders,
    Tray,
    Updates,
    About,
}

impl SettingsTab {
    /// In the order the tab bar shows them.
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Appearance,
        Self::Reminders,
        Self::Tray,
        Self::Updates,
        Self::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Reminders => "Reminders",
            Self::Tray => "Tray",
            Self::Updates => "Updates",
            Self::About => "About",
        }
    }

    /// The accessible name of the tab's panel.
    fn panel_label(self) -> &'static str {
        match self {
            Self::General => "General settings",
            Self::Appearance => "Appearance settings",
            Self::Reminders => "Reminder settings",
            Self::Tray => "Tray settings",
            Self::Updates => "Update settings",
            Self::About => "App information",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

/// What the user asked for, for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    /// The user picked how the app looks; apply and save it.
    SetAppearance(Appearance),
    /// The user picked the theme shown while light, by name; apply and save it.
    SetLightTheme(SharedString),
    /// The user picked the theme shown while dark, by name; apply and save it.
    SetDarkTheme(SharedString),
    /// Check for a newer version, then call [`SettingsView::set_update_status`].
    CheckForUpdates,
    /// Install the available version and restart.
    InstallUpdate,
    /// The user turned checking for updates when the app opens on or off; save it.
    SetCheckForUpdatesOnOpen(bool),
    /// The user picked what double-clicking a reminder does; apply and save it.
    SetReminderDoubleClick(DoubleClickAction),
    /// The user changed the tray's settings; apply and save them.
    SetTray(TraySettings),
    /// The user picked what the app does at login; change it, then call
    /// [`SettingsView::set_start_at_login`] with how it stands.
    SetStartAtLogin(StartupAction),
}

/// The settings view.
pub struct SettingsView {
    tab: SettingsTab,
    double_click: DoubleClickAction,
    about: About,
    version_label: SharedString,
    update: UpdateStatus,
    update_message: SharedString,
    check_on_open: bool,
    appearance: Appearance,
    light_theme: Entity<ThemeSelect>,
    dark_theme: Entity<ThemeSelect>,
    tray: Entity<TraySettingsView>,
    start_at_login: StartAtLogin,
    _subscriptions: [Subscription; 3],
}

type ThemeSelect = SelectState<Vec<SharedString>>;

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    /// `check_on_open` is the saved "Check for updates on open" preference and `theme`
    /// the saved appearance and themes.
    pub fn new(
        about: About,
        check_on_open: bool,
        theme: ThemeChoices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let version_label = format!("Version {}", about.version).into();
        let update = UpdateStatus::default();
        let select = |names: Vec<SharedString>,
                      picked: &SharedString,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            let selected = names.iter().position(|name| name == picked);
            cx.new(|cx| SelectState::new(names, selected.map(IndexPath::new), window, cx))
        };
        let light_theme = select(theme.light_themes, &theme.light, window, cx);
        let dark_theme = select(theme.dark_themes, &theme.dark, window, cx);
        let theme_events = [
            cx.subscribe(
                &light_theme,
                |_, _, event: &SelectEvent<Vec<SharedString>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        cx.emit(SettingsEvent::SetLightTheme(name.clone()));
                    }
                },
            ),
            cx.subscribe(
                &dark_theme,
                |_, _, event: &SelectEvent<Vec<SharedString>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        cx.emit(SettingsEvent::SetDarkTheme(name.clone()));
                    }
                },
            ),
        ];
        let tray = cx.new(|cx| TraySettingsView::new(TraySettings::default(), window, cx));
        let [light_events, dark_events] = theme_events;
        let tray_events = cx.subscribe(&tray, |_, _, event: &TraySettingsEvent, cx| {
            cx.emit(SettingsEvent::SetTray(event.0.clone()));
        });
        Self {
            tab: SettingsTab::default(),
            double_click: DoubleClickAction::default(),
            about,
            version_label,
            update_message: update.message(),
            update,
            check_on_open,
            appearance: theme.appearance,
            light_theme,
            dark_theme,
            tray,
            start_at_login: StartAtLogin::default(),
            _subscriptions: [light_events, dark_events, tray_events],
        }
    }

    /// The tab shown.
    #[must_use]
    pub fn tab(&self) -> SettingsTab {
        self.tab
    }

    /// Show `tab`.
    pub fn set_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    #[must_use]
    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Show `appearance` and the `light` and `dark` themes as chosen, e.g. after they were
    /// changed from the menu bar. Unlike picking them here, this reports no event.
    pub fn show_theme(
        &mut self,
        appearance: Appearance,
        light: &SharedString,
        dark: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.light_theme.update(cx, |select, cx| {
            select.set_selected_value(light, window, cx);
        });
        self.dark_theme
            .update(cx, |select, cx| select.set_selected_value(dark, window, cx));
        self.appearance = appearance;
        cx.notify();
    }

    fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        if self.appearance != appearance {
            self.appearance = appearance;
            cx.emit(SettingsEvent::SetAppearance(appearance));
            cx.notify();
        }
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("settings-appearance")
            .gap_2()
            .child(
                ButtonGroup::new("appearance-buttons")
                    .outline()
                    .small()
                    .children(Appearance::ALL.iter().enumerate().map(|(ix, appearance)| {
                        Button::new(("appearance", ix))
                            .label(appearance.label())
                            .accessibility_label(format!("{} appearance", appearance.label()))
                            .selected(*appearance == self.appearance)
                    }))
                    .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                        if let Some(appearance) =
                            clicked.first().and_then(|ix| Appearance::ALL.get(*ix))
                        {
                            this.set_appearance(*appearance, cx);
                        }
                    })),
            )
            .child(
                v_form()
                    .child(
                        field().label("Light theme").child(
                            Select::new(&self.light_theme)
                                .id("light-theme")
                                .accessibility_label("Light theme"),
                        ),
                    )
                    .child(
                        field().label("Dark theme").child(
                            Select::new(&self.dark_theme)
                                .id("dark-theme")
                                .accessibility_label("Dark theme"),
                        ),
                    ),
            )
    }

    /// The Tray tab, to give it the saved settings and the projects and filters to pick.
    #[must_use]
    pub fn tray(&self) -> &Entity<TraySettingsView> {
        &self.tray
    }

    #[must_use]
    pub fn reminder_double_click(&self) -> DoubleClickAction {
        self.double_click
    }

    /// Show `action` as what double-clicking a reminder does, e.g. the saved choice.
    pub fn set_reminder_double_click(&mut self, action: DoubleClickAction, cx: &mut Context<Self>) {
        if self.double_click != action {
            self.double_click = action;
            cx.notify();
        }
    }

    fn pick_reminder_double_click(&mut self, action: DoubleClickAction, cx: &mut Context<Self>) {
        if self.double_click != action {
            self.double_click = action;
            cx.emit(SettingsEvent::SetReminderDoubleClick(action));
            cx.notify();
        }
    }

    #[must_use]
    pub fn start_at_login(&self) -> &StartAtLogin {
        &self.start_at_login
    }

    /// Show whether the app starts at login, e.g. once read from the system.
    pub fn set_start_at_login(&mut self, state: StartAtLogin, cx: &mut Context<Self>) {
        if self.start_at_login != state {
            self.start_at_login = state;
            cx.notify();
        }
    }

    fn pick_start_at_login(&mut self, action: StartupAction, cx: &mut Context<Self>) {
        let Some(current) = self.start_at_login.action() else {
            return;
        };
        if current != action {
            // Unknown until the app reports the change made.
            self.start_at_login = StartAtLogin::Unknown;
            cx.emit(SettingsEvent::SetStartAtLogin(action));
            cx.notify();
        }
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = StartupAction::all();
        let selected = self
            .start_at_login
            .action()
            .and_then(|action| actions.iter().position(|shown| *shown == action));
        let message = match &self.start_at_login {
            StartAtLogin::Failed { message, .. } => Some(message.clone()),
            StartAtLogin::Unknown | StartAtLogin::Known(_) => None,
        };
        v_flex()
            .id("settings-general")
            .gap_2()
            .child(
                v_form().child(
                    field().label("When you log in").child(
                        RadioGroup::vertical("start-at-login")
                            .selected_index(selected)
                            .disabled(self.start_at_login == StartAtLogin::Unknown)
                            .children(
                                actions
                                    .iter()
                                    .enumerate()
                                    .map(|(ix, action)| Radio::new(ix).label(action.label())),
                            )
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                if let Some(action) = StartupAction::all().get(*ix) {
                                    this.pick_start_at_login(*action, cx);
                                }
                            })),
                    ),
                ),
            )
            .when_some(message, |panel, message| {
                panel.child(
                    div()
                        .id("start-at-login-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .test_support()
                        .text_color(cx.theme().danger)
                        .child(message),
                )
            })
    }

    fn render_reminders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = DoubleClickAction::ALL
            .iter()
            .position(|action| *action == self.double_click);
        v_flex().id("settings-reminders").gap_2().child(
            v_form().child(
                field().label("Double-clicking a reminder").child(
                    RadioGroup::vertical("reminder-double-click")
                        .selected_index(selected)
                        .children(
                            DoubleClickAction::ALL
                                .iter()
                                .enumerate()
                                .map(|(ix, action)| Radio::new(ix).label(action.label())),
                        )
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            if let Some(action) = DoubleClickAction::ALL.get(*ix) {
                                this.pick_reminder_double_click(*action, cx);
                            }
                        })),
                ),
            ),
        )
    }

    #[must_use]
    pub fn check_for_updates_on_open(&self) -> bool {
        self.check_on_open
    }

    fn toggle_check_on_open(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.check_on_open != on {
            self.check_on_open = on;
            cx.emit(SettingsEvent::SetCheckForUpdatesOnOpen(on));
            cx.notify();
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
            .gap_2()
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
                                cx.emit(event.clone());
                            }
                        })),
                ),
            )
            .child(
                Switch::new("check-updates-on-open")
                    .checked(self.check_on_open)
                    .label("Check for updates on open")
                    .accessibility_label("Check for updates on open")
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.toggle_check_on_open(*on, cx);
                    })),
            )
    }

    fn render_about(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let repository = self.about.repository.clone();
        let bug_report = self.about.bug_report.clone();
        let feature_request = self.about.feature_request.clone();
        v_flex()
            .id("settings-about")
            .gap_2()
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
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .child(
                        Button::new("report-bug")
                            .outline()
                            .small()
                            .label("Report a Bug…")
                            .accessibility_label("Report a bug")
                            .on_click(move |_, _, cx| cx.open_url(&bug_report)),
                    )
                    .child(
                        Button::new("request-feature")
                            .outline()
                            .small()
                            .label("Request a Feature…")
                            .accessibility_label("Request a feature")
                            .on_click(move |_, _, cx| cx.open_url(&feature_request)),
                    ),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = TabBar::new("settings-tabs")
            .underline()
            .px_4()
            .selected_index(self.tab.index())
            .children(
                SettingsTab::ALL
                    .iter()
                    .map(|tab| Tab::new().label(tab.label())),
            )
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Some(tab) = SettingsTab::ALL.get(*ix) {
                    this.set_tab(*tab, cx);
                }
            }));
        let panel = match self.tab {
            SettingsTab::General => self.render_general(cx).into_any_element(),
            SettingsTab::Appearance => self.render_appearance(cx).into_any_element(),
            SettingsTab::Reminders => self.render_reminders(cx).into_any_element(),
            SettingsTab::Tray => self.tray.clone().into_any_element(),
            SettingsTab::Updates => self.render_updates(cx).into_any_element(),
            SettingsTab::About => self.render_about(cx).into_any_element(),
        };
        let theme = cx.theme();
        v_flex()
            .id("settings")
            .role(Role::Region)
            .aria_label("Settings")
            .test_support()
            .size_full()
            .pt_2()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(tabs)
            .child(
                div()
                    .id("settings-panel")
                    .role(Role::TabPanel)
                    .aria_label(self.tab.panel_label())
                    .test_support()
                    .flex_1()
                    .p_6()
                    .child(panel),
            )
    }
}

/// Open the settings window; see [`SettingsView::new`] for `check_on_open` and `theme`.
pub fn open(
    options: WindowOptions,
    about: About,
    check_on_open: bool,
    theme: ThemeChoices,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<SettingsView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsView::new(about, check_on_open, theme, window, cx))
    })
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    use super::*;

    fn about() -> About {
        About {
            name: "Vikui".into(),
            version: "1.2.3".into(),
            repository: "https://example.com/vikui".into(),
            bug_report: "https://example.com/vikui/bug".into(),
            feature_request: "https://example.com/vikui/feature".into(),
        }
    }

    /// Click `tab` in the settings window's tab bar.
    fn click_tab(window: &mut Window, tab: SettingsTab, cx: &mut App) {
        window.render_frame(cx);
        window.within("settings-tabs").click(tab.index(), cx);
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn shows_the_app_and_opens_the_repository(cx: &mut TestAppContext) {
        let about = about();
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                true,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
            .0
        });
        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::About, cx);
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

        for (id, label, url) in [
            (
                "report-bug",
                "Report a bug",
                "https://example.com/vikui/bug",
            ),
            (
                "request-feature",
                "Request a feature",
                "https://example.com/vikui/feature",
            ),
        ] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label(label);
                window.click(id, cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(cx.opened_url().as_deref(), Some(url));
        }
    }

    #[gpui_kit::test]
    fn checks_for_and_installs_updates(cx: &mut TestAppContext) {
        let about = about();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                true,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
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
        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::Updates, cx);
        })
        .unwrap();

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

    #[gpui_kit::test]
    fn picks_the_appearance_and_themes(cx: &mut TestAppContext) {
        let about = about();
        let theme = ThemeChoices {
            appearance: Appearance::Auto,
            light: "Solarized Light".into(),
            dark: "Default Dark".into(),
            light_themes: vec!["Default Light".into(), "Solarized Light".into()],
            dark_themes: vec!["Default Dark".into(), "Gruvbox Dark".into()],
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(WindowOptions::default(), about, true, theme, cx).unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });

        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::Appearance, cx);
            window.find_by_label("Appearance");
            window.find_by_label("Light theme");
            window.find_by_label("Dark theme");
            window.find_by_label("Auto appearance");
            window.click(gpui_kit::ElementId::from(("appearance", 1_usize)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).appearance()), Appearance::Dark);
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(
                view.light_theme
                    .read(cx)
                    .selected_value()
                    .map(AsRef::as_ref),
                Some("Solarized Light")
            );
            assert_eq!(
                view.dark_theme.read(cx).selected_value().map(AsRef::as_ref),
                Some("Default Dark")
            );
        });

        // Confirming a theme in a picker reports it.
        cx.update(|cx| {
            let dark = view.read(cx).dark_theme.clone();
            dark.update(cx, |_, cx| {
                cx.emit(SelectEvent::Confirm(Some("Gruvbox Dark".into())));
            });
        });
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [
                SettingsEvent::SetAppearance(Appearance::Dark),
                SettingsEvent::SetDarkTheme("Gruvbox Dark".into()),
            ]
        );
    }

    #[gpui_kit::test]
    fn shows_theme_changes_made_elsewhere_without_reporting_them(cx: &mut TestAppContext) {
        let about = about();
        let theme = ThemeChoices {
            appearance: Appearance::Auto,
            light: "Default Light".into(),
            dark: "Default Dark".into(),
            light_themes: vec!["Default Light".into(), "Solarized Light".into()],
            dark_themes: vec!["Default Dark".into(), "Gruvbox Dark".into()],
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(WindowOptions::default(), about, true, theme, cx).unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.show_theme(
                    Appearance::Light,
                    &"Solarized Light".into(),
                    &"Gruvbox Dark".into(),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let view = view.read(cx);
            assert_eq!(view.appearance(), Appearance::Light);
            assert_eq!(
                view.light_theme
                    .read(cx)
                    .selected_value()
                    .map(AsRef::as_ref),
                Some("Solarized Light")
            );
            assert_eq!(
                view.dark_theme.read(cx).selected_value().map(AsRef::as_ref),
                Some("Gruvbox Dark")
            );
        });
        assert!(events.borrow().is_empty());
    }

    #[gpui_kit::test]
    fn picks_what_happens_at_login_once_it_is_known(cx: &mut TestAppContext) {
        let about = about();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                true,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let pick = |cx: &mut TestAppContext, ix: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.within("start-at-login").click(ix, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let state =
            |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).start_at_login().clone());

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Do not open at login");
            window.find_by_label("Open the main window");
            let tray = window
                .try_find_by_label("Open only the tray icon")
                .is_some();
            assert_eq!(
                tray,
                StartupAction::all().contains(&StartupAction::OpenTray)
            );
        })
        .unwrap();

        // Not read from the system yet: picking does nothing.
        pick(cx, 1);
        assert!(events.borrow().is_empty());

        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_start_at_login(StartAtLogin::Known(StartupAction::DoNotOpen), cx);
            });
        });
        pick(cx, 1);
        assert_eq!(
            *events.borrow(),
            [SettingsEvent::SetStartAtLogin(StartupAction::OpenWindow)]
        );
        assert_eq!(
            state(cx),
            StartAtLogin::Unknown,
            "until the app says it changed"
        );

        let failed = StartAtLogin::Failed {
            action: StartupAction::DoNotOpen,
            message: "Couldn't start Vikui at login: denied".into(),
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_start_at_login(failed, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let alert = window.find_by_label("Couldn't start Vikui at login: denied");
            assert_eq!(alert.role(), Some(Role::Alert));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn toggles_checking_for_updates_on_open(cx: &mut TestAppContext) {
        let about = about();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                false,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::Updates, cx);
        })
        .unwrap();
        let toggle = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label("Check for updates on open");
                window.click("check-updates-on-open", cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let on =
            |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).check_for_updates_on_open());

        assert!(!on(cx));
        toggle(cx);
        assert!(on(cx));
        toggle(cx);
        assert!(!on(cx));
        assert_eq!(
            *events.borrow(),
            [
                SettingsEvent::SetCheckForUpdatesOnOpen(true),
                SettingsEvent::SetCheckForUpdatesOnOpen(false)
            ]
        );
    }

    #[gpui_kit::test]
    fn switches_tabs_by_click_or_from_the_app(cx: &mut TestAppContext) {
        let about = about();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                true,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
        });
        let tab = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).tab());
        let shows = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };

        // General first; the other tabs' controls are not shown.
        assert_eq!(tab(cx), SettingsTab::General);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let tabs = window.find_all_by_role(Role::Tab);
            assert_eq!(tabs.len(), 6);
            assert_eq!(
                window.within("settings-tabs").find(0_usize).selected(),
                Some(true)
            );
            window.find_by_label("General");
            window.find_by_label("Appearance");
            window.find_by_label("Reminders");
            window.find_by_label("Tray");
            window.find_by_label("Updates");
            window.find_by_label("About");
            window.find_by_label("General settings");
            assert_eq!(window.find_all_by_role(Role::TabPanel).len(), 1);
        })
        .unwrap();
        assert!(shows(cx, "Do not open at login"));
        assert!(!shows(cx, "Light theme"));
        assert!(!shows(cx, "Check for Updates"));
        assert!(!shows(cx, "Version 1.2.3"));

        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::About, cx);
        })
        .unwrap();
        assert_eq!(tab(cx), SettingsTab::About);
        assert!(shows(cx, "Version 1.2.3"));
        assert!(!shows(cx, "Light theme"));

        // The app brings Updates forward, e.g. when one is available.
        cx.update(|cx| view.update(cx, |view, cx| view.set_tab(SettingsTab::Updates, cx)));
        assert!(shows(cx, "Check for Updates"));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window
                    .within("settings-tabs")
                    .find(SettingsTab::Updates.index())
                    .selected(),
                Some(true)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn picks_what_double_clicking_a_reminder_does(cx: &mut TestAppContext) {
        let about = about();
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(
                WindowOptions::default(),
                about,
                true,
                ThemeChoices::default(),
                cx,
            )
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
            // The saved choice, shown without reporting it back.
            view.update(cx, |view, cx| {
                view.set_reminder_double_click(DoubleClickAction::Edit, cx);
            });
        });
        let checked = |cx: &mut TestAppContext, ix: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.within("reminder-double-click").find(ix).checked()
            })
            .unwrap()
        };

        cx.update_window(handle, |_, window, cx| {
            click_tab(window, SettingsTab::Reminders, cx);
            window.find_by_label("Reminder settings");
            window.find_by_label("Open details in a new window");
            window.find_by_label("Edit the task");
            window.find_by_label("Mark as done or not done");
            window.find_by_label("Open in browser");
        })
        .unwrap();
        assert_eq!(checked(cx, 1), Some(true));
        assert!(events.borrow().is_empty());

        cx.update_window(handle, |_, window, cx| {
            window.within("reminder-double-click").click(2_usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| view.read(cx).reminder_double_click()),
            DoubleClickAction::ToggleDone
        );
        assert_eq!(checked(cx, 2), Some(true));
        assert_eq!(
            *events.borrow(),
            [SettingsEvent::SetReminderDoubleClick(
                DoubleClickAction::ToggleDone
            )]
        );
    }
}
