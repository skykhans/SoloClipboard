#![windows_subsystem = "windows"]

mod clipboard;
mod desktop;
mod instance;
mod search;
mod storage;
mod ui;

use gpui::*;
use gpui_windows::WindowsPlatform;
use std::rc::Rc;
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

fn main() {
    let startup = std::env::args_os()
        .skip(1)
        .any(|arg| arg == desktop::STARTUP_ARG);
    let launch = instance::LaunchLock::acquire().expect("无法获取启动锁");
    if launch
        .reuse_window(!startup)
        .expect("无法打开已有的剪贴板窗口")
    {
        return;
    }
    let platform = WindowsPlatform::new(false).expect("无法初始化 Windows 平台");
    Application::with_platform(Rc::new(platform)).run(move |cx| {
        cx.set_prompt_builder(ui::render_prompt);
        cx.on_action(|_action: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(600.0), px(500.0)), cx);
        let previous_window = if startup {
            std::ptr::null_mut()
        } else {
            unsafe { GetForegroundWindow() }
        };

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: None,
                        appears_transparent: false,
                        traffic_light_position: None,
                    }),
                    window_min_size: Some(size(px(400.0), px(300.0))),
                    kind: WindowKind::Normal,
                    show: !startup,
                    focus: false,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| ui::ClipboardView::new(window, previous_window, cx)),
            )
            .unwrap();
        if startup && !window.read(cx).unwrap().has_tray() {
            cx.quit();
        } else {
            window
                .update(cx, |_, window, _| instance::register_window(window))
                .expect("无法访问剪贴板窗口")
                .expect("无法注册剪贴板窗口");
        }
        drop(launch);
    });
}

actions!([Quit]);
