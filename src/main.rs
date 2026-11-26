#![windows_subsystem = "windows"]

mod clipboard;
mod search;
mod storage;
mod ui;

use gpui::*;
use gpui_windows::WindowsPlatform;
use std::rc::Rc;

fn main() {
    let platform = WindowsPlatform::new(false).expect("无法初始化 Windows 平台");
    Application::with_platform(Rc::new(platform)).run(|cx| {
        cx.on_action(|_action: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(600.0), px(500.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(400.0), px(300.0))),
                kind: WindowKind::Normal,
                ..Default::default()
            },
            |_, cx| cx.new(ui::ClipboardView::new),
        )
        .unwrap();
        cx.activate(true);
    });
}

actions!([Quit]);
