//! Unpacking release archives. Both formats refuse entries that would land outside the
//! destination, and keep Unix permissions so executables stay executable.

use std::fs::File;
use std::path::Path;

use crate::Error;
use crate::platform::Os;

/// Unpack the archive at `archive` (in the format releases use on `os`) into `dest`.
pub fn unpack(archive: &Path, os: Os, dest: &Path) -> Result<(), Error> {
    let file = File::open(archive)?;
    match os {
        Os::Linux => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            tar.set_preserve_permissions(true);
            // `unpack` skips entries with `..` or absolute paths.
            tar.unpack(dest)
                .map_err(|err| Error::Archive(err.to_string()))
        }
        Os::Mac | Os::Windows => {
            let mut zip =
                zip::ZipArchive::new(file).map_err(|err| Error::Archive(err.to_string()))?;
            // Sanitizes paths with `enclosed_name` and applies stored Unix modes.
            zip.extract(dest)
                .map_err(|err| Error::Archive(err.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use super::*;

    fn read(path: &Path) -> String {
        let mut text = String::new();
        File::open(path).unwrap().read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn unpacks_tar_gz_keeping_the_executable_bit() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::fast(),
            );
            let mut tar = tar::Builder::new(gz);
            let body = b"#!/bin/sh\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "vikui-1.0.0-linux-x64/vikui", &body[..])
                .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let dest = dir.path().join("out");
        unpack(&archive, Os::Linux, &dest).unwrap();
        let exe = dest.join("vikui-1.0.0-linux-x64/vikui");
        assert_eq!(read(&exe), "#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn unpacks_zip() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            zip.start_file("Vikui.app/Contents/MacOS/vikui", options)
                .unwrap();
            zip.write_all(b"binary").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.path().join("out");
        unpack(&archive, Os::Mac, &dest).unwrap();
        let exe = dest.join("Vikui.app/Contents/MacOS/vikui");
        assert_eq!(read(&exe), "binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        std::fs::write(&archive, b"not an archive").unwrap();
        assert!(matches!(
            unpack(&archive, Os::Windows, dir.path()),
            Err(Error::Archive(_))
        ));
    }
}
