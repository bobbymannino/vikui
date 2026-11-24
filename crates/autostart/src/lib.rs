//! Starts Vikui when the user logs in, or stops doing so. No UI.
//!
//! The system keeps the setting, so it is read from there rather than the preferences:
//! - macOS: a launch agent, `~/Library/LaunchAgents/<id>.plist`, which opens the app
//!   bundle (or runs the executable, for a build outside one).
//! - Windows: the current user's `Run` registry key.
//! - Linux: an XDG autostart entry, `~/.config/autostart/<id>.desktop`.
//!
//! Each points at where the app is now; the updater replaces it in place, so the path
//! stays right. Reading and writing block briefly; run them on a background task.

use std::path::{Path, PathBuf};

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// Why starting at login couldn't be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't find the app: {0}")]
    AppPath(#[source] std::io::Error),
    #[error("{0}")]
    System(#[from] auto_launch::Error),
}

/// Starts the app at login, under `id` (the app's reverse-DNS identifier).
pub struct Autostart(AutoLaunch);

impl Autostart {
    /// For the running app.
    ///
    /// # Errors
    /// If the app's own path can't be found.
    pub fn new(id: &str) -> Result<Self, Error> {
        let exe = std::env::current_exe().map_err(Error::AppPath)?;
        let (program, args) = launch_command(&exe);
        let mut builder = AutoLaunchBuilder::new();
        builder
            .set_app_name(id)
            .set_app_path(&program.to_string_lossy())
            .set_args(&args)
            .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
            .set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser)
            .set_linux_launch_mode(auto_launch::LinuxLaunchMode::XdgAutostart);
        Ok(Self(builder.build()?))
    }

    /// Whether the app starts at login.
    ///
    /// # Errors
    /// If the system's setting can't be read.
    pub fn is_enabled(&self) -> Result<bool, Error> {
        Ok(self.0.is_enabled()?)
    }

    /// Start the app at login, or stop doing so.
    ///
    /// # Errors
    /// If the system's setting can't be changed.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        if enabled {
            self.0.enable()?;
        } else {
            self.0.disable()?;
        }
        Ok(())
    }
}

/// What to run at login for the executable at `exe`: inside a macOS app bundle, `open`
/// on the bundle, so it starts as an app does from Finder; otherwise the executable.
fn launch_command(exe: &Path) -> (PathBuf, Vec<String>) {
    match app_bundle(exe) {
        Some(bundle) if cfg!(target_os = "macos") => (
            PathBuf::from("/usr/bin/open"),
            vec![bundle.to_string_lossy().into_owned()],
        ),
        _ => (exe.to_path_buf(), Vec::new()),
    }
}

/// The `.app` bundle `exe` is the executable of (`X.app/Contents/MacOS/x`), if any.
fn app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then_some(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_an_executable_is_in() {
        let exe = Path::new("/Applications/Vikui.app/Contents/MacOS/vikui");
        assert_eq!(app_bundle(exe), Some(Path::new("/Applications/Vikui.app")));
        assert_eq!(app_bundle(Path::new("/usr/local/bin/vikui")), None);
        assert_eq!(app_bundle(Path::new("/tmp/Contents/MacOS/vikui")), None);
    }

    #[test]
    fn opens_the_bundle_on_macos_and_runs_the_executable_elsewhere() {
        let exe = Path::new("/Applications/Vikui.app/Contents/MacOS/vikui");
        let (program, args) = launch_command(exe);
        if cfg!(target_os = "macos") {
            assert_eq!(program, Path::new("/usr/bin/open"));
            assert_eq!(args, ["/Applications/Vikui.app"]);
        } else {
            assert_eq!(program, exe);
            assert_eq!(args, Vec::<String>::new());
        }
        let loose = Path::new("/home/me/vikui/vikui");
        assert_eq!(launch_command(loose), (loose.to_path_buf(), Vec::new()));
    }
}
