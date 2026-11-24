//! Vikui's window to create and edit saved filters: every saved filter, and a form for a
//! filter's name, query and description.
//!
//! The app fills the list with [`FilterEditorView::set_filters`] and saves what the
//! form reports through [`FilterEditorEvent`], answering with
//! [`FilterEditorView::saved`] or [`FilterEditorView::set_error`]; this crate never
//! fetches or saves. Descriptions are edited as Markdown and reported as the rich text
//! (HTML) Vikunja stores.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, WindowOptions, div, px,
};
use vikunja::{FilterDraft, SavedFilter};

const CONTEXT: &str = "FilterEditor";
const LIST_WIDTH: f32 = 220.;

gpui_kit::actions!(filter_editor, [NewFilter, Save]);

/// Register the window's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`.
    // In this window Mod+N starts a new filter rather than a new reminder.
    let context = gpui_kit::KeyBindingContextPredicate::parse(CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    let bindings: [(&str, Box<dyn gpui_kit::Action>); 2] = [
        ("secondary-n", Box::new(NewFilter)),
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
pub enum FilterEditorEvent {
    /// Create this filter; answer with [`FilterEditorView::saved`] or
    /// [`FilterEditorView::set_error`].
    Create(FilterDraft),
    /// Save this over `filter_id`; answer as for `Create`.
    Update { filter_id: i64, draft: FilterDraft },
}

/// What the form is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    New,
    Filter(i64),
}

/// The saved filters list and the form editing one of them, or a new one.
pub struct FilterEditorView {
    /// Sorted by title.
    filters: Vec<SavedFilter>,
    editing: Editing,
    title: Entity<InputState>,
    query: Entity<InputState>,
    include_nulls: bool,
    description: Entity<TextareaState>,
    /// Waiting for the filters.
    loading: bool,
    load_error: Option<SharedString>,
    saving: bool,
    error: Option<SharedString>,
    /// Said after a save, until the form changes.
    status: Option<SharedString>,
    /// Asked to open before the filters loaded; opened once they do.
    pending_edit: Option<i64>,
    _subscriptions: [Subscription; 3],
}

impl EventEmitter<FilterEditorEvent> for FilterEditorView {}

impl FilterEditorView {
    /// An empty list, loading until [`Self::set_filters`] or [`Self::set_load_error`].
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Filter name"));
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("done = false && due_date < now/w+1w")
        });
        let description = cx.new(|cx| TextareaState::new(window, cx).placeholder("Description"));
        let subscriptions = [
            cx.subscribe(&title, Self::on_input),
            cx.subscribe(&query, Self::on_input),
            cx.subscribe(&description, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.clear_status(cx);
                }
            }),
        ];
        title.read(cx).focus_handle(cx).focus(window, cx);
        Self {
            filters: Vec::new(),
            editing: Editing::New,
            title,
            query,
            include_nulls: false,
            description,
            loading: true,
            load_error: None,
            saving: false,
            error: None,
            status: None,
            pending_edit: None,
            _subscriptions: subscriptions,
        }
    }

    /// List `filters`, sorted by title. The form keeps what it is editing if that filter
    /// is still listed, and otherwise starts a new one.
    pub fn set_filters(
        &mut self,
        mut filters: Vec<SavedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        filters.sort_by_key(|filter| (filter.title.to_lowercase(), filter.id));
        self.filters = filters;
        self.loading = false;
        self.load_error = None;
        if let Editing::Filter(id) = self.editing
            && !self.filters.iter().any(|filter| filter.id == id)
        {
            self.new_filter(window, cx);
        }
        if let Some(filter_id) = self.pending_edit.take() {
            self.edit(filter_id, window, cx);
        }
        cx.notify();
    }

    /// Open `filter_id` in the form, e.g. from the projects table's context menu; if the
    /// filters haven't loaded yet, once they do. Does nothing while a save runs.
    pub fn show_filter(&mut self, filter_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.loading {
            self.pending_edit = Some(filter_id);
        } else {
            self.edit(filter_id, window, cx);
        }
    }

    /// Show why the filters couldn't be loaded.
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

    /// Show `filter` as saved: listed (replacing the filter it was) and open in the form.
    pub fn saved(&mut self, filter: SavedFilter, window: &mut Window, cx: &mut Context<Self>) {
        let created = self.editing == Editing::New;
        self.saving = false;
        self.error = None;
        let id = filter.id;
        let mut filters = std::mem::take(&mut self.filters);
        filters.retain(|listed| listed.id != id);
        filters.push(filter);
        self.editing = Editing::Filter(id);
        self.set_filters(filters, window, cx);
        self.edit(id, window, cx);
        self.status = Some(
            if created {
                "Filter created."
            } else {
                "Changes saved."
            }
            .into(),
        );
        cx.notify();
    }

    /// The filter the form is editing, by id; `None` for a new one.
    #[must_use]
    pub fn editing(&self) -> Option<i64> {
        match self.editing {
            Editing::New => None,
            Editing::Filter(id) => Some(id),
        }
    }

    /// The form as filled in, its description as rich text.
    #[must_use]
    pub fn draft(&self, cx: &App) -> FilterDraft {
        let description = self.description.read(cx).value();
        FilterDraft {
            title: self.title.read(cx).value().trim().to_owned(),
            description: if description.trim().is_empty() {
                String::new()
            } else {
                description::to_html(&description)
            },
            filter: self.query.read(cx).value().trim().to_owned(),
            include_nulls: self.include_nulls,
        }
    }

    /// Open `filter_id` in the form.
    fn edit(&mut self, filter_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(filter) = self.filters.iter().find(|filter| filter.id == filter_id) else {
            return;
        };
        let fields = Fields {
            title: filter.title.clone().into(),
            query: filter.filters.filter.clone().into(),
            include_nulls: filter.filters.filter_include_nulls,
            description: description::to_markdown(
                &filter.description,
                description::LineBreaks::Newline,
            )
            .into(),
        };
        self.editing = Editing::Filter(filter_id);
        self.fill(fields, window, cx);
    }

    /// Clear the form for a new filter.
    fn new_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Editing::New;
        self.fill(Fields::default(), window, cx);
    }

    fn fill(&mut self, fields: Fields, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.title
            .update(cx, |input, cx| input.set_value(fields.title, window, cx));
        self.query
            .update(cx, |input, cx| input.set_value(fields.query, window, cx));
        self.description.update(cx, |input, cx| {
            input.set_value(fields.description, window, cx);
        });
        self.include_nulls = fields.include_nulls;
        // Setting the fields reports changes; the form starts out unchanged.
        self.status = None;
        self.title.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn set_include_nulls(&mut self, include: bool, cx: &mut Context<Self>) {
        if self.include_nulls != include {
            self.include_nulls = include;
            self.status = None;
            cx.notify();
        }
    }

    /// Enter in a one-line field saves; typing clears the "saved" note.
    fn on_input(&mut self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::PressEnter { .. } => self.save(cx),
            InputEvent::Change => self.clear_status(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
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
            Some("Enter a name.")
        } else if draft.filter.is_empty() {
            Some("Enter a query, e.g. done = false.")
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
            Editing::New => FilterEditorEvent::Create(draft),
            Editing::Filter(filter_id) => FilterEditorEvent::Update { filter_id, draft },
        });
        cx.notify();
    }

    fn on_new_filter(&mut self, _: &NewFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.new_filter(window, cx);
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
                .id("filters-load-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .child(Alert::error("filters-load-error-alert", error))
                .test_support()
                .into_any_element()
        } else if self.filters.is_empty() {
            message("No saved filters yet.")
        } else {
            v_flex()
                .gap_0p5()
                .children(self.filters.iter().enumerate().map(|(ix, filter)| {
                    let id = filter.id;
                    let title = SharedString::from(filter.title.clone());
                    Button::new(("filter", ix))
                        .ghost()
                        .w_full()
                        .justify_start()
                        .icon(IconName::Search)
                        .label(title.clone())
                        .accessibility_label(format!("Edit filter {title}"))
                        .selected(self.editing == Editing::Filter(id))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit(id, window, cx);
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("filters-list")
            .role(Role::List)
            .aria_label("Saved filters")
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
            Editing::New => ("New Saved Filter", "Create Filter", "Create filter"),
            Editing::Filter(_) => ("Edit Saved Filter", "Save Changes", "Save changes"),
        };
        let error = self.error.clone().map(|message| {
            // `Alert` has no accessible name of its own; announce the message.
            div()
                .id("filter-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(Alert::error("filter-error-alert", message))
                .test_support()
        });
        let status = self.status.clone().map(|message| {
            div()
                .id("filter-status")
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
                    .id("filter-fields")
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
                                        .id("filter-title")
                                        .aria_label("Name"),
                                ),
                            )
                            .child(
                                field()
                                    .label("Query")
                                    .required(true)
                                    .description(
                                        "Vikunja's filter syntax: compare done, priority, \
                                         due_date, labels, project and more with =, !=, <, >, \
                                         in, and join with && or ||.",
                                    )
                                    .child(
                                        Input::new(&self.query)
                                            .id("filter-query")
                                            .aria_label("Query"),
                                    ),
                            )
                            .child(
                                field().child(
                                    Checkbox::new("filter-include-nulls")
                                        .label("Also show tasks without the compared fields")
                                        .checked(self.include_nulls)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.set_include_nulls(*checked, cx);
                                        })),
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
                        Button::new("save-filter")
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

/// What the form shows for one filter.
#[derive(Default)]
struct Fields {
    title: SharedString,
    query: SharedString,
    include_nulls: bool,
    /// Markdown.
    description: SharedString,
}

impl Render for FilterEditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.render_list(cx);
        let form = self.render_form(cx);
        let theme = cx.theme();
        v_flex()
            .id("filter-editor")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_new_filter))
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
                            .child("Saved Filters"),
                    )
                    .child(
                        Button::new("new-filter")
                            .outline()
                            .icon(IconName::Plus)
                            .label("New Filter")
                            .accessibility_label("New filter")
                            .tooltip_with_action("New filter", &NewFilter, Some(CONTEXT))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_filter(window, cx);
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

/// Open the window, loading until [`FilterEditorView::set_filters`].
pub fn open(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<FilterEditorView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| FilterEditorView::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use vikunja::SavedQuery;

    use super::*;

    type Events = Rc<RefCell<Vec<FilterEditorEvent>>>;

    fn filter(id: i64, title: &str, query: &str) -> SavedFilter {
        SavedFilter {
            id,
            title: title.into(),
            description: String::new(),
            filters: SavedQuery {
                filter: query.into(),
                ..SavedQuery::default()
            },
        }
    }

    fn opened(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<FilterEditorView>, Events) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx).unwrap()
        });
        let events: Events = Rc::default();
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &FilterEditorEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let filters = vec![
            filter(2, "this week", "due_date < now/w+1w"),
            SavedFilter {
                description: "<p>Things <em>overdue</em></p>".into(),
                filters: SavedQuery {
                    filter: "done = false && due_date < now".into(),
                    filter_include_nulls: true,
                    ..SavedQuery::default()
                },
                ..filter(1, "Overdue", "")
            },
        ];
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.set_filters(filters, window, cx));
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

    #[gpui_kit::test]
    fn edits_a_listed_filter(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        assert!(has_label(cx, handle, "Edit filter Overdue"));
        assert!(has_label(cx, handle, "Edit filter this week"));

        // Sorted by title, ignoring case: Overdue first.
        click(cx, handle, ("filter", 0_usize));
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(1));
        assert!(has_label(cx, handle, "Save changes"));
        assert_eq!(
            cx.update(|cx| view.read(cx).description.read(cx).value()),
            "Things *overdue*"
        );

        // Unticking the box keeps the rest.
        click(cx, handle, "filter-include-nulls");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-s", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [FilterEditorEvent::Update {
                filter_id: 1,
                draft: FilterDraft {
                    title: "Overdue".into(),
                    description: "<p>Things <em>overdue</em></p>".into(),
                    filter: "done = false && due_date < now".into(),
                    include_nulls: false,
                },
            }]
        );

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.saved(filter(1, "Late", "done = false"), window, cx);
            });
        })
        .unwrap();
        assert!(has_label(cx, handle, "Changes saved."));
        assert!(has_label(cx, handle, "Edit filter Late"));
        assert!(!has_label(cx, handle, "Edit filter Overdue"));
    }

    #[gpui_kit::test]
    fn creates_a_filter_with_a_name_and_query(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        let (title, query) = cx.update(|cx| {
            let view = view.read(cx);
            (view.title.clone(), view.query.clone())
        });
        assert!(has_label(cx, handle, "Create filter"));

        click(cx, handle, "save-filter");
        assert!(has_label(cx, handle, "Enter a name."));
        set_input(cx, handle, &title, "Urgent");
        click(cx, handle, "save-filter");
        assert!(has_label(cx, handle, "Enter a query, e.g. done = false."));
        assert!(events.borrow().is_empty());

        set_input(cx, handle, &query, " priority >= 4 ");
        click(cx, handle, "save-filter");
        assert_eq!(
            *events.borrow(),
            [FilterEditorEvent::Create(FilterDraft {
                title: "Urgent".into(),
                description: String::new(),
                filter: "priority >= 4".into(),
                include_nulls: false,
            })]
        );

        // The server rejects a query it can't parse; the form stays to fix it.
        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Invalid filter", cx)));
        assert!(has_label(cx, handle, "Invalid filter"));
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.saved(filter(7, "Urgent", "priority >= 4"), window, cx);
            });
        })
        .unwrap();
        assert!(has_label(cx, handle, "Filter created."));
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(7));

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-n", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), None);
    }

    #[gpui_kit::test]
    fn shows_a_filter_asked_for_before_or_after_loading(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx).unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.show_filter(2, window, cx));
            let filters = vec![
                filter(1, "Overdue", "due_date < now"),
                filter(2, "Week", "x"),
            ];
            view.update(cx, |view, cx| view.set_filters(filters, window, cx));
        })
        .unwrap();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(2));
        assert_eq!(cx.update(|cx| view.read(cx).draft(cx).title), "Week");

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.show_filter(1, window, cx));
        })
        .unwrap();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(1));
    }

    #[gpui_kit::test]
    fn shows_why_the_filters_did_not_load(cx: &mut TestAppContext) {
        let (handle, view, _) = opened(cx);
        cx.update(|cx| view.update(cx, |view, cx| view.set_load_error("Offline", cx)));
        assert!(has_label(cx, handle, "Offline"));
    }
}
