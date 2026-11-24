//! The tray icon (menu bar on macOS, notification area on Windows): lists the reminders
//! from the project or saved filter picked in Settings, optionally with their count.
//!
//! It shows while it is turned on and the user is signed in, reloads every two minutes
//! and whenever the reminders windows do, and keeps the app running with no windows
//! open. Linux has no tray (see the `tray` crate), so there it never shows.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Global, Task};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use preferences::{TrayPreferences, TraySource};
use reminders::{Reminder, Source, Status};
use tray::{Command, Entry, Tray};

use crate::app_session;

/// How often the reminders reload by themselves, so new and passed ones show.
const RELOAD_EVERY: Duration = Duration::from_mins(2);

/// The icon while it shows, and what keeps it up to date.
struct Shown {
    tray: Tray,
    /// The list its reminders come from, once loaded.
    source: Option<Source>,
    /// Bumped by each load, so only the latest one's result shows.
    load: u64,
    _commands: Task<()>,
    _reload: Task<()>,
}

#[derive(Default)]
struct AppTray(Option<Shown>);

impl Global for AppTray {}

/// Whether the icon shows, so the app keeps running when its last window closes.
pub fn is_shown(cx: &App) -> bool {
    cx.try_global::<AppTray>()
        .is_some_and(|tray| tray.0.is_some())
}

/// Show or remove the icon to match the preferences and whether the user is signed in,
/// and reload it. Call after either changes.
pub fn apply(cx: &mut App) {
    let preferences = crate::app_preferences::get(cx).tray;
    let wanted = preferences.enabled && tray::is_supported() && app_session::get(cx).is_some();
    if !wanted {
        cx.default_global::<AppTray>().0 = None;
        return;
    }
    if !is_shown(cx) {
        match show(cx) {
            Ok(shown) => cx.default_global::<AppTray>().0 = Some(shown),
            Err(err) => return log::warn!("{err}"),
        }
    }
    reload(cx);
}

fn show(cx: &mut App) -> Result<Shown, tray::Error> {
    let tray = Tray::new(crate::APP_NAME)?;
    let commands = tray.commands();
    let commands = cx.spawn(async move |cx| {
        while let Ok(command) = commands.recv().await {
            cx.update(|cx| run(command, cx));
        }
    });
    let reload = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(RELOAD_EVERY).await;
            cx.update(reload);
        }
    });
    Ok(Shown {
        tray,
        source: None,
        load: 0,
        _commands: commands,
        _reload: reload,
    })
}

/// Load the reminders into the icon's menu again, if it shows.
pub fn reload(cx: &mut App) {
    let Some(signed_in) = app_session::get(cx) else {
        return;
    };
    let preferences = crate::app_preferences::get(cx).tray;
    let Some(shown) = cx.default_global::<AppTray>().0.as_mut() else {
        return;
    };
    shown.load += 1;
    let load = shown.load;
    let Some(picked) = preferences.source.clone() else {
        shown.source = None;
        shown.tray.set_menu(&menu(&Loaded::NotPicked, &preferences));
        shown.tray.set_count(None, crate::APP_NAME);
        return;
    };
    let session = signed_in.session;
    let fetch = cx.background_spawn(async move {
        let projects = session.call(vikunja::Client::projects)?;
        let Some(source) = find(&picked, &projects) else {
            return Ok(None);
        };
        let tasks = session.call(|client, token| source.tasks(client, token))?;
        Ok(Some((source, reminders::from_tasks(&tasks))))
    });
    cx.spawn(async move |cx| {
        let result: Result<_, vikunja::Error> = fetch.await;
        cx.update(|cx| {
            if let Err(vikunja::Error::Unauthorized) = result {
                return (signed_in.on_session_expired)(cx);
            }
            let Some(shown) = cx
                .default_global::<AppTray>()
                .0
                .as_mut()
                .filter(|shown| shown.load == load)
            else {
                return;
            };
            let (loaded, source) = match result {
                Ok(Some((source, reminders))) => (
                    Loaded::Reminders {
                        title: source_title(&source),
                        reminders,
                    },
                    Some(source),
                ),
                Ok(None) => (Loaded::Gone, None),
                Err(err) => {
                    log::warn!("tray reminders didn't load: {err:#}");
                    (Loaded::Failed, None)
                }
            };
            let count = count(&loaded, Timestamp::now());
            shown.tray.set_menu(&menu(&loaded, &preferences));
            let tooltip = match count {
                Some(count) => format!("{} – {count} {}", crate::APP_NAME, reminder_s(count)),
                None => crate::APP_NAME.to_owned(),
            };
            shown
                .tray
                .set_count(count.filter(|_| preferences.show_count), &tooltip);
            if source.is_some() {
                shown.source = source;
            }
        });
    })
    .detach();
}

/// The list `picked` means among `projects`, with its subprojects; `None` if it is gone.
fn find(picked: &TraySource, projects: &[vikunja::Project]) -> Option<Source> {
    reminders::destinations(projects)
        .into_iter()
        .map(|destination| destination.source)
        .find(|source| match (picked, source) {
            (TraySource::Project { id, .. }, Source::Project { id: found, .. }) => id == found,
            (
                TraySource::SavedFilter { filter_id, .. },
                Source::SavedFilter {
                    filter_id: found, ..
                },
            ) => filter_id == found,
            _ => false,
        })
}

fn source_title(source: &Source) -> String {
    match source {
        Source::Project { title, .. } | Source::SavedFilter { title, .. } => title.clone(),
        Source::Filter(filter) | Source::Search(filter) => filter.clone(),
    }
}

/// Carry out what the user picked from the icon's menu.
fn run(command: Command, cx: &mut App) {
    // The menu doesn't bring the app forward, and the windows it opens should be.
    cx.activate(true);
    match command {
        Command::OpenTask(task_id) => crate::edit_task_window::open(task_id, cx),
        Command::OpenList => {
            let source = cx
                .try_global::<AppTray>()
                .and_then(|tray| tray.0.as_ref())
                .and_then(|shown| shown.source.clone());
            if let (Some(source), Some(signed_in)) = (source, app_session::get(cx)) {
                let expire = signed_in.on_session_expired;
                crate::reminders_window::open(&source, signed_in.session, move |cx| expire(cx), cx);
            }
        }
        Command::OpenApp => crate::open_projects(cx),
        Command::Refresh => reload(cx),
        Command::OpenSettings => crate::settings_window::open_tray(cx),
        Command::Quit => cx.quit(),
    }
}

/// What the last load found.
enum Loaded {
    /// No list is picked in Settings.
    NotPicked,
    Reminders {
        title: String,
        reminders: Vec<Reminder>,
    },
    /// The picked list is no longer on the server.
    Gone,
    Failed,
}

/// The reminders the menu lists: those still to come or passed while their task isn't
/// done, earliest first.
fn listed(reminders: &[Reminder], now: Timestamp) -> impl Iterator<Item = &Reminder> {
    reminders
        .iter()
        .filter(move |reminder| matches!(reminder.status(now), Status::Past | Status::Upcoming))
}

/// How many reminders the menu has, before its limit; `None` without a list.
fn count(loaded: &Loaded, now: Timestamp) -> Option<usize> {
    match loaded {
        Loaded::Reminders { reminders, .. } => Some(listed(reminders, now).count()),
        Loaded::NotPicked | Loaded::Gone | Loaded::Failed => None,
    }
}

fn reminder_s(count: usize) -> &'static str {
    if count == 1 { "reminder" } else { "reminders" }
}

/// The icon's menu: the list's reminders (at most `limit`), then the app's commands.
fn menu(loaded: &Loaded, preferences: &TrayPreferences) -> Vec<Entry> {
    menu_at(loaded, preferences, Timestamp::now(), &TimeZone::system())
}

fn menu_at(
    loaded: &Loaded,
    preferences: &TrayPreferences,
    now: Timestamp,
    zone: &TimeZone,
) -> Vec<Entry> {
    let item = |label: &str, command| Entry::Item {
        label: label.to_owned(),
        command,
    };
    let picked = preferences.source.as_ref().map(TraySource::title);
    let mut entries = Vec::new();
    match loaded {
        Loaded::NotPicked => {
            entries.push(Entry::Text(
                "Pick a project or saved filter in Settings".into(),
            ));
        }
        Loaded::Gone => entries.push(Entry::Text(format!(
            "“{}” is no longer on the server",
            picked.unwrap_or_default()
        ))),
        Loaded::Failed => entries.push(Entry::Text("Couldn't load the reminders".into())),
        Loaded::Reminders { title, reminders } => {
            entries.push(Entry::Text(title.clone()));
            let listed: Vec<&Reminder> = listed(reminders, now).collect();
            let limit = preferences
                .limit
                .and_then(|limit| usize::try_from(limit).ok())
                .unwrap_or(usize::MAX);
            for reminder in listed.iter().take(limit) {
                let when = reminder
                    .at
                    .map_or_default(|at| reminders::format_time(at, zone));
                // Said in words, not only by the menu's look.
                let passed = if reminder.is_past(now) {
                    " (passed)"
                } else {
                    ""
                };
                entries.push(item(
                    &format!("{when}{passed} – {}", reminder.task_title),
                    Command::OpenTask(reminder.task_id),
                ));
            }
            match listed.len().checked_sub(limit) {
                _ if listed.is_empty() => entries.push(Entry::Text("No reminders".into())),
                Some(more @ 1..) => entries.push(Entry::Text(format!("and {more} more"))),
                _ => {}
            }
            entries.push(Entry::Separator);
            entries.push(item(&format!("Open “{title}”"), Command::OpenList));
        }
    }
    if !matches!(loaded, Loaded::NotPicked) {
        entries.push(item("Refresh", Command::Refresh));
    }
    entries.extend([
        Entry::Separator,
        item(&format!("Open {}", crate::APP_NAME), Command::OpenApp),
        item("Settings…", Command::OpenSettings),
        Entry::Separator,
        item(&format!("Quit {}", crate::APP_NAME), Command::Quit),
    ]);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reminder(task_id: i64, title: &str, at: &str, done: bool) -> Reminder {
        Reminder {
            at: Some(at.parse().unwrap()),
            relative: None,
            task_id,
            task_title: title.into(),
            task_identifier: String::new(),
            task_priority: None,
            task_due: None,
            task_labels: Vec::new(),
            task_rgb: None,
            project_id: 1,
            task_done: done,
            is_set: true,
        }
    }

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Text(text) => format!("[{text}]"),
                Entry::Item { label, .. } => label.clone(),
                Entry::Separator => "---".into(),
            })
            .collect()
    }

    fn preferences(limit: Option<u32>) -> TrayPreferences {
        TrayPreferences {
            enabled: true,
            source: Some(TraySource::Project {
                id: 1,
                title: "Home".into(),
            }),
            limit,
            show_count: true,
        }
    }

    #[test]
    fn lists_open_reminders_up_to_the_limit() {
        let now: Timestamp = "2026-12-01T12:00:00Z".parse().unwrap();
        let loaded = Loaded::Reminders {
            title: "Home".into(),
            reminders: vec![
                reminder(1, "Bins", "2026-12-01T09:00:00Z", false),
                reminder(2, "Done already", "2026-12-01T10:00:00Z", true),
                reminder(3, "Rent", "2026-12-02T09:00:00Z", false),
                reminder(4, "Dentist", "2026-12-03T09:00:00Z", false),
            ],
        };
        assert_eq!(count(&loaded, now), Some(3));
        let entries = menu_at(&loaded, &preferences(Some(2)), now, &TimeZone::UTC);
        assert_eq!(
            labels(&entries),
            [
                "[Home]",
                "Tue 1 Dec 2026, 09:00 (passed) – Bins",
                "Wed 2 Dec 2026, 09:00 – Rent",
                "[and 1 more]",
                "---",
                "Open “Home”",
                "Refresh",
                "---",
                "Open Vikui",
                "Settings…",
                "---",
                "Quit Vikui",
            ]
        );
        assert!(entries.contains(&Entry::Item {
            label: "Wed 2 Dec 2026, 09:00 – Rent".into(),
            command: Command::OpenTask(3),
        }));

        let all = menu_at(&loaded, &preferences(None), now, &TimeZone::UTC);
        assert!(labels(&all).contains(&"Thu 3 Dec 2026, 09:00 – Dentist".to_owned()));
        assert!(!labels(&all).iter().any(|label| label.contains("more")));
    }

    #[test]
    fn says_why_nothing_is_listed() {
        let now = Timestamp::UNIX_EPOCH;
        let empty = Loaded::Reminders {
            title: "Home".into(),
            reminders: Vec::new(),
        };
        let first = |loaded: &Loaded| {
            labels(&menu_at(loaded, &preferences(None), now, &TimeZone::UTC))
                .into_iter()
                .take(2)
                .collect::<Vec<_>>()
        };
        assert_eq!(first(&empty), ["[Home]", "[No reminders]"]);
        assert_eq!(count(&empty, now), Some(0));
        assert_eq!(
            first(&Loaded::Gone),
            ["[“Home” is no longer on the server]", "Refresh"]
        );
        assert_eq!(
            first(&Loaded::Failed),
            ["[Couldn't load the reminders]", "Refresh"]
        );
        assert_eq!(
            first(&Loaded::NotPicked),
            ["[Pick a project or saved filter in Settings]", "---"]
        );
        assert_eq!(count(&Loaded::Failed, now), None);
    }

    #[test]
    fn finds_the_picked_list_with_its_subprojects() {
        let project = |id, title: &str, parent| vikunja::Project {
            id,
            title: title.into(),
            parent_project_id: parent,
            ..Default::default()
        };
        let projects = [
            project(1, "Home", 0),
            project(2, "Bills", 1),
            project(-3, "This week", 0),
        ];
        let home = TraySource::Project {
            id: 1,
            title: "Home".into(),
        };
        assert_eq!(
            find(&home, &projects),
            Some(Source::Project {
                id: 1,
                title: "Home".into(),
                subproject_ids: vec![2],
            })
        );
        let week = TraySource::SavedFilter {
            filter_id: 2,
            title: "Old name".into(),
        };
        assert_eq!(
            find(&week, &projects),
            Some(Source::SavedFilter {
                filter_id: 2,
                title: "This week".into(),
            })
        );
        let gone = TraySource::Project {
            id: 9,
            title: "Gone".into(),
        };
        assert_eq!(find(&gone, &projects), None);
    }
}
