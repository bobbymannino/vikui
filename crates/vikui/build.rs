//! Embeds the app icon in `vikui.exe`. GPUI loads icon resource 1 for the window and
//! taskbar; other platforms get their icon from the package (see `packaging/`).

const ICON: &str = "../../packaging/icon/vikui.ico";

// Resources can only be compiled on a Windows host, which is how releases are built.
#[cfg(windows)]
fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed={ICON}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon(ICON)
            .set("ProductName", "Vikui")
            .set("FileDescription", "Vikui")
            .compile()?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("cargo::rerun-if-changed={ICON}");
}
