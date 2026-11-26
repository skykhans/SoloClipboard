use crate::clipboard::{ClipboardEntry, ClipboardMonitor};
use crate::desktop::{self, Settings, Shortcut, TrayWindow};
use crate::search::SearchInput;
use crate::storage::Storage;
use gpui::{prelude::*, *};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use windows_sys::Win32::Foundation::HWND;

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

pub struct ClipboardView {
    monitor: ClipboardMonitor,
    storage: Option<Storage>,
    search: Entity<SearchInput>,
    selection: Option<HashSet<usize>>,
    status: Option<(String, bool)>,
    export_status_timeout: Option<Task<()>>,
    tray: Option<TrayWindow>,
    settings: Settings,
    settings_open: bool,
    start_with_windows: Option<bool>,
    shortcut_focus: FocusHandle,
    dirty: bool,
    _search_subscription: Subscription,
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
        let search_subscription = cx.observe(&search, |_, _, cx| cx.notify());
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
            selection: None,
            status,
            export_status_timeout: None,
            tray,
            settings,
            settings_open: false,
            start_with_windows: None,
            shortcut_focus: cx.focus_handle(),
            dirty: false,
            _search_subscription: search_subscription,
        }
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

    fn import(&mut self, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入剪贴板历史".into()),
        });
        cx.spawn(async move |view, cx| {
            let result = match choice.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        cx.background_executor()
                            .spawn(async move {
                                Storage::read(&path).map_err(|error| error.to_string())
                            })
                            .await
                    } else {
                        return;
                    }
                }
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(history) => {
                        let count = view.monitor.import(history);
                        view.status = Some((format!("已导入 {count} 条新记录"), false));
                        view.dirty |= count > 0;
                        if view.dirty {
                            view.persist(cx);
                        }
                    }
                    Err(error) => view.status = Some((format!("导入失败：{error}"), true)),
                }
                cx.notify();
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
                view.status = Some(status.clone());
                view.export_status_timeout = (!status.1).then(|| {
                    cx.spawn(async move |view, cx| {
                        cx.background_executor().timer(Duration::from_secs(3)).await;
                        let _ = view.update(cx, |view, cx| {
                            if view.status.as_ref() == Some(&status) {
                                view.status = None;
                                cx.notify();
                            }
                        });
                    })
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = self.selection.take() {
            self.monitor
                .history
                .retain(|entry| !selection.contains(&entry.id));
            self.dirty = true;
            self.persist(cx);
        }
    }

    fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let response = window.prompt(
            PromptLevel::Warning,
            "清空剪贴板历史？",
            Some("本地保存的全部记录都会删除。"),
            &["取消", "清空"],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if response.await == Ok(1) {
                let _ = view.update(cx, |view, cx| {
                    view.monitor.history.clear();
                    view.selection = None;
                    view.dirty = true;
                    view.persist(cx);
                });
            }
        })
        .detach();
    }

    fn button_base(label: &'static str) -> Stateful<Div> {
        div()
            .id(label)
            .role(Role::Button)
            .aria_label(label)
            .flex_shrink_0()
            .rounded_md()
            .bg(rgb(0x30343e))
            .hover(|style| style.bg(rgb(0x404655)))
            .cursor_pointer()
    }

    fn button(label: &'static str) -> Stateful<Div> {
        Self::button_base(label).px_3().py_1p5().child(label)
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
                        cx.notify();
                    },
                )))
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
                    .child(Self::button("返回历史").on_click(cx.listener(
                        |view, _, window, cx| {
                            window.blur(cx);
                            view.settings_open = false;
                            cx.notify();
                        },
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(0x252831))
                    .p_3()
                    .child("开机启动")
                    .when_some(self.start_with_windows, |section, enabled| {
                        section.child(div().flex().gap_2().children(
                            [("关闭", false), ("开启", true)].map(|(label, selected)| {
                                Self::button(label)
                                    .when(enabled == selected, |button| {
                                        button
                                            .bg(rgb(0x31564b))
                                            .hover(|style| style.bg(rgb(0x31564b)))
                                    })
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
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(0x252831))
                    .p_3()
                    .child("关闭窗口时")
                    .child(div().flex().flex_wrap().gap_2().children(
                        [("退出程序", false), ("留在通知区域", true)].map(
                            |(label, close_to_tray)| {
                                Self::button(label)
                                    .when(self.settings.close_to_tray == close_to_tray, |button| {
                                        button
                                            .bg(rgb(0x31564b))
                                            .hover(|style| style.bg(rgb(0x31564b)))
                                    })
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
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(0x252831))
                    .p_3()
                    .child("打开剪贴板的全局快捷键")
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
                                        view.shortcut_focus.focus(window, cx);
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        |view, event: &KeyDownEvent, window, cx| {
                                            if event.keystroke.key == "escape" {
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
                                                return;
                                            }
                                            cx.stop_propagation();
                                            window.prevent_default();
                                            cx.notify();
                                        },
                                    )),
                            )
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

    fn history_item(&self, entry: &ClipboardEntry, cx: &Context<Self>) -> impl IntoElement {
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
        let preview = match entry.content.char_indices().nth(500) {
            Some((end, _)) => format!("{}…", &entry.content[..end]),
            None => entry.content.clone(),
        }
        .replace('\t', "    ");

        div()
            .id(("entry", id))
            .min_w_0()
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_start()
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
                    cx.notify();
                } else {
                    let result = view
                        .tray
                        .as_ref()
                        .ok_or_else(|| "无法恢复原来的输入窗口".to_string())
                        .and_then(|tray| tray.paste(|| view.monitor.copy_entry(id, window, cx)));
                    view.status = result
                        .err()
                        .map(|error| (format!("粘贴失败：{error}"), true));
                    cx.notify();
                }
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
                        content.child(div().line_clamp(3).child(preview))
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap_1()
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(rgb(0xaab2c3))
                            .child(entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
                    )
                    .when(self.selection.is_some(), |row| {
                        row.child(div().text_color(rgb(0x87baff)).child(if selected {
                            "☑"
                        } else {
                            "☐"
                        }))
                    })
                    .when(self.selection.is_none(), |details| {
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
                                                view.dirty = true;
                                                view.persist(cx);
                                            }
                                        })),
                                )
                                .child(
                                    Self::icon_button("删除", "\u{e74d}")
                                        .size(px(28.))
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            cx.stop_propagation();
                                            view.monitor.history.retain(|entry| entry.id != id);
                                            view.dirty = true;
                                            view.persist(cx);
                                        })),
                                ),
                        )
                    }),
            )
    }
}

impl Render for ClipboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text.trim().to_lowercase();
        let matching = self.monitor.history.iter().rev().filter(|entry| {
            query.is_empty()
                || entry.content.to_lowercase().contains(&query)
                || (entry.image.is_some() && ("图片".contains(&query) || "image".contains(&query)))
        });
        let mut entries = matching
            .clone()
            .filter(|entry| entry.pinned)
            .chain(matching.filter(|entry| !entry.pinned))
            .peekable();
        let empty = entries.peek().is_none();
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x1b1e25))
            .text_color(rgb(0xe8ebf2))
            .text_sm()
            .when(self.settings_open, |view| {
                view.child(self.settings(window, cx))
            })
            .when(!self.settings_open, |view| {
                view.child(self.toolbar(cx)).child(
                    div()
                        .id("clipboard-history")
                        .min_h_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .gap_2()
                        .px_3()
                        .pb_3()
                        .when(empty, |list| {
                            list.child(div().p_4().text_center().text_color(rgb(0xaab2c3)).child(
                                if query.is_empty() {
                                    "暂无剪贴板记录"
                                } else {
                                    "没有匹配的记录"
                                },
                            ))
                        })
                        .children(entries.map(|entry| self.history_item(entry, cx))),
                )
            })
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
