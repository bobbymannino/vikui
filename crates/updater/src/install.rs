//! Downloading a release and putting it in place of the running app.
//!
//! The archive is downloaded and unpacked into a staging folder beside the app, so the
//! final step is two renames on one file system: the running app is moved aside, the new
//! one moved in, and the old one restored if that fails. A running program may be renamed
//! on every platform; it keeps running from the old files until it restarts.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use ureq::Agent;

use crate::github::Asset;
use crate::platform::{Os, Platform};
use crate::{Error, Release, archive};

/// What gets replaced, and what to launch afterwards.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    /// The app bundle on macOS, the executable elsewhere.
    path: PathBuf,
}

impl Target {
    /// The running app, if it was installed from a release.
    fn current() -> Result<Self, Error> {
        if cfg!(debug_assertions) {
            return Err(Error::NotInstalled("it's a development build"));
        }
        let exe = std::env::current_exe()?.canonicalize()?;
        Self::for_exe(&exe, Platform::current().os)
    }

    fn for_exe(exe: &Path, os: Os) -> Result<Self, Error> {
        match os {
            Os::Mac => {
                // macOS runs quarantined apps from a read-only random folder.
                if exe
                    .components()
                    .any(|part| part.as_os_str() == "AppTranslocation")
                {
                    return Err(Error::NotInstalled(
                        "move Vikui to the Applications folder first",
                    ));
                }
                // …/Vikui.app/Contents/MacOS/vikui
                let bundle = exe
                    .ancestors()
                    .nth(3)
                    .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
                    .filter(|bundle| exe.parent() == Some(&bundle.join("Contents").join("MacOS")))
                    .ok_or(Error::NotInstalled("it isn't running from Vikui.app"))?;
                Ok(Self {
                    path: bundle.to_path_buf(),
                })
            }
            Os::Linux | Os::Windows => Ok(Self {
                path: exe.to_path_buf(),
            }),
        }
    }
}

/// Where the running app is moved while the new one takes its place. On Windows a running
/// executable can't be deleted, so it is left beside the new one until [`clean_up`].
fn backup_path(target: &Path, staging: &Path, os: Os) -> PathBuf {
    match os {
        Os::Windows => windows_backup(target),
        Os::Mac | Os::Linux => staging.join("previous"),
    }
}

/// `vikui.exe` → `vikui.exe.old`.
fn windows_backup(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Remove what a previous update left behind. Call once at startup; cheap and quiet.
pub fn clean_up() {
    #[cfg(target_os = "windows")]
    if let Ok(exe) = std::env::current_exe() {
        match fs::remove_file(windows_backup(&exe)) {
            Ok(()) => log::info!("removed the previous version after an update"),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => log::warn!("couldn't remove the previous version: {err}"),
        }
    }
    // macOS and Linux delete the previous version during the update itself.
    #[cfg(not(target_os = "windows"))]
    {}
}

pub fn install(agent: &Agent, release: &Release) -> Result<PathBuf, Error> {
    let platform = Platform::current();
    let target = Target::current()?;
    let parent = target
        .path
        .parent()
        .ok_or(Error::NotInstalled("it has no parent folder"))?;
    let staging = tempfile::Builder::new()
        .prefix(".vikui-update-")
        .tempdir_in(parent)
        .map_err(|err| writable(err, parent))?;

    let archive = staging.path().join("download");
    download(agent, &release.asset, &archive)?;
    let unpacked = staging.path().join("unpacked");
    fs::create_dir(&unpacked)?;
    archive::unpack(&archive, platform.os, &unpacked)?;
    let new = unpacked_app(&unpacked, platform, &release.version)?;

    let backup = backup_path(&target.path, staging.path(), platform.os);
    swap(&target.path, &new, &backup).map_err(|err| writable(err, parent))?;
    log::info!(
        "installed Vikui {} at {}",
        release.version,
        target.path.display()
    );
    Ok(target.path)
}

/// Permission errors name the folder the user must fix.
fn writable(err: io::Error, folder: &Path) -> Error {
    if err.kind() == io::ErrorKind::PermissionDenied {
        Error::NotWritable(folder.to_path_buf())
    } else {
        Error::Io(err)
    }
}

/// The app inside an unpacked archive: `Vikui.app` on macOS, `<stem>/vikui(.exe)` else.
fn unpacked_app(
    unpacked: &Path,
    platform: Platform,
    version: &semver::Version,
) -> Result<PathBuf, Error> {
    let (path, is_dir) = match platform.os {
        Os::Mac => (unpacked.join("Vikui.app"), true),
        Os::Linux => (unpacked.join(platform.stem(version)).join("vikui"), false),
        Os::Windows => (
            unpacked.join(platform.stem(version)).join("vikui.exe"),
            false,
        ),
    };
    let found = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir() == is_dir);
    if found {
        Ok(path)
    } else {
        Err(Error::Archive(format!(
            "{} is missing from the download",
            path.strip_prefix(unpacked).unwrap_or(&path).display()
        )))
    }
}

/// Move `target` to `backup` and `new` to `target`, restoring `target` if that fails.
fn swap(target: &Path, new: &Path, backup: &Path) -> io::Result<()> {
    match fs::remove_file(backup) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    fs::rename(target, backup)?;
    if let Err(err) = fs::rename(new, target) {
        if let Err(restore) = fs::rename(backup, target) {
            log::error!(
                "couldn't restore {} from {}: {restore}",
                target.display(),
                backup.display()
            );
        }
        return Err(err);
    }
    Ok(())
}

/// Download `asset` to `dest`, checking its size and SHA-256 digest.
fn download(agent: &Agent, asset: &Asset, dest: &Path) -> Result<(), Error> {
    let expected = asset
        .sha256()
        .ok_or_else(|| Error::BadResponse("the download has no checksum".to_owned()))?;
    let response = agent.get(&asset.url).call()?;
    let mut body = response
        .into_body()
        .into_with_config()
        .limit(asset.size)
        .reader();
    let mut file = HashingWriter {
        file: File::create(dest)?,
        hasher: Sha256::new(),
        written: 0,
    };
    io::copy(&mut body, &mut file)?;
    file.file.sync_all()?;
    if file.written != asset.size || hex(&file.hasher.finalize()) != expected {
        return Err(Error::Checksum);
    }
    Ok(())
}

/// Writes to a file while hashing what passes through.
struct HashingWriter {
    file: File,
    hasher: Sha256,
    written: u64,
}

impl Write for HashingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        let chunk = buf.get(..written).unwrap_or_default();
        self.hasher.update(chunk);
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_or_executable_to_replace() {
        let mac = Target::for_exe(
            Path::new("/Applications/Vikui.app/Contents/MacOS/vikui"),
            Os::Mac,
        )
        .unwrap();
        assert_eq!(mac.path, Path::new("/Applications/Vikui.app"));
        assert!(matches!(
            Target::for_exe(Path::new("/Users/me/code/target/release/vikui"), Os::Mac),
            Err(Error::NotInstalled(_))
        ));
        assert!(matches!(
            Target::for_exe(
                Path::new(
                    "/private/var/folders/x/AppTranslocation/1/d/Vikui.app/Contents/MacOS/vikui"
                ),
                Os::Mac
            ),
            Err(Error::NotInstalled(_))
        ));
        let linux = Target::for_exe(Path::new("/opt/vikui/vikui"), Os::Linux).unwrap();
        assert_eq!(linux.path, Path::new("/opt/vikui/vikui"));
    }

    #[test]
    fn windows_keeps_the_old_exe_beside_the_new_one() {
        let staging = Path::new("staging");
        assert_eq!(
            backup_path(Path::new("C:/Vikui/vikui.exe"), staging, Os::Windows),
            Path::new("C:/Vikui/vikui.exe.old")
        );
        assert_eq!(
            backup_path(Path::new("/opt/vikui"), staging, Os::Linux),
            staging.join("previous")
        );
    }

    #[test]
    fn swaps_in_the_new_app_and_restores_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Vikui.app");
        let new = dir.path().join("new.app");
        let backup = dir.path().join("previous");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("version"), "1").unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(new.join("version"), "2").unwrap();

        swap(&target, &new, &backup).unwrap();
        assert_eq!(fs::read_to_string(target.join("version")).unwrap(), "2");
        assert_eq!(fs::read_to_string(backup.join("version")).unwrap(), "1");

        // A missing new app leaves the current one in place.
        let file = dir.path().join("vikui");
        fs::write(&file, "current").unwrap();
        let file_backup = dir.path().join("vikui.old");
        assert!(swap(&file, &dir.path().join("missing"), &file_backup).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "current");
    }

    #[test]
    fn finds_the_app_in_the_unpacked_archive() {
        let dir = tempfile::tempdir().unwrap();
        let version = semver::Version::new(1, 0, 0);
        let linux = Platform {
            os: Os::Linux,
            arch: "x64",
        };
        assert!(matches!(
            unpacked_app(dir.path(), linux, &version),
            Err(Error::Archive(_))
        ));
        let folder = dir.path().join("vikui-1.0.0-linux-x64");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("vikui"), "").unwrap();
        assert_eq!(
            unpacked_app(dir.path(), linux, &version).unwrap(),
            folder.join("vikui")
        );
    }

    #[test]
    fn hashes_what_it_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = HashingWriter {
            file: File::create(dir.path().join("f")).unwrap(),
            hasher: Sha256::new(),
            written: 0,
        };
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.written, 3);
        assert_eq!(
            hex(&writer.hasher.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
