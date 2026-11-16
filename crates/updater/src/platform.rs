//! Which release archive fits this build, named as `packaging/package.sh` names them.

/// The OS and CPU a release archive is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    /// `x64` or `arm64`.
    pub arch: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    fn name(self) -> &'static str {
        match self {
            Self::Mac => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Linux => "tar.gz",
            Self::Mac | Self::Windows => "zip",
        }
    }
}

impl Platform {
    /// The platform this binary was built for; an update keeps the same CPU type.
    pub fn current() -> Self {
        // `cfg!` rather than `#[cfg]`, so every variant is compiled on every platform.
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        Self { os, arch }
    }

    /// e.g. `vikui-1.2.0-linux-x64`, the archive's name without its extension. Linux and
    /// Windows archives unpack into a folder of this name.
    pub fn stem(self, version: &semver::Version) -> String {
        format!(
            "{}-{version}-{}-{}",
            crate::ASSET_PREFIX,
            self.os.name(),
            self.arch
        )
    }

    /// e.g. `vikui-1.2.0-linux-x64.tar.gz`.
    pub fn asset_name(self, version: &semver::Version) -> String {
        format!("{}.{}", self.stem(version), self.os.extension())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_assets_like_the_packaging_script() {
        let version = semver::Version::new(1, 2, 0);
        let name = |os, arch| Platform { os, arch }.asset_name(&version);
        assert_eq!(name(Os::Mac, "arm64"), "vikui-1.2.0-macos-arm64.zip");
        assert_eq!(name(Os::Linux, "x64"), "vikui-1.2.0-linux-x64.tar.gz");
        assert_eq!(name(Os::Windows, "arm64"), "vikui-1.2.0-windows-arm64.zip");
    }
}
