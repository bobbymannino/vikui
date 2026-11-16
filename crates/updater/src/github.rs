//! The GitHub releases API: the latest release and the asset for this platform.

use serde::Deserialize;
use ureq::Agent;

use crate::platform::Platform;
use crate::{Error, Release, Version};

/// One downloadable file of a release.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    pub size: u64,
    /// `sha256:<hex>`, computed by GitHub on upload; missing on very old assets.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Asset {
    /// The expected SHA-256 as lowercase hex, if GitHub reported one.
    pub fn sha256(&self) -> Option<String> {
        self.digest
            .as_deref()?
            .strip_prefix("sha256:")
            .map(str::to_ascii_lowercase)
    }
}

/// `GET /repos/{owner}/{repo}/releases/latest`.
#[derive(Clone, Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// `https://github.com/owner/name(.git)` → `owner/name`.
pub fn repository_path(url: &str) -> Option<String> {
    let path = url
        .trim()
        .strip_prefix("https://github.com/")?
        .trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then(|| format!("{owner}/{name}"))
}

/// The repository's latest published release, or `None` if it has none yet. GitHub never
/// returns drafts or prereleases here.
pub fn latest_release(agent: &Agent, repository: &str) -> Result<Option<GitHubRelease>, Error> {
    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let response = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call();
    let mut response = match response {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        // Unauthenticated clients get 60 requests an hour.
        Err(ureq::Error::StatusCode(403 | 429)) => {
            return Err(Error::Network(
                "too many update checks; try again later".to_owned(),
            ));
        }
        Err(err) => return Err(err.into()),
    };
    response
        .body_mut()
        .read_json()
        .map(Some)
        .map_err(|err| Error::BadResponse(err.to_string()))
}

/// `latest` as a [`Release`] for `platform` if it is newer than `current`.
pub fn newer_release(
    latest: GitHubRelease,
    current: &Version,
    platform: &Platform,
) -> Result<Option<Release>, Error> {
    if latest.draft || latest.prerelease {
        return Ok(None);
    }
    let tag = latest.tag_name.trim();
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|err| Error::BadResponse(format!("bad release tag {tag:?}: {err}")))?;
    if version <= *current {
        return Ok(None);
    }
    let name = platform.asset_name(&version);
    let asset = latest
        .assets
        .into_iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| Error::NoAsset(version.clone()))?;
    Ok(Some(Release {
        version,
        url: latest.html_url,
        asset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Os;

    const LINUX: Platform = Platform {
        os: Os::Linux,
        arch: "x64",
    };

    fn release(tag: &str) -> GitHubRelease {
        serde_json::from_value(serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/o/r/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
            "assets": [
                {
                    "name": format!("vikui-{}-linux-x64.tar.gz", tag.trim_start_matches('v')),
                    "browser_download_url": "https://example.com/linux.tar.gz",
                    "size": 42,
                    "digest": "sha256:ABCDEF"
                },
                {
                    "name": format!("vikui-{}-macos-arm64.zip", tag.trim_start_matches('v')),
                    "browser_download_url": "https://example.com/mac.zip",
                    "size": 7,
                    "digest": null
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn parses_repository_urls() {
        let path = |url| repository_path(url);
        assert_eq!(path("https://github.com/o/r").as_deref(), Some("o/r"));
        assert_eq!(path("https://github.com/o/r.git/").as_deref(), Some("o/r"));
        assert_eq!(path("https://gitlab.com/o/r"), None);
        assert_eq!(path("https://github.com/o"), None);
        assert_eq!(path("https://github.com/o/r/../x"), None);
    }

    #[test]
    fn offers_only_newer_releases_for_this_platform() {
        let current = Version::new(0, 1, 0);
        let offered = newer_release(release("v0.2.0"), &current, &LINUX)
            .unwrap()
            .unwrap();
        assert_eq!(offered.version, Version::new(0, 2, 0));
        assert_eq!(offered.asset.url, "https://example.com/linux.tar.gz");
        assert_eq!(offered.asset.sha256().as_deref(), Some("abcdef"));

        assert_eq!(
            newer_release(release("v0.1.0"), &current, &LINUX).unwrap(),
            None
        );
        assert_eq!(
            newer_release(release("0.0.9"), &current, &LINUX).unwrap(),
            None
        );

        let windows = Platform {
            os: Os::Windows,
            arch: "x64",
        };
        assert!(matches!(
            newer_release(release("v0.2.0"), &current, &windows),
            Err(Error::NoAsset(_))
        ));
        assert!(matches!(
            newer_release(release("nightly"), &current, &LINUX),
            Err(Error::BadResponse(_))
        ));
    }

    #[test]
    fn skips_drafts_and_prereleases() {
        let current = Version::new(0, 1, 0);
        let draft = GitHubRelease {
            draft: true,
            ..release("v0.2.0")
        };
        assert_eq!(newer_release(draft, &current, &LINUX).unwrap(), None);
        let pre = GitHubRelease {
            prerelease: true,
            ..release("v0.2.0")
        };
        assert_eq!(newer_release(pre, &current, &LINUX).unwrap(), None);
    }
}
