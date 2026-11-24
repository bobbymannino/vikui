//! Opens the settings window, or brings the open one to the front, and carries out the
//! update checks and installs it asks for.
//!
//! With "Check for updates on open" on, the app checks when it starts and opens this
//! window only if an update is available; opening the window checks too. Checking from
//! the menu, or finding an update at launch, shows the Updates tab.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::ThemeMode;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use preferences::{Appearance, DoubleClickAction, TrayPreferences, TraySource};
use reminders::Source;
use settings_ui::{
    About, SettingsEvent, SettingsTab, SettingsView, StartAtLogin, StartupAction, ThemeChoices,
    TraySettings, TraySourceChoice, TraySourceId, UpdateStatus,
};
use updater::{Release, Updater};

/// The open settings window, so a second request focuses it instead of opening another.
#[derive(Default)]
struct SettingsWindow(Option<Opened>);

#[derive(Clone)]
struct Opened {
    handle: AnyWindowHandle,
    view: WeakEntity<SettingsView>,
    updates: Rc<Updates>,
}

impl Global for SettingsWindow {}

impl Opened {
    /// Bring the Updates tab forward, where the check's result shows.
    fn show_updates(&self, cx: &mut App) {
        if let Some(view) = self.view.upgrade() {
            view.update(cx, |view, cx| view.set_tab(SettingsTab::Updates, cx));
        }
    }
}

/// What the settings window says about this build.
fn about() -> About {
    About {
        name: crate::APP_NAME.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repository: env!("CARGO_PKG_REPOSITORY").into(),
        bug_report: crate::app_issues::bug_report().into(),
        feature_request: crate::app_issues::feature_request().into(),
    }
}

/// The saved appearance, the themes shown and the themes to pick from.
fn theme_choices(cx: &App) -> ThemeChoices {
    let appearance = match crate::app_preferences::get(cx).theme.appearance {
        Appearance::Light => settings_ui::Appearance::Light,
        Appearance::Dark => settings_ui::Appearance::Dark,
        Appearance::Auto => settings_ui::Appearance::Auto,
    };
    let (light, dark) = crate::app_theme::current(cx);
    ThemeChoices {
        appearance,
        light,
        dark,
        light_themes: crate::app_theme::names(ThemeMode::Light, cx),
        dark_themes: crate::app_theme::names(ThemeMode::Dark, cx),
    }
}

/// Show the saved appearance and the themes shown in the settings window, if it is open,
/// e.g. after they were changed from the menu bar.
pub fn show_theme(cx: &mut App) {
    let Some(opened) = cx
        .try_global::<SettingsWindow>()
        .and_then(|open| open.0.clone())
    else {
        return;
    };
    let Some(view) = opened.view.upgrade() else {
        return;
    };
    let choices = theme_choices(cx);
    let shown = opened.handle.update(cx, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.show_theme(
                choices.appearance,
                &choices.light,
                &choices.dark,
                window,
                cx,
            );
        });
    });
    if let Err(err) = shown {
        log::warn!("settings window not updated: {err:#}");
    }
}

/// Open the settings window, or activate it if it is already open. Opening it checks for
/// updates if "Check for updates on open" is on, so an available one is a click away.
pub fn open(cx: &mut App) {
    let check = crate::app_preferences::get(cx).check_for_updates_on_open;
    if let Some((opened, true)) = open_view(cx)
        && check
    {
        opened.updates.check(cx);
    }
}

/// Open the settings window on its Tray tab.
pub fn open_tray(cx: &mut App) {
    open(cx);
    if let Some(view) = opened(cx).and_then(|opened| opened.view.upgrade()) {
        view.update(cx, |view, cx| view.set_tab(SettingsTab::Tray, cx));
    }
}

/// Offer the projects and saved filters in the Tray tab, if the window is open, e.g.
/// after they load.
pub fn show_tray_sources(cx: &mut App) {
    let Some(opened) = opened(cx) else {
        return;
    };
    let Some(view) = opened.view.upgrade() else {
        return;
    };
    let choices = tray_choices(&crate::app_preferences::get(cx).tray, cx);
    let tray = view.read(cx).tray().clone();
    opened
        .handle
        .update(cx, |_, window, cx| {
            tray.update(cx, |tray, cx| tray.set_sources(choices, window, cx));
        })
        .ok();
}

fn opened(cx: &App) -> Option<Opened> {
    cx.try_global::<SettingsWindow>()
        .and_then(|open| open.0.clone())
}

/// The lists the tray can show: the Projects menu's, or only the saved one while they
/// haven't loaded.
fn tray_choices(saved: &TrayPreferences, cx: &App) -> Vec<TraySourceChoice> {
    let mut choices: Vec<TraySourceChoice> = crate::projects_menu::destinations(cx)
        .into_iter()
        .filter_map(|destination| {
            let id = match destination.source {
                Source::Project { id, .. } => TraySourceId::Project(id),
                Source::SavedFilter { filter_id, .. } => TraySourceId::SavedFilter(filter_id),
                Source::Filter(_) | Source::Search(_) => return None,
            };
            Some(TraySourceChoice {
                id,
                name: destination.name.into(),
            })
        })
        .collect();
    if let Some(source) = &saved.source {
        let id = tray_source_id(source);
        if !choices.iter().any(|choice| choice.id == id) {
            choices.push(TraySourceChoice {
                id,
                name: source.title().to_owned().into(),
            });
        }
    }
    choices
}

fn tray_source_id(source: &TraySource) -> TraySourceId {
    match *source {
        TraySource::Project { id, .. } => TraySourceId::Project(id),
        TraySource::SavedFilter { filter_id, .. } => TraySourceId::SavedFilter(filter_id),
    }
}

fn tray_settings(saved: &TrayPreferences) -> TraySettings {
    TraySettings {
        enabled: saved.enabled,
        source: saved.source.as_ref().map(tray_source_id),
        limit: saved.limit,
        show_count: saved.show_count,
    }
}

/// Save `settings` from the Tray tab, naming its list after the choice picked, and
/// show the tray to match.
fn save_tray(settings: &TraySettings, cx: &mut App) {
    let saved = crate::app_preferences::get(cx).tray;
    let choices = tray_choices(&saved, cx);
    let source = settings.source.and_then(|id| {
        let title = choices
            .iter()
            .find(|choice| choice.id == id)
            .map(|choice| choice.name.to_string())?;
        Some(match id {
            TraySourceId::Project(id) => TraySource::Project { id, title },
            TraySourceId::SavedFilter(filter_id) => TraySource::SavedFilter { filter_id, title },
        })
    });
    crate::app_preferences::update(cx, |preferences| {
        preferences.tray = TrayPreferences {
            enabled: settings.enabled,
            source,
            limit: settings.limit,
            show_count: settings.show_count,
        };
    });
    crate::app_tray::apply(cx);
}

/// Make the app do `change` at login, or just read what it does, then show that in
/// `view`. The system keeps whether it starts and with which arguments; the preferences
/// keep which of the two ways it starts. Reading and changing it block, so they run in
/// the background.
fn start_at_login(view: WeakEntity<SettingsView>, change: Option<StartupAction>, cx: &mut App) {
    let work = cx.background_spawn(async move {
        let args: &[&str] = match change {
            Some(StartupAction::OpenTray) => &[crate::TRAY_ARG],
            _ => &[],
        };
        let autostart = autostart::Autostart::new(crate::APP_ID, args)?;
        let failed = change.and_then(|action| {
            autostart
                .set_enabled(action != StartupAction::DoNotOpen)
                .err()
        });
        Ok::<_, autostart::Error>((autostart.is_enabled()?, failed))
    });
    cx.spawn(async move |cx| {
        let result = work.await;
        cx.update(|cx| {
            if let (Some(action), Ok((_, None))) = (change, &result) {
                changed_start_at_login(action, &view, cx);
            }
            let in_tray = crate::app_preferences::get(cx).login_opens_tray;
            let action = |on: bool| match (on, in_tray) {
                (false, _) => StartupAction::DoNotOpen,
                (true, false) => StartupAction::OpenWindow,
                (true, true) => StartupAction::OpenTray,
            };
            let state = match result {
                Ok((on, None)) => StartAtLogin::Known(action(on)),
                Ok((on, Some(err))) => {
                    log::warn!("couldn't change what opens at login: {err}");
                    StartAtLogin::Failed {
                        action: action(on),
                        message: format!("Couldn't change what opens at login: {err}").into(),
                    }
                }
                Err(err) => {
                    log::warn!("couldn't read whether the app starts at login: {err}");
                    StartAtLogin::Failed {
                        action: StartupAction::DoNotOpen,
                        message: format!(
                            "Couldn't read whether {} opens at login: {err}",
                            crate::APP_NAME
                        )
                        .into(),
                    }
                }
            };
            if let Some(view) = view.upgrade() {
                view.update(cx, |view, cx| view.set_start_at_login(state, cx));
            }
        });
    })
    .detach();
}

/// Remember how the app now starts at login. Opening only the tray icon turns the icon on
/// (and shows that in `view`'s Tray tab), or there would be nothing to see.
fn changed_start_at_login(action: StartupAction, view: &WeakEntity<SettingsView>, cx: &mut App) {
    let in_tray = action == StartupAction::OpenTray;
    let turn_on_tray = in_tray && !crate::app_preferences::get(cx).tray.enabled;
    crate::app_preferences::update(cx, |preferences| {
        if action != StartupAction::DoNotOpen {
            preferences.login_opens_tray = in_tray;
        }
        if turn_on_tray {
            preferences.tray.enabled = true;
        }
    });
    if !turn_on_tray {
        return;
    }
    crate::app_tray::apply(cx);
    let (Some(opened), Some(view)) = (opened(cx), view.upgrade()) else {
        return;
    };
    let settings = tray_settings(&crate::app_preferences::get(cx).tray);
    let tray = view.read(cx).tray().clone();
    opened
        .handle
        .update(cx, |_, window, cx| {
            tray.update(cx, |tray, cx| tray.set_settings(settings, window, cx));
        })
        .ok();
}

/// Open the settings window on its Updates tab and check for updates.
pub fn check_for_updates(cx: &mut App) {
    if let Some((opened, _)) = open_view(cx) {
        opened.show_updates(cx);
        opened.updates.check(cx);
    }
}

/// Check for updates as the app starts, opening the settings window only if one is
/// available. Failures are logged, not shown: the user didn't ask for this check.
pub fn check_on_launch(cx: &mut App) {
    let updater = match new_updater() {
        Ok(updater) => updater,
        Err(err) => return log::warn!("not checking for updates: {err}"),
    };
    let check = cx.background_spawn(async move { updater.check() });
    cx.spawn(async move |cx| match check.await {
        Ok(Some(release)) => {
            log::info!("Vikui {} is available", release.version);
            cx.update(|cx| {
                if let Some((opened, _)) = open_view(cx) {
                    opened.show_updates(cx);
                    opened.updates.found(release, cx);
                }
            });
        }
        Ok(None) => log::info!("Vikui is up to date"),
        Err(err) => log::warn!("update check on launch failed: {err}"),
    })
    .detach();
}

/// The settings window, and whether it was just opened.
fn open_view(cx: &mut App) -> Option<(Opened, bool)> {
    if let Some(opened) = cx
        .try_global::<SettingsWindow>()
        .and_then(|open| open.0.clone())
        && opened.view.upgrade().is_some()
        && opened
            .handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Some((opened, false));
    }
    let title = format!("Settings – {}", crate::APP_NAME);
    let options = crate::app_windows::options(
        crate::app_windows::WindowKind::Settings,
        title,
        480.,
        560.,
        cx,
    );
    let preferences = crate::app_preferences::get(cx);
    let check_on_open = preferences.check_for_updates_on_open;
    let (handle, view) = settings_ui::open(options, about(), check_on_open, theme_choices(cx), cx)
        .inspect_err(|err| log::error!("failed to open settings window: {err:#}"))
        .ok()?;
    let double_click = match preferences.reminder_double_click {
        DoubleClickAction::OpenDetails => settings_ui::DoubleClickAction::OpenDetails,
        DoubleClickAction::Edit => settings_ui::DoubleClickAction::Edit,
        DoubleClickAction::ToggleDone => settings_ui::DoubleClickAction::ToggleDone,
        DoubleClickAction::OpenInBrowser => settings_ui::DoubleClickAction::OpenInBrowser,
    };
    view.update(cx, |view, cx| {
        view.set_reminder_double_click(double_click, cx);
    });
    let tray = view.read(cx).tray().clone();
    let (settings, choices) = (
        tray_settings(&preferences.tray),
        tray_choices(&preferences.tray, cx),
    );
    handle
        .update(cx, |_, window, cx| {
            tray.update(cx, |tray, cx| {
                tray.set_settings(settings, window, cx);
                tray.set_sources(choices, window, cx);
            });
        })
        .ok();
    crate::app_windows::track(crate::app_windows::WindowKind::Settings, handle, cx);
    start_at_login(view.downgrade(), None, cx);
    let opened = Opened {
        handle,
        view: view.downgrade(),
        updates: Updates::attach(&view, cx),
    };
    cx.set_global(SettingsWindow(Some(opened.clone())));
    Some((opened, true))
}

fn new_updater() -> Result<Updater, updater::Error> {
    Updater::new(env!("CARGO_PKG_REPOSITORY"), env!("CARGO_PKG_VERSION"))
}

/// Checks for and installs updates for one settings window.
struct Updates {
    view: WeakEntity<SettingsView>,
    updater: Result<Updater, String>,
    /// The release the last check found.
    available: RefCell<Option<Release>>,
}

impl Updates {
    fn attach(view: &Entity<SettingsView>, cx: &mut App) -> Rc<Self> {
        let this = Rc::new(Self {
            view: view.downgrade(),
            updater: new_updater().map_err(|err| err.to_string()),
            available: RefCell::default(),
        });
        let updates = Rc::clone(&this);
        cx.subscribe(view, move |_, event, cx| match event {
            SettingsEvent::CheckForUpdates => updates.check(cx),
            SettingsEvent::InstallUpdate => updates.install(cx),
            SettingsEvent::SetAppearance(appearance) => {
                let appearance = match appearance {
                    settings_ui::Appearance::Light => Appearance::Light,
                    settings_ui::Appearance::Dark => Appearance::Dark,
                    settings_ui::Appearance::Auto => Appearance::Auto,
                };
                crate::app_theme::change(cx, |theme| theme.appearance = appearance);
            }
            SettingsEvent::SetLightTheme(name) => {
                crate::app_theme::change(cx, |theme| theme.light = Some(name.to_string()));
            }
            SettingsEvent::SetDarkTheme(name) => {
                crate::app_theme::change(cx, |theme| theme.dark = Some(name.to_string()));
            }
            &SettingsEvent::SetCheckForUpdatesOnOpen(on) => {
                crate::app_preferences::update(cx, |preferences| {
                    preferences.check_for_updates_on_open = on;
                });
            }
            SettingsEvent::SetReminderDoubleClick(action) => {
                let action = match action {
                    settings_ui::DoubleClickAction::OpenDetails => DoubleClickAction::OpenDetails,
                    settings_ui::DoubleClickAction::Edit => DoubleClickAction::Edit,
                    settings_ui::DoubleClickAction::ToggleDone => DoubleClickAction::ToggleDone,
                    settings_ui::DoubleClickAction::OpenInBrowser => {
                        DoubleClickAction::OpenInBrowser
                    }
                };
                crate::app_preferences::update(cx, |preferences| {
                    preferences.reminder_double_click = action;
                });
                crate::reminders_window::set_double_click(action, cx);
            }
            SettingsEvent::SetTray(settings) => save_tray(settings, cx),
            &SettingsEvent::SetStartAtLogin(action) => {
                start_at_login(updates.view.clone(), Some(action), cx);
            }
        })
        .detach();
        this
    }

    /// Offer `release`, found by a check made elsewhere.
    fn found(&self, release: Release, cx: &mut App) {
        let version = release.version.to_string().into();
        *self.available.borrow_mut() = Some(release);
        self.set_status(UpdateStatus::Available { version }, cx);
    }

    fn set_status(&self, status: UpdateStatus, cx: &mut App) {
        if let Some(view) = self.view.upgrade() {
            view.update(cx, |view, cx| view.set_update_status(status, cx));
        }
    }

    fn updater(&self, cx: &mut App) -> Option<Updater> {
        match &self.updater {
            Ok(updater) => Some(updater.clone()),
            Err(message) => {
                self.set_status(failed(message.clone()), cx);
                None
            }
        }
    }

    fn check(self: &Rc<Self>, cx: &mut App) {
        let Some(updater) = self.updater(cx) else {
            return;
        };
        self.set_status(UpdateStatus::Checking, cx);
        let check = cx.background_spawn(async move { updater.check() });
        let this = Rc::clone(self);
        cx.spawn(async move |cx| {
            let result = check.await;
            cx.update(|cx| {
                let status = match result {
                    Ok(Some(release)) => return this.found(release, cx),
                    Ok(None) => UpdateStatus::UpToDate,
                    Err(err) => {
                        log::warn!("update check failed: {err}");
                        failed(err.to_string())
                    }
                };
                this.set_status(status, cx);
            });
        })
        .detach();
    }

    fn install(self: &Rc<Self>, cx: &mut App) {
        let Some(updater) = self.updater(cx) else {
            return;
        };
        let Some(release) = self.available.borrow().clone() else {
            return self.check(cx);
        };
        self.set_status(
            UpdateStatus::Installing {
                version: release.version.to_string().into(),
            },
            cx,
        );
        let install = cx.background_spawn(async move { updater.install(&release) });
        let this = Rc::clone(self);
        cx.spawn(async move |cx| {
            let result = install.await;
            cx.update(|cx| match result {
                Ok(path) => {
                    // The new version is in place; relaunch it explicitly, since the
                    // running executable's own path may now name the old, removed file.
                    cx.set_restart_path(path);
                    cx.restart();
                }
                Err(err) => {
                    log::warn!("update failed: {err}");
                    this.available.borrow_mut().take();
                    this.set_status(failed(err.to_string()), cx);
                }
            });
        })
        .detach();
    }
}

fn failed(message: String) -> UpdateStatus {
    UpdateStatus::Failed {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_this_build() {
        let about = about();
        assert_eq!(about.name, "Vikui");
        assert_eq!(about.version, env!("CARGO_PKG_VERSION"));
        assert!(about.repository.starts_with("https://"));
    }

    #[test]
    fn updates_come_from_this_repository() {
        let updater = Updater::new(env!("CARGO_PKG_REPOSITORY"), env!("CARGO_PKG_VERSION"));
        assert!(updater.is_ok());
    }
}
