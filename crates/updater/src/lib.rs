//! Updates Vikui in place from its GitHub releases, on macOS, Linux and Windows.
//!
//! [`Updater::check`] asks GitHub for the latest release (drafts and prereleases are
//! never offered) and [`Updater::install`] downloads this platform's archive, checks its
//! SHA-256 digest, unpacks it next to the running app and swaps it in. The caller then
//! restarts the app at the path `install` returns.
//!
//! Everything here blocks; run it on a background task. No UI.

mod archive;
mod github;
mod install;
mod platform;

use std::path::PathBuf;
use std::time::Duration;

pub use semver::Version;
use ureq::Agent;

pub use install::clean_up;

/// Asset names are `vikui-<version>-<os>-<arch>.<ext>`, as `packaging/package.sh` builds.
const ASSET_PREFIX: &str = "vikui";
const USER_AGENT: &str = concat!("vikui-updater/", env!("CARGO_PKG_VERSION"));

/// Why checking for or installing an update failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't reach GitHub: {0}")]
    Network(String),
    #[error("GitHub sent an unexpected reply: {0}")]
    BadResponse(String),
    #[error("release {0} has no download for this computer")]
    NoAsset(Version),
    #[error("the download was damaged (checksum mismatch); try again")]
    Checksum,
    #[error("the download couldn't be unpacked: {0}")]
    Archive(String),
    #[error("this copy of Vikui can't update itself: {0}")]
    NotInstalled(&'static str),
    #[error("Vikui can't write to {}; update it manually", .0.display())]
    NotWritable(PathBuf),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl From<ureq::Error> for Error {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::Io(err) => Self::Io(err),
            err => Self::Network(err.to_string()),
        }
    }
}

/// A newer release, ready to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release's page on GitHub, for its notes.
    pub url: String,
    asset: github::Asset,
}

/// Checks for and installs updates of one app from one GitHub repository.
#[derive(Clone, Debug)]
pub struct Updater {
    agent: Agent,
    /// `owner/name`.
    repository: String,
    current: Version,
}

impl Updater {
    /// `repository` is the project's GitHub URL (e.g. `https://github.com/owner/name`);
    /// `current_version` is the running app's version.
    pub fn new(repository: &str, current_version: &str) -> Result<Self, Error> {
        let repository = github::repository_path(repository)
            .ok_or(Error::NotInstalled("its repository isn't on GitHub"))?;
        let current = Version::parse(current_version)
            .map_err(|err| Error::BadResponse(format!("bad current version: {err}")))?;
        let agent = Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .build()
            .into();
        Ok(Self {
            agent,
            repository,
            current,
        })
    }

    #[must_use]
    pub fn current_version(&self) -> &Version {
        &self.current
    }

    /// The latest release if it is newer than the running app, or `None`.
    pub fn check(&self) -> Result<Option<Release>, Error> {
        let Some(latest) = github::latest_release(&self.agent, &self.repository)? else {
            return Ok(None);
        };
        github::newer_release(latest, &self.current, &platform::Platform::current())
    }

    /// Download `release` and put it in place of the running app. Returns the path to
    /// restart: the app bundle on macOS, the executable elsewhere.
    pub fn install(&self, release: &Release) -> Result<PathBuf, Error> {
        install::install(&self.agent, release)
    }
}
