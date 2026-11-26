use gpui::{prelude::*, *};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const LABEL: &str = "搜索剪贴板历史";

pub struct SearchInput {
    pub text: String,
    focus: FocusHandle,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    layout: Option<(Bounds<Pixels>, ShapedLine)>,
}

impl SearchInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            focus: cx.focus_handle(),
            anchor: 0,
            cursor: 0,
            marked: None,
            layout: None,
        }
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
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
            (false, "left" | "right" | "home" | "end") => {
                let extend = key.modifiers.shift;
                let cursor = match key.key.as_str() {
                    "home" => 0,
                    "end" => self.text.len(),
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

impl EntityInputHandler for SearchInput {
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
        let text = text.replace("\r\n", " ").replace(['\r', '\n', '\t'], " ");
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
        let (bounds, line) = self.layout.as_ref()?;
        let left = bounds.left() + line.x_for_index(self.byte_offset(range.start));
        let right = bounds.left() + line.x_for_index(self.byte_offset(range.end));
        Some(Bounds::from_corners(
            point(left, bounds.top()),
            point(right, bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let (bounds, line) = self.layout.as_ref()?;
        Some(
            self.utf16_offset(
                line.closest_index_for_x(point.x - bounds.left())
                    .min(self.text.len()),
            ),
        )
    }
}

impl Render for SearchInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("search")
            .role(Role::TextInput)
            .aria_label(LABEL)
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
            .line_height(px(22.))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|input, event: &MouseDownEvent, window, cx| {
                    input.focus.focus(window, cx);
                    let cursor =
                        input
                            .layout
                            .as_ref()
                            .map_or(input.text.len(), |(bounds, line)| {
                                line.closest_index_for_x(event.position.x - bounds.left())
                                    .min(input.text.len())
                            });
                    input.move_cursor(cursor, event.modifiers.shift, window, cx);
                }),
            )
            .child(div().min_w_0().flex_1().child(SearchText(cx.entity())))
            .when(!self.text.is_empty(), |input| {
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

struct SearchText(Entity<SearchInput>);

impl Element for SearchText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = self.0.read(cx);
        let style = window.text_style();
        let empty = input.text.is_empty();
        let text: SharedString = if empty {
            LABEL.into()
        } else {
            input.text.clone().into()
        };
        let mut run = TextRun {
            len: text.len(),
            font: style.font(),
            color: if empty {
                rgb(0xa2aaba).into()
            } else {
                style.color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut runs = Vec::new();
        if let Some(marked) = &input.marked {
            runs.push(TextRun {
                len: marked.start,
                ..run.clone()
            });
            runs.push(TextRun {
                len: marked.len(),
                underline: Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.),
                    wavy: false,
                }),
                ..run.clone()
            });
            run.len -= marked.end;
        }
        runs.push(run);
        runs.retain(|run| run.len > 0);
        let line = window.text_system().shape_line(
            text,
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let focus = input.focus.clone();
        let selection = input.selection();
        let cursor = line.x_for_index(input.cursor);
        let offset = (cursor + px(2.) - bounds.size.width).max(px(0.));
        let mut line_bounds = bounds;
        line_bounds.origin.x -= offset;
        if empty {
            line_bounds.origin.x = bounds.left();
        }
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let height = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focus.is_focused(window) && !selection.is_empty() {
                let start = line.x_for_index(selection.start);
                let end = line.x_for_index(selection.end);
                let position = point(line_bounds.left() + start.min(end), bounds.top());
                window.paint_quad(fill(
                    Bounds::new(position, size((end - start).abs(), height)),
                    rgb(0x325b96),
                ));
            }
            let _ = line.paint(
                line_bounds.origin,
                height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            if focus.is_focused(window) && selection.is_empty() {
                let position = point(line_bounds.left() + cursor, bounds.top() + px(2.));
                window.paint_quad(fill(
                    Bounds::new(position, size(px(1.), height - px(4.))),
                    rgb(0xcce0ff),
                ));
            }
        });
        self.0
            .update(cx, |input, _| input.layout = Some((line_bounds, line)));
    }
}
