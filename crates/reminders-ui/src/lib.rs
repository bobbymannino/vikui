//! A window listing reminders.
//!
//! The view does not fetch anything: the caller passes in the reminders (from one
//! project, a filter, or search results) and can replace them later.
//!
//! Pressing Cmd/Ctrl+Enter on a row reports [`RemindersEvent::OpenTask`], to open its
//! task in a window of its own. Cmd/Ctrl+O reports [`RemindersEvent::OpenInBrowser`] and
//! Cmd/Ctrl+E [`RemindersEvent::EditTask`]. Each row's context menu offers all three.
//! Double-clicking a row opens, edits or marks its task done, as
//! [`RemindersView::set_double_click`] chose ([`DoubleClickAction`]; opening by default).
//!
//! Clicking a row (or pressing Enter on it) toggles a details side panel. The view only
//! reserves the space and reports [`RemindersEvent::ShowDetails`]; the caller supplies
//! the panel with [`RemindersView::set_details_panel`] and fills it in.
//!
//! The Refresh button (or Cmd/Ctrl+R) reports [`RemindersEvent::Refresh`]; the caller
//! loads the reminders again and passes them to [`RemindersView::set_reminders`].
//!
//! The search field (Cmd/Ctrl+F) filters the list by task title, ignoring case, spaces
//! and punctuation (see [`reminders::Search`]). Enter moves to the results; Escape clears.
//! A search window ([`open_search`]) instead holds the server search the list shows:
//! Enter reports [`RemindersEvent::Search`] and the caller lists the new results.
//!
//! Below it, a status filter lists all tasks, only those done or not done, or only
//! overdue ones (see [`reminders::StatusFilter`]), and one chip per label on the listed
//! tasks filters to tasks with every selected label (see [`reminders::LabelFilter`]).
//!
//! Pressing T (as in Vikunja's web app), or picking "Mark as done" from a row's context
//! menu, reports [`RemindersEvent::SetDone`] for that row's task; the caller saves it
//! and answers with [`RemindersView::set_task_done`].

mod label_tag;
mod reminders_table;

use std::rc::Rc;

use gpui_kit::base::{Selectable as _, TestSupportExt as _};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyBinding,
    KeyBindingContextPredicate, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowOptions, div,
    prelude::FluentBuilder as _, px,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::{LabelFilter, Reminder, Search, StatusFilter};
use table_layout::TableLayout;

use label_tag::LabelTag;
use reminders_table::{ReminderRow, RemindersTable, RowCommand};

gpui_kit::actions!(
    reminders,
    [
        ToggleDetails,
        ToggleDone,
        Refresh,
        FocusSearch,
        OpenTask,
        OpenInBrowser,
        EditTask
    ]
);

const CONTEXT: &str = "Reminders";
/// Plain keys act on the list only while it has focus, so they type into the search field.
const LIST_CONTEXT: &str = "Reminders > DataTable";
const SEARCH_WIDTH: f32 = 240.;
const DETAILS_WIDTH: f32 = 340.;

/// Register the reminders view's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", ToggleDetails, Some(LIST_CONTEXT)),
        KeyBinding::new("t", ToggleDone, Some(LIST_CONTEXT)),
    ]);
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`
    // to Cmd on macOS and Ctrl elsewhere.
    let context = KeyBindingContextPredicate::parse(CONTEXT).ok().map(Rc::new);
    let bindings: [(&str, Box<dyn gpui_kit::Action>); 2] = [
        ("secondary-r", Box::new(Refresh)),
        ("secondary-f", Box::new(FocusSearch)),
    ];
    for (keystrokes, action) in bindings {
        match KeyBinding::load(keystrokes, action, context.clone(), false, None, cx) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::error!("invalid key binding {keystrokes}: {err}"),
        }
    }
    let list_context = KeyBindingContextPredicate::parse(LIST_CONTEXT)
        .ok()
        .map(Rc::new);
    let list_bindings: [(&str, Box<dyn gpui_kit::Action>); 3] = [
        ("secondary-enter", Box::new(OpenTask)),
        ("secondary-o", Box::new(OpenInBrowser)),
        ("secondary-e", Box::new(EditTask)),
    ];
    for (keystrokes, action) in list_bindings {
        match KeyBinding::load(keystrokes, action, list_context.clone(), false, None, cx) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::error!("invalid key binding {keystrokes}: {err}"),
        }
    }
}

/// Events for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemindersEvent {
    /// The details panel opened on, or moved to, this reminder; fill the panel in.
    ShowDetails(Reminder),
    /// The user asked to reload the reminders; answer with [`RemindersView::set_reminders`]
    /// or [`RemindersView::set_error`].
    Refresh,
    /// The user asked to mark the selected row's task done (or not done).
    SetDone { task_id: i64, done: bool },
    /// The user asked to open this reminder's task in a window of its own.
    OpenTask(Reminder),
    /// The user asked to open this task's page in the web browser.
    OpenInBrowser { task_id: i64 },
    /// The user asked to edit this task.
    EditTask { task_id: i64 },
    /// The user asked to add a new reminder.
    NewReminder,
    /// In a search window: the user asked to search every project for this text (never
    /// blank). Answer with [`RemindersView::set_reminders`] or [`RemindersView::set_error`].
    Search(SharedString),
    /// The user moved, resized or sorted a column. Save the layout and restore it with
    /// [`RemindersView::set_table_layout`].
    TableLayoutChanged(TableLayout),
}

/// What double-clicking a row does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoubleClickAction {
    /// Report [`RemindersEvent::OpenTask`].
    #[default]
    OpenDetails,
    /// Report [`RemindersEvent::EditTask`].
    Edit,
    /// Report [`RemindersEvent::SetDone`], marking the task done or not done.
    ToggleDone,
}

/// What the search field does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SearchMode {
    /// Filters the listed reminders as the user types.
    #[default]
    List,
    /// Holds the server search the list shows; Enter searches again.
    Server,
}

/// A titled, sortable list of reminders with an optional details panel.
pub struct RemindersView {
    title: SharedString,
    table: Entity<TableState<RemindersTable>>,
    search: Entity<InputState>,
    search_mode: SearchMode,
    /// What the search field currently filters by.
    query: Search,
    /// The search field's text, for the empty-list message.
    query_text: SharedString,
    /// Every label on the listed tasks, one chip each.
    labels: Vec<LabelTag>,
    /// The labels a task must all have to be listed.
    label_filter: LabelFilter,
    status_filter: StatusFilter,
    summary: SharedString,
    error: Option<SharedString>,
    /// Waiting for reminders (the first load or a refresh).
    loading: bool,
    /// What the rows were made from; rows refer to it by `order`.
    reminders: Vec<Reminder>,
    matched_tasks: Option<usize>,
    details_panel: Option<AnyView>,
    /// `order` of the reminder the details panel is open on.
    details_for: Option<usize>,
    /// The table layout last reported or restored, by [`RemindersTable::layout_revision`].
    layout_revision: u64,
    double_click: DoubleClickAction,
    _table_events: Subscription,
    _table_layout: Subscription,
    _search_events: Subscription,
}

impl RemindersView {
    /// Show `reminders` under `title`. Pass `None` to show a loading state until
    /// [`Self::set_reminders`] or [`Self::set_error`] is called.
    pub fn new(
        title: impl Into<SharedString>,
        reminders: Option<Vec<Reminder>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| TableState::new(RemindersTable::default(), window, cx));
        table.read(cx).focus_handle(cx).focus(window, cx);

        let view = cx.entity().downgrade();
        table.update(cx, |table, _| {
            table.delegate_mut().set_on_row_click(Rc::new({
                let view = view.clone();
                move |row_ix, _, cx| {
                    view.update(cx, |this, cx| this.toggle_row(row_ix, cx)).ok();
                }
            }));
            table
                .delegate_mut()
                .set_on_row_command(Rc::new(move |row_ix, command, _, cx| {
                    view.update(cx, |this, cx| match command {
                        RowCommand::OpenInWindow => this.open_task(row_ix, cx),
                        RowCommand::OpenInBrowser => this.open_in_browser(row_ix, cx),
                        RowCommand::Edit => this.edit_task(row_ix, cx),
                        RowCommand::ToggleDone => this.toggle_done(row_ix, cx),
                    })
                    .ok();
                }));
        });
        let table_events = cx.subscribe(&table, |this, _, event: &TableEvent, cx| match event {
            // Clicks are handled by `toggle_row`; this follows keyboard selection while the
            // panel is open.
            TableEvent::SelectRow(row_ix) if this.details_for.is_some() => {
                this.show_row(*row_ix, cx);
            }
            TableEvent::DoubleClickedRow(row_ix) => this.double_click_row(*row_ix, cx),
            TableEvent::ClearSelection => this.hide_details(cx),
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
                cx.emit(RemindersEvent::TableLayoutChanged(table.layout()));
            }
        });

        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search reminders")
                .clean_on_escape()
        });
        let search_events = cx.subscribe_in(
            &search,
            window,
            |this, search, event: &InputEvent, window, cx| match event {
                InputEvent::Change if this.search_mode == SearchMode::List => {
                    let text = search.read(cx).value();
                    this.set_search(text, cx);
                }
                InputEvent::PressEnter { .. } => {
                    if this.search_mode == SearchMode::Server {
                        let text = search.read(cx).value();
                        if !this.search_server(text.trim(), cx) {
                            return;
                        }
                    }
                    this.table.read(cx).focus_handle(cx).focus(window, cx);
                }
                // A server search only runs on Enter.
                InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {}
            },
        );

        let mut this = Self {
            title: title.into(),
            table,
            search,
            search_mode: SearchMode::List,
            query: Search::default(),
            labels: Vec::new(),
            label_filter: LabelFilter::default(),
            status_filter: StatusFilter::default(),
            query_text: SharedString::default(),
            summary: SharedString::default(),
            error: None,
            loading: reminders.is_none(),
            reminders: Vec::new(),
            matched_tasks: None,
            details_panel: None,
            details_for: None,
            layout_revision: 0,
            double_click: DoubleClickAction::default(),
            _table_events: table_events,
            _table_layout: table_layout,
            _search_events: search_events,
        };
        match reminders {
            Some(reminders) => this.set_reminders(&reminders, None, cx),
            None => this.table.update(cx, |table, cx| {
                table.delegate_mut().set_loading(true);
                cx.notify();
            }),
        }
        this
    }

    /// Choose what double-clicking a row does.
    pub fn set_double_click(&mut self, action: DoubleClickAction) {
        self.double_click = action;
    }

    fn double_click_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        match self.double_click {
            DoubleClickAction::OpenDetails => self.open_task(row_ix, cx),
            DoubleClickAction::Edit => self.edit_task(row_ix, cx),
            DoubleClickAction::ToggleDone => self.toggle_done(row_ix, cx),
        }
    }

    /// Arrange the table's columns and sort as the user last left them, from a
    /// [`RemindersEvent::TableLayoutChanged`].
    pub fn set_table_layout(&mut self, layout: &TableLayout, cx: &mut Context<Self>) {
        self.layout_revision = self.table.update(cx, |table, cx| {
            table.delegate_mut().set_layout(layout);
            table.refresh(cx);
            cx.notify();
            table.delegate().layout_revision()
        });
    }

    /// Replace the listed reminders.
    ///
    /// `matched_tasks` is how many tasks the reminders came from, if known; tasks without
    /// a reminder are listed too, so an empty list means no tasks matched.
    ///
    /// An open details panel stays open if its reminder is still listed (and is shown
    /// again with [`RemindersEvent::ShowDetails`]); otherwise it closes.
    pub fn set_reminders(
        &mut self,
        reminders: &[Reminder],
        matched_tasks: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .details_for
            .and_then(|order| self.reminders.get(order))
            .cloned();
        self.error = None;
        self.loading = false;
        self.reminders = reminders.to_vec();
        self.matched_tasks = matched_tasks;
        let labels = reminders::labels_in(&self.reminders);
        self.label_filter.retain_in(&labels);
        self.labels = labels.iter().map(LabelTag::new).collect();
        self.details_for = shown.and_then(|shown| {
            self.reminders
                .iter()
                .position(|reminder| same_reminder(reminder, &shown))
        });
        self.refresh_rows(cx);
        if let Some(reminder) = self
            .details_for
            .and_then(|order| self.reminders.get(order))
            .cloned()
        {
            cx.emit(RemindersEvent::ShowDetails(reminder));
        }
    }

    /// Show the server search for `text`: the search field holds it (instead of
    /// filtering the list) and the list waits for its results, passed to
    /// [`Self::set_reminders`].
    pub fn set_server_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search_mode = SearchMode::Server;
        self.title = search_title(text);
        self.loading = true;
        let text = SharedString::from(text.to_owned());
        self.search.update(cx, |search, cx| {
            search.set_placeholder("Search all tasks", window, cx);
            search.set_value(text, window, cx);
        });
        cx.notify();
    }

    /// Show the results for a new server search, `text`. Returns whether it was asked for.
    fn search_server(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if text.is_empty() || self.loading {
            return false;
        }
        self.loading = true;
        self.title = search_title(text);
        cx.emit(RemindersEvent::Search(text.to_owned().into()));
        cx.notify();
        true
    }

    /// Filter the list by `text`; see [`reminders::Search`].
    fn set_search(&mut self, text: SharedString, cx: &mut Context<Self>) {
        let query = Search::new(&text);
        self.query_text = text;
        if query != self.query {
            self.query = query;
            self.refresh_rows(cx);
        }
    }

    /// Select `label_id` in the label filter, or deselect it.
    fn toggle_label(&mut self, label_id: i64, cx: &mut Context<Self>) {
        self.label_filter.toggle(label_id);
        self.refresh_rows(cx);
    }

    fn set_status_filter(&mut self, status: StatusFilter, cx: &mut Context<Self>) {
        if status != self.status_filter {
            self.status_filter = status;
            self.refresh_rows(cx);
        }
    }

    fn clear_labels(&mut self, cx: &mut Context<Self>) {
        if !self.label_filter.is_empty() {
            self.label_filter.clear();
            self.refresh_rows(cx);
        }
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        if !self.loading {
            self.loading = true;
            cx.emit(RemindersEvent::Refresh);
            cx.notify();
        }
    }

    /// Mark every reminder of `task_id` done or not done, keeping the list's order,
    /// selection and details panel.
    pub fn set_task_done(&mut self, task_id: i64, done: bool, cx: &mut Context<Self>) {
        let mut changed = false;
        for reminder in self
            .reminders
            .iter_mut()
            .filter(|reminder| reminder.task_id == task_id && reminder.task_done != done)
        {
            reminder.task_done = done;
            changed = true;
        }
        if changed {
            self.refresh_rows(cx);
        }
    }

    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        let now = Timestamp::now();
        let mut rows = ReminderRow::from_reminders(&self.reminders, now, &TimeZone::system());
        let set = rows.iter().filter(|row| row.is_set).count();
        let upcoming = rows
            .iter()
            .filter(|row| row.is_set && !row.inactive)
            .count();
        let total = rows.len();
        rows.retain(|row| {
            self.query.matches_normalized(&row.search_key)
                && self.reminders.get(row.order).is_some_and(|reminder| {
                    self.label_filter.matches(reminder) && self.status_filter.matches(reminder, now)
                })
        });
        let mut summary = summary(set, upcoming, self.matched_tasks);
        let query = self.query_text.trim();
        let empty_message = if self.status_filter == StatusFilter::All {
            match (self.query.is_empty(), self.label_filter.is_empty()) {
                (true, true) => empty_message(self.matched_tasks),
                (false, true) => format!("Nothing matches “{query}”.").into(),
                (true, false) => "No reminders have all the selected labels.".into(),
                (false, false) => {
                    format!("Nothing matches “{query}” with all the selected labels.").into()
                }
            }
        } else {
            let status = self.status_filter.label().to_lowercase();
            // e.g. "No overdue tasks matching “rent” with all the selected labels."
            let search = if self.query.is_empty() {
                String::new()
            } else {
                format!(" matching “{query}”")
            };
            let labels = if self.label_filter.is_empty() {
                ""
            } else {
                " with all the selected labels"
            };
            format!("No {status} tasks{search}{labels}.").into()
        };
        if !self.query.is_empty()
            || !self.label_filter.is_empty()
            || self.status_filter != StatusFilter::All
        {
            summary = format!("{summary} · {} of {total} shown", rows.len()).into();
        }
        self.summary = summary;
        // A filtered-out reminder can't stay open in the details panel.
        if let Some(order) = self.details_for
            && !rows.iter().any(|row| row.order == order)
        {
            self.details_for = None;
        }
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_empty_message(empty_message);
            table.delegate_mut().set_rows(rows);
            cx.notify();
        });
        cx.notify();
    }

    /// The view shown in the details panel; it stays hidden until a row is opened.
    pub fn set_details_panel(&mut self, panel: AnyView, cx: &mut Context<Self>) {
        self.details_panel = Some(panel);
        cx.notify();
    }

    /// Close the details panel and return focus to the list.
    pub fn close_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_details(cx);
        self.table.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn hide_details(&mut self, cx: &mut Context<Self>) {
        if self.details_for.take().is_some() {
            cx.notify();
        }
    }

    fn order_of(&self, row_ix: usize, cx: &App) -> Option<usize> {
        self.table
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| row.order)
    }

    /// Open the panel on the row, or close it if it is already open there.
    fn toggle_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let order = self.order_of(row_ix, cx);
        if order.is_some() && order == self.details_for {
            self.hide_details(cx);
        } else {
            self.show_row(row_ix, cx);
        }
    }

    fn show_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let Some(order) = self.order_of(row_ix, cx) else {
            return;
        };
        if self.details_for == Some(order) {
            return;
        }
        let Some(reminder) = self.reminders.get(order).cloned() else {
            return;
        };
        self.details_for = Some(order);
        cx.emit(RemindersEvent::ShowDetails(reminder));
        cx.notify();
    }

    fn open_task(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        if let Some(reminder) = self
            .order_of(row_ix, cx)
            .and_then(|order| self.reminders.get(order))
        {
            cx.emit(RemindersEvent::OpenTask(reminder.clone()));
        }
    }

    fn open_in_browser(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        if let Some(reminder) = self
            .order_of(row_ix, cx)
            .and_then(|order| self.reminders.get(order))
        {
            cx.emit(RemindersEvent::OpenInBrowser {
                task_id: reminder.task_id,
            });
        }
    }

    fn edit_task(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        if let Some(reminder) = self
            .order_of(row_ix, cx)
            .and_then(|order| self.reminders.get(order))
        {
            cx.emit(RemindersEvent::EditTask {
                task_id: reminder.task_id,
            });
        }
    }

    fn on_edit_task(&mut self, _: &EditTask, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.edit_task(row_ix, cx);
        }
    }

    fn on_open_in_browser(&mut self, _: &OpenInBrowser, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.open_in_browser(row_ix, cx);
        }
    }

    fn on_open_task(&mut self, _: &OpenTask, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.open_task(row_ix, cx);
        }
    }

    fn on_toggle_details(&mut self, _: &ToggleDetails, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.toggle_row(row_ix, cx);
        }
    }

    fn toggle_done(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        if let Some(reminder) = self
            .order_of(row_ix, cx)
            .and_then(|order| self.reminders.get(order))
        {
            cx.emit(RemindersEvent::SetDone {
                task_id: reminder.task_id,
                done: !reminder.task_done,
            });
        }
    }

    fn on_toggle_done(&mut self, _: &ToggleDone, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.toggle_done(row_ix, cx);
        }
    }

    /// Show why the reminders could not be loaded.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.loading = false;
        self.error = Some(message.into());
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(false);
            cx.notify();
        });
        cx.notify();
    }
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// Whether `a` and `b` are the same reminder of the same task, across reloads.
fn same_reminder(a: &Reminder, b: &Reminder) -> bool {
    a.task_id == b.task_id && a.is_set == b.is_set && a.at == b.at && a.relative == b.relative
}

/// "2 reminders on 5 tasks, 1 upcoming".
fn summary(reminders: usize, upcoming: usize, matched_tasks: Option<usize>) -> SharedString {
    let tasks = matched_tasks.map_or_else(String::new, |tasks| {
        format!(" on {}", plural(tasks, "task"))
    });
    format!(
        "{}{tasks}, {upcoming} upcoming",
        plural(reminders, "reminder")
    )
    .into()
}

fn empty_message(matched_tasks: Option<usize>) -> SharedString {
    match matched_tasks {
        None => "No reminders.".into(),
        Some(_) => "No tasks match.".into(),
    }
}

impl EventEmitter<RemindersEvent> for RemindersView {}

impl Render for RemindersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status_filter = self.render_status_filter(cx);
        let label_filter = (!self.labels.is_empty()).then(|| self.render_label_filter(cx));
        let theme = cx.theme();
        let details = self
            .details_panel
            .clone()
            .filter(|_| self.details_for.is_some());
        v_flex()
            .id("reminders")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_toggle_details))
            .on_action(cx.listener(Self::on_toggle_done))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_open_task))
            .on_action(cx.listener(Self::on_open_in_browser))
            .on_action(cx.listener(Self::on_edit_task))
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
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(self.summary.clone()),
                    )
                    .child(
                        Input::new(&self.search)
                            .id("search-reminders")
                            .aria_label(match self.search_mode {
                                SearchMode::List => "Search reminders",
                                SearchMode::Server => "Search all tasks",
                            })
                            .prefix(Icon::new(IconName::Search).text_color(theme.muted_foreground))
                            .cleanable(true)
                            .w(px(SEARCH_WIDTH)),
                    )
                    .child(
                        Button::new("new-reminder")
                            .primary()
                            .icon(IconName::Plus)
                            .label("New Reminder")
                            .accessibility_label("New reminder")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(RemindersEvent::NewReminder);
                            })),
                    )
                    .child(
                        Button::new("refresh-reminders")
                            .outline()
                            .label(if self.loading {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            })
                            .accessibility_label("Refresh reminders")
                            .tooltip_with_action("Refresh reminders", &Refresh, Some(CONTEXT))
                            .loading(self.loading)
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_refresh(&Refresh, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_3()
                    .items_center()
                    .child(status_filter)
                    .children(label_filter),
            )
            .when_some(self.error.clone(), |this, message| {
                this.child(
                    // `Alert` has no accessible name of its own; announce the message.
                    div()
                        .id("reminders-error")
                        .role(Role::Alert)
                        .aria_label(format!("Couldn't load reminders: {message}"))
                        .child(
                            Alert::error("reminders-error-alert", message)
                                .title("Couldn't load reminders"),
                        )
                        .test_support(),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            DataTable::new(&self.table)
                                .stripe(true)
                                .accessibility_label("Reminders"),
                        ),
                    )
                    .when_some(details, |this, panel| {
                        this.child(
                            div()
                                .w(px(DETAILS_WIDTH))
                                .flex_none()
                                .h_full()
                                .ml_4()
                                .border_l_1()
                                .border_color(theme.border)
                                .child(panel),
                        )
                    }),
            )
    }
}

impl RemindersView {
    /// One button per status; the selected one is pressed.
    fn render_status_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("status-filter")
            .role(Role::Group)
            .aria_label("Filter by status")
            .child(
                ButtonGroup::new("status-filter-buttons")
                    .outline()
                    .small()
                    .children(StatusFilter::ALL.iter().enumerate().map(|(ix, status)| {
                        Button::new(("status-filter", ix))
                            .label(status.label())
                            .accessibility_label(format!("Show {}", status.label().to_lowercase()))
                            .selected(*status == self.status_filter)
                    }))
                    .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                        if let Some(status) =
                            clicked.first().and_then(|ix| StatusFilter::ALL.get(*ix))
                        {
                            this.set_status_filter(*status, cx);
                        }
                    })),
            )
            .test_support()
            .into_any_element()
    }

    /// One toggle chip per label, and a button to clear the selection.
    fn render_label_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("label-filter")
            .role(Role::Group)
            .aria_label("Filter by labels")
            .flex_wrap()
            .gap_1()
            .items_center()
            .children(self.labels.iter().enumerate().map(|(ix, label)| {
                let id = label.id;
                let checked = self.label_filter.is_selected(id);
                let name: SharedString = format!("Filter by {}", label.title).into();
                // A toggled button rather than `Toggle`, which draws no focus ring, so the
                // focused label stands out when tabbing through many.
                Button::new(("filter-label", ix))
                    .ghost()
                    .small()
                    .px_0p5()
                    .selected(checked)
                    .toggled(checked)
                    .accessibility_label(name.clone())
                    .tooltip(name)
                    .child(label.render(checked, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_label(id, cx)))
            }))
            .when(!self.label_filter.is_empty(), |this| {
                this.child(
                    Button::new("clear-label-filter")
                        .ghost()
                        .xsmall()
                        .label("Clear")
                        .accessibility_label("Clear label filter")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_labels(cx))),
                )
            })
            .test_support()
            .into_any_element()
    }
}

/// The heading for the results of searching for `text`.
fn search_title(text: &str) -> SharedString {
    format!("Search – “{text}”").into()
}

/// Open a search window for the tasks matching `text` in every project, loading until
/// [`RemindersView::set_reminders`] is called. Its search field holds `text`; changing
/// it and pressing Enter reports [`RemindersEvent::Search`].
pub fn open_search(
    options: WindowOptions,
    text: &str,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<RemindersView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut view = RemindersView::new(search_title(text), None, window, cx);
            view.set_server_search(text, window, cx);
            view
        })
    })
}

/// Open a reminders window; see [`RemindersView::new`] for `reminders`.
pub fn open(
    options: WindowOptions,
    title: impl Into<SharedString>,
    reminders: Option<Vec<Reminder>>,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<RemindersView>)> {
    let title = title.into();
    let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| RemindersView::new(title, reminders, window, cx))
    })?;
    Ok((handle, view))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{Bounds, ElementId, Point, TestAppContext, WindowBounds, px, size};

    use super::*;

    fn reminder(at: Timestamp, title: &str) -> Reminder {
        Reminder {
            at: Some(at),
            relative: Some(vikunja::RelativeReminder {
                period_secs: -3600,
                to: vikunja::RelativeTo::Due,
            }),
            task_id: 1,
            task_title: title.into(),
            task_identifier: "HOME-1".into(),
            task_priority: None,
            task_due: None,
            task_labels: Vec::new(),
            task_rgb: None,
            project_id: 1,
            task_done: false,
            is_set: true,
        }
    }

    fn options() -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(960.), px(640.)),
            })),
            ..Default::default()
        }
    }

    #[gpui_kit::test]
    fn lists_the_reminders_it_is_given(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            let labelled = Reminder {
                task_labels: vec![
                    reminders::TaskLabel {
                        id: 1,
                        title: "Bills".into(),
                        rgb: Some(0x19_73_ff),
                    },
                    reminders::TaskLabel {
                        id: 2,
                        title: "Home".into(),
                        rgb: None,
                    },
                ],
                ..reminder(soon, "Pay rent")
            };
            open(options(), "Reminders – Home", Some(vec![labelled]), cx)
                .unwrap()
                .0
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Reminders");
            window.find_by_label("Pay rent");
            window.find_by_label("HOME-1");
            window.find_by_label("Bills, Home");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn loads_later_or_shows_an_error(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(options(), "Reminders – search “rent”", None, cx).unwrap()
        });
        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Server unavailable", cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Couldn't load reminders: Server unavailable");
        })
        .unwrap();

        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(1);
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_reminders(&[reminder(soon, "Call landlord")], Some(1), cx);
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Call landlord");
            assert!(
                window
                    .try_find_by_label("Couldn't load reminders: Server unavailable")
                    .is_none()
            );
        })
        .unwrap();
    }

    struct TestPanel;

    impl Render for TestPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("test-panel")
                .aria_label("Test panel")
                .child("Details")
                .test_support()
        }
    }

    #[gpui_kit::test]
    fn clicking_a_row_toggles_the_details_panel(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let later = soon + jiff::SignedDuration::from_hours(24);
        let second = Reminder {
            task_id: 2,
            ..reminder(later, "Call landlord")
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent"), second.clone()];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let shown = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            view.update(cx, |view, cx| view.set_details_panel(panel.into(), cx));
            let shown = Rc::clone(&shown);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                if let RemindersEvent::ShowDetails(reminder) = event {
                    shown.borrow_mut().push(reminder.task_title.clone());
                }
            })
            .detach();
        });
        let panel_visible = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label("Test panel").is_some()
            })
            .unwrap()
        };
        assert!(!panel_visible(cx));

        let click = |cx: &mut TestAppContext, row_ix: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(("row", row_ix), cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        click(cx, 0);
        assert!(panel_visible(cx));
        click(cx, 1);
        assert!(panel_visible(cx));
        click(cx, 1);
        assert!(!panel_visible(cx));
        assert_eq!(*shown.borrow(), ["Pay rent", "Call landlord"]);

        // Keyboard: Enter opens it on the selected row, Up follows, Escape closes.
        let key = |cx: &mut TestAppContext, key: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.key_down(key, false, cx);
                window.key_up(key, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        key(cx, "enter");
        assert!(panel_visible(cx));
        key(cx, "up");
        assert_eq!(
            *shown.borrow(),
            ["Pay rent", "Call landlord", "Call landlord", "Pay rent"]
        );
        key(cx, "escape");
        assert!(!panel_visible(cx));
    }

    #[gpui_kit::test]
    fn marking_a_task_done_keeps_the_panel_open(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent")];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            view.update(cx, |view, cx| {
                view.set_details_panel(panel.into(), cx);
                view.toggle_row(0, cx);
                view.set_task_done(1, true, cx);
            });
        });
        let inactive = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let table = view.read(cx).table.read(cx);
                table.delegate().row(0).map(|row| row.inactive)
            })
        };
        assert_eq!(inactive(cx), Some(true));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Test panel");
        })
        .unwrap();
        cx.update(|cx| view.update(cx, |view, cx| view.set_task_done(1, false, cx)));
        assert_eq!(inactive(cx), Some(false));
    }

    #[gpui_kit::test]
    fn refresh_reloads_and_keeps_the_open_reminder(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let first = reminder(soon, "Pay rent");
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(options(), "Reminders – Home", None, cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
            view.update(cx, |view, cx| {
                view.set_details_panel(panel.into(), cx);
                view.set_reminders(std::slice::from_ref(&first), Some(1), cx);
                view.toggle_row(0, cx);
            });
        });
        let click_refresh = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("refresh-reminders", cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        click_refresh(cx);
        // Ignored while the reload is pending.
        click_refresh(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Test panel");
        })
        .unwrap();

        // The task was renamed elsewhere and another one added; the panel stays on it.
        let renamed = Reminder {
            task_title: "Pay the rent".into(),
            ..first.clone()
        };
        let added = Reminder {
            task_id: 2,
            ..reminder(soon - jiff::SignedDuration::from_hours(1), "Call landlord")
        };
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_reminders(&[added, renamed.clone()], Some(2), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [
                RemindersEvent::ShowDetails(first.clone()),
                RemindersEvent::Refresh,
                RemindersEvent::ShowDetails(renamed),
            ]
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Test panel");
        })
        .unwrap();

        // Gone after the next refresh: the panel closes.
        click_refresh(cx);
        cx.update(|cx| view.update(cx, |view, cx| view.set_reminders(&[], Some(0), cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find_by_label("Test panel").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn t_asks_to_toggle_the_selected_task(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent")];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let key = |cx: &mut TestAppContext, key: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.key_down(key, false, cx);
                window.key_up(key, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        // Nothing is selected yet.
        key(cx, "t");
        assert!(events.borrow().is_empty());

        key(cx, "down");
        key(cx, "t");
        cx.update(|cx| view.update(cx, |view, cx| view.set_task_done(1, true, cx)));
        key(cx, "t");
        assert_eq!(
            *events.borrow(),
            [
                RemindersEvent::SetDone {
                    task_id: 1,
                    done: true
                },
                RemindersEvent::SetDone {
                    task_id: 1,
                    done: false
                },
            ]
        );
    }

    #[test]
    fn summary_and_empty_message_mention_matched_tasks() {
        assert_eq!(summary(2, 1, Some(5)), "2 reminders on 5 tasks, 1 upcoming");
        assert_eq!(summary(1, 0, None), "1 reminder, 0 upcoming");
        assert_eq!(summary(0, 0, Some(1)), "0 reminders on 1 task, 0 upcoming");
        assert_eq!(empty_message(None), "No reminders.");
        assert_eq!(empty_message(Some(0)), "No tasks match.");
    }

    #[gpui_kit::test]
    fn lists_tasks_without_reminders(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(options(), "Reminders – Today", None, cx).unwrap()
        });
        let plain = Reminder {
            at: None,
            relative: None,
            is_set: false,
            ..reminder(Timestamp::now(), "Buy milk")
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_reminders(&[plain], Some(1), cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Buy milk");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn search_filters_by_normalized_title(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![
                reminder(soon, "Pay-Rent!"),
                Reminder {
                    task_id: 2,
                    ..reminder(soon, "Call landlord")
                },
                Reminder {
                    task_id: 3,
                    ..reminder(soon, "pay rent (flat 2)")
                },
            ];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let titles = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let table = view.read(cx).table.read(cx);
                (0..)
                    .map_while(|ix| table.delegate().row(ix).map(|row| row.task.to_string()))
                    .collect::<Vec<_>>()
            })
        };
        let search = |cx: &mut TestAppContext, text: &str| {
            let text = SharedString::from(text.to_owned());
            cx.update(|cx| view.update(cx, |view, cx| view.set_search(text, cx)));
        };
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };
        assert!(has_label(cx, "Search reminders"));

        search(cx, "PAY rent");
        assert_eq!(titles(cx), ["Pay-Rent!", "pay rent (flat 2)"]);
        assert!(
            cx.update(|cx| view.read(cx).summary.ends_with("· 2 of 3 shown")),
            "summary counts shown rows"
        );
        search(cx, "flat-2");
        assert_eq!(titles(cx), ["pay rent (flat 2)"]);
        search(cx, "zzz");
        assert_eq!(titles(cx), Vec::<String>::new());
        assert!(has_label(cx, "Nothing matches “zzz”."));
        // Punctuation alone filters nothing.
        search(cx, " -! ");
        assert_eq!(titles(cx).len(), 3);

        // A reminder filtered out of the list closes its details panel.
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            view.update(cx, |view, cx| {
                view.set_details_panel(panel.into(), cx);
                view.toggle_row(1, cx);
            });
        });
        assert!(has_label(cx, "Test panel"));
        search(cx, "pay");
        assert!(!has_label(cx, "Test panel"));
    }

    #[gpui_kit::test]
    fn status_filter_shows_done_not_done_or_overdue_tasks(cx: &mut TestAppContext) {
        let now = Timestamp::now();
        let hours = |h: i64| now + jiff::SignedDuration::from_hours(h);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![
                Reminder {
                    task_due: Some(hours(-2)),
                    ..reminder(hours(-3), "Late")
                },
                Reminder {
                    task_id: 2,
                    task_due: Some(hours(48)),
                    ..reminder(hours(24), "Later")
                },
                Reminder {
                    task_id: 3,
                    task_due: Some(hours(-2)),
                    task_done: true,
                    ..reminder(hours(-3), "Paid")
                },
            ];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let titles = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let table = view.read(cx).table.read(cx);
                let mut titles: Vec<String> = (0..)
                    .map_while(|ix| table.delegate().row(ix).map(|row| row.task.to_string()))
                    .collect();
                titles.sort();
                titles
            })
        };
        // The buttons, by index: All, Not done, Done, Overdue.
        let show = |cx: &mut TestAppContext, label: &'static str, ix: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label("Filter by status");
                window.find_by_label(label);
                window.click(ElementId::from(("status-filter", ix)), cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        assert_eq!(titles(cx), ["Late", "Later", "Paid"]);

        show(cx, "Show overdue", 3);
        assert_eq!(titles(cx), ["Late"]);
        assert!(cx.update(|cx| view.read(cx).summary.ends_with("· 1 of 3 shown")));
        show(cx, "Show done", 2);
        assert_eq!(titles(cx), ["Paid"]);
        show(cx, "Show not done", 1);
        assert_eq!(titles(cx), ["Late", "Later"]);

        // With a search too, the empty list says which status it looked for.
        let text = SharedString::from("zzz");
        cx.update(|cx| view.update(cx, |view, cx| view.set_search(text, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("No not done tasks matching “zzz”.");
        })
        .unwrap();
        let text = SharedString::default();
        cx.update(|cx| view.update(cx, |view, cx| view.set_search(text, cx)));

        show(cx, "Show all", 0);
        assert_eq!(titles(cx), ["Late", "Later", "Paid"]);
    }

    #[gpui_kit::test]
    fn search_window_searches_the_server_on_enter(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open_search(options(), "rent", cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };
        let results = vec![
            reminder(soon, "Pay rent"),
            Reminder {
                task_id: 2,
                ..reminder(soon, "Rent car")
            },
        ];
        cx.update(|cx| view.update(cx, |view, cx| view.set_reminders(&results, Some(2), cx)));
        assert_eq!(
            cx.update(|cx| view.read(cx).title.clone()),
            "Search – “rent”"
        );
        assert!(has_label(cx, "Search all tasks"));

        let enter = |text: &'static str, cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.key_down("secondary-f", false, cx);
                let search = view.read(cx).search.clone();
                search.update(cx, |search, cx| search.set_value(text, window, cx));
                window.key_down("enter", false, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        // Typing doesn't filter the results: they are the server's.
        enter("car", cx);
        let shown = cx.update(|cx| view.read(cx).table.read(cx).delegate().row(1).is_some());
        assert!(shown, "both results stay listed");
        assert_eq!(*events.borrow(), [RemindersEvent::Search("car".into())]);
        assert_eq!(
            cx.update(|cx| view.read(cx).title.clone()),
            "Search – “car”"
        );

        // A blank search, or one while the last is loading, is ignored.
        enter("  ", cx);
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_reminders(&results[1..], Some(1), cx);
            });
        });
        enter("  ", cx);
        enter(" boat ", cx);
        enter("bike", cx);
        assert_eq!(
            *events.borrow(),
            [
                RemindersEvent::Search("car".into()),
                RemindersEvent::Search("boat".into())
            ]
        );
    }

    #[gpui_kit::test]
    fn typing_in_search_does_not_trigger_list_shortcuts(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(
                options(),
                "Reminders – Home",
                Some(vec![reminder(soon, "Pay rent")]),
                cx,
            )
            .unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let key = |cx: &mut TestAppContext, key: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.key_down(key, false, cx);
                window.key_up(key, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let search_focused = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                view.read(cx)
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            })
            .unwrap()
        };

        key(cx, "down");
        key(cx, "secondary-f");
        assert!(search_focused(cx));
        key(cx, "t");
        assert_eq!(*events.borrow(), []);

        // Enter moves to the list, where T marks the selected task done again.
        key(cx, "enter");
        assert!(!search_focused(cx));
        key(cx, "t");
        assert_eq!(
            *events.borrow(),
            [RemindersEvent::SetDone {
                task_id: 1,
                done: true
            }]
        );
    }

    #[gpui_kit::test]
    fn double_click_or_mod_enter_opens_the_task(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let second = Reminder {
            task_id: 2,
            ..reminder(soon, "Call landlord")
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent"), second.clone()];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let opened = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            view.update(cx, |view, cx| view.set_details_panel(panel.into(), cx));
            let opened = Rc::clone(&opened);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                if let RemindersEvent::OpenTask(reminder) = event {
                    opened.borrow_mut().push(reminder.task_title.clone());
                }
            })
            .detach();
        });
        let panel_visible = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label("Test panel").is_some()
            })
            .unwrap()
        };

        // A double click: its first click opens the panel, its second the task.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.double_click(("row", 1_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["Call landlord"]);
        // The second click doesn't close the panel the first one opened.
        assert!(panel_visible(cx));

        cx.update_window(handle, |_, window, cx| {
            window.key_down("up", false, cx);
            window.key_down("secondary-enter", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["Call landlord", "Pay rent"]);
    }

    #[gpui_kit::test]
    fn double_click_does_what_was_chosen(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(
                options(),
                "Reminders – Home",
                Some(vec![reminder(soon, "Pay rent")]),
                cx,
            )
            .unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| match event {
                RemindersEvent::OpenTask(reminder) => {
                    events
                        .borrow_mut()
                        .push(format!("open {}", reminder.task_id));
                }
                RemindersEvent::EditTask { task_id } => {
                    events.borrow_mut().push(format!("edit {task_id}"));
                }
                RemindersEvent::SetDone { task_id, done } => {
                    events.borrow_mut().push(format!("done {task_id} {done}"));
                }
                _ => {}
            })
            .detach();
        });
        let double_click = |cx: &mut TestAppContext, action: DoubleClickAction| {
            cx.update(|cx| view.update(cx, |view, _| view.set_double_click(action)));
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.double_click(("row", 0_usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
        };

        double_click(cx, DoubleClickAction::OpenDetails);
        double_click(cx, DoubleClickAction::Edit);
        double_click(cx, DoubleClickAction::ToggleDone);
        assert_eq!(*events.borrow(), ["open 1", "edit 1", "done 1 true"]);
    }

    #[gpui_kit::test]
    fn row_menu_and_mod_o_open_the_task_elsewhere(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let second = Reminder {
            task_id: 2,
            ..reminder(soon, "Call landlord")
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent"), second];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| match event {
                RemindersEvent::OpenTask(reminder) => {
                    events
                        .borrow_mut()
                        .push(format!("window {}", reminder.task_id));
                }
                RemindersEvent::EditTask { task_id } => {
                    events.borrow_mut().push(format!("edit {task_id}"));
                }
                RemindersEvent::OpenInBrowser { task_id } => {
                    events.borrow_mut().push(format!("browser {task_id}"));
                }
                RemindersEvent::SetDone { task_id, done } => {
                    events.borrow_mut().push(format!("done {task_id} {done}"));
                }
                _ => {}
            })
            .detach();
        });
        let pick = |cx: &mut TestAppContext, label: &str, item: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.right_click(("row", 1_usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label(label);
                window.within("popup-menu").click(item, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        // The menu's items, by index: Edit…, the done toggle, a separator, then the two
        // open items.
        pick(cx, "Edit…", 0);
        pick(cx, "Mark as done", 1);
        pick(cx, "Open in new window", 3);
        pick(cx, "Open in browser", 4);
        assert_eq!(
            *events.borrow(),
            ["edit 2", "done 2 true", "window 2", "browser 2"]
        );

        // Once saved, the item offers to undo it.
        cx.update(|cx| view.update(cx, |view, cx| view.set_task_done(2, true, cx)));
        pick(cx, "Mark as not done", 1);
        assert_eq!(
            events.borrow().last().map(String::as_str),
            Some("done 2 false")
        );
        events.borrow_mut().clear();

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("row", 0_usize), cx);
            window.key_down("secondary-o", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.key_down("secondary-e", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*events.borrow(), ["browser 1", "edit 1"]);
    }

    /// The chips are kit buttons so they draw the focus ring; they must still be reachable
    /// by Tab and report their pressed state.
    #[gpui_kit::test]
    fn reports_and_restores_the_table_layout(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(
                options(),
                "Reminders – Home",
                Some(vec![reminder(soon, "Pay rent")]),
                cx,
            )
            .unwrap()
        });
        let layouts = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let layouts = Rc::clone(&layouts);
            cx.subscribe(&view, move |_, event: &RemindersEvent, _| {
                if let RemindersEvent::TableLayoutChanged(layout) = event {
                    layouts.borrow_mut().push(layout.clone());
                }
            })
            .detach();
        });
        let last = |layouts: &RefCell<Vec<TableLayout>>| {
            layouts
                .borrow()
                .last()
                .cloned()
                .expect("a layout was reported")
        };
        let keys = |layout: &TableLayout| -> Vec<String> {
            layout.columns.iter().map(|col| col.key.clone()).collect()
        };

        // Sort by Due.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(ElementId::from(("icon-sort", 3_usize)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let sort = last(&layouts).sort.expect("sorted");
        assert_eq!(sort.column, "due");
        assert_eq!(layouts.borrow().len(), 1);

        // Widen Task by dragging its right edge.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // The resize handle straddles the header's right edge.
            let header = window
                .find(ElementId::from(("col-header", 1_usize)))
                .bounds();
            let edge = Point::new(header.right() - px(1.), header.center().y);
            window.drag(edge, edge + Point::new(px(60.), px(0.)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let widened = last(&layouts);
        let task = widened
            .columns
            .iter()
            .find(|col| col.key == "task")
            .unwrap();
        assert!(task.width > 380, "Task widened from 340 to {}", task.width);

        // Drag Due to the front.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let due = window
                .find(ElementId::from(("col-header", 3_usize)))
                .bounds();
            let first = window
                .find(ElementId::from(("col-header", 0_usize)))
                .bounds();
            // Left of the first header's middle, so Due lands before it.
            let target = Point::new(first.left() + px(8.), first.center().y);
            window.drag(due.center(), target, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let saved = last(&layouts);
        assert_eq!(
            keys(&saved),
            ["due", "identifier", "task", "priority", "labels"]
        );

        // Another window takes the saved layout, without reporting it back.
        let reported = layouts.borrow().len();
        let (_, restored) = cx.update(|cx| {
            open(
                options(),
                "Reminders – Work",
                Some(vec![reminder(soon, "Plan")]),
                cx,
            )
            .unwrap()
        });
        cx.update(|cx| {
            let layouts = Rc::clone(&layouts);
            cx.subscribe(&restored, move |_, event: &RemindersEvent, _| {
                if let RemindersEvent::TableLayoutChanged(layout) = event {
                    layouts.borrow_mut().push(layout.clone());
                }
            })
            .detach();
            restored.update(cx, |view, cx| view.set_table_layout(&saved, cx));
        });
        cx.run_until_parked();
        assert_eq!(layouts.borrow().len(), reported);
        cx.update(|cx| {
            let table = restored.read(cx).table.read(cx);
            assert_eq!(
                table.headers(cx),
                ["Due", "Task ID", "Task", "Priority", "Labels"]
            );
            let width = f32::from(u16::try_from(task.width).unwrap());
            assert_eq!(table.col_width(2), Some(px(width)), "Task keeps its width");
            assert_eq!(table.delegate().layout(), saved);
        });
    }

    #[gpui_kit::test]
    fn label_chips_take_focus_and_report_pressed(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let (handle, _view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![Reminder {
                task_labels: vec![reminders::TaskLabel {
                    id: 1,
                    title: "Bills".into(),
                    rgb: None,
                }],
                ..reminder(soon, "Pay rent")
            }];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let chip = window.find_by_label("Filter by Bills");
            assert_eq!(chip.checked(), Some(false));
            assert_ne!(chip.focused(), Some(true));
            window.click(ElementId::from(("filter-label", 0_usize)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        // Tab through the window until the chip has keyboard focus.
        let reached = cx
            .update_window(handle, |_, window, cx| {
                (0..20).any(|_| {
                    window.focus_next(cx);
                    window.render_frame(cx);
                    window.find_by_label("Filter by Bills").focused() == Some(true)
                })
            })
            .unwrap();
        assert!(reached, "Tab must reach the label chip");
        cx.update_window(handle, |_, window, _| {
            assert_eq!(
                window.find_by_label("Filter by Bills").checked(),
                Some(true)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn label_chips_filter_to_tasks_with_every_selected_label(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let label = |id, title: &str| reminders::TaskLabel {
            id,
            title: title.into(),
            rgb: None,
        };
        let labelled = |task_id, title: &str, labels: Vec<reminders::TaskLabel>| Reminder {
            task_id,
            task_labels: labels,
            ..reminder(soon, title)
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![
                labelled(1, "Pay rent", vec![label(1, "Bills"), label(2, "Home")]),
                labelled(2, "Pay phone", vec![label(1, "Bills")]),
                labelled(3, "Water plants", vec![label(2, "Home")]),
                labelled(4, "Read", Vec::new()),
            ];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let titles = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let table = view.read(cx).table.read(cx);
                let mut titles: Vec<_> = (0..)
                    .map_while(|ix| table.delegate().row(ix).map(|row| row.task.to_string()))
                    .collect();
                titles.sort();
                titles
            })
        };
        let click = |cx: &mut TestAppContext, id: ElementId| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(id, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Filter by labels");
            // Sorted by title: Bills, then Home.
            window.find_by_label("Filter by Bills");
            window.find_by_label("Filter by Home");
            assert!(window.try_find_by_label("Clear label filter").is_none());
        })
        .unwrap();
        assert_eq!(titles(cx).len(), 4, "nothing selected shows everything");

        click(cx, ("filter-label", 0_usize).into());
        assert_eq!(titles(cx), ["Pay phone", "Pay rent"]);
        click(cx, ("filter-label", 1_usize).into());
        assert_eq!(
            titles(cx),
            ["Pay rent"],
            "every selected label must be present"
        );
        assert!(cx.update(|cx| view.read(cx).summary.ends_with("· 1 of 4 shown")));

        // Combined with search, both must match.
        let text = SharedString::from("water");
        cx.update(|cx| view.update(cx, |view, cx| view.set_search(text, cx)));
        assert_eq!(titles(cx), Vec::<String>::new());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Nothing matches “water” with all the selected labels.");
        })
        .unwrap();
        let text = SharedString::default();
        cx.update(|cx| view.update(cx, |view, cx| view.set_search(text, cx)));

        // Clicking a selected chip deselects it; Clear deselects them all.
        click(cx, ("filter-label", 0_usize).into());
        assert_eq!(titles(cx), ["Pay rent", "Water plants"]);
        click(cx, "clear-label-filter".into());
        assert_eq!(titles(cx).len(), 4);

        // A reload drops selected labels that no longer appear.
        click(cx, ("filter-label", 0_usize).into());
        let reloaded = vec![labelled(3, "Water plants", vec![label(2, "Home")])];
        cx.update(|cx| view.update(cx, |view, cx| view.set_reminders(&reloaded, None, cx)));
        assert_eq!(titles(cx), ["Water plants"]);
        assert!(cx.update(|cx| view.read(cx).label_filter.is_empty()));
    }
}
