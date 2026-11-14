//! Vikui's main window, shown once the user is signed in.
//!
//! The window is a shell: the app passes in the view to show, so this crate does not
//! depend on other UI crates.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, IntoElement, ParentElement as _,
    Render, Styled as _, Window, WindowOptions,
};

/// The main application view, hosting the current content view.
pub struct Overview {
    content: AnyView,
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.content.clone())
    }
}

/// Open the main window; `content` builds the view it shows.
pub fn open(
    options: WindowOptions,
    cx: &mut App,
    content: impl FnOnce(&mut Window, &mut App) -> AnyView,
) -> gpui_kit::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        let content = content(window, cx);
        cx.new(|_| Overview { content })
    })?;
    Ok(handle)
}
