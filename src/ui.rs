use crate::clipboard::{ClipboardEntry, ClipboardMonitor};
use crate::search::SearchInput;
use crate::storage::Storage;
use gpui::{prelude::*, *};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

pub struct ClipboardView {
    monitor: ClipboardMonitor,
    storage: Option<Storage>,
    search: Entity<SearchInput>,
    selection: Option<HashSet<usize>>,
    status: Option<(String, bool)>,
    export_status_timeout: Option<Task<()>>,
    dirty: bool,
    _search_subscription: Subscription,
}

impl ClipboardView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (storage, history, status) = match Storage::open() {
            Ok((storage, history)) => (Some(storage), history, None),
            Err(error) => (
                None,
                Vec::new(),
                Some((format!("无法打开历史记录：{error}"), true)),
            ),
        };
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
            dirty: false,
            _search_subscription: search_subscription,
        }
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

    fn button(label: &'static str) -> Stateful<Div> {
        div()
            .id(label)
            .role(Role::Button)
            .aria_label(label)
            .flex_shrink_0()
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(rgb(0x30343e))
            .hover(|style| style.bg(rgb(0x404655)))
            .cursor_pointer()
            .child(label)
    }

    fn toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(Self::button("导入").on_click(cx.listener(|view, _, _, cx| view.import(cx))))
            .child(Self::button("导出").on_click(cx.listener(|view, _, _, cx| view.export(cx))))
            .child(
                Self::button(if self.selection.is_some() {
                    "取消选择"
                } else {
                    "批量删除"
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    view.selection = if view.selection.is_some() {
                        None
                    } else {
                        Some(HashSet::new())
                    };
                    cx.notify();
                })),
            )
            .when(
                self.selection
                    .as_ref()
                    .is_some_and(|selection| !selection.is_empty()),
                |toolbar| {
                    toolbar.child(
                        Self::button("删除选中")
                            .on_click(cx.listener(|view, _, _, cx| view.delete_selected(cx))),
                    )
                },
            )
            .child(
                Self::button("清空全部")
                    .on_click(cx.listener(|view, _, window, cx| view.clear_history(window, cx))),
            )
    }

    fn history_item(&self, entry: &ClipboardEntry, cx: &Context<Self>) -> impl IntoElement {
        let id = entry.id;
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
            .flex_col()
            .gap_2()
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
                } else if let Err(error) = view.monitor.copy_entry(id, window, cx) {
                    view.status = Some((format!("复制失败：{error}"), true));
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
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
                    .when(self.selection.is_none(), |row| {
                        row.child(Self::button("删除").text_xs().on_click(cx.listener(
                            move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.monitor.history.retain(|entry| entry.id != id);
                                view.dirty = true;
                                view.persist(cx);
                            },
                        )))
                    }),
            )
            .when_some(
                entry.image.as_ref().and_then(|image| image.load()),
                |card, image| {
                    card.child(
                        img(image)
                            .w_full()
                            .h(px(124.))
                            .object_fit(ObjectFit::Contain),
                    )
                },
            )
            .when(!preview.is_empty(), |card| {
                card.child(div().w_full().min_w_0().line_clamp(3).child(preview))
            })
    }
}

impl Render for ClipboardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text.trim().to_lowercase();
        let mut entries = self
            .monitor
            .history
            .iter()
            .rev()
            .filter(|entry| {
                query.is_empty()
                    || entry.content.to_lowercase().contains(&query)
                    || (entry.image.is_some()
                        && ("图片".contains(&query) || "image".contains(&query)))
            })
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
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(self.toolbar(cx))
                    .child(self.search.clone()),
            )
            .child(
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
