# Vikui

> [!NOTE]
> This project is not at all affiliated with Vikunja, it is just a pet project.

**Vikui** is a [GPUI](https://gpui.rs) application for interacting with a
[Vikunja](https://vikunja.io) instance.

## Install

Download the archive for your platform and CPU from [Releases](../../releases).
Pick `arm64` for Apple silicon and ARM PCs, `x64` for everything else.

| Platform | Archive                                |
| -------- | -------------------------------------- |
| macOS    | `vikui-<version>-macos-<arch>.zip`     |
| Linux    | `vikui-<version>-linux-<arch>.tar.gz`  |
| Windows  | `vikui-<version>-windows-<arch>.zip`   |

Vikui keeps your sign-in in the system credential store: Keychain on macOS,
Credential Manager on Windows and the Secret Service on Linux.

### macOS

macOS 11 (Big Sur) or later.

1. Unzip the archive and move `Vikui.app` to **Applications**.
2. The app isn't notarized, so macOS blocks it the first time. Right-click it and
   choose **Open**, then **Open** again. Or clear the quarantine flag:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Vikui.app
   ```

### Linux

Vikui needs a Vulkan driver, `libxkbcommon` and a Secret Service keyring
(GNOME Keyring or KWallet), which most desktops already have. On Debian or Ubuntu:

```sh
sudo apt install libvulkan1 mesa-vulkan-drivers libxkbcommon0 libxkbcommon-x11-0 \
  libwayland-client0 libx11-xcb1 libfontconfig1 gnome-keyring
```

On Fedora: `sudo dnf install vulkan-loader mesa-vulkan-drivers libxkbcommon-x11 gnome-keyring`.

Then extract the archive, put the binary on your `PATH` and add Vikui to your app
launcher:

```sh
tar -xzf vikui-<version>-linux-<arch>.tar.gz
cd vikui-<version>-linux-<arch>
install -Dm755 vikui ~/.local/bin/vikui
install -Dm644 dev.bobman.vikui.desktop ~/.local/share/applications/dev.bobman.vikui.desktop
install -Dm644 dev.bobman.vikui.png ~/.local/share/icons/hicolor/512x512/apps/dev.bobman.vikui.png
```

### Windows

Windows 10 or later.

1. Extract the zip (right-click → **Extract All…**) to a folder of your choice,
   e.g. `%LOCALAPPDATA%\Programs\Vikui`.
2. Run `vikui.exe`. It isn't code-signed, so SmartScreen may warn the first time:
   choose **More info** → **Run anyway**.
3. Optionally right-click `vikui.exe` → **Pin to Start**.

### From source

Install [Rust](https://rustup.rs) 1.100 or later, then:

```sh
cargo install --locked --git https://github.com/bobbymannino/vikui vikui
```

On Linux, also install the `-dev` packages for the libraries above
(`libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev
libfontconfig-dev libfreetype-dev pkg-config`).

## Updating

Vikui checks [Releases](../../releases) when it opens and shows the **Updates** tab of
**Settings** if a newer version is out; opening **Settings** (`Mod+,`) checks too. Turn
off **Check for updates on open** under **Updates** to check only when you choose **Check
for Updates…** from the Vikui menu or the button there. Click **Update to <version> and
Restart**: Vikui downloads it, checks its SHA-256 checksum, replaces itself and
relaunches. Prereleases are never offered.

Pick an appearance on the **Appearance** tab of **Settings**: **Light**, **Dark** or
**Auto**, which follows the system as it switches. Pick the theme shown while light and
the one shown while dark from the themes GPUI Kit publishes (Solarized, Catppuccin,
Gruvbox, Tokyo Night and more), bundled from its repository under the Apache 2.0 licence
(`crates/vikui/themes`).

Pick what double-clicking a reminder does on the **Reminders** tab of **Settings**: open
its details in a new window (the default), edit the task, mark it done or not done, or
open it in the browser.

Search by label as well as text: `*label` (as in Vikunja's quick add) finds tasks with a
label starting with that name, and `*"two words"` one with a space in it. `rent *bills`
finds tasks matching "rent" labelled Bills; several labels must all be on the task. This
works in the main window's search, the search window and a reminders window's filter.

Task descriptions are written in Markdown: **bold**, *italic*, `code`, links, headings,
lists and task lists (`- [ ]`) and quotes. The details panel shows them formatted, and
Vikui saves them in the rich text Vikunja's web app shows, so they look the same there.

Preferences are saved in `preferences.json` in `~/Library/Application Support/vikui` on
macOS, `~/.config/vikui` on Linux and `%APPDATA%\vikui` on Windows. They include how you
last arranged the projects and reminders tables: drag a header to move its column, drag
a header's edge to resize it and click a sort icon to sort, and windows opened later
come back the same way. Each kind of window (the main window, reminders, search, settings
and so on) also reopens at the size and position you last left one in, moved back onto a
connected display if its own was unplugged.

Vikui must be able to write to the folder it is installed in. On macOS, move
`Vikui.app` to **Applications** first; copies built from source update with `cargo install`.

## Keyboard shortcuts

`Mod` is Cmd on macOS and Ctrl on Linux and Windows.

| Shortcut          | Action                                   |
| ----------------- | ---------------------------------------- |
| `Mod+N`           | Add a new reminder                       |
| `Mod+0`           | Open the projects window                 |
| `Mod+1`–`Mod+9`   | Open one of the first nine projects      |
| `Mod+R`           | Refresh the projects or reminders list   |
| `Mod+F`           | Search all tasks / filter reminders      |
| `Mod+Shift+F`     | Search all tasks, from any window        |
| `Enter`           | Open a project / toggle reminder details |
| `Mod+Enter`       | Open the selected task in its own window |
| `Mod+O`           | Open the selected task in the browser    |
| `Mod+E`           | Edit the selected task                   |
| `Escape`          | Close reminder details                   |
| `T`               | Mark the selected task done or not done  |
| `Mod+,`           | Open settings                            |
| `Mod+W`           | Close window                             |
| `Mod+Q`           | Quit                                     |
| `Cmd+M`           | Minimize window (macOS)                  |
| `Cmd+H` / `⌥⌘H`   | Hide Vikui / hide others (macOS)         |

## Feedback

Found a bug or have an idea? Choose **Report a Bug…** or **Request a Feature…** on the
**About** tab of **Settings**, or open a [new issue](../../issues/new/choose). Reporting a
bug from Settings fills in your OS and Vikui version.
