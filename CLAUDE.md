# Agent Instructions

Vikui: a cross-platform GPUI desktop client for Vikunja (see `README.md`).

## Layout

- Cargo workspace; crates live in `crates/`.
- `crates/vikui` is the app binary and the workspace default member; it only boots the app and routes between windows.
- `crates/vikunja`: blocking Vikunja REST client, no UI.
- `crates/login`: sign-in and keychain session storage, no UI.
- `crates/login-ui`: sign-in window; reports success via callback, never opens other windows itself.
- `crates/overview-ui`: main window shell; hosts a content view passed in by the app.
- `crates/projects-ui`: project list; reports opening a project and session expiry via events.
- `crates/reminders`: turns tasks into a sorted reminder list, no UI.
- `crates/reminders-ui`: reminders window; lists the reminders it is given, never fetches. Clicking a row toggles a details panel supplied by the app.
- `crates/reminder-details`: builds the details of one reminder and its task, no UI.
- `crates/reminder-details-ui`: details side panel; shows the details it is given and reports close via an event.
- `crates/settings-ui`: settings window; shows the app info and update status it is given, reports update requests via events.
- `crates/preferences`: loads and saves the user's preferences file, no UI.
- `crates/updater`: checks GitHub releases and installs an update in place, no UI.
- UI crates must not depend on each other; wire them together in `crates/vikui`.
- Shared `version`/`edition` live in root `Cargo.toml` `[workspace.package]`; new crates inherit them.
- Put shared dependency versions in root `[workspace.dependencies]` once a second crate needs them.

## GPUI Kit

- Depend on `gpui-kit` only (keep it on the latest release); never add `gpui`, `gpui-component` or `gpui-base` directly.
- Prefer existing `gpui_kit::component` widgets over hand-built `div()` controls.
- Use theme colors/spacing from the active theme; never hard-code colors or font sizes.
- Docs: https://gpui-kit.com and https://docs.rs/gpui-kit; read source in `~/.cargo/registry/src/*/gpui-kit-*` when unsure.
- UI tests: enable `gpui-kit = { ..., features = ["test-support"] }` under `[dev-dependencies]` and use `#[gpui_kit::test]` with `gpui_kit::test::*`.

## Performance

- Keep `render` cheap and side-effect free: no I/O, allocation-heavy work or blocking calls.
- Run network/disk work with `cx.spawn` / `cx.background_spawn`; update entities on completion.
- Call `cx.notify()` only when state actually changed.
- Split large views into separate entities so updates re-render only what changed.
- Use virtualized lists (`uniform_list` / component `List`/`Table`) for task collections.
- Use `SharedString` for text passed into elements; avoid per-frame `String` clones.
- Never `unwrap()`/`expect()` on network or user data; surface errors in the UI.

## Cross-Platform (Linux, macOS, Windows)

- Use `std::path`/`PathBuf` and the `dirs` crate for config/data paths; never hard-code separators or home paths.
- Gate OS-specific code with `#[cfg(target_os = "...")]` and provide every platform's branch.
- Bind shortcuts with `secondary-` (Cmd on macOS, Ctrl elsewhere), not `cmd-` or `ctrl-`.
- Do not assume window decorations, titlebar height or a single display scale factor.

## Accessibility

- Every interactive element needs a stable `id` and an accessible name (`.aria_label(...)` / component `accessibility_label(...)`).
- Set `.role(...)` on custom interactive `div`s.
- All actions must be reachable by keyboard; use `track_focus`, `tab_index` and `on_action` with key bindings.
- Keep focus visible and order logical; restore focus when dialogs close.
- Never convey state by color alone; meet WCAG AA contrast in light and dark themes.
- Assert labels/roles in UI tests with `find_by_label` / `find_all_by_role`.

## Code Style

- Edition 2024; code must be `cargo fmt` clean and `clippy -D warnings` clean.
- One view/entity per module; keep `main.rs` to app bootstrap.
- Keep Vikunja API types and HTTP client separate from UI code.
- No `unsafe` without a `// SAFETY:` comment.

## Before Finishing

- Run `cargo fmt --all`, `cargo lint` (clippy, `-D warnings`) and `cargo test -p <crate>`.
- Lints live in root `Cargo.toml` `[workspace.lints]`; every crate sets `[lints] workspace = true`. Fix lints rather than `#[allow]`-ing them; justify any `#[expect(...)]` with a reason.
