//! Opens the settings window, or brings the open one to the front.

use gpui_kit::{AnyWindowHandle, App, Global};
use settings_ui::About;

/// The open settings window, so a second request focuses it instead of opening another.
#[derive(Default)]
struct SettingsWindow(Option<AnyWindowHandle>);

impl Global for SettingsWindow {}

/// What the settings window says about this build.
fn about() -> About {
    About {
        name: crate::APP_NAME.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repository: env!("CARGO_PKG_REPOSITORY").into(),
    }
}

/// Open the settings window, or activate it if it is already open.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<SettingsWindow>().and_then(|open| open.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let title = format!("Settings – {}", crate::APP_NAME);
    let options = crate::window_options(title, 480., 320., cx);
    match settings_ui::open(options, about(), cx) {
        Ok(handle) => cx.set_global(SettingsWindow(Some(handle))),
        Err(err) => log::error!("failed to open settings window: {err:#}"),
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
}
