//! Window options for every window, and remembering where each kind of window was.
//!
//! Each kind (the main window, a reminders window, settings…) keeps one saved frame. A
//! window opens where the last one of its kind was moved or resized to, kept on a
//! display that is still connected; a second window of a kind open at the same time is
//! offset so it doesn't hide the first.

use gpui_kit::{
    AnyWindowHandle, App, Bounds, Global, Pixels, Point, SharedString, Size, Window, WindowBounds,
    WindowOptions, point, px, size,
};
use preferences::WindowPlacement;

/// How far each extra window of a kind is moved right and down.
const CASCADE: f32 = 28.;

/// The kinds of window, each remembered separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Login,
    Main,
    /// A project's or saved filter's reminders; they share one frame.
    Reminders,
    Search,
    Settings,
    NewReminder,
    EditTask,
    /// A task's details in a window of its own.
    Task,
}

impl WindowKind {
    /// Names the kind in the preferences file; keep it the same across versions.
    fn key(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Main => "main",
            Self::Reminders => "reminders",
            Self::Search => "search",
            Self::Settings => "settings",
            Self::NewReminder => "new-reminder",
            Self::EditTask => "edit-task",
            Self::Task => "task",
        }
    }
}

/// The open windows and their kinds.
#[derive(Default)]
struct OpenWindows(Vec<(AnyWindowHandle, WindowKind)>);

impl Global for OpenWindows {}

/// Options for a `kind` window titled `title`: where the last one was, or `width` by
/// `height` centred if none was saved. Pass the opened window to [`track`].
pub fn options(
    kind: WindowKind,
    title: impl Into<SharedString>,
    width: f32,
    height: f32,
    cx: &mut App,
) -> WindowOptions {
    let default_size = size(px(width), px(height));
    WindowOptions {
        window_bounds: Some(initial_bounds(kind, default_size, cx)),
        window_min_size: Some(size(px(width * 0.75), px(height * 0.75))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        // Matches the Linux desktop entry, so the dock shows the app's name and icon.
        app_id: Some(crate::APP_ID.to_owned()),
        ..Default::default()
    }
}

/// Remember where `handle`, a `kind` window opened with [`options`], is moved or resized.
pub fn track(kind: WindowKind, handle: AnyWindowHandle, cx: &mut App) {
    cx.default_global::<OpenWindows>().0.push((handle, kind));
    let Some(root) = handle.downcast::<gpui_kit::base::Root>() else {
        return log::warn!("not remembering a window opened without a kit root");
    };
    root.update(cx, |_, window, cx| {
        // Lives as long as the window's root.
        cx.observe_window_bounds(window, move |_, window, cx| record(kind, window, cx))
            .detach();
    })
    .ok();
}

fn initial_bounds(kind: WindowKind, default_size: Size<Pixels>, cx: &mut App) -> WindowBounds {
    let saved = crate::app_preferences::get(cx)
        .windows
        .get(kind.key())
        .copied();
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.visible_bounds())
        .collect();
    let open = {
        let windows = cx.windows();
        let open = &mut cx.default_global::<OpenWindows>().0;
        open.retain(|(handle, _)| windows.contains(handle));
        open.iter().filter(|(_, open)| *open == kind).count()
    };
    match saved.and_then(|saved| place(saved, &displays, open)) {
        Some((bounds, true)) => WindowBounds::Maximized(bounds),
        Some((bounds, false)) => WindowBounds::Windowed(bounds),
        None => WindowBounds::Windowed(Bounds::centered(None, default_size, cx)),
    }
}

/// Where to open a window saved at `saved`, moved `cascade` steps for the windows of its
/// kind already open, and whether it is maximized. The frame is kept on the display it
/// overlaps most, or put on the first display if it overlaps none, and shrunk to fit.
/// `None` for an empty saved frame or no displays.
fn place(
    saved: WindowPlacement,
    displays: &[Bounds<Pixels>],
    cascade: usize,
) -> Option<(Bounds<Pixels>, bool)> {
    if saved.width == 0 || saved.height == 0 {
        return None;
    }
    let step = px(CASCADE * small_f32(cascade));
    let frame = Bounds::new(
        point(px(small_f32(saved.x)), px(small_f32(saved.y))) + point(step, step),
        size(px(small_f32(saved.width)), px(small_f32(saved.height))),
    );
    let display = displays
        .iter()
        .max_by(|a, b| overlap(&frame, a).total_cmp(&overlap(&frame, b)))
        .filter(|display| overlap(&frame, display) > 0.)
        .or_else(|| displays.first())?;
    let fitted = size(
        frame.size.width.min(display.size.width),
        frame.size.height.min(display.size.height),
    );
    let origin = Point::new(
        frame
            .origin
            .x
            .clamp(display.left(), display.right() - fitted.width),
        frame
            .origin
            .y
            .clamp(display.top(), display.bottom() - fitted.height),
    );
    Some((Bounds::new(origin, fitted), saved.maximized))
}

/// The area `frame` and `display` share.
fn overlap(frame: &Bounds<Pixels>, display: &Bounds<Pixels>) -> f32 {
    let shared = frame.intersect(display);
    f32::from(shared.size.width).max(0.) * f32::from(shared.size.height).max(0.)
}

/// Save where the `kind` window `window` now is, unless it is already saved there.
fn record(kind: WindowKind, window: &Window, cx: &mut App) {
    let (bounds, maximized, fullscreen) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (bounds, false, false),
        WindowBounds::Maximized(bounds) => (bounds, true, false),
        WindowBounds::Fullscreen(bounds) => (bounds, false, true),
    };
    let size = reopen_size(window, bounds.size, fullscreen);
    let placement = WindowPlacement {
        x: whole(bounds.origin.x),
        y: whole(bounds.origin.y),
        width: whole(size.width).unsigned_abs(),
        height: whole(size.height).unsigned_abs(),
        maximized,
    };
    if crate::app_preferences::get(cx).windows.get(kind.key()) == Some(&placement) {
        return;
    }
    crate::app_preferences::update_soon(cx, |preferences| {
        preferences.windows.insert(kind.key().to_owned(), placement);
    });
}

/// The size to save so the window reopens at the size it is now.
///
/// On macOS GPUI reports a window's frame, titlebar included, but opens a window with
/// that size as its content, so saving the frame would grow the window by its titlebar
/// every time it reopened; save the content size instead. A full-screen window has no
/// titlebar to measure, so its restore frame is kept. Windows and Linux report and open
/// windows by the same size.
fn reopen_size(window: &Window, frame: Size<Pixels>, fullscreen: bool) -> Size<Pixels> {
    #[cfg(target_os = "macos")]
    if !fullscreen {
        return window.viewport_size();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, fullscreen);
    frame
}

/// Rounded to whole logical pixels.
fn whole(value: Pixels) -> i32 {
    let value = f32::from(value).round();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "window coordinates are far inside i32; `as` saturates beyond it"
    )]
    let value = value as i32;
    value
}

/// Window coordinates and counts are far below 2^24, so they convert to `f32` exactly.
fn small_f32(value: impl TryInto<i32>) -> f32 {
    let value: i32 = value.try_into().unwrap_or(i32::MAX);
    #[expect(clippy::cast_precision_loss, reason = "window coordinates are small")]
    let value = value as f32;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    fn saved(x: i32, y: i32, width: u32, height: u32) -> WindowPlacement {
        WindowPlacement {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    /// The main display, and a second one to its left.
    const MAIN: (f32, f32, f32, f32) = (0., 0., 1440., 900.);
    const LEFT: (f32, f32, f32, f32) = (-1920., 0., 1920., 1080.);

    fn displays() -> Vec<Bounds<Pixels>> {
        [MAIN, LEFT]
            .into_iter()
            .map(|(x, y, w, h)| rect(x, y, w, h))
            .collect()
    }

    #[test]
    fn reopens_where_it_was_on_any_display() {
        assert_eq!(
            place(saved(100, 80, 960, 640), &displays(), 0),
            Some((rect(100., 80., 960., 640.), false))
        );
        assert_eq!(
            place(saved(-1800, 50, 1200, 800), &displays(), 0),
            Some((rect(-1800., 50., 1200., 800.), false))
        );
        let maximized = WindowPlacement {
            maximized: true,
            ..saved(100, 80, 960, 640)
        };
        assert_eq!(
            place(maximized, &displays(), 0).map(|(_, max)| max),
            Some(true)
        );
    }

    #[test]
    fn cascades_extra_windows_of_a_kind() {
        assert_eq!(
            place(saved(100, 80, 960, 640), &displays(), 2),
            Some((rect(156., 136., 960., 640.), false))
        );
    }

    #[test]
    fn keeps_windows_on_a_connected_display() {
        // Its display was unplugged: it moves to the first one.
        assert_eq!(
            place(saved(3000, 200, 960, 640), &displays(), 0),
            Some((rect(480., 200., 960., 640.), false))
        );
        // Hanging off the bottom right of the main display: pulled back on.
        assert_eq!(
            place(saved(1000, 700, 960, 640), &displays(), 0),
            Some((rect(480., 260., 960., 640.), false))
        );
        // Bigger than the display it is on: shrunk to fit.
        assert_eq!(
            place(saved(0, 0, 2000, 1200), &displays()[..1], 0),
            Some((rect(0., 0., 1440., 900.), false))
        );
    }

    #[test]
    fn ignores_empty_frames_and_missing_displays() {
        assert_eq!(place(saved(0, 0, 0, 640), &displays(), 0), None);
        assert_eq!(place(saved(0, 0, 960, 640), &[], 0), None);
    }

    struct Blank;

    impl gpui_kit::Render for Blank {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }

    #[gpui_kit::test]
    fn remembers_where_a_tracked_window_is(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            let options = options(WindowKind::Settings, "Settings", 480., 560., cx);
            let (handle, _) = gpui_kit::open_window(options, cx, |_, cx| {
                gpui_kit::AppContext::new(cx, |_| Blank)
            })
            .unwrap();
            track(WindowKind::Settings, handle, cx);
            handle
        });
        cx.update_window(handle, |_, window, cx| window.bounds_changed(cx))
            .unwrap();
        cx.run_until_parked();
        let (saved, bounds, reopen) = cx
            .update_window(handle, |_, window, cx| {
                let saved = crate::app_preferences::get(cx)
                    .windows
                    .get("settings")
                    .copied();
                let bounds = window.window_bounds().get_bounds();
                (saved, bounds, reopen_size(window, bounds.size, false))
            })
            .unwrap();
        let saved = saved.expect("the settings window's frame is saved");
        assert_eq!(saved.x, whole(bounds.origin.x));
        assert_eq!(saved.y, whole(bounds.origin.y));
        // Saved at the size it reopens at: its content size on macOS.
        assert_eq!(saved.width, whole(reopen.width).unsigned_abs());
        assert_eq!(saved.height, whole(reopen.height).unsigned_abs());

        // The next settings window opens there, offset from the one still open.
        let next = cx.update(|cx| options(WindowKind::Settings, "Settings", 480., 560., cx));
        let Some(WindowBounds::Windowed(next)) = next.window_bounds else {
            panic!("expected a windowed frame");
        };
        assert_eq!(next.size, reopen);
    }

    #[test]
    fn kinds_have_distinct_keys() {
        let kinds = [
            WindowKind::Login,
            WindowKind::Main,
            WindowKind::Reminders,
            WindowKind::Search,
            WindowKind::Settings,
            WindowKind::NewReminder,
            WindowKind::EditTask,
            WindowKind::Task,
        ];
        let mut keys: Vec<_> = kinds.iter().map(|kind| kind.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), kinds.len());
    }
}
