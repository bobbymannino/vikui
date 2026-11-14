//! Vikui's sign-in window.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputContentType, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, FontWeight, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, WindowOptions,
    div, prelude::FluentBuilder as _, px,
};
use login::{Credentials, SignIn};
use vikunja::ServerUrl;

type OnSignedIn = Box<dyn FnOnce(Credentials, &mut App)>;

/// Sign-in form shown on launch when no credentials are stored.
pub struct LoginWindow {
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    totp: Entity<InputState>,
    /// Shown once the server reports the account uses two-factor authentication.
    needs_totp: bool,
    submitting: bool,
    error: Option<SharedString>,
    /// Called once, after credentials are saved; the login window then closes itself.
    on_signed_in: Option<OnSignedIn>,
    _subscriptions: Vec<Subscription>,
}

impl LoginWindow {
    fn new(on_signed_in: OnSignedIn, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://tasks.example.com"));
        let username = cx.new(|cx| InputState::new(window, cx));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let totp = cx.new(|cx| InputState::new(window, cx).placeholder("123456"));

        // Enter in any field submits the form.
        let subscriptions = [&server, &username, &password, &totp]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit(window, cx);
                    }
                })
            })
            .collect();

        server.update(cx, |input, cx| input.focus(window, cx));

        Self {
            server,
            username,
            password,
            totp,
            needs_totp: false,
            submitting: false,
            error: None,
            on_signed_in: Some(on_signed_in),
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }

        let server = match ServerUrl::parse(&self.server.read(cx).value()) {
            Ok(server) => server,
            Err(err) => return self.fail(err.to_string(), &self.server.clone(), window, cx),
        };
        let username = self.username.read(cx).value().trim().to_owned();
        if username.is_empty() {
            return self.fail("Enter your username.", &self.username.clone(), window, cx);
        }
        let password = self.password.read(cx).value().to_string();
        if password.is_empty() {
            return self.fail("Enter your password.", &self.password.clone(), window, cx);
        }
        let totp = self
            .needs_totp
            .then(|| self.totp.read(cx).value().trim().to_owned());
        if totp.as_deref() == Some("") {
            return self.fail(
                "Enter the code from your authenticator app.",
                &self.totp.clone(),
                window,
                cx,
            );
        }

        self.submitting = true;
        self.error = None;
        cx.notify();

        let task = cx.background_spawn(async move {
            login::sign_in(SignIn {
                server,
                username,
                password,
                totp,
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.submitting = false;
                match result {
                    Ok(credentials) => {
                        if let Some(on_signed_in) = this.on_signed_in.take() {
                            on_signed_in(credentials, cx);
                        }
                        window.remove_window();
                    }
                    Err(login::Error::Api(vikunja::Error::TotpRequired)) if !this.needs_totp => {
                        this.needs_totp = true;
                        this.error = None;
                        this.totp.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    }
                    Err(err) => {
                        log::warn!("sign-in failed: {err:#}");
                        let focus = match &err {
                            login::Error::Api(vikunja::Error::TotpRequired) => this.totp.clone(),
                            login::Error::Api(vikunja::Error::InvalidCredentials) => {
                                this.password.clone()
                            }
                            _ => this.server.clone(),
                        };
                        this.fail(err.to_string(), &focus, window, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn fail(
        &mut self,
        message: impl Into<SharedString>,
        focus: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = Some(message.into());
        focus.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}

impl Render for LoginWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let disabled = self.submitting;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(380.))
                    .gap_6()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Sign in to Vikunja"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child("Connect Vikui to your Vikunja server."),
                            ),
                    )
                    .when_some(self.error.clone(), |this, message| {
                        this.child(Alert::error("login-error", message))
                    })
                    .child(
                        v_form()
                            .child(
                                field().label("Server").required(true).child(
                                    Input::new(&self.server)
                                        .id("server")
                                        .aria_label("Server address")
                                        .content_type(InputContentType::Url)
                                        .disabled(disabled),
                                ),
                            )
                            .child(
                                field().label("Username").required(true).child(
                                    Input::new(&self.username)
                                        .id("username")
                                        .aria_label("Username")
                                        .content_type(InputContentType::Username)
                                        .disabled(disabled),
                                ),
                            )
                            .child(
                                field().label("Password").required(true).child(
                                    Input::new(&self.password)
                                        .id("password")
                                        .aria_label("Password")
                                        .content_type(InputContentType::Password)
                                        .mask_toggle()
                                        .disabled(disabled),
                                ),
                            )
                            .when(self.needs_totp, |form| {
                                form.child(
                                    field()
                                        .label("Two-factor code")
                                        .description("From your authenticator app.")
                                        .required(true)
                                        .child(
                                            Input::new(&self.totp)
                                                .id("totp")
                                                .aria_label("Two-factor code")
                                                .content_type(InputContentType::OneTimeCode)
                                                .disabled(disabled),
                                        ),
                                )
                            }),
                    )
                    .child(
                        Button::new("sign-in")
                            .primary()
                            .large()
                            .w_full()
                            .label(if disabled { "Signing in…" } else { "Sign in" })
                            .loading(disabled)
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

/// Open the sign-in window. `on_signed_in` runs after a successful sign-in, just before
/// the window closes, so the caller can open whatever comes next.
pub fn open(
    options: WindowOptions,
    cx: &mut App,
    on_signed_in: impl FnOnce(Credentials, &mut App) + 'static,
) -> gpui_kit::Result<AnyWindowHandle> {
    let on_signed_in: OnSignedIn = Box::new(on_signed_in);
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| LoginWindow::new(on_signed_in, window, cx))
    })?;
    Ok(handle)
}
