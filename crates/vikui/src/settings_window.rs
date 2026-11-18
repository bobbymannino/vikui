//! Opens the settings window, or brings the open one to the front, and carries out the
//! update checks and installs it asks for.
//!
//! With "Check for updates on open" on, the app checks when it starts and opens this
//! window only if an update is available; opening the window checks too.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use settings_ui::{About, SettingsEvent, SettingsView, ThemeChoices, UpdateStatus};
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

/// What the settings window says about this build.
fn about() -> About {
    About {
        name: crate::APP_NAME.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repository: env!("CARGO_PKG_REPOSITORY").into(),
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

/// Open the settings window and check for updates.
pub fn check_for_updates(cx: &mut App) {
    if let Some((opened, _)) = open_view(cx) {
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
    let options = crate::window_options(title, 480., 400., cx);
    let check_on_open = crate::app_preferences::get(cx).check_for_updates_on_open;
    let (handle, view) =
        settings_ui::open(options, about(), check_on_open, ThemeChoices::default(), cx)
            .inspect_err(|err| log::error!("failed to open settings window: {err:#}"))
            .ok()?;
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
        cx.subscribe(view, move |_, event, cx| match *event {
            SettingsEvent::CheckForUpdates => updates.check(cx),
            SettingsEvent::InstallUpdate => updates.install(cx),
            SettingsEvent::SetAppearance(_)
            | SettingsEvent::SetLightTheme(_)
            | SettingsEvent::SetDarkTheme(_) => {}
            SettingsEvent::SetCheckForUpdatesOnOpen(on) => {
                crate::app_preferences::update(cx, |preferences| {
                    preferences.check_for_updates_on_open = on;
                });
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
