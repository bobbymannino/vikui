//! The list of the signed-in user's Vikunja projects.

mod projects_table;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, KeyBindingContextPredicate,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use table_layout::TableLayout;
use vikunja::{Project, ProjectKind};

use projects_table::{ProjectRow, ProjectsTable};

gpui_kit::actions!(projects, [Refresh, OpenProject]);

const CONTEXT: &str = "Projects";

/// Fetches the user's projects; blocking, so it runs on the background executor.
pub type LoadProjects = Arc<dyn Fn() -> Result<Vec<Project>, vikunja::Error> + Send + Sync>;

/// Events for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectsEvent {
    /// The server rejected the session and it could not be renewed; sign in again.
    SessionExpired,
    /// The user opened a project (double-click, or Enter on the selected row).
    OpenProject {
        id: i64,
        title: SharedString,
        /// Every project below it, at any depth.
        subproject_ids: Vec<i64>,
    },
    /// The user opened a saved filter, the same way.
    OpenSavedFilter { filter_id: i64, title: SharedString },
    /// The projects (and saved filters) loaded, on open or after a refresh.
    Loaded(Vec<Project>),
    /// The user moved, resized or sorted a column. Save the layout and restore it with
    /// [`ProjectsView::set_table_layout`].
    TableLayoutChanged(TableLayout),
}

/// Register the projects view's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`
    // to Cmd on macOS and Ctrl elsewhere.
    let context = KeyBindingContextPredicate::parse(CONTEXT).ok().map(Rc::new);
    match KeyBinding::load("secondary-r", Box::new(Refresh), context, false, None, cx) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(err) => log::error!("invalid refresh key binding: {err}"),
    }
    cx.bind_keys([KeyBinding::new("enter", OpenProject, Some(CONTEXT))]);
}

/// A table of the user's projects with a refresh control.
pub struct ProjectsView {
    focus_handle: FocusHandle,
    load_projects: LoadProjects,
    table: Entity<TableState<ProjectsTable>>,
    loading: bool,
    error: Option<SharedString>,
    summary: SharedString,
    /// Dropping the task discards a superseded load's result.
    load_task: Option<Task<()>>,
    /// The table layout last reported or restored, by [`ProjectsTable::layout_revision`].
    layout_revision: u64,
    _table_events: Subscription,
    _table_layout: Subscription,
}

impl ProjectsView {
    /// Starts loading immediately and focuses the table.
    pub fn new(load_projects: LoadProjects, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableState::new(ProjectsTable::default(), window, cx));
        table.read(cx).focus_handle(cx).focus(window, cx);
        let table_events = cx.subscribe(&table, |this, _, event: &TableEvent, cx| match event {
            TableEvent::DoubleClickedRow(row_ix) => this.open_row(*row_ix, cx),
            TableEvent::ColumnWidthsChanged(widths) => this.table.update(cx, |table, cx| {
                table.delegate_mut().set_widths(widths);
                cx.notify();
            }),
            _ => {}
        });
        // Moves, resizes and sorts all notify the table; report the layout once it changed.
        let table_layout = cx.observe(&table, |this, table, cx| {
            let table = table.read(cx).delegate();
            if table.layout_revision() != this.layout_revision {
                this.layout_revision = table.layout_revision();
                cx.emit(ProjectsEvent::TableLayoutChanged(table.layout()));
            }
        });

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            load_projects,
            table,
            loading: false,
            error: None,
            summary: SharedString::default(),
            load_task: None,
            layout_revision: 0,
            _table_events: table_events,
            _table_layout: table_layout,
        };
        this.reload(window, cx);
        this
    }

    /// Arrange the table's columns and sort as the user last left them, from a
    /// [`ProjectsEvent::TableLayoutChanged`].
    pub fn set_table_layout(&mut self, layout: &TableLayout, cx: &mut Context<Self>) {
        self.layout_revision = self.table.update(cx, |table, cx| {
            table.delegate_mut().set_layout(layout);
            table.refresh(cx);
            cx.notify();
            table.delegate().layout_revision()
        });
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.table.update(cx, |table, cx| {
            // Keep showing stale rows while refreshing; only skeleton the first load.
            if table.delegate().rows().is_empty() {
                table.delegate_mut().set_loading(true);
                cx.notify();
            }
        });
        cx.notify();

        let load = Arc::clone(&self.load_projects);
        let task = cx.background_spawn(async move {
            load().map(|projects| (ProjectRow::from_projects(&projects), projects))
        });
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.finish_load(result, cx))
                .ok();
        }));
    }

    fn finish_load(
        &mut self,
        result: Result<(Vec<ProjectRow>, Vec<Project>), vikunja::Error>,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        match result {
            Ok((rows, projects)) => {
                self.summary = summary(&rows);
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(rows);
                    cx.notify();
                });
                cx.emit(ProjectsEvent::Loaded(projects));
            }
            Err(vikunja::Error::Unauthorized) => {
                log::info!("session expired while loading projects");
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_loading(false);
                    cx.notify();
                });
                cx.emit(ProjectsEvent::SessionExpired);
            }
            Err(err) => {
                log::warn!("failed to load projects: {err:#}");
                self.error = Some(err.to_string().into());
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_loading(false);
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    fn open_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let table = self.table.read(cx).delegate();
        let Some(row) = table.row(row_ix) else {
            return;
        };
        let title = row.title.clone();
        let event = match row.kind {
            ProjectKind::SavedFilter { filter_id } => {
                ProjectsEvent::OpenSavedFilter { filter_id, title }
            }
            ProjectKind::Project | ProjectKind::Favorites => ProjectsEvent::OpenProject {
                id: row.id,
                title,
                subproject_ids: table.subproject_ids(row.id),
            },
        };
        cx.emit(event);
    }

    fn on_open_project(&mut self, _: &OpenProject, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.open_row(row_ix, cx);
        }
    }

    fn on_refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loading {
            self.reload(window, cx);
        }
    }
}

/// "3 projects, 1 saved filter".
fn summary(rows: &[ProjectRow]) -> SharedString {
    let filters = rows
        .iter()
        .filter(|row| matches!(row.kind, ProjectKind::SavedFilter { .. }))
        .count();
    let projects = rows.len() - filters;
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut text = plural(projects, "project");
    if filters > 0 {
        text.push_str(", ");
        text.push_str(&plural(filters, "saved filter"));
    }
    text.into()
}

impl EventEmitter<ProjectsEvent> for ProjectsView {}

impl Focusable for ProjectsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProjectsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("projects")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_project))
            .size_full()
            .gap_4()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Projects"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(self.summary.clone()),
                    )
                    .child(
                        Button::new("refresh-projects")
                            .outline()
                            .label(if self.loading {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            })
                            .accessibility_label("Refresh projects")
                            .tooltip_with_action("Refresh projects", &Refresh, Some(CONTEXT))
                            .loading(self.loading)
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_refresh(&Refresh, window, cx);
                            })),
                    ),
            )
            .when_some(self.error.clone(), |this, message| {
                this.child(
                    // `Alert` has no accessible name of its own; announce the message.
                    div()
                        .id("projects-error")
                        .role(Role::Alert)
                        .aria_label(format!("Couldn't load projects: {message}"))
                        .child(
                            Alert::error("projects-error-alert", message)
                                .title("Couldn't load projects"),
                        )
                        .test_support(),
                )
            })
            .child(
                div().flex_1().min_h_0().child(
                    DataTable::new(&self.table)
                        .stripe(true)
                        .accessibility_label("Projects"),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui_kit::test::{TestAppContextExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{
        AnyWindowHandle, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    fn project(id: i64, title: &str) -> Project {
        Project {
            id,
            title: title.into(),
            description: "<p>About it</p>".into(),
            identifier: String::new(),
            parent_project_id: 0,
            is_archived: false,
            is_favorite: false,
            hex_color: String::new(),
        }
    }

    fn open_projects(cx: &mut TestAppContext, load: LoadProjects) -> AnyWindowHandle {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(960.), px(640.)),
                })),
                ..Default::default()
            };
            let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| ProjectsView::new(load, window, cx))
            })
            .unwrap();
            handle
        })
    }

    #[test]
    fn summary_counts_projects_and_saved_filters() {
        let rows = |projects: &[Project]| ProjectRow::from_projects(projects);
        assert_eq!(summary(&rows(&[project(1, "A")])), "1 project");
        assert_eq!(
            summary(&rows(&[project(1, "A"), project(2, "B"), project(-2, "F")])),
            "2 projects, 1 saved filter"
        );
    }

    #[gpui_kit::test]
    async fn lists_projects_from_the_server(cx: &mut TestAppContext) {
        let load: LoadProjects = Arc::new(|| {
            Ok(vec![
                project(-1, "Favorites"),
                project(1, "Work"),
                project(2, "Home"),
            ])
        });
        let handle = open_projects(cx, load);

        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Work").is_some()
        })
        .await;

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Projects");
            window.find_by_label("Home");
            assert!(window.try_find_by_label("Favorites").is_none());
            let refresh = window.find_by_label("Refresh projects");
            assert_eq!(refresh.role(), Some(Role::Button));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    async fn restores_and_reports_the_table_layout(cx: &mut TestAppContext) {
        let load: LoadProjects = Arc::new(|| Ok(vec![project(1, "Work"), project(2, "Home")]));
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ProjectsView::new(load, window, cx))
            })
            .unwrap()
        });
        let layouts = Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let layouts = Rc::clone(&layouts);
            cx.subscribe(&view, move |_, event: &ProjectsEvent, _| {
                if let ProjectsEvent::TableLayoutChanged(layout) = event {
                    layouts.borrow_mut().push(layout.clone());
                }
            })
            .detach();
        });
        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Work").is_some()
        })
        .await;

        let saved = TableLayout {
            columns: vec![table_layout::ColumnLayout {
                key: "description".into(),
                width: 300,
            }],
            sort: Some(table_layout::Sort {
                column: "title".into(),
                direction: table_layout::SortDirection::Ascending,
            }),
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_table_layout(&saved, cx)));
        cx.run_until_parked();
        assert!(
            layouts.borrow().is_empty(),
            "restoring isn't a change to save"
        );
        cx.update(|cx| {
            let table = view.read(cx).table.read(cx);
            assert_eq!(
                table.headers(cx),
                ["Description", "Title", "Type", "Identifier"]
            );
            assert_eq!(table.col_width(0), Some(px(300.)));
            let titles: Vec<_> = table
                .delegate()
                .rows()
                .iter()
                .map(|r| r.title.as_ref())
                .collect();
            assert_eq!(titles, ["Home", "Work"], "sorted by title, as saved");
        });

        // Title is now second; sorting it again after ascending clears the sort.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(gpui_kit::ElementId::from(("icon-sort", 1_usize)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let reported = layouts
            .borrow()
            .last()
            .cloned()
            .expect("a layout was reported");
        assert_eq!(
            reported.columns.first().map(|col| col.key.as_str()),
            Some("description")
        );
        assert_eq!(reported.sort, None);
    }

    #[gpui_kit::test]
    async fn secondary_r_refreshes(cx: &mut TestAppContext) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let load: LoadProjects = Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![project(1, "Work")])
        });
        let handle = open_projects(cx, load);
        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Work").is_some()
        })
        .await;

        let key = if cfg!(target_os = "macos") {
            "cmd-r"
        } else {
            "ctrl-r"
        };
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[gpui_kit::test]
    async fn shows_load_errors(cx: &mut TestAppContext) {
        let load: LoadProjects = Arc::new(|| {
            Err(vikunja::Error::Api {
                status: 500,
                code: None,
                message: "Database unavailable".into(),
            })
        });
        let handle = open_projects(cx, load);

        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window
                .try_find_by_label("Couldn't load projects: Database unavailable")
                .is_some()
        })
        .await;
    }

    #[gpui_kit::test]
    fn rejected_session_emits_session_expired(cx: &mut TestAppContext) {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let load: LoadProjects = Arc::new(|| Err(vikunja::Error::Unauthorized));
            let (_, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| ProjectsView::new(load, window, cx))
            })
            .unwrap();
            cx.subscribe(&view, move |_, event: &ProjectsEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
        });
        cx.run_until_parked();
        assert_eq!(*events.borrow(), [ProjectsEvent::SessionExpired]);
    }

    #[gpui_kit::test]
    async fn enter_opens_the_selected_project(cx: &mut TestAppContext) {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let load: LoadProjects = Arc::new(|| Ok(vec![project(1, "Work"), project(2, "Home")]));
            let (handle, view) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| ProjectsView::new(load, window, cx))
                })
                .unwrap();
            cx.subscribe(&view, move |_, event: &ProjectsEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
            (handle, view)
        });
        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Home").is_some()
        })
        .await;

        cx.update(|cx| {
            let table = view.read(cx).table.clone();
            table.update(cx, |table, cx| table.set_selected_row(1, cx));
        });
        cx.update_window(handle, |_, window, cx| window.press("enter", cx))
            .unwrap();
        assert_eq!(
            *events.borrow(),
            [
                ProjectsEvent::Loaded(vec![project(1, "Work"), project(2, "Home")]),
                ProjectsEvent::OpenProject {
                    id: 2,
                    title: "Home".into(),
                    subproject_ids: Vec::new(),
                }
            ]
        );
    }

    #[gpui_kit::test]
    async fn enter_opens_a_saved_filter(cx: &mut TestAppContext) {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let load: LoadProjects =
                Arc::new(|| Ok(vec![project(1, "Work"), project(-3, "Due this week")]));
            let (handle, view) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| ProjectsView::new(load, window, cx))
                })
                .unwrap();
            cx.subscribe(&view, move |_, event: &ProjectsEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
            (handle, view)
        });
        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window.try_find_by_label("Saved filter").is_some()
        })
        .await;

        cx.update(|cx| {
            let table = view.read(cx).table.clone();
            table.update(cx, |table, cx| table.set_selected_row(1, cx));
        });
        cx.update_window(handle, |_, window, cx| window.press("enter", cx))
            .unwrap();
        assert_eq!(
            events.borrow()[1..],
            [ProjectsEvent::OpenSavedFilter {
                filter_id: 2,
                title: "Due this week".into()
            }]
        );
    }
}
