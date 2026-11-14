//! Vikui's main window, shown once the user is signed in.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowOptions,
};

/// The main application view. Placeholder until task views land.
pub struct Overview;

impl Render for Overview {
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

/// Open the overview in a new window.
pub fn open(options: WindowOptions, cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Overview))?;
    Ok(handle)
}
