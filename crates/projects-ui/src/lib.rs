//! The list of the signed-in user's Vikunja projects.

mod projects_table;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::table::{DataTable, TableState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, KeyBindingContextPredicate,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _,
};
use vikunja::{Client, Project, Token};

use projects_table::{ProjectRow, ProjectsTable};

gpui_kit::actions!(projects, [Refresh]);

const CONTEXT: &str = "Projects";

/// Fetches the user's projects; blocking, so it runs on the background executor.
pub type LoadProjects = Arc<dyn Fn() -> Result<Vec<Project>, vikunja::Error> + Send + Sync>;

/// Load projects from `client` as the user identified by `token`.
pub fn loader(client: Client, token: Token) -> LoadProjects {
    Arc::new(move || client.projects(&token))
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
}

impl ProjectsView {
    /// Starts loading immediately and focuses the table.
    pub fn new(load_projects: LoadProjects, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| TableState::new(ProjectsTable::default(), window, cx));
        table.read(cx).focus_handle(cx).focus(window, cx);

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            load_projects,
            table,
            loading: false,
            error: None,
            summary: SharedString::default(),
            load_task: None,
        };
        this.reload(window, cx);
        this
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
            load().map(|projects| ProjectRow::from_projects(&projects))
        });
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.finish_load(result, cx))
                .ok();
        }));
    }

    fn finish_load(
        &mut self,
        result: Result<Vec<ProjectRow>, vikunja::Error>,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        match result {
            Ok(rows) => {
                self.summary = match rows.len() {
                    1 => "1 project".into(),
                    n => format!("{n} projects").into(),
                };
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(rows);
                    cx.notify();
                });
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

    fn on_refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loading {
            self.reload(window, cx);
        }
    }
}

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
        let load: LoadProjects = Arc::new(|| Err(vikunja::Error::Unauthorized));
        let handle = open_projects(cx, load);

        cx.wait_for(handle, Duration::from_secs(5), |window, _| {
            window
                .try_find_by_label(
                    "Couldn't load projects: Your session has expired. Sign in again.",
                )
                .is_some()
        })
        .await;
    }
}
