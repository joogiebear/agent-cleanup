//! The window, built with GPUI (Zed's UI framework) and gpui-component.
//!
//! Layout, left to right: an icon rail that switches pages, a sidebar (filters
//! on the Leftovers page), the page itself, and a details panel for the
//! selected repo. Overlays (confirm dialog, toast) sit on top.

mod app;
mod assets;
mod details;
mod leftovers;
mod pages;
mod theme;
mod trail;
mod widgets;

use gpui_kit::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use std::borrow::Cow;

pub fn run() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            let fonts = assets::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                eprintln!("couldn't load fonts: {e}");
            }
            theme::install(cx);

            let bounds = Bounds::centered(None, size(px(1280.), px(780.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(860.), px(520.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Agent Cleanup".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::AgentCleanup::new(window, cx))
            })
            .expect("failed to open the window");
            cx.activate(true);
        });
}
