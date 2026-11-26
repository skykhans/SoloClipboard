use crate::clipboard::{ClipboardEntry, ClipboardMonitor};
use crate::desktop::{self, Settings, Shortcut, TrayWindow};
use crate::search::SearchInput;
use crate::storage::Storage;
use chrono::NaiveDate;
use gpui::{prelude::*, *};
use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use windows_sys::Win32::Foundation::HWND;

fn search_highlights(text: &str, query: &str) -> Vec<(Range<usize>, HighlightStyle)> {
    if query.is_empty() {
        return Vec::new();
    }
    let lowercase = text.to_lowercase();
    let mut matches = lowercase.match_indices(query).peekable();
    if matches.peek().is_none() {
        return Vec::new();
    }

    // Lowercasing can change a character's byte length.
    let source_ranges: Vec<_> = text
        .char_indices()
        .flat_map(|(start, character)| {
            let range = start..start + character.len_utf8();
            let lowercase_len = character.to_lowercase().map(char::len_utf8).sum();
            std::iter::repeat_n(range, lowercase_len)
        })
        .collect();
    let style = HighlightStyle {
        color: Some(rgb(0x241c0b).into()),
        background_color: Some(rgb(0xffcf66).into()),
        ..Default::default()
    };
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (start, matched) in matches {
        let range = source_ranges[start].start..source_ranges[start + matched.len() - 1].end;
        if let Some((previous, _)) = highlights
            .last_mut()
            .filter(|(previous, _)| previous.end >= range.start)
        {
            previous.end = range.end;
        } else {
            highlights.push((range, style));
        }
    }
    highlights
}

struct ToolbarTooltip(&'static str);

impl Render for ToolbarTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x30343e))
            .text_color(rgb(0xe8ebf2))
            .text_xs()
            .child(self.0)
    }
}

pub fn render_prompt(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    buttons: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let prompt = cx.new(|cx| ConfirmationPrompt {
        focus: cx.focus_handle(),
        title: message.into(),
        detail: detail.map(SharedString::from),
        buttons: buttons.to_vec(),
        warning: matches!(level, PromptLevel::Warning | PromptLevel::Critical),
    });
    handle.with_view(prompt, window, cx)
}

struct ConfirmationPrompt {
    focus: FocusHandle,
    title: SharedString,
    detail: Option<SharedString>,
    buttons: Vec<PromptButton>,
    warning: bool,
}

impl EventEmitter<PromptResponse> for ConfirmationPrompt {}

impl Focusable for ConfirmationPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConfirmationPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cancel = self.buttons.iter().position(PromptButton::is_cancel);

        div()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(0.55))
            .child(
                div()
                    .id("confirmation")
                    .role(Role::Dialog)
                    .aria_label(self.title.clone())
                    .track_focus(&self.focus)
                    .tab_group()
                    .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                        if let Some(index) = cancel.filter(|_| event.keystroke.key == "escape") {
                            cx.emit(PromptResponse(index));
                            cx.stop_propagation();
                        }
                    }))
                    .w(px(340.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x393e4a))
                    .bg(rgb(0x252831))
                    .text_color(rgb(0xe8ebf2))
                    .text_sm()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .when_some(self.detail.clone(), |dialog, detail| {
                        dialog.child(div().text_color(rgb(0xaab2c3)).child(detail))
                    })
                    .child(div().mt_2().flex().justify_end().gap_2().children(
                        self.buttons.iter().enumerate().map(|(index, button)| {
                            ClipboardView::button(button.label().clone())
                                .tab_index(index as isize)
                                .focus(|style| style.bg(rgb(0x404655)))
                                .when(
                                    self.warning && matches!(button, PromptButton::Ok(_)),
                                    |button| {
                                        button
                                            .bg(rgb(0x713c42))
                                            .hover(|style| style.bg(rgb(0x8b494e)))
                                            .focus(|style| style.bg(rgb(0x8b494e)))
                                    },
                                )
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(PromptResponse(index));
                                }))
                        }),
                    )),
            )
    }
}

pub struct ClipboardView {
    monitor: ClipboardMonitor,
    storage: Option<Storage>,
    search: Entity<SearchInput>,
    history_scroll: ScrollHandle,
    date_menu: Option<Bounds<Pixels>>,
    scrollbar_drag: Rc<Cell<Option<Pixels>>>,
    selection: Option<HashSet<usize>>,
    status: Option<(String, bool)>,
    status_timeout: Option<Task<()>>,
    tray: Option<TrayWindow>,
    settings: Settings,
    settings_open: bool,
    start_with_windows: Option<bool>,
    shortcut_focus: FocusHandle,
    dirty: bool,
    _search_subscription: Subscription,
    _escape_subscription: Subscription,
}

impl ClipboardView {
    pub fn new(window: &Window, previous_window: HWND, cx: &mut Context<Self>) -> Self {
        let (storage, history, mut status) = match Storage::open() {
            Ok((storage, history)) => (Some(storage), history, None),
            Err(error) => (
                None,
                Vec::new(),
                Some((format!("无法打开历史记录：{error}"), true)),
            ),
        };
        let mut settings = Settings::default();
        let tray = storage.as_ref().and_then(|storage| {
            match storage.load_settings() {
                Ok(saved) => settings = saved,
                Err(error) => status = Some((format!("无法读取设置：{error}"), true)),
            }
            match TrayWindow::new(window, settings.close_to_tray, previous_window) {
                Ok(tray) => {
                    if let Err(error) = tray.set_shortcut(settings.shortcut, || Ok(())) {
                        status = Some((error, true));
                    }
                    Some(tray)
                }
                Err(error) => {
                    status = Some((format!("无法初始化通知区域：{error}"), true));
                    None
                }
            }
        });
        let search = cx.new(SearchInput::new);
        let search_subscription = cx.observe(&search, |view, _, cx| {
            view.date_menu = None;
            cx.notify();
        });
        let escape_subscription = cx.observe_keystrokes(|view, event, window, cx| {
            if event.keystroke.key == "escape"
                && event.keystroke.modifiers == Modifiers::default()
                && !window.has_active_prompt()
            {
                if view.date_menu.take().is_some() {
                    cx.notify();
                } else if let Some(tray) = &view.tray {
                    tray.hide();
                }
            }
        });
        cx.spawn(async |view, cx| loop {
            if view
                .update(cx, |view, cx| {
                    let captured = view.monitor.capture(cx);
                    view.dirty |= captured;
                    if view.storage.is_some() && view.dirty {
                        view.persist(cx);
                    } else if captured {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
        })
        .detach();
        Self {
            monitor: ClipboardMonitor::new(history),
            storage,
            search,
            history_scroll: ScrollHandle::new(),
            date_menu: None,
            scrollbar_drag: Rc::default(),
            selection: None,
            status,
            status_timeout: None,
            tray,
            settings,
            settings_open: false,
            start_with_windows: None,
            shortcut_focus: cx.focus_handle(),
            dirty: false,
            _search_subscription: search_subscription,
            _escape_subscription: escape_subscription,
        }
    }

    fn history_scrollbar(&self) -> impl IntoElement {
        let scroll = self.history_scroll.clone();
        let drag = self.scrollbar_drag.clone();
        canvas(
            {
                let scroll = scroll.clone();
                move |bounds, window, _| {
                    (scroll.max_offset().y > px(0.)).then(|| {
                        window.insert_hitbox(bounds, HitboxBehavior::BlockMouseExceptScroll)
                    })
                }
            },
            move |bounds, hitbox, window, _| {
                let Some(hitbox) = hitbox else {
                    drag.set(None);
                    return;
                };
                let max_offset = scroll.max_offset().y;
                let track = Bounds {
                    origin: point(bounds.left() + px(3.), bounds.top()),
                    size: size(px(6.), bounds.size.height),
                };
                let viewport_height = scroll.bounds().size.height;
                let thumb_height = (track.size.height
                    * (viewport_height / (viewport_height + max_offset)))
                    .max(px(24.))
                    .min(track.size.height);
                let travel = track.size.height - thumb_height;
                let thumb = Bounds {
                    origin: point(
                        track.left(),
                        track.top() + travel * (-scroll.offset().y / max_offset),
                    ),
                    size: size(track.size.width, thumb_height),
                };
                window.paint_quad(fill(track, rgb(0x30343e)).corner_radii(px(3.)));
                window.paint_quad(fill(thumb, rgb(0x89919f)).corner_radii(px(3.)));

                window.on_mouse_event({
                    let drag = drag.clone();
                    let scroll = scroll.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if !phase.capture() || event.button != MouseButton::Left {
                            return;
                        }
                        drag.set(None);
                        if !hitbox.is_hovered(window) {
                            return;
                        }
                        let y = event.position.y;
                        if y < thumb.top() || y > thumb.bottom() {
                            let page = if y < thumb.top() {
                                viewport_height
                            } else {
                                -viewport_height
                            };
                            let mut offset = scroll.offset();
                            offset.y = (offset.y + page).clamp(-max_offset, px(0.));
                            scroll.set_offset(offset);
                            window.refresh();
                        } else {
                            drag.set(Some(y - thumb.top()));
                        }
                        cx.stop_propagation();
                    }
                });
                window.on_mouse_event({
                    let drag = drag.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if !phase.capture() {
                            return;
                        }
                        if event.pressed_button != Some(MouseButton::Left) {
                            drag.set(None);
                            return;
                        }
                        let Some(grab_offset) = drag.get().filter(|_| travel > px(0.)) else {
                            return;
                        };
                        let position = event.position.y - track.top() - grab_offset;
                        let mut offset = scroll.offset();
                        offset.y = -max_offset * (position / travel).clamp(0., 1.);
                        scroll.set_offset(offset);
                        window.refresh();
                        cx.stop_propagation();
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase.capture() && event.button == MouseButton::Left && drag.take().is_some()
                    {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .right_0()
        .top_1()
        .bottom_3()
        .w(px(12.))
    }

    pub fn has_tray(&self) -> bool {
        self.tray.is_some()
    }

    fn update_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let (Some(tray), Some(storage)) = (&self.tray, &self.storage) else {
            return;
        };
        let save = || storage.save_settings(settings);
        let result = if settings.shortcut == self.settings.shortcut {
            save()
        } else {
            tray.set_shortcut(settings.shortcut, save)
        };
        match result {
            Ok(()) => {
                tray.set_close_to_tray(settings.close_to_tray);
                self.settings = settings;
                self.status = None;
            }
            Err(error) => self.status = Some((format!("无法应用设置：{error}"), true)),
        }
        cx.notify();
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if let Some(storage) = &self.storage {
            match storage.save_history(&self.monitor.history) {
                Ok(()) => {
                    self.dirty = false;
                    if self
                        .status
                        .as_ref()
                        .is_some_and(|(message, _)| message.starts_with("自动保存失败"))
                    {
                        self.status = None;
                    }
                }
                Err(error) => self.status = Some((format!("自动保存失败：{error}"), true)),
            }
        }
        cx.notify();
    }

    fn save_history_change(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.persist(cx);
    }

    fn set_transfer_status(&mut self, status: (String, bool), cx: &mut Context<Self>) {
        let success = !status.1;
        self.status = Some(status);
        self.status_timeout = success.then(|| {
            cx.spawn(async move |view, cx| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = view.update(cx, |view, cx| {
                    if matches!(view.status, Some((_, false))) {
                        view.status = None;
                        cx.notify();
                    }
                });
            })
        });
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入剪贴板历史".into()),
        });
        cx.spawn(async move |view, cx| {
            let result = match choice.await {
                Ok(Ok(paths)) => {
                    let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                        return;
                    };
                    cx.background_executor()
                        .spawn(
                            async move { Storage::read(&path).map_err(|error| error.to_string()) },
                        )
                        .await
                }
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = view.update(cx, |view, cx| match result {
                Ok(history) => {
                    let count = view.monitor.import(history);
                    view.set_transfer_status((format!("已导入 {count} 条新记录"), false), cx);
                    if count > 0 || view.dirty {
                        view.save_history_change(cx);
                    }
                }
                Err(error) => view.set_transfer_status((format!("导入失败：{error}"), true), cx),
            });
        })
        .detach();
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let directory = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();
        let choice = cx.prompt_for_new_path(&directory, Some("clipboard-history.json"));
        cx.spawn(async move |view, cx| {
            let result = choice.await;
            let _ = view.update(cx, |view, cx| {
                let status = match result {
                    Ok(Ok(Some(path))) => match Storage::write(&path, &view.monitor.history) {
                        Ok(()) => (
                            format!("已导出 {} 条记录", view.monitor.history.len()),
                            false,
                        ),
                        Err(error) => (format!("导出失败：{error}"), true),
                    },
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => (format!("导出失败：{error}"), true),
                    Err(error) => (format!("导出失败：{error}"), true),
                };
                view.set_transfer_status(status, cx);
            });
        })
        .detach();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = self.selection.take() {
            self.monitor
                .history
                .retain(|entry| !selection.contains(&entry.id));
            self.save_history_change(cx);
        }
    }

    fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            window.activate_window();
        }
        let response = window.prompt(
            PromptLevel::Warning,
            "清空剪贴板历史？",
            Some("本地保存的全部记录都会删除。"),
            &[PromptButton::cancel("取消"), PromptButton::ok("清空")],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if response.await == Ok(1) {
                let _ = view.update(cx, |view, cx| {
                    view.monitor.history.clear();
                    view.selection = None;
                    view.save_history_change(cx);
                });
            }
        })
        .detach();
    }

    fn button_base(label: impl Into<SharedString>) -> Stateful<Div> {
        let label = label.into();
        div()
            .id(label.clone())
            .role(Role::Button)
            .aria_label(label)
            .flex_shrink_0()
            .rounded_md()
            .bg(rgb(0x30343e))
            .hover(|style| style.bg(rgb(0x404655)))
            .cursor_pointer()
    }

    fn button(label: impl Into<SharedString>) -> Stateful<Div> {
        let label = label.into();
        Self::button_base(label.clone())
            .px_3()
            .py_1p5()
            .child(label)
    }

    fn icon_button(label: &'static str, glyph: &'static str) -> Stateful<Div> {
        Self::button_base(label)
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_, cx| cx.new(|_| ToolbarTooltip(label)).into())
            .child(
                div()
                    .font_family("Segoe MDL2 Assets")
                    .text_size(px(16.))
                    .line_height(px(16.))
                    .child(glyph),
            )
    }

    fn toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let (selection_label, selection_icon) = if self.selection.is_some() {
            ("取消选择", "\u{e711}")
        } else {
            ("批量删除", "\u{e762}")
        };

        div()
            .flex_shrink_0()
            .flex()
            .flex_nowrap()
            .items_center()
            .gap_1p5()
            .p_3()
            .child(div().min_w_0().flex_1().child(self.search.clone()))
            .child(
                Self::icon_button("导入", "\u{e896}")
                    .on_click(cx.listener(|view, _, _, cx| view.import(cx))),
            )
            .child(
                Self::icon_button("导出", "\u{e11c}")
                    .on_click(cx.listener(|view, _, _, cx| view.export(cx))),
            )
            .child(
                Self::icon_button(selection_label, selection_icon).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.selection = if view.selection.is_some() {
                            None
                        } else {
                            Some(HashSet::new())
                        };
                        cx.notify();
                    },
                )),
            )
            .when(
                self.selection
                    .as_ref()
                    .is_some_and(|selection| !selection.is_empty()),
                |toolbar| {
                    toolbar.child(
                        Self::icon_button("删除选中", "\u{e74d}")
                            .on_click(cx.listener(|view, _, _, cx| view.delete_selected(cx))),
                    )
                },
            )
            .child(
                Self::icon_button("清空全部", "\u{e894}")
                    .on_click(cx.listener(|view, _, window, cx| view.clear_history(window, cx))),
            )
            .when(self.has_tray(), |toolbar| {
                toolbar.child(Self::icon_button("设置", "\u{e713}").on_click(cx.listener(
                    |view, _, _, cx| {
                        view.start_with_windows = match desktop::starts_with_windows() {
                            Ok(enabled) => Some(enabled),
                            Err(error) => {
                                view.status =
                                    Some((format!("无法读取开机启动设置：{error}"), true));
                                None
                            }
                        };
                        view.settings_open = true;
                        if let Some(tray) = &view.tray {
                            tray.set_settings_open(true);
                        }
                        cx.notify();
                    },
                )))
            })
    }

    fn settings_section(title: &'static str) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .bg(rgb(0x252831))
            .p_3()
            .child(title)
    }

    fn setting_option(label: &'static str, selected: bool) -> Stateful<Div> {
        Self::button(label).when(selected, |button| {
            button
                .bg(rgb(0x31564b))
                .hover(|style| style.bg(rgb(0x31564b)))
        })
    }

    fn settings(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let recording = self.shortcut_focus.is_focused(window);
        let shortcut_label = self
            .settings
            .shortcut
            .map_or_else(|| "已禁用".into(), Shortcut::label);

        div()
            .flex_1()
            .min_h_0()
            .id("settings")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_base().child("设置"))
                    .child(Self::button("返回列表").on_click(cx.listener(
                        |view, _, window, cx| {
                            window.blur(cx);
                            view.settings_open = false;
                            if let Some(tray) = &view.tray {
                                tray.set_settings_open(false);
                            }
                            cx.notify();
                        },
                    ))),
            )
            .child(
                Self::settings_section("开机启动")
                    .when_some(self.start_with_windows, |section, enabled| {
                        section.child(div().flex().gap_2().children(
                            [("关闭", false), ("开启", true)].map(|(label, selected)| {
                                Self::setting_option(label, enabled == selected)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        match desktop::set_start_with_windows(selected) {
                                            Ok(()) => {
                                                view.start_with_windows = Some(selected);
                                                view.status = None;
                                            }
                                            Err(error) => {
                                                view.status = Some((
                                                    format!("无法更改开机启动设置：{error}"),
                                                    true,
                                                ));
                                            }
                                        }
                                        cx.notify();
                                    }))
                            }),
                        ))
                    }),
            )
            .child(
                Self::settings_section("关闭窗口时")
                    .child(div().flex().flex_wrap().gap_2().children(
                        [("最小化到通知区域", true), ("退出程序", false)].map(
                            |(label, close_to_tray)| {
                                Self::setting_option(label, self.settings.close_to_tray == close_to_tray)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.update_settings(
                                            Settings {
                                                close_to_tray,
                                                ..view.settings
                                            },
                                            cx,
                                        );
                                    }))
                            },
                        ),
                    )),
            )
            .child(
                Self::settings_section("打开剪贴板的全局快捷键")
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Self::button("更改快捷键")
                                    .track_focus(&self.shortcut_focus)
                                    .tab_index(0)
                                    .focus(|style| style.bg(rgb(0x31564b)))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.status = None;
                                        view.shortcut_focus.focus(window, cx);
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        |view, event: &KeyDownEvent, window, cx| {
                                            if event.keystroke.key == "escape" {
                                                view.status = None;
                                                window.blur(cx);
                                            } else if let Some(shortcut) =
                                                Shortcut::from_keystroke(&event.keystroke)
                                            {
                                                view.update_settings(
                                                    Settings {
                                                        shortcut: Some(shortcut),
                                                        ..view.settings
                                                    },
                                                    cx,
                                                );
                                                window.blur(cx);
                                            } else {
                                                view.status = Some((
                                                    "请用修饰键搭配字母、数字或功能键。F12 由 Windows 保留。"
                                                        .into(),
                                                    true,
                                                ));
                                            }
                                            cx.stop_propagation();
                                            window.prevent_default();
                                            cx.notify();
                                        },
                                    )),
                            )
                            .when(self.settings.shortcut != Some(Shortcut::WIN_V), |row| {
                                row.child(Self::button("使用 Win + V").on_click(cx.listener(
                                    |view, _, window, cx| {
                                        window.blur(cx);
                                        view.update_settings(
                                            Settings {
                                                shortcut: Some(Shortcut::WIN_V),
                                                ..view.settings
                                            },
                                            cx,
                                        );
                                    },
                                )))
                            })
                            .when(self.settings.shortcut.is_some(), |row| {
                                row.child(Self::button("禁用").on_click(cx.listener(
                                    |view, _, window, cx| {
                                        window.blur(cx);
                                        view.update_settings(
                                            Settings {
                                                shortcut: None,
                                                ..view.settings
                                            },
                                            cx,
                                        );
                                    },
                                )))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .text_color(rgb(0xc2cadd))
                                    .child(shortcut_label),
                            ),
                    )
                    .when(recording, |section| {
                        section.child(div().text_xs().text_color(rgb(0xaab2c3)).child(
                            "使用 Ctrl、Alt、Shift 或 Win 搭配字母、数字或功能键。Esc 取消。",
                        ))
                    }),
            )
    }

    fn history_date(date: NaiveDate, index: usize, view: WeakEntity<Self>) -> Stateful<Div> {
        let label = date.to_string();
        Self::button_base(label.clone())
            .bg(rgb(0x21252d))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(rgb(0xc2cadd))
            .child(label)
            .child(div().font_family("Segoe MDL2 Assets").child("\u{e70d}"))
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    let scroll = &view.history_scroll;
                    view.date_menu = scroll.bounds_for_item(index).map(|mut bounds| {
                        bounds.origin += scroll.offset();
                        bounds.origin.y = bounds.origin.y.max(scroll.bounds().top());
                        bounds
                    });
                    cx.notify();
                });
            })
    }

    fn sticky_history_date(
        &self,
        dates: Vec<(NaiveDate, usize)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let scroll = self.history_scroll.clone();
        let view = cx.weak_entity();
        canvas(
            move |_, window, cx| {
                // The scroll list has already laid out its rows and applied the current offset.
                let viewport = scroll.bounds();
                let top = viewport.top() - scroll.offset().y;
                let visible = dates.partition_point(|&(_, index)| {
                    scroll
                        .bounds_for_item(index)
                        .is_some_and(|bounds| bounds.top() <= top)
                });
                let &(date, index) = dates[..visible].last()?;
                let bounds = scroll.bounds_for_item(index)?;
                if bounds.top() == top {
                    return None;
                }
                let mut header = div()
                    .w(bounds.size.width)
                    .bg(rgb(0x1b1e25))
                    .child(Self::history_date(date, index, view).block_mouse_except_scroll())
                    .into_any_element();
                header.prepaint_as_root(
                    point(bounds.left(), viewport.top()),
                    bounds.size.into(),
                    window,
                    cx,
                );
                Some(header)
            },
            |_, header, window, cx| {
                if let Some(mut header) = header {
                    header.paint(window, cx);
                }
            },
        )
        .absolute()
        .inset_0()
    }

    fn history_date_menu(
        anchor: Bounds<Pixels>,
        dates: &[(NaiveDate, usize)],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().occlude().on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.date_menu = None;
                    cx.notify();
                }),
            ))
            .child(deferred(
                anchored()
                    .attach(anchor, Anchor::BottomLeft)
                    .offset(point(px(0.), px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("history-date-menu")
                            .aria_label("跳转到日期")
                            .occlude()
                            .w(px(148.))
                            .max_h(px(224.))
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .p_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x393e4a))
                            .bg(rgb(0x252831))
                            .shadow_lg()
                            .children(dates.iter().map(|&(date, index)| {
                                Self::button(date.to_string()).bg(rgb(0x252831)).on_click(
                                    cx.listener(move |view, _, _, cx| {
                                        view.date_menu = None;
                                        view.history_scroll.scroll_to_top_of_item(index);
                                        cx.notify();
                                    }),
                                )
                            })),
                    ),
            ))
    }

    fn history_item(
        &self,
        entry: &ClipboardEntry,
        query: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = entry.id;
        let (pin_label, pin_icon) = if entry.pinned {
            ("取消置顶", "\u{e77a}")
        } else {
            ("置顶", "\u{e718}")
        };
        let selected = self
            .selection
            .as_ref()
            .is_some_and(|selection| selection.contains(&id));
        let end = entry.content.char_indices().nth(500);
        let mut preview = end
            .map_or(entry.content.as_str(), |(end, _)| &entry.content[..end])
            .replace('\t', "    ");
        let highlights = search_highlights(&preview, query);
        if end.is_some() {
            preview.push('…');
        }

        div()
            .id(("entry", id))
            .role(Role::ListItem)
            .aria_label(if preview.is_empty() {
                SharedString::from("图片")
            } else {
                SharedString::from(preview.clone())
            })
            .min_w_0()
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_md()
            .overflow_hidden()
            .bg(if selected {
                rgb(0x2a3d56)
            } else {
                rgb(0x252831)
            })
            .hover(|style| style.bg(rgb(0x303747)))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, window, cx| {
                if let Some(selection) = &mut view.selection {
                    if !selection.remove(&id) {
                        selection.insert(id);
                    }
                } else {
                    let result = view
                        .tray
                        .as_ref()
                        .ok_or_else(|| "无法恢复原来的输入窗口".to_string())
                        .and_then(|tray| tray.paste(|| view.monitor.copy_entry(id, window)));
                    view.status = result
                        .err()
                        .map(|error| (format!("粘贴失败：{error}"), true));
                }
                cx.notify();
            }))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when_some(
                        entry.image.as_ref().and_then(|image| image.load()),
                        |content, image| {
                            content.child(
                                img(image)
                                    .w_full()
                                    .h(px(124.))
                                    .object_fit(ObjectFit::Contain),
                            )
                        },
                    )
                    .when(!preview.is_empty(), |content| {
                        content.child(
                            div()
                                .line_clamp(3)
                                .child(StyledText::new(preview).with_highlights(highlights)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .self_stretch()
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(rgb(0xaab2c3))
                            .child(entry.timestamp.format("%H:%M:%S").to_string()),
                    )
                    .when_else(
                        self.selection.is_some(),
                        |row| {
                            row.child(div().text_color(rgb(0x87baff)).child(if selected {
                                "☑"
                            } else {
                                "☐"
                            }))
                        },
                        |details| {
                            details.child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        Self::icon_button(pin_label, pin_icon)
                                            .size(px(28.))
                                            .when(entry.pinned, |button| {
                                                button
                                                    .bg(rgb(0x31564b))
                                                    .hover(|style| style.bg(rgb(0x406a5e)))
                                            })
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                cx.stop_propagation();
                                                if let Some(entry) = view
                                                    .monitor
                                                    .history
                                                    .iter_mut()
                                                    .find(|entry| entry.id == id)
                                                {
                                                    entry.pinned = !entry.pinned;
                                                    view.save_history_change(cx);
                                                }
                                            })),
                                    )
                                    .child(
                                        Self::icon_button("删除", "\u{e74d}")
                                            .size(px(28.))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                cx.stop_propagation();
                                                view.monitor.history.retain(|entry| entry.id != id);
                                                view.save_history_change(cx);
                                            })),
                                    ),
                            )
                        },
                    ),
            )
    }
}

impl Render for ClipboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text.trim().to_lowercase();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x1b1e25))
            .text_color(rgb(0xe8ebf2))
            .text_sm()
            .when_else(
                self.settings_open,
                |view| view.child(self.settings(window, cx)),
                |view| {
                    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
                    for entry in self.monitor.history.iter().rev().filter(|entry| {
                        query.is_empty()
                            || entry.content.to_lowercase().contains(&query)
                            || (entry.image.is_some()
                                && ("图片".contains(&query) || "image".contains(&query)))
                    }) {
                        groups
                            .entry(entry.timestamp.date_naive())
                            .or_default()
                            .push(entry);
                    }
                    let mut dates = Vec::with_capacity(groups.len());
                    let mut rows = Vec::new();
                    for (date, mut entries) in groups.into_iter().rev() {
                        let index = rows.len();
                        dates.push((date, index));
                        rows.push(
                            Self::history_date(date, index, cx.weak_entity()).into_any_element(),
                        );
                        entries.sort_by_key(|entry| !entry.pinned);
                        rows.extend(
                            entries.into_iter().map(|entry| {
                                self.history_item(entry, &query, cx).into_any_element()
                            }),
                        );
                    }
                    let empty = rows.is_empty();
                    let date_menu = self
                        .date_menu
                        .map(|anchor| Self::history_date_menu(anchor, &dates, cx));
                    view.child(self.toolbar(cx))
                        .child(
                            div()
                                .relative()
                                .min_h_0()
                                .flex_1()
                                .overflow_hidden()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .id("clipboard-history")
                                        .min_h_0()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.history_scroll)
                                        .gap_2()
                                        .px_3()
                                        .pb_3()
                                        .when(empty, |list| {
                                            list.child(
                                                div()
                                                    .p_4()
                                                    .text_center()
                                                    .text_color(rgb(0xaab2c3))
                                                    .child(if query.is_empty() {
                                                        "暂无剪贴板记录"
                                                    } else {
                                                        "没有匹配的记录"
                                                    }),
                                            )
                                        })
                                        .children(rows),
                                )
                                .when(!empty, |list| {
                                    list.child(self.sticky_history_date(dates, cx))
                                })
                                .child(self.history_scrollbar()),
                        )
                        .children(date_menu)
                },
            )
            .when_some(self.status.as_ref(), |view, (message, is_error)| {
                view.child(
                    div()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .line_clamp(2)
                        .text_color(if *is_error {
                            rgb(0xffacaa)
                        } else {
                            rgb(0xa8d3b5)
                        })
                        .child(message.clone()),
                )
            })
    }
}
