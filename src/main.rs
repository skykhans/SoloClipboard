mod clipboard;
mod storage;
mod ui;

use gpui::*;
use std::sync::Arc;

fn main() {
    App::new().run(|cx: &mut AppContext| {
        cx.activate(true);
        cx.on_action(|_action: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(600.0), px(500.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Solo Clipboard".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(400.0), px(300.0))),
                kind: WindowKind::Normal,
                ..Default::default()
            },
            |cx| {
                let app_state = Arc::new(ui::AppState::new());
                cx.new_view(|cx| ui::ClipboardView::new(app_state, cx))
            },
        )
        .unwrap();
    });
}

actions!(Quit);
