//! Vikui's main window, shown once the user is signed in.
//!
//! The window is a shell: the app passes in the view to show, so this crate does not
//! depend on other UI crates. Above it, a search field looks for tasks in every project,
//! by text and by `*label`: Enter reports [`OverviewEvent::Search`] and the app shows the
//! results. Cmd/Ctrl+F
//! focuses the field; Escape clears it. Beside it, a Settings button reports
//! [`OverviewEvent::OpenSettings`].

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, KeyBindingContextPredicate,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, WindowOptions, px,
};

gpui_kit::actions!(overview, [FocusSearch]);

const CONTEXT: &str = "Overview";
const SEARCH_WIDTH: f32 = 360.;

/// Register the main window's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`
    // to Cmd on macOS and Ctrl elsewhere.
    let context = KeyBindingContextPredicate::parse(CONTEXT).ok().map(Rc::new);
    match KeyBinding::load(
        "secondary-f",
        Box::new(FocusSearch),
        context,
        false,
        None,
        cx,
    ) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(err) => log::error!("invalid search key binding: {err}"),
    }
}

/// Events for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverviewEvent {
    /// The user searched every project for tasks matching this text (never blank).
    Search(SharedString),
    /// The user asked to open the settings window.
    OpenSettings,
}

/// The main application view: a search field above the current content view.
pub struct Overview {
    /// Keeps the window's shortcuts working when nothing inside it has focus.
    focus_handle: FocusHandle,
    content: AnyView,
    search: Entity<InputState>,
    _search_events: Subscription,
}

impl EventEmitter<OverviewEvent> for Overview {}

impl Overview {
    fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search all tasks, or *label")
                .clean_on_escape()
        });
        let search_events = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            content,
            search,
            _search_events: search_events,
        }
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            cx.emit(OverviewEvent::Search(trimmed.to_owned().into()));
        }
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
    }
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("overview")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_focus_search))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .pt_4()
                    .pb_2()
                    .child(
                        Input::new(&self.search)
                            .id("search-all-tasks")
                            .aria_label("Search all tasks")
                            .prefix(Icon::new(IconName::Search).text_color(theme.muted_foreground))
                            .cleanable(true)
                            .w(px(SEARCH_WIDTH)),
                    )
                    .child(
                        Button::new("open-settings")
                            .icon(IconName::Settings)
                            .ghost()
                            .accessibility_label("Settings")
                            .tooltip("Settings")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(OverviewEvent::OpenSettings);
                            })),
                    ),
            )
            .child(v_flex().flex_1().min_h_0().child(self.content.clone()))
    }
}

/// Open the main window; `content` builds the view it shows.
pub fn open(
    options: WindowOptions,
    cx: &mut App,
    content: impl FnOnce(&mut Window, &mut App) -> AnyView,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Overview>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        let content = content(window, cx);
        cx.new(|cx| Overview::new(content, window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{TestAppContext, div};

    use super::*;

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn reports_searches_but_not_blank_ones(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Empty).into()
            })
            .unwrap()
        });
        let searches = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let searches = Rc::clone(&searches);
            cx.subscribe(&view, move |_, event: &OverviewEvent, _| {
                if let OverviewEvent::Search(text) = event {
                    searches.borrow_mut().push(text.to_string());
                }
            })
            .detach();
        });
        let search = |text: &'static str, cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_by_label("Search all tasks");
                view.read(cx).focus_handle.clone().focus(window, cx);
                window.key_down("secondary-f", false, cx);
                let field = view.read(cx).search.clone();
                assert!(field.read(cx).focus_handle(cx).is_focused(window));
                field.update(cx, |field, cx| field.set_value(text, window, cx));
                window.key_down("enter", false, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        search("   ", cx);
        search("  rent ", cx);
        assert_eq!(*searches.borrow(), ["rent"]);
    }

    #[gpui_kit::test]
    fn settings_button_asks_to_open_settings(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Empty).into()
            })
            .unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &OverviewEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Settings");
            window.click("open-settings", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*events.borrow(), [OverviewEvent::OpenSettings]);
    }
}
