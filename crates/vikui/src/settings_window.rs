//! Opens the settings window, or brings the open one to the front, and carries out the
//! update checks and installs it asks for.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use settings_ui::{About, SettingsEvent, SettingsView, UpdateStatus};
use updater::{Release, Updater};

/// The open settings window, so a second request focuses it instead of opening another.
#[derive(Default)]
struct SettingsWindow(Option<(AnyWindowHandle, WeakEntity<SettingsView>)>);

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
/// updates, so an available one is a click away.
pub fn open(cx: &mut App) {
    if open_view(cx).is_none() {
        log::error!("failed to open settings window");
    }
}

/// Open the settings window and check for updates, even if it was already open.
pub fn check_for_updates(cx: &mut App) {
    if let Some((view, opened)) = open_view(cx)
        && !opened
    {
        view.update(cx, |_, cx| cx.emit(SettingsEvent::CheckForUpdates));
    }
}

/// The settings view, and whether it was just opened.
fn open_view(cx: &mut App) -> Option<(Entity<SettingsView>, bool)> {
    if let Some((handle, view)) = cx
        .try_global::<SettingsWindow>()
        .and_then(|open| open.0.clone())
        && let Some(view) = view.upgrade()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return Some((view, false));
    }
    let title = format!("Settings – {}", crate::APP_NAME);
    let options = crate::window_options(title, 480., 360., cx);
    let (handle, view) = settings_ui::open(options, about(), cx)
        .inspect_err(|err| log::error!("failed to open settings window: {err:#}"))
        .ok()?;
    cx.set_global(SettingsWindow(Some((handle, view.downgrade()))));
    Updates::attach(&view, cx);
    view.update(cx, |_, cx| cx.emit(SettingsEvent::CheckForUpdates));
    Some((view, true))
}

/// Checks for and installs updates for one settings window.
struct Updates {
    view: WeakEntity<SettingsView>,
    updater: Result<Updater, String>,
    /// The release the last check found.
    available: RefCell<Option<Release>>,
}

impl Updates {
    fn attach(view: &Entity<SettingsView>, cx: &mut App) {
        let updater = Updater::new(env!("CARGO_PKG_REPOSITORY"), env!("CARGO_PKG_VERSION"))
            .map_err(|err| err.to_string());
        let this = Rc::new(Self {
            view: view.downgrade(),
            updater,
            available: RefCell::default(),
        });
        cx.subscribe(view, move |_, event, cx| match event {
            SettingsEvent::CheckForUpdates => this.check(cx),
            SettingsEvent::InstallUpdate => this.install(cx),
        })
        .detach();
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
                    Ok(Some(release)) => {
                        let version = release.version.to_string().into();
                        *this.available.borrow_mut() = Some(release);
                        UpdateStatus::Available { version }
                    }
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
