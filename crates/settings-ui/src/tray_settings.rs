//! The Tray tab: whether the tray icon shows, which project or saved filter its menu
//! lists reminders from, how many it lists and whether it shows their count.
//!
//! The app gives the choices with [`TraySettingsView::set_sources`] and saves what the
//! view reports through [`TraySettingsEvent`].

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};

/// A project or saved filter the tray can list reminders from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TraySourceId {
    Project(i64),
    SavedFilter(i64),
}

/// One choice of where the tray's reminders come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraySourceChoice {
    pub id: TraySourceId,
    /// e.g. "Home › Bills", or "This week" for a saved filter.
    pub name: SharedString,
}

impl SearchableListItem for TraySourceChoice {
    type Value = TraySourceId;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &TraySourceId {
        &self.id
    }
}

/// The tray's settings as the tab shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraySettings {
    pub enabled: bool,
    pub source: Option<TraySourceId>,
    /// At most this many reminders; `None` for all.
    pub limit: Option<u32>,
    pub show_count: bool,
}

/// The user changed a tray setting; save and apply all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraySettingsEvent(pub TraySettings);

/// The limits offered, besides "No limit".
const LIMITS: [u32; 6] = [3, 5, 10, 15, 20, 30];

#[derive(Clone, Debug, PartialEq, Eq)]
struct LimitChoice {
    limit: Option<u32>,
    title: SharedString,
}

impl LimitChoice {
    fn new(limit: Option<u32>) -> Self {
        let title = limit.map_or_else(|| "No limit".into(), |limit| limit.to_string().into());
        Self { limit, title }
    }
}

impl SearchableListItem for LimitChoice {
    type Value = Option<u32>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Option<u32> {
        &self.limit
    }
}

/// The limits to offer: "No limit", the usual ones and `saved` if it is another.
fn limit_choices(saved: Option<u32>) -> Vec<LimitChoice> {
    let mut limits: Vec<u32> = LIMITS.to_vec();
    if let Some(saved) = saved
        && !limits.contains(&saved)
    {
        limits.push(saved);
        limits.sort_unstable();
    }
    std::iter::once(None)
        .chain(limits.into_iter().map(Some))
        .map(LimitChoice::new)
        .collect()
}

/// The tray tab.
pub struct TraySettingsView {
    settings: TraySettings,
    source: Entity<SelectState<SearchableVec<TraySourceChoice>>>,
    limit: Entity<SelectState<SearchableVec<LimitChoice>>>,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<TraySettingsEvent> for TraySettingsView {}

impl TraySettingsView {
    /// Shows `settings`, the saved ones; their source is listed once
    /// [`Self::set_sources`] gives the choices.
    pub fn new(settings: TraySettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let limits = limit_choices(settings.limit);
        let selected = limits
            .iter()
            .position(|choice| choice.limit == settings.limit)
            .map(gpui_kit::component::IndexPath::new);
        let limit = cx.new(|cx| SelectState::new(SearchableVec::new(limits), selected, window, cx));
        let subscriptions = [
            cx.subscribe(
                &source,
                |this, _, event: &SelectEvent<SearchableVec<TraySourceChoice>>, cx| {
                    let SelectEvent::Confirm(Some(id)) = event else {
                        return;
                    };
                    this.change(cx, |settings| settings.source = Some(*id));
                },
            ),
            cx.subscribe(
                &limit,
                |this, _, event: &SelectEvent<SearchableVec<LimitChoice>>, cx| {
                    if let SelectEvent::Confirm(Some(limit)) = event {
                        this.change(cx, |settings| settings.limit = *limit);
                    }
                },
            ),
        ];
        Self {
            settings,
            source,
            limit,
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn settings(&self) -> &TraySettings {
        &self.settings
    }

    /// Show `settings`, e.g. the saved ones, without reporting them.
    pub fn set_settings(
        &mut self,
        settings: TraySettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings == settings {
            return;
        }
        let limits = limit_choices(settings.limit);
        self.limit.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(limits), window, cx);
            select.set_selected_value(&settings.limit, window, cx);
        });
        if let Some(source) = settings.source {
            self.source.update(cx, |select, cx| {
                select.set_selected_value(&source, window, cx);
            });
        }
        self.settings = settings;
        cx.notify();
    }

    /// Offer `choices` as where the reminders come from, keeping the chosen one.
    pub fn set_sources(
        &mut self,
        choices: Vec<TraySourceChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.settings.source;
        self.source.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(choices), window, cx);
            if let Some(source) = source {
                select.set_selected_value(&source, window, cx);
            }
        });
        cx.notify();
    }

    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut TraySettings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings != before {
            cx.emit(TraySettingsEvent(self.settings.clone()));
            cx.notify();
        }
    }
}

/// What the icon is called on this platform.
#[cfg(target_os = "macos")]
const SHOW_LABEL: &str = "Show reminders in the menu bar";
#[cfg(target_os = "windows")]
const SHOW_LABEL: &str = "Show reminders in the notification area";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SHOW_LABEL: &str = "Show reminders in the tray";

/// Where the count shows on this platform.
#[cfg(target_os = "macos")]
const COUNT_LABEL: &str = "Show the number of reminders beside the icon";
#[cfg(not(target_os = "macos"))]
const COUNT_LABEL: &str = "Show the number of reminders when hovering the icon";

/// Only macOS and Windows have a tray icon (see the `tray` crate).
pub(crate) const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

impl Render for TraySettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let off = !SUPPORTED || !self.settings.enabled;
        v_flex()
            .id("settings-tray")
            .gap_4()
            .when(!SUPPORTED, |panel| {
                panel.child(
                    div()
                        .id("tray-unavailable")
                        .role(Role::Status)
                        .aria_label("The tray icon isn't available on this platform yet.")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child("The tray icon isn't available on this platform yet."),
                )
            })
            .child(
                Switch::new("tray-enabled")
                    .checked(self.settings.enabled && SUPPORTED)
                    .disabled(!SUPPORTED)
                    .label(SHOW_LABEL)
                    .accessibility_label(SHOW_LABEL)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.change(cx, |settings| settings.enabled = *on);
                    })),
            )
            .child(
                v_form()
                    .child(
                        field().label("Reminders from").child(
                            Select::new(&self.source)
                                .id("tray-source")
                                .placeholder("Pick a project or saved filter")
                                .accessibility_label("Tray reminders from")
                                .disabled(off),
                        ),
                    )
                    .child(
                        field().label("At most").child(
                            Select::new(&self.limit)
                                .id("tray-limit")
                                .accessibility_label("Most reminders in the tray")
                                .disabled(off),
                        ),
                    ),
            )
            .child(
                Switch::new("tray-show-count")
                    .checked(self.settings.show_count)
                    .disabled(off)
                    .label(COUNT_LABEL)
                    .accessibility_label(COUNT_LABEL)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.change(cx, |settings| settings.show_count = *on);
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{TestAppContext, WindowOptions};

    use super::*;

    #[test]
    fn offers_the_saved_limit_among_the_usual_ones() {
        let titles = |saved| -> Vec<SharedString> {
            limit_choices(saved)
                .into_iter()
                .map(|choice| choice.title)
                .collect()
        };
        assert_eq!(titles(None), ["No limit", "3", "5", "10", "15", "20", "30"]);
        assert_eq!(
            titles(Some(7)),
            ["No limit", "3", "5", "7", "10", "15", "20", "30"]
        );
        assert_eq!(titles(Some(5)).len(), 7);
    }

    #[gpui_kit::test]
    fn reports_every_change(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| TraySettingsView::new(TraySettings::default(), window, cx))
            })
            .unwrap()
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &TraySettingsEvent, _| {
                events.borrow_mut().push(event.0.clone());
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.set_sources(
                    vec![
                        TraySourceChoice {
                            id: TraySourceId::Project(1),
                            name: "Home".into(),
                        },
                        TraySourceChoice {
                            id: TraySourceId::SavedFilter(4),
                            name: "This week".into(),
                        },
                    ],
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.find_by_label(SHOW_LABEL);
            window.find_by_label("Tray reminders from");
            window.find_by_label("Most reminders in the tray");
            window.find_by_label(COUNT_LABEL);
            if SUPPORTED {
                window.click("tray-enabled", cx);
            }
        })
        .unwrap();
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.change(cx, |settings| {
                    settings.source = Some(TraySourceId::SavedFilter(4));
                });
                view.change(cx, |settings| settings.limit = Some(5));
                // No change, no event.
                view.change(cx, |settings| settings.limit = Some(5));
            });
        });
        let enabled = SUPPORTED;
        let mut expected = Vec::new();
        if SUPPORTED {
            expected.push(TraySettings {
                enabled,
                ..TraySettings::default()
            });
        }
        expected.push(TraySettings {
            enabled,
            source: Some(TraySourceId::SavedFilter(4)),
            ..TraySettings::default()
        });
        expected.push(TraySettings {
            enabled,
            source: Some(TraySourceId::SavedFilter(4)),
            limit: Some(5),
            show_count: false,
        });
        assert_eq!(*events.borrow(), expected);
    }
}
