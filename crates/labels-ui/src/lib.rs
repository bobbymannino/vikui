//! Vikui's labels window: every label the user can see, and a form to create one or
//! change one's title and colour.
//!
//! The app fills the list with [`LabelsView::set_labels`] and saves what the form
//! reports through [`LabelsEvent`], answering with [`LabelsView::saved`] or
//! [`LabelsView::set_error`]; this crate never fetches or saves.

mod label_chip;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState, ColorSelect};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter,
    Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, Rgba, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, WindowOptions, div, px,
};
use vikunja::{Label, LabelDraft};

use label_chip::LabelChip;

const CONTEXT: &str = "Labels";
const LIST_WIDTH: f32 = 220.;

gpui_kit::actions!(labels, [NewLabel, Save]);

/// Register the window's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`.
    // In this window Mod+N starts a new label rather than a new reminder.
    let context = gpui_kit::KeyBindingContextPredicate::parse(CONTEXT)
        .ok()
        .map(std::rc::Rc::new);
    let bindings: [(&str, Box<dyn gpui_kit::Action>); 2] = [
        ("secondary-n", Box::new(NewLabel)),
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
pub enum LabelsEvent {
    /// Create this label; answer with [`LabelsView::saved`] or [`LabelsView::set_error`].
    Create(LabelDraft),
    /// Save this title and colour over `label_id`; answer as for `Create`.
    Update { label_id: i64, draft: LabelDraft },
}

/// What the form is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    New,
    Label(i64),
}

/// The labels list and the form editing one of them, or a new one.
pub struct LabelsView {
    labels: Vec<Label>,
    /// One per label, in the same order.
    chips: Vec<LabelChip>,
    editing: Editing,
    title: Entity<InputState>,
    color: Entity<ColorPickerState>,
    /// Waiting for the labels.
    loading: bool,
    load_error: Option<SharedString>,
    saving: bool,
    error: Option<SharedString>,
    /// Said after a save, until the form changes.
    status: Option<SharedString>,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<LabelsEvent> for LabelsView {}

impl LabelsView {
    /// An empty list, loading until [`Self::set_labels`] or [`Self::set_load_error`].
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Label name"));
        let color = cx.new(|cx| ColorPickerState::new(window, cx));
        let subscriptions = [
            cx.subscribe_in(
                &title,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.save(cx),
                    InputEvent::Change => this.clear_status(cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
            cx.subscribe(&color, |this, _, _: &ColorPickerEvent, cx| {
                this.clear_status(cx);
            }),
        ];
        title.read(cx).focus_handle(cx).focus(window, cx);
        Self {
            labels: Vec::new(),
            chips: Vec::new(),
            editing: Editing::New,
            title,
            color,
            loading: true,
            load_error: None,
            saving: false,
            error: None,
            status: None,
            _subscriptions: subscriptions,
        }
    }

    /// List `labels`, sorted by title. The form keeps what it is editing if that label
    /// is still listed, and otherwise starts a new label.
    pub fn set_labels(
        &mut self,
        mut labels: Vec<Label>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        labels.sort_by_key(|label| label.title.to_lowercase());
        self.labels = labels;
        self.chips = self.labels.iter().map(LabelChip::new).collect();
        self.loading = false;
        self.load_error = None;
        if let Editing::Label(id) = self.editing
            && !self.labels.iter().any(|label| label.id == id)
        {
            self.new_label(window, cx);
        }
        cx.notify();
    }

    /// Show why the labels couldn't be loaded.
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

    /// Show `label` as saved: listed (replacing the label it was) and open in the form.
    pub fn saved(&mut self, label: Label, window: &mut Window, cx: &mut Context<Self>) {
        let created = self.editing == Editing::New;
        self.saving = false;
        self.error = None;
        let id = label.id;
        let mut labels = std::mem::take(&mut self.labels);
        labels.retain(|listed| listed.id != id);
        labels.push(label);
        self.set_labels(labels, window, cx);
        self.edit(id, window, cx);
        self.status = Some(
            if created {
                "Label created."
            } else {
                "Changes saved."
            }
            .into(),
        );
        cx.notify();
    }

    /// Whether the form is editing an existing label, by id; `None` for a new one.
    #[must_use]
    pub fn editing(&self) -> Option<i64> {
        match self.editing {
            Editing::New => None,
            Editing::Label(id) => Some(id),
        }
    }

    /// The form as filled in.
    #[must_use]
    pub fn draft(&self, cx: &App) -> LabelDraft {
        LabelDraft {
            title: self.title.read(cx).value().trim().to_owned(),
            rgb: self.color.read(cx).value().map(to_rgb),
        }
    }

    /// Open `label_id` in the form.
    fn edit(&mut self, label_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(label) = self.labels.iter().find(|label| label.id == label_id) else {
            return;
        };
        let (title, rgb) = (SharedString::from(label.title.clone()), label.rgb());
        self.editing = Editing::Label(label_id);
        self.fill(title, rgb, window, cx);
    }

    /// Clear the form for a new label.
    fn new_label(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Editing::New;
        self.fill(SharedString::default(), None, window, cx);
    }

    fn fill(
        &mut self,
        title: SharedString,
        rgb: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        self.color.update(cx, |color, cx| match rgb {
            Some(rgb) => color.set_value(gpui_kit::rgb(rgb), window, cx),
            None => color.clear_value(window, cx),
        });
        // Setting the fields reports changes; the form starts out unchanged.
        self.status = None;
        self.title.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
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
        if draft.title.is_empty() {
            self.error = Some("Enter a name.".into());
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.emit(match self.editing {
            Editing::New => LabelsEvent::Create(draft),
            Editing::Label(label_id) => LabelsEvent::Update { label_id, draft },
        });
        cx.notify();
    }

    fn on_new_label(&mut self, _: &NewLabel, window: &mut Window, cx: &mut Context<Self>) {
        self.new_label(window, cx);
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let body = if self.loading {
            div()
                .p_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading…")
                .into_any_element()
        } else if let Some(message) = self.load_error.clone() {
            div()
                .id("labels-load-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(Alert::error("labels-load-error-alert", message))
                .test_support()
                .into_any_element()
        } else if self.labels.is_empty() {
            div()
                .p_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("No labels yet.")
                .into_any_element()
        } else {
            v_flex()
                .gap_0p5()
                .children(self.chips.iter().enumerate().map(|(ix, chip)| {
                    let id = chip.id;
                    Button::new(("label", ix))
                        .ghost()
                        .w_full()
                        .justify_start()
                        .accessibility_label(format!("Edit label {}", chip.title))
                        .selected(self.editing == Editing::Label(id))
                        .child(chip.render(cx))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit(id, window, cx);
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("labels-list")
            .role(Role::List)
            .aria_label("Labels")
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
            Editing::New => ("New Label", "Create Label", "Create label"),
            Editing::Label(_) => ("Edit Label", "Save Changes", "Save changes"),
        };
        let preview = LabelChip {
            id: 0,
            title: match self.title.read(cx).value().trim() {
                "" => "Label".into(),
                title => title.to_owned().into(),
            },
            rgb: self.color.read(cx).value().map(to_rgb),
        };
        let has_color = self.color.read(cx).value().is_some();
        let error = self.error.clone().map(|message| {
            // `Alert` has no accessible name of its own; announce the message.
            div()
                .id("label-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(Alert::error("label-error-alert", message))
                .test_support()
        });
        let status = self.status.clone().map(|message| {
            div()
                .id("label-status")
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
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(heading),
            )
            .children(error)
            .child(
                v_form()
                    .child(
                        field()
                            .label("Name")
                            .required(true)
                            .child(Input::new(&self.title).id("label-title").aria_label("Name")),
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
                                    Button::new("clear-label-color")
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
                            .label("Preview")
                            .child(div().id("label-preview").child(preview.render(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().children(status))
                    .child(
                        Button::new("save-label")
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

/// `color` as `0xRRGGBB`, ignoring its opacity: Vikunja's colours have none.
fn to_rgb(color: Hsla) -> u32 {
    let rgba = Rgba::from(color);
    let channel = |value: f32| {
        // Clamped to 0–255 first, so the cast can't truncate or wrap.
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

impl Render for LabelsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.render_list(cx);
        let form = self.render_form(cx);
        let theme = cx.theme();
        v_flex()
            .id("labels")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_new_label))
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
                            .child("Labels"),
                    )
                    .child(
                        Button::new("new-label")
                            .outline()
                            .icon(IconName::Plus)
                            .label("New Label")
                            .accessibility_label("New label")
                            .tooltip_with_action("New label", &NewLabel, Some(CONTEXT))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_label(window, cx);
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

/// Open the labels window, loading until [`LabelsView::set_labels`].
pub fn open(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<LabelsView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| LabelsView::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};

    use super::*;

    fn label(id: i64, title: &str, hex_color: &str) -> Label {
        Label {
            id,
            title: title.into(),
            hex_color: hex_color.into(),
        }
    }

    fn opened(
        cx: &mut TestAppContext,
    ) -> (
        AnyWindowHandle,
        Entity<LabelsView>,
        Rc<RefCell<Vec<LabelsEvent>>>,
    ) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &LabelsEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        (handle, view, events)
    }

    fn set_labels(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<LabelsView>,
        labels: Vec<Label>,
    ) {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.set_labels(labels, window, cx));
        })
        .unwrap();
    }

    fn type_title(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        view: &Entity<LabelsView>,
        title: &str,
    ) {
        let title = SharedString::from(title.to_owned());
        cx.update_window(handle, |_, window, cx| {
            let input = view.read(cx).title.clone();
            input.update(cx, |input, cx| input.set_value(title, window, cx));
        })
        .unwrap();
    }

    fn has_label(cx: &mut TestAppContext, handle: AnyWindowHandle, label: &str) -> bool {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find_by_label(label).is_some()
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn edits_a_listed_label_and_shows_it_saved(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        assert!(has_label(cx, handle, "Create label"));
        set_labels(
            cx,
            handle,
            &view,
            vec![label(2, "home", ""), label(1, "Bills", "#ff0000")],
        );
        assert!(has_label(cx, handle, "Edit label Bills"));
        assert!(has_label(cx, handle, "Edit label home"));

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Sorted by title, ignoring case: Bills first.
            window.click(("label", 0_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(1));
        assert!(has_label(cx, handle, "Save changes"));
        assert_eq!(
            cx.update(|cx| view.read(cx).draft(cx)),
            LabelDraft {
                title: "Bills".into(),
                rgb: Some(0xff_00_00),
            }
        );

        type_title(cx, handle, &view, "Bills & rent");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-s", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let update = LabelsEvent::Update {
            label_id: 1,
            draft: LabelDraft {
                title: "Bills & rent".into(),
                rgb: Some(0xff_00_00),
            },
        };
        assert_eq!(*events.borrow(), [update]);
        assert!(cx.update(|cx| view.read(cx).saving));

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.saved(label(1, "Bills & rent", "ff0000"), window, cx);
            });
        })
        .unwrap();
        assert!(has_label(cx, handle, "Changes saved."));
        assert!(has_label(cx, handle, "Edit label Bills & rent"));
        assert!(!has_label(cx, handle, "Edit label Bills"));
    }

    #[gpui_kit::test]
    fn creates_a_label_with_a_name_and_no_colour(cx: &mut TestAppContext) {
        let (handle, view, events) = opened(cx);
        set_labels(cx, handle, &view, vec![label(1, "Bills", "ff0000")]);
        assert!(has_label(cx, handle, "No colour"));

        // Without a name, saying what's missing rather than saving.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("save-label", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(has_label(cx, handle, "Enter a name."));
        assert!(events.borrow().is_empty());

        type_title(cx, handle, &view, "  Car  ");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("save-label", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [LabelsEvent::Create(LabelDraft {
                title: "Car".into(),
                rgb: None,
            })]
        );

        // A failed save says why and allows another try.
        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Forbidden", cx)));
        assert!(has_label(cx, handle, "Forbidden"));
        assert!(!cx.update(|cx| view.read(cx).saving));

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.saved(label(5, "Car", ""), window, cx));
        })
        .unwrap();
        assert!(has_label(cx, handle, "Label created."));
        assert_eq!(cx.update(|cx| view.read(cx).editing()), Some(5));

        // Mod+N starts another.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-n", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).editing()), None);
        assert_eq!(
            cx.update(|cx| view.read(cx).draft(cx)),
            LabelDraft::default()
        );
    }

    #[gpui_kit::test]
    fn changes_and_clears_the_colour(cx: &mut TestAppContext) {
        let (handle, view, _) = opened(cx);
        set_labels(cx, handle, &view, vec![label(1, "Bills", "")]);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("label", 0_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let color = view.read(cx).color.clone();
            color.update(cx, |color, cx| {
                color.select_color(gpui_kit::rgb(0x19_73_ff).into(), window, cx);
            });
        })
        .unwrap();
        assert_eq!(
            cx.update(|cx| view.read(cx).draft(cx).rgb),
            Some(0x19_73_ff)
        );

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("clear-label-color", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| view.read(cx).draft(cx).rgb), None);
    }

    #[gpui_kit::test]
    fn shows_why_the_labels_did_not_load(cx: &mut TestAppContext) {
        let (handle, view, _) = opened(cx);
        cx.update(|cx| view.update(cx, |view, cx| view.set_load_error("Offline", cx)));
        assert!(has_label(cx, handle, "Offline"));
    }

    #[test]
    fn colours_round_trip_through_hsla() {
        for rgb in [0x00_00_00, 0xff_ff_ff, 0xff_00_00, 0x19_73_ff, 0xe8_e8_e8] {
            assert_eq!(to_rgb(gpui_kit::rgb(rgb).into()), rgb, "{rgb:06x}");
        }
    }
}
