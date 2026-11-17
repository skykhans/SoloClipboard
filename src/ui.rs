use crate::clipboard::{ClipboardEntry, ClipboardMonitor};
use crate::storage::Storage;
use gpui::*;
use std::sync::Arc;
use parking_lot::RwLock;

pub struct AppState {
    pub monitor: ClipboardMonitor,
    pub storage: Storage,
    pub search_query: Arc<RwLock<String>>,
    pub selected_ids: Arc<RwLock<Vec<usize>>>,
    pub selection_mode: Arc<RwLock<bool>>,
}

impl AppState {
    pub fn new() -> Self {
        let monitor = ClipboardMonitor::new();
        let storage = Storage::new();

        // 加载历史记录
        if let Ok(history) = storage.load_history() {
            // 这里可以将历史记录加载到 monitor 中
        }

        // 启动剪贴板监控
        monitor.start_monitoring();

        Self {
            monitor,
            storage,
            search_query: Arc::new(RwLock::new(String::new())),
            selected_ids: Arc::new(RwLock::new(Vec::new())),
            selection_mode: Arc::new(RwLock::new(false)),
        }
    }
}

pub struct ClipboardView {
    app_state: Arc<AppState>,
    focus_handle: FocusHandle,
}

impl ClipboardView {
    pub fn new(app_state: Arc<AppState>, cx: &mut ViewContext<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        Self {
            app_state,
            focus_handle,
        }
    }

    fn render_header(&self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let selection_mode = *self.app_state.selection_mode.read();
        let selected_count = self.app_state.selected_ids.read().len();

        div()
            .flex()
            .items_center()
            .justify_between()
            .p_4()
            .bg(rgb(0x2d2d2d))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_bold()
                            .text_color(rgb(0xffffff))
                            .child("Solo Clipboard")
                    )
                    .when(selection_mode, |div| {
                        div.child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x4a9eff))
                                .child(format!("已选择 {} 项", selected_count))
                        )
                    })
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.render_button("保存", cx, |view, cx| {
                        let history = view.app_state.monitor.get_history();
                        if let Err(e) = view.app_state.storage.save_history(&history) {
                            eprintln!("保存失败: {}", e);
                        }
                    }))
                    .child(self.render_button(
                        if selection_mode { "取消选择" } else { "批量删除" },
                        cx,
                        |view, cx| {
                            let mut mode = view.app_state.selection_mode.write();
                            *mode = !*mode;
                            if !*mode {
                                view.app_state.selected_ids.write().clear();
                            }
                            cx.notify();
                        }
                    ))
                    .when(selection_mode && selected_count > 0, |div| {
                        div.child(self.render_button("删除选中", cx, |view, cx| {
                            let ids = view.app_state.selected_ids.read().clone();
                            view.app_state.monitor.delete_entries(&ids);
                            view.app_state.selected_ids.write().clear();
                            *view.app_state.selection_mode.write() = false;
                            cx.notify();
                        }))
                    })
                    .child(self.render_button("清空全部", cx, |view, cx| {
                        view.app_state.monitor.clear_history();
                        view.app_state.selected_ids.write().clear();
                        *view.app_state.selection_mode.write() = false;
                        cx.notify();
                    }))
            )
    }

    fn render_button<F>(&self, label: &str, cx: &mut ViewContext<Self>, on_click: F) -> impl IntoElement
    where
        F: Fn(&mut Self, &mut ViewContext<Self>) + 'static,
    {
        let label = label.to_string();
        div()
            .px_4()
            .py_2()
            .bg(rgb(0x3a3a3a))
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x4a4a4a)))
            .text_color(rgb(0xffffff))
            .child(label)
    }

    fn render_search(&self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        div()
            .flex()
            .p_4()
            .bg(rgb(0x252525))
            .child(
                div()
                    .flex_1()
                    .px_3()
                    .py_2()
                    .bg(rgb(0x1e1e1e))
                    .rounded_md()
                    .text_color(rgb(0xcccccc))
                    .child("搜索剪贴板历史...")
            )
    }

    fn render_history_list(&self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let history = self.app_state.monitor.get_history();
        let mut reversed_history = history.clone();
        reversed_history.reverse();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .bg(rgb(0x1e1e1e))
            .children(reversed_history.iter().map(|entry| {
                self.render_history_item(entry, cx)
            }))
    }

    fn render_history_item(&self, entry: &ClipboardEntry, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let content = entry.content.clone();
        let id = entry.id;
        let timestamp = entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
        let selection_mode = *self.app_state.selection_mode.read();
        let is_selected = self.app_state.selected_ids.read().contains(&id);

        let preview = if content.len() > 100 {
            format!("{}...", &content[..100])
        } else {
            content.clone()
        };

        div()
            .flex()
            .p_3()
            .m_2()
            .bg(if is_selected { rgb(0x3a4a5a) } else { rgb(0x2d2d2d) })
            .rounded_md()
            .hover(|style| style.bg(if is_selected { rgb(0x4a5a6a) } else { rgb(0x3a3a3a) }))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .mb_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x888888))
                                    .child(timestamp)
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x4a9eff))
                                            .child(format!("#{}", id))
                                    )
                                    .when(!selection_mode, |div| {
                                        div.child(
                                            div()
                                                .text_xs()
                                                .px_2()
                                                .py_1()
                                                .bg(rgb(0xff4a4a))
                                                .rounded_sm()
                                                .text_color(rgb(0xffffff))
                                                .hover(|style| style.bg(rgb(0xff6a6a)))
                                                .child("删除")
                                        )
                                    })
                            )
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xcccccc))
                            .child(preview)
                    )
            )
            .when(selection_mode, |div| {
                div.child(
                    div()
                        .ml_3()
                        .w_5()
                        .h_5()
                        .border_1()
                        .border_color(rgb(0x4a9eff))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(is_selected, |div| {
                            div.bg(rgb(0x4a9eff))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0xffffff))
                                        .child("✓")
                                )
                        })
                )
            })
    }
}

impl Render for ClipboardView {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .child(self.render_header(cx))
            .child(self.render_search(cx))
            .child(self.render_history_list(cx))
    }
}