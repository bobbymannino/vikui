# Vikui

> [!NOTE]
> This project is not at all affiliated with Vikunja, it is just a pet project.

**Vikui** is a [GPUI](https://gpui.rs) application for interacting with a
[Vikunja](https://vikunja.io) instance.

## Install

Download the archive for your platform from
[Releases](../../releases) (x64 and arm64 builds for macOS, Linux and Windows).

On macOS, unzip it and move `Vikui.app` to Applications. The app isn't notarized,
so macOS blocks it the first time it opens: right-click it and choose **Open**, or run
`xattr -dr com.apple.quarantine /Applications/Vikui.app`.

## Keyboard shortcuts

`Mod` is Cmd on macOS and Ctrl on Linux and Windows.

| Shortcut          | Action                                   |
| ----------------- | ---------------------------------------- |
| `Mod+R`           | Refresh the projects or reminders list   |
| `Enter`           | Open a project / toggle reminder details |
| `Escape`          | Close reminder details                   |
| `T`               | Mark the selected task done or not done  |
| `Mod+W`           | Close window                             |
| `Mod+Q`           | Quit                                     |
| `Cmd+M`           | Minimize window (macOS)                  |
| `Cmd+H` / `⌥⌘H`   | Hide Vikui / hide others (macOS)         |
