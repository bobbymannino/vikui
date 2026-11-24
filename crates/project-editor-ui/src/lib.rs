//! Vikui's window to create and edit projects: every project in tree order, and a form
//! for a project's name, identifier, parent, colour and description.
//!
//! The app fills the list with [`ProjectEditorView::set_projects`] and saves what the
//! form reports through [`ProjectEditorEvent`], answering with
//! [`ProjectEditorView::saved`] or [`ProjectEditorView::set_error`]; this crate never
//! fetches or saves. Descriptions are edited as Markdown and reported as the rich text
//! (HTML) Vikunja stores.

mod choice;
mod project_tree;

use std::collections::HashSet;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState, ColorSelect};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter,
    Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, Rgba, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, WindowOptions, div, px,
};
use vikunja::{Project, ProjectDraft};

use choice::Choice;
use project_tree::TreeRow;

const CONTEXT: &str = "ProjectEditor";
const LIST_WIDTH: f32 = 240.;
/// How far each level of the tree is indented.
const INDENT: f32 = 14.;
/// The parent select's value for a top-level project.
const TOP_LEVEL: i64 = 0;
/// Vikunja's limit on a project identifier.
const IDENTIFIER_MAX: usize = 10;

gpui_kit::actions!(project_editor, [NewProject, Save]);

type ChoiceSelect = SelectState<SearchableVec<Choice>>;

/// Register the window's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`.
    // In this window Mod+N starts a new project rather than a new reminder.
    let context = gpui_kit::KeyBindingContextPredicate::parse(CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    let bindings: [(&str, Box<dyn gpui_kit::Action>); 2] = [
        ("secondary-n", Box::new(NewProject)),
        ("secondary-s", Box::new(Save)),
    ];
    for (keystrokes, action) in bindings {
        match KeyBinding::load(keystrokes, action, context.clone(), false, None, cx) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::error!("invalid key binding {keystrokes}: {err}"),
        }
    }
}

/// Events for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectEditorEvent {
    /// Create this project; answer with [`ProjectEditorView::saved`] or
    /// [`ProjectEditorView::set_error`].
    Create(ProjectDraft),
    /// Save this over `project_id`; answer as for `Create`.
    Update {
        project_id: i64,
        draft: ProjectDraft,
    },
}

/// What the form is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    New,
    Project(i64),
}

/// The projects list and the form editing one of them, or a new one.
pub struct ProjectEditorView {
    projects: Vec<Project>,
    /// `projects` in tree order.
    rows: Vec<TreeRow>,
    editing: Editing,
    title: Entity<InputState>,
    identifier: Entity<InputState>,
    parent: Entity<ChoiceSelect>,
    color: Entity<ColorPickerState>,
    description: Entity<TextareaState>,
    /// Waiting for the projects.
    loading: bool,
    load_error: Option<SharedString>,
    saving: bool,
    error: Option<SharedString>,
    /// Said after a save, until the form changes.
    status: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProjectEditorEvent> for ProjectEditorView {}

impl ProjectEditorView {
    /// An empty list, loading until [`Self::set_projects`] or [`Self::set_load_error`].
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let identifier = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. HOME"));
        let parent = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let color = cx.new(|cx| ColorPickerState::new(window, cx));
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        let subscriptions = vec![
            cx.subscribe(&title, Self::on_input),
            cx.subscribe(&identifier, Self::on_input),
            cx.subscribe(&description, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.clear_status(cx);
                }
            }),
            cx.subscribe(
                &parent,
                |this, _, _: &SelectEvent<SearchableVec<Choice>>, cx| {
                    this.clear_status(cx);
                },
            ),
            cx.subscribe(&color, |this, _, _: &ColorPickerEvent, cx| {
                this.clear_status(cx);
            }),
        ];
        title.read(cx).focus_handle(cx).focus(window, cx);
        Self {
            projects: Vec::new(),
            rows: Vec::new(),
            editing: Editing::New,
            title,
            identifier,
            parent,
            color,
            description,
            loading: true,
            load_error: None,
            saving: false,
            error: None,
            status: None,
            _subscriptions: subscriptions,
        }
    }

    /// List `projects` (Favorites and saved filters are left out). The form keeps what
    /// it is editing if that project is still listed, and otherwise starts a new one.
    pub fn set_projects(
        &mut self,
        projects: Vec<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows = project_tree::rows(&projects);
        self.projects = projects;
        self.loading = false;
        self.load_error = None;
        match self.editing {
            Editing::Project(id) if self.rows.iter().any(|row| row.id == id) => {
                // Other projects may have moved; refresh the parents on offer.
                let parent = self.parent.read(cx).selected_value().copied();
                self.offer_parents(parent, window, cx);
            }
            Editing::Project(_) | Editing::New => self.new_project(window, cx),
        }
        cx.notify();
    }

    /// Show why the projects couldn't be loaded.
    pub fn set_load_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.loading = false;
        self.load_error = Some(message.into());
        cx.notify();
    }

    /// Show why the save failed, keeping the form as it is to try again.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message.into());
        cx.notify();
    }

    /// Show `project` as saved: listed (replacing the project it was) and open in the
    /// form.
    pub fn saved(&mut self, project: Project, window: &mut Window, cx: &mut Context<Self>) {
        let created = self.editing == Editing::New;
        self.saving = false;
        self.error = None;
        let id = project.id;
        let mut projects = std::mem::take(&mut self.projects);
        projects.retain(|listed| listed.id != id);
        projects.push(project);
        self.editing = Editing::Project(id);
        self.set_projects(projects, window, cx);
        self.edit(id, window, cx);
        self.status = Some(
            if created {
                "Project created."
            } else {
                "Changes saved."
            }
            .into(),
        );
        cx.notify();
    }

    /// The project the form is editing, by id; `None` for a new one.
    #[must_use]
    pub fn editing(&self) -> Option<i64> {
        match self.editing {
            Editing::New => None,
            Editing::Project(id) => Some(id),
        }
    }

    /// The form as filled in, its description as rich text.
    #[must_use]
    pub fn draft(&self, cx: &App) -> ProjectDraft {
        let description = self.description.read(cx).value();
        ProjectDraft {
            title: self.title.read(cx).value().trim().to_owned(),
            description: if description.trim().is_empty() {
                String::new()
            } else {
                description::to_html(&description)
            },
            identifier: self.identifier.read(cx).value().trim().to_owned(),
            rgb: self.color.read(cx).value().map(to_rgb),
            parent_id: self
                .parent
                .read(cx)
                .selected_value()
                .copied()
                .filter(|&id| id != TOP_LEVEL),
        }
    }

    /// Open `project_id` in the form.
    fn edit(&mut self, project_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
        else {
            return;
        };
        let fields = Fields {
            title: project.title.clone().into(),
            identifier: project.identifier.clone().into(),
            description: description::to_markdown(
                &project.description,
                description::LineBreaks::Newline,
            )
            .into(),
            rgb: project.rgb(),
            parent: project.parent_id(),
        };
        self.editing = Editing::Project(project_id);
        self.fill(fields, window, cx);
    }

    /// Clear the form for a new project.
    fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Editing::New;
        self.fill(Fields::default(), window, cx);
    }

    fn fill(&mut self, fields: Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.title
            .update(cx, |input, cx| input.set_value(fields.title, window, cx));
        self.identifier.update(cx, |input, cx| {
            input.set_value(fields.identifier, window, cx);
        });
        self.description.update(cx, |input, cx| {
            input.set_value(fields.description, window, cx);
        });
        self.color.update(cx, |color, cx| match fields.rgb {
            Some(rgb) => color.set_value(gpui_kit::rgb(rgb), window, cx),
            None => color.clear_value(window, cx),
        });
        self.offer_parents(fields.parent, window, cx);
        // Setting the fields reports changes; the form starts out unchanged.
        self.status = None;
        self.title.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Offer every project the one edited may sit under, selecting `parent` (or the top
    /// level). It can't sit under itself or a project under it.
    fn offer_parents(&mut self, parent: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.parent_choices();
        let selected = parent
            .filter(|id| choices.iter().any(|choice| choice.id == *id))
            .unwrap_or(TOP_LEVEL);
        self.parent.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(choices), window, cx);
            select.set_selected_value(&selected, window, cx);
        });
    }

    /// The top level, then every project the one edited may sit under: not itself or a
    /// project under it.
    fn parent_choices(&self) -> Vec<Choice> {
        let excluded = match self.editing {
            Editing::New => HashSet::new(),
            Editing::Project(id) => project_tree::self_and_descendants(&self.rows, id),
        };
        std::iter::once(Choice::new(TOP_LEVEL, "None (top level)"))
            .chain(
                self.rows
                    .iter()
                    .filter(|row| !excluded.contains(&row.id))
                    .map(|row| Choice::new(row.id, row.path.clone())),
            )
            .collect()
    }

    fn clear_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.color
            .update(cx, |color, cx| color.clear_value(window, cx));
        self.clear_status(cx);
    }

    fn clear_status(&mut self, cx: &mut Context<Self>) {
        if self.status.take().is_some() {
            cx.notify();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.loading {
            return;
        }
        let draft = self.draft(cx);
        let problem = if draft.title.is_empty() {
            Some("Enter a name.".to_owned())
        } else if draft.identifier.chars().count() > IDENTIFIER_MAX {
            Some(format!(
                "Keep the identifier to {IDENTIFIER_MAX} characters."
            ))
        } else {
            None
        };
        if let Some(problem) = problem {
            self.error = Some(problem.into());
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.emit(match self.editing {
            Editing::New => ProjectEditorEvent::Create(draft),
            Editing::Project(project_id) => ProjectEditorEvent::Update { project_id, draft },
        });
        cx.notify();
    }

    /// Enter in a one-line field saves; typing clears the "saved" note.
    fn on_input(&mut self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::PressEnter { .. } => self.save(cx),
            InputEvent::Change => self.clear_status(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn on_new_project(&mut self, _: &NewProject, window: &mut Window, cx: &mut Context<Self>) {
        self.new_project(window, cx);
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let message = |text: &'static str| {
            div()
                .p_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let body = if self.loading {
            message("Loading…")
        } else if let Some(error) = self.load_error.clone() {
            div()
                .id("projects-load-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .child(Alert::error("projects-load-error-alert", error))
                .test_support()
                .into_any_element()
        } else if self.rows.is_empty() {
            message("No projects yet.")
        } else {
            v_flex()
                .gap_0p5()
                .children(self.rows.iter().enumerate().map(|(ix, row)| {
                    let id = row.id;
                    // Decoration only: the title names the project.
                    let dot = div().flex_none().size_2p5().rounded_full();
                    let dot = match row.rgb {
                        Some(rgb) => dot.bg(gpui_kit::rgb(rgb)),
                        None => dot,
                    };
                    #[expect(clippy::cast_precision_loss, reason = "tree depths are small")]
                    let indent = px(INDENT * row.depth as f32);
                    Button::new(("project", ix))
                        .ghost()
                        .w_full()
                        .justify_start()
                        .accessibility_label(format!("Edit project {}", row.path))
                        .selected(self.editing == Editing::Project(id))
                        .child(
                            h_flex()
                                .pl(indent)
                                .gap_2()
                                .items_center()
                                .min_w_0()
                                .child(dot)
                                .child(div().truncate().child(row.title.clone())),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit(id, window, cx);
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("projects-list")
            .role(Role::List)
            .aria_label("Projects")
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .pr_3()
            .child(body)
            .test_support()
            .into_any_element()
    }

    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (heading, submit, submit_name) = match self.editing {
            Editing::New => ("New Project", "Create Project", "Create project"),
            Editing::Project(_) => ("Edit Project", "Save Changes", "Save changes"),
        };
        let has_color = self.color.read(cx).value().is_some();
        let error = self.error.clone().map(|message| {
            // `Alert` has no accessible name of its own; announce the message.
            div()
                .id("project-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(Alert::error("project-error-alert", message))
                .test_support()
        });
        let status = self.status.clone().map(|message| {
            div()
                .id("project-status")
                .role(Role::Status)
                .aria_label(message.clone())
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(message)
                .test_support()
        });
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(heading),
            )
            .children(error)
            .child(
                div()
                    .id("project-fields")
                    .flex_1()
                    .min_h_0()
                    // Room for the focus rings, which the scroll area would clip.
                    .m_neg_1()
                    .p_1()
                    .overflow_y_scroll()
                    .child(
                        v_form()
                            .child(
                                field().label("Name").required(true).child(
                                    Input::new(&self.title)
                                        .id("project-title")
                                        .aria_label("Name"),
                                ),
                            )
                            .child(
                                field()
                                    .label("Identifier")
                                    .description("Starts its tasks' IDs, e.g. HOME-12.")
                                    .child(
                                        Input::new(&self.identifier)
                                            .id("project-identifier")
                                            .aria_label("Identifier"),
                                    ),
                            )
                            .child(
                                field().label("Parent project").child(
                                    Select::new(&self.parent)
                                        .id("project-parent")
                                        .accessibility_label("Parent project"),
                                ),
                            )
                            .child(
                                field().label("Colour").child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            ColorSelect::new(&self.color)
                                                .placeholder("No colour")
                                                .accessibility_label("Colour"),
                                        )
                                        .child(
                                            Button::new("clear-project-color")
                                                .ghost()
                                                .icon(IconName::Close)
                                                .label("No colour")
                                                .accessibility_label("No colour")
                                                .disabled(!has_color)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.clear_color(window, cx);
                                                })),
                                        ),
                                ),
                            )
                            .child(
                                field()
                                    .label("Description")
                                    .description("Supports Markdown.")
                                    .child(
                                        Textarea::new(&self.description)
                                            .h(px(120.))
                                            .aria_label("Description"),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().children(status))
                    .child(
                        Button::new("save-project")
                            .primary()
                            .label(if self.saving { "Saving…" } else { submit })
                            .accessibility_label(submit_name)
                            .tooltip_with_action(submit_name, &Save, Some(CONTEXT))
                            .loading(self.saving)
                            .disabled(self.saving || self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .into_any_element()
    }
}

/// What the form shows for one project.
#[derive(Default)]
struct Fields {
    title: SharedString,
    identifier: SharedString,
    /// Markdown.
    description: SharedString,
    rgb: Option<u32>,
    parent: Option<i64>,
}

/// `color` as `0xRRGGBB`, ignoring its opacity: Vikunja's colours have none.
fn to_rgb(color: Hsla) -> u32 {
    let rgba = Rgba::from(color);
    let channel = |value: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded and clamped to 0..=255"
        )]
        let byte = (value.clamp(0., 1.) * 255.).round() as u32;
        byte
    };
    (channel(rgba.r) << 16) | (channel(rgba.g) << 8) | channel(rgba.b)
}

impl Render for ProjectEditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.render_list(cx);
        let form = self.render_form(cx);
        let theme = cx.theme();
        v_flex()
            .id("project-editor")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_new_project))
            .on_action(cx.listener(Self::on_save))
            .size_full()
            .gap_4()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Projects"),
                    )
                    .child(
                        Button::new("new-project")
                            .outline()
                            .icon(IconName::Plus)
                            .label("New Project")
                            .accessibility_label("New project")
                            .tooltip_with_action("New project", &NewProject, Some(CONTEXT))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_project(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .gap_6()
                    .child(list)
                    .child(form),
            )
            .test_support()
    }
}

/// Open the window, loading until [`ProjectEditorView::set_projects`].
pub fn open(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<ProjectEditorView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| ProjectEditorView::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};

    use super::*;

    type Events = Rc<RefCell<Vec<ProjectEditorEvent>>>;

    fn project(id: i64, title: &str, parent: i64) -> Project {
        Project {
            id,
            title: title.into(),
            parent_project_id: parent,
            ..Project::default()
        }
    }

    fn projects() -> Vec<Project> {
        vec![
            Project {
                description: "<p>Chores and <strong>bills</strong></p>".into(),
                identifier: "HOME".into(),
                hex_color: "1973ff".into(),
                ..project(1, "Home", 0)
            },
            project(2, "Garden", 1),
            project(3, "Work", 0),
            project(-2, "This week", 0),
        ]
    }

    fn opened(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ProjectEditorView>, Events) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx).unwrap()
        });
        let events: Events = Rc::default();
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &ProjectEditorEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.set_projects(projects(), window, cx));
        })
        .unwrap();
        (handle, view, events)
    }

    fn has_label(cx: &mut TestAppContext, handle: AnyWindowHandle, label: &str) -> bool {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find_by_label(label).is_some()
        })
        .unwrap()
    }

    fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn set_input(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        input: &Entity<InputState>,
        value: &str,
    ) {
        let value = SharedString::from(value.to_owned());
        cx.update_window(handle, |_, window, cx| {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        })
        .unwrap();
    }

    /// The parent select's choices, by title.
    fn parents(view: &Entity<ProjectEditorView>, cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            view.read(cx)
                .parent_choices()
                .into_iter()
                .map(|choice| choice.title.to_string())
                .collect()
        })
    }

    #[gpui_kit::test]
    fn lists_projects_as_a_tree_and_edits_one(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        // Projects only, each under its parent; not the saved filter.
        assert!(has_label(cx, handle, "Edit project Home"));
        assert!(has_label(cx, handle, "Edit project Home › Garden"));
        assert!(has_label(cx, handle, "Edit project Work"));
        assert!(!has_label(cx, handle, "Edit project This week"));

        click(cx, handle, ("project", 0_usize));
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(1));
        assert!(has_label(cx, handle, "Save changes"));
        let draft = cx.update(|cx| view.read(cx).draft(cx));
        assert_eq!(
            (draft.title.as_str(), draft.identifier.as_str()),
            ("Home", "HOME")
        );
        assert_eq!((draft.rgb, draft.parent_id), (Some(0x19_73_ff), None));
        assert_eq!(
            cx.update(|cx| view.read(cx).description.read(cx).value()),
            "Chores and **bills**"
        );
        // Home can't sit under itself or Garden, which is under it.
        assert_eq!(parents(&view, cx), ["None (top level)", "Work"]);

        cx.update_window(handle, |_, window, cx| {
            let parent = view.read(cx).parent.clone();
            parent.update(cx, |select, cx| select.set_selected_value(&3, window, cx));
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-s", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let events_now = events.borrow().clone();
        let [ProjectEditorEvent::Update { project_id, draft }] = events_now.as_slice() else {
            panic!("expected one update, got {events_now:?}");
        };
        let project_id = *project_id;
        assert_eq!(project_id, 1);
        assert_eq!(draft.parent_id, Some(3));
        assert_eq!(
            draft.description,
            "<p>Chores and <strong>bills</strong></p>"
        );

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.saved(project(1, "Home", 3), window, cx);
            });
        })
        .unwrap();
        assert!(has_label(cx, handle, "Changes saved."));
        assert!(has_label(cx, handle, "Edit project Work › Home"));
    }

    #[gpui_kit::test]
    fn creates_a_project_once_it_is_named(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        assert!(has_label(cx, handle, "Create project"));
        let (title, identifier) = cx.update(|cx| {
            let view = view.read(cx);
            (view.title.clone(), view.identifier.clone())
        });
        assert_eq!(
            parents(&view, cx),
            ["None (top level)", "Home", "Home › Garden", "Work"]
        );

        click(cx, handle, "save-project");
        assert!(has_label(cx, handle, "Enter a name."));
        set_input(cx, handle, &title, "Car");
        set_input(cx, handle, &identifier, "WAYTOOLONGID");
        click(cx, handle, "save-project");
        assert!(has_label(
            cx,
            handle,
            "Keep the identifier to 10 characters."
        ));
        assert!(events.borrow().is_empty());

        set_input(cx, handle, &identifier, "CAR");
        click(cx, handle, "save-project");
        assert_eq!(
            *events.borrow(),
            [ProjectEditorEvent::Create(ProjectDraft {
                title: "Car".into(),
                description: String::new(),
                identifier: "CAR".into(),
                rgb: None,
                parent_id: None,
            })]
        );

        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Forbidden", cx)));
        assert!(has_label(cx, handle, "Forbidden"));
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.saved(project(9, "Car", 0), window, cx));
        })
        .unwrap();
        assert!(has_label(cx, handle, "Project created."));
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(9));

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-n", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), None);
    }

    #[gpui_kit::test]
    fn shows_why_the_projects_did_not_load(cx: &mut TestAppContext) {
        let (handle, view, _) = opened(cx);
        cx.update(|cx| view.update(cx, |view, cx| view.set_load_error("Offline", cx)));
        assert!(has_label(cx, handle, "Offline"));
    }
}
