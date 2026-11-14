use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
};

/// The main application window. Placeholder until task views land.
pub(crate) struct MainWindow;

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_2xl()
            .child("Hello")
    }
}

pub(crate) fn open(cx: &mut App) {
    let options = crate::window_options("Vikui", 960., 640., cx);
    if let Err(err) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| MainWindow)) {
        log::error!("failed to open main window: {err:#}");
    }
}
