use crate::desktop::window_hwnd;
use gpui::{prelude::*, *};
use std::borrow::Cow;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

const LINE_HEIGHT: Pixels = px(22.);

pub struct TextInput {
    pub text: String,
    label: &'static str,
    placeholder: &'static str,
    multiline: bool,
    clear_button: bool,
    focus: FocusHandle,
    anchor: usize,
    cursor: usize,
    selecting: bool,
    marked: Option<Range<usize>>,
    layout: Option<InputLayout>,
    scroll: Point<Pixels>,
    reveal_cursor: bool,
}

struct InputLayout {
    origin: Point<Pixels>,
    lines: Vec<(usize, ShapedLine)>,
}

impl InputLayout {
    fn row(&self, index: usize) -> usize {
        self.lines.partition_point(|(start, _)| *start <= index) - 1
    }

    fn index_at(&self, row: usize, x: Pixels) -> usize {
        let (start, line) = &self.lines[row];
        start + line.closest_index_for_x(x - self.origin.x)
    }

    fn position(&self, row: usize, index: usize) -> Point<Pixels> {
        let (start, line) = &self.lines[row];
        self.origin
            + point(
                line.x_for_index((index - start).min(line.len)),
                LINE_HEIGHT * row,
            )
    }
}

impl TextInput {
    pub fn search(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new("搜索剪贴板历史", "搜索剪贴板历史", String::new(), false, cx);
        input.clear_button = true;
        input
    }

    pub fn new(
        label: &'static str,
        placeholder: &'static str,
        text: String,
        multiline: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            text,
            label,
            placeholder,
            multiline,
            clear_button: false,
            focus: cx.focus_handle(),
            anchor: 0,
            cursor: 0,
            selecting: false,
            marked: None,
            layout: None,
            scroll: point(px(0.), px(0.)),
            reveal_cursor: true,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn index_for_point(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let row =
            (((position.y - layout.origin.y) / LINE_HEIGHT) as usize).min(layout.lines.len() - 1);
        Some(layout.index_at(row, position.x).min(self.text.len()))
    }

    fn utf16_offset(&self, byte: usize) -> usize {
        self.text[..byte].encode_utf16().count()
    }

    fn byte_offset(&self, offset: usize) -> usize {
        let mut units = 0;
        for (byte, character) in self.text.char_indices() {
            units += character.len_utf16();
            if units > offset {
                return byte;
            }
        }
        self.text.len()
    }

    fn edit_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .map(|range| self.byte_offset(range.start)..self.byte_offset(range.end))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection())
    }

    fn move_cursor(
        &mut self,
        cursor: usize,
        extend: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.cursor = cursor;
        self.reveal_cursor = true;
        if !extend {
            self.anchor = cursor;
        }
        window.notify_text_selection_changed(&self.focus);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        let key = &event.keystroke;
        let selection = self.selection();
        let previous = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(offset, _)| offset);
        let next = self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |character| self.cursor + character.len());
        match (key.modifiers.control, key.key.as_str()) {
            (true, "a") => {
                self.anchor = 0;
                self.move_cursor(self.text.len(), true, window, cx);
            }
            (true, "v") => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace_text_in_range(None, &text, window, cx);
                }
            }
            (true, "c" | "x") => {
                if !selection.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[selection].to_owned(),
                    ));
                    if key.key == "x" {
                        self.replace_text_in_range(None, "", window, cx);
                    }
                }
            }
            (true, "home" | "end") if self.multiline => {
                let cursor = if key.key == "home" {
                    0
                } else {
                    self.text.len()
                };
                self.move_cursor(cursor, key.modifiers.shift, window, cx);
            }
            (false, "enter") if self.multiline => {
                self.replace_text_in_range(None, "\n", window, cx);
            }
            (false, "left" | "right" | "home" | "end" | "up" | "down") => {
                let extend = key.modifiers.shift;
                let cursor = match key.key.as_str() {
                    "home" | "end" | "up" | "down" => {
                        let Some(layout) = &self.layout else { return };
                        let row = layout.row(self.cursor);
                        let (start, line) = &layout.lines[row];
                        match key.key.as_str() {
                            "home" => *start,
                            "end" => start + line.len,
                            "up" if row > 0 => {
                                layout.index_at(row - 1, layout.position(row, self.cursor).x)
                            }
                            "down" if row + 1 < layout.lines.len() => {
                                layout.index_at(row + 1, layout.position(row, self.cursor).x)
                            }
                            _ => return,
                        }
                    }
                    "left" if !extend && !selection.is_empty() => selection.start,
                    "right" if !extend && !selection.is_empty() => selection.end,
                    "left" => previous,
                    _ => next,
                };
                self.move_cursor(cursor, extend, window, cx);
            }
            (false, "backspace" | "delete") => {
                if selection.is_empty() {
                    self.anchor = if key.key == "backspace" {
                        previous
                    } else {
                        next
                    };
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        window.prevent_default();
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.byte_offset(range.start)..self.byte_offset(range.end);
        *adjusted = Some(self.utf16_offset(range.start)..self.utf16_offset(range.end));
        Some(self.text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.selection();
        Some(UTF16Selection {
            range: self.utf16_offset(range.start)..self.utf16_offset(range.end),
            reversed: self.cursor < self.anchor,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.anchor = self.byte_offset(range.start);
        self.move_cursor(self.byte_offset(range.end), true, window, cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.utf16_offset(self.text.len()))
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|range| self.utf16_offset(range.start)..self.utf16_offset(range.end))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.reveal_cursor = true;
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range);
        let text = if self.multiline {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(text.replace("\r\n", " ").replace(['\r', '\n', '\t'], " "))
        };
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.text.replace_range(range, &text);
        self.unmark_text(window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range);
        self.reveal_cursor = true;
        let start = range.start;
        let end = start + text.len();
        self.text.replace_range(range, text);
        self.marked = (!text.is_empty()).then_some(start..end);
        let offset = self.utf16_offset(start);
        let limit = text.encode_utf16().count();
        if let Some(selection) = selection {
            self.anchor = self.byte_offset(offset + selection.start.min(limit));
            self.cursor = self.byte_offset(offset + selection.end.min(limit));
        } else {
            self.anchor = end;
            self.cursor = end;
        }
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let start = self.byte_offset(range.start);
        let row = layout.row(start);
        let left = layout.position(row, start);
        let right = layout.position(row, self.byte_offset(range.end));
        Some(Bounds::from_corners(
            point(left.x.min(right.x), left.y),
            point(left.x.max(right.x), left.y + LINE_HEIGHT),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16_offset(self.index_for_point(point)?))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = cx.entity();
        div()
            .id(self.label)
            .role(Role::TextInput)
            .aria_label(self.label)
            .aria_value(self.text.clone())
            .track_focus(&self.focus)
            .tab_index(0)
            .cursor(CursorStyle::IBeam)
            .focus(|style| style.border_color(rgb(0x73a9fc)))
            .border_1()
            .border_color(rgb(0x393e4a))
            .rounded_md()
            .bg(rgb(0x17191f))
            .text_color(rgb(0xf1f3f7))
            .text_sm()
            .line_height(LINE_HEIGHT)
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .when(self.multiline, |input| input.flex_1().min_h_0())
            .on_key_down(cx.listener(Self::key_down))
            .when(self.multiline, |input| {
                input.on_scroll(cx.listener(|input, event: &ScrollEvent, _, cx| {
                    let mut delta = event.delta.pixel_delta(LINE_HEIGHT);
                    if event.modifiers.shift {
                        delta = point(delta.x + delta.y, px(0.));
                    }
                    input.scroll -= delta;
                    input.reveal_cursor = false;
                    cx.notify();
                }))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|input, event: &MouseDownEvent, window, cx| {
                    if let Ok(hwnd) = window_hwnd(window) {
                        // Activate during the click while Windows permits the foreground change.
                        unsafe { SetForegroundWindow(hwnd) };
                    }
                    input.focus.focus(window, cx);
                    input.selecting = true;
                    let cursor = input
                        .index_for_point(event.position)
                        .unwrap_or(input.text.len());
                    input.move_cursor(cursor, event.modifiers.shift, window, cx);
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| Self::paint_text(&input, bounds, window, cx),
                )
                .min_w_0()
                .flex_1()
                .when_else(
                    self.multiline,
                    |canvas| canvas.h_full(),
                    |canvas| canvas.h(LINE_HEIGHT),
                ),
            )
            .when(self.clear_button && !self.text.is_empty(), |input| {
                input.child(
                    div()
                        .id("clear-search")
                        .role(Role::Button)
                        .aria_label("清空搜索")
                        .tab_index(0)
                        .flex_shrink_0()
                        .w(px(22.))
                        .text_center()
                        .rounded_md()
                        .hover(|style| style.bg(rgb(0x30343e)))
                        .focus(|style| style.bg(rgb(0x30343e)))
                        .cursor_pointer()
                        .on_click(cx.listener(|input, _, window, cx| {
                            input.focus.focus(window, cx);
                            let end = input.utf16_offset(input.text.len());
                            input.replace_text_in_range(Some(0..end), "", window, cx);
                        }))
                        .child("×"),
                )
            })
    }
}

impl TextInput {
    fn paint_text(
        entity: &Entity<Self>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = entity.read(cx);
        let style = window.text_style();
        let empty = input.text.is_empty();
        let display = if empty {
            input.placeholder
        } else {
            &input.text
        };
        let mut start = 0;
        let lines = display
            .match_indices(['\r', '\n'])
            .filter(|(end, separator)| {
                *separator != "\r" || display.as_bytes().get(end + 1) != Some(&b'\n')
            })
            .chain(std::iter::once((display.len(), "")))
            .map(|(end, separator)| {
                let offset = start;
                start = end + separator.len();
                let text = display[offset..end]
                    .strip_suffix('\r')
                    .unwrap_or(&display[offset..end]);
                let mut run = style.to_run(text.len());
                if empty {
                    run.color = rgb(0xa2aaba).into();
                }
                let mut runs = Vec::new();
                if let Some(marked) = &input.marked {
                    let from = marked.start.saturating_sub(offset).min(text.len());
                    let to = marked.end.saturating_sub(offset).min(text.len());
                    if from < to {
                        runs.push(TextRun {
                            len: from,
                            ..run.clone()
                        });
                        runs.push(TextRun {
                            len: to - from,
                            underline: Some(UnderlineStyle {
                                color: Some(run.color),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            ..run.clone()
                        });
                        run.len -= to;
                    }
                }
                runs.push(run);
                runs.retain(|run| run.len > 0);
                let line = window.text_system().shape_line(
                    text.replace('\t', " ").into(),
                    style.font_size.to_pixels(window.rem_size()),
                    &runs,
                    None,
                );
                (offset, line)
            })
            .collect();
        let mut layout = InputLayout {
            origin: bounds.origin,
            lines,
        };
        let focus = input.focus.clone();
        let focused = focus.is_focused(window);
        let selection = input.selection();
        let row = layout.row(input.cursor);
        let cursor = layout.position(row, input.cursor) - bounds.origin;
        let mut scroll = input.scroll;
        if input.reveal_cursor {
            scroll.x = scroll
                .x
                .min(cursor.x)
                .max(cursor.x + px(2.) - bounds.size.width);
            scroll.y = scroll
                .y
                .min(cursor.y)
                .max(cursor.y + LINE_HEIGHT - bounds.size.height);
        }
        let width = layout
            .lines
            .iter()
            .map(|(_, line)| line.width)
            .max()
            .unwrap();
        scroll.x = if empty {
            px(0.)
        } else {
            scroll
                .x
                .clamp(px(0.), (width + px(2.) - bounds.size.width).max(px(0.)))
        };
        scroll.y = scroll.y.clamp(
            px(0.),
            (LINE_HEIGHT * layout.lines.len() - bounds.size.height).max(px(0.)),
        );
        layout.origin = bounds.origin - scroll;
        window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (row, (start, line)) in layout.lines.iter().enumerate() {
                let origin = layout.position(row, *start);
                if origin.y + LINE_HEIGHT <= bounds.top() {
                    continue;
                }
                if origin.y >= bounds.bottom() {
                    break;
                }
                let end = *start + line.len;
                if focused
                    && !selection.is_empty()
                    && selection.end > *start
                    && selection.start <= end
                {
                    let left = layout.position(row, selection.start.max(*start)).x;
                    let right = layout.position(row, selection.end.min(end)).x
                        + if selection.end > end { px(4.) } else { px(0.) };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(left.min(right), origin.y),
                            size((right - left).abs(), LINE_HEIGHT),
                        ),
                        rgb(0x325b96),
                    ));
                }
                let _ = line.paint(origin, LINE_HEIGHT, TextAlign::Left, None, window, cx);
            }
            if focused && selection.is_empty() {
                let position = layout.origin + cursor + point(px(0.), px(2.));
                window.paint_quad(fill(
                    Bounds::new(position, size(px(1.), LINE_HEIGHT - px(4.))),
                    rgb(0xcce0ff),
                ));
            }
        });
        entity.update(cx, |input, _| {
            input.layout = Some(layout);
            input.scroll = scroll;
            input.reveal_cursor = false;
        });

        let drag_selection =
            window.listener_for(entity, |input, event: &MouseMoveEvent, window, cx| {
                if !input.selecting || !event.dragging() {
                    return;
                }
                if let Some(cursor) = input
                    .index_for_point(event.position)
                    .filter(|&cursor| cursor != input.cursor)
                {
                    input.move_cursor(cursor, true, window, cx);
                }
            });
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase.capture() {
                drag_selection(event, window, cx);
            }
        });

        let end_selection = window.handler_for(entity, |input, _, _| input.selecting = false);
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase.capture() && event.button == MouseButton::Left {
                end_selection(window, cx);
            }
        });
    }
}
