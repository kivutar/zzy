use super::{BufferSnapshot, TerminalEventProxy, TerminalView};
use crate::colors::palette_for;
use crate::colors::Palette;
use crate::theme::ThemeChoice;
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::cell::Flags,
    term::Term,
    vte::ansi,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, px, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme};

pub const DEFAULT_COLS: usize = 80;
pub const DEFAULT_ROWS: usize = 24;
pub const DEFAULT_CELL_WIDTH: f32 = 8.0;
pub const DEFAULT_CELL_HEIGHT: f32 = 24.0;
pub const DEFAULT_FONT_SIZE: f32 = 15.0;

impl TerminalView {
    pub(crate) fn buffer_segments(&self) -> BufferSnapshot {
        let (start_line, end_line) = self.visible_line_range().unwrap_or_else(|| {
            let total = self.term.grid().total_lines();
            (0, total)
        });
        Self::buffer_segments_for_term(&self.term, self.theme, start_line, end_line)
    }

    fn visible_line_range(&self) -> Option<(usize, usize)> {
        let bounds = self.last_bounds?;
        let grid_ref = self.term.grid();
        let total_lines = grid_ref.total_lines();
        if total_lines == 0 {
            return Some((0, 0));
        }

        let cell_height = self.cell_height.max(1.0);
        let viewport_height: f32 = bounds.size.height.into();
        if viewport_height <= f32::EPSILON {
            return None;
        }

        let max_visible = (viewport_height / cell_height).ceil() as usize;
        let overscan = 8usize;
        let offset = self.scroll_handle.offset();
        let max_offset = self.scroll_handle.max_offset();
        let mut offset_y: f32 = (-offset.y).into();
        offset_y = offset_y.max(0.0);
        let max_scroll_y: f32 = max_offset.height.into();
        if max_scroll_y > f32::EPSILON {
            offset_y = offset_y.min(max_scroll_y);
        }

        let mut start_line = (offset_y / cell_height).floor() as usize;
        if start_line > total_lines {
            start_line = total_lines;
        }
        start_line = start_line.saturating_sub(overscan);

        let mut end_line = start_line + max_visible + overscan * 2;
        end_line = end_line.min(total_lines);

        Some((start_line, end_line))
    }

    fn buffer_segments_for_term(
        term: &Term<TerminalEventProxy>,
        theme: ThemeChoice,
        start_line: usize,
        end_line: usize,
    ) -> BufferSnapshot {
        let content = term.renderable_content();
        let palette = Palette::new(content.colors, palette_for(theme));
        let cursor = content.cursor;
        let grid_ref = term.grid();
        let total_lines = grid_ref.total_lines();
        let clamped_end = end_line.min(total_lines);
        let clamped_start = start_line.min(clamped_end);
        let visible_lines = clamped_end.saturating_sub(clamped_start);
        let screen_lines = grid_ref.screen_lines();
        let history_size = total_lines.saturating_sub(screen_lines);
        let cols = grid_ref.columns();
        let mut cursor_position = None;

        let mut grid = vec![
            vec![
                super::CellRender {
                    text: " ".into(),
                    fg: palette.default_fg,
                    bg: palette.default_bg,
                    is_cursor: false,
                    is_bold: false,
                };
                cols
            ];
            visible_lines
        ];

        for line_idx in clamped_start..clamped_end {
            let snapshot_idx = line_idx - clamped_start;
            let grid_line = Line(line_idx as i32 - history_size as i32);
            for col_idx in 0..cols {
                let cell = &grid_ref[grid_line][Column(col_idx)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }

                let mut ch = String::new();
                ch.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    for c in extra {
                        ch.push(*c);
                    }
                }

                let (fg, bg) = palette.fg_bg(cell);
                let is_cursor = cursor.point.line == grid_line
                    && cursor.point.column == Column(col_idx)
                    && cursor.shape != ansi::CursorShape::Hidden;
                let is_bold = cell.flags.contains(Flags::BOLD);
                grid[snapshot_idx][col_idx] = super::CellRender {
                    text: ch,
                    fg,
                    bg,
                    is_cursor,
                    is_bold,
                };
                if is_cursor {
                    cursor_position = Some(super::CellPosition {
                        line: line_idx,
                        column: col_idx,
                    });
                }
            }
        }

        let segments = grid
            .into_iter()
            .map(|row| {
                let mut segments = Vec::new();
                let mut current: Option<super::CellRender> = None;

                for cell in row {
                    if let Some(seg) = &mut current {
                        if seg.fg == cell.fg
                            && seg.bg == cell.bg
                            && seg.is_cursor == cell.is_cursor
                            && seg.is_bold == cell.is_bold
                        {
                            seg.text.push_str(&cell.text);
                        } else {
                            segments.push(seg.clone());
                            current = Some(cell);
                        }
                    } else {
                        current = Some(cell);
                    }
                }

                if let Some(seg) = current {
                    segments.push(seg);
                }

                segments
            })
            .collect();

        BufferSnapshot {
            segments_by_line: segments,
            palette,
            columns: cols,
            total_lines,
            start_line: clamped_start,
            cursor_shape: cursor.shape,
            cursor_position,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.update_cell_metrics(window, cx);

        let error = self.error.clone();
        let buffer = self.buffer_segments();
        self.cursor_shape = buffer.cursor_shape;
        let palette = buffer.palette;
        let cursor_shape = buffer.cursor_shape;
        let cursor_position = buffer.cursor_position;
        let selection = self.selection_range();
        let total_height = buffer.total_lines as f32 * self.cell_height;
        let top_padding = buffer.start_line as f32 * self.cell_height;
        let visible_height = buffer.segments_by_line.len() as f32 * self.cell_height;
        let bottom_padding = (total_height - top_padding - visible_height).max(0.0);
        let scrollbar = {
            let max_offset = self.scroll_handle.max_offset();
            self.last_bounds.and_then(|bounds| {
                let viewport_height: f32 = bounds.size.height.into();
                let scroll_max: f32 = max_offset.height.into();
                match self.scrollbar {
                    crate::settings::ScrollbarVisibility::Never => None,
                    crate::settings::ScrollbarVisibility::Auto
                        if scroll_max <= 0.5 || viewport_height <= 0.5 =>
                    {
                        None
                    }
                    _ => {
                        let effective_scroll_max = scroll_max.max(0.0);
                        let content_height = viewport_height + effective_scroll_max;
                        let thumb_height = (viewport_height / content_height * viewport_height)
                            .clamp(20.0, viewport_height);
                        let track_height = (viewport_height - thumb_height).max(0.0);
                        let offset_y: f32 = (-self.scroll_handle.offset().y).into();
                        let progress = if effective_scroll_max.abs() < f32::EPSILON {
                            0.0
                        } else {
                            (offset_y / effective_scroll_max).clamp(0.0, 1.0)
                        };
                        let top = progress * track_height;

                        Some(
                            div()
                                .absolute()
                                .right(px(-4.0))
                                .top(px(top))
                                .w(px(8.0))
                                .h(px(thumb_height))
                                .rounded(px(4.0))
                                .bg(cx.theme().border.alpha(0.65)),
                        )
                    }
                }
            })
        };

        let view = cx.entity();
        let mut container = div()
            .id("terminal-view")
            .bg(cx.theme().secondary)
            .flex_1()
            .w_full()
            .h_full()
            .relative()
            .track_focus(&self.focus_handle)
            .focusable()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .child({
                let scroll_area = div()
                    .id("terminal-scroll")
                    .w_full()
                    .h_full()
                    .font_family(self.font_family.clone())
                    .text_size(px(self.font_size))
                    .line_height(px(self.cell_height))
                    .whitespace_nowrap()
                    .cursor_text()
                    .overflow_y_scroll()
                    .scrollbar_width(px(8.0))
                    .track_scroll(&self.scroll_handle)
                    .child(
                        canvas(
                            {
                                let view = view.clone();
                                move |bounds, _, cx| {
                                    let _ =
                                        view.update(cx, |view, cx| view.record_bounds(bounds, cx));
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    );

                let mut lines_view = v_flex().id("terminal-lines").w_full();
                if let Some(err) = error {
                    lines_view = lines_view.child(
                        div()
                            .w_full()
                            .whitespace_nowrap()
                            .text_size(px(self.font_size))
                            .line_height(px(self.cell_height))
                            .font_family(self.font_family.clone())
                            .child(err),
                    );
                }
                if top_padding > 0.0 {
                    lines_view = lines_view.child(div().w_full().h(px(top_padding)));
                }
                for (idx, segments) in buffer.segments_by_line.into_iter().enumerate() {
                    let real_line = buffer.start_line + idx;
                    let line_highlight = selection.as_ref().and_then(|range| {
                        self.selection_span_for_line(range, real_line, buffer.columns)
                    });
                    let mut line_content = h_flex()
                        .w_full()
                        .whitespace_nowrap()
                        .text_size(px(self.font_size))
                        .line_height(px(self.cell_height))
                        .font_family(self.font_family.clone());

                    for seg in segments {
                        let show_cursor = seg.is_cursor && self.cursor_visible;
                        let is_block_cursor = matches!(
                            cursor_shape,
                            ansi::CursorShape::Block | ansi::CursorShape::HollowBlock
                        );
                        let text_color = if show_cursor && is_block_cursor {
                            palette.cursor_color()
                        } else {
                            seg.fg
                        };
                        let bg_color = if show_cursor && is_block_cursor {
                            palette.cursor_bg()
                        } else {
                            seg.bg
                        };
                        line_content = line_content.child(
                            div()
                                .whitespace_nowrap()
                                .when(seg.is_bold && !self.ignore_bold, |this| {
                                    this.font_weight(FontWeight::BOLD)
                                })
                                .text_color(text_color)
                                .bg(bg_color)
                                .child(seg.text),
                        );
                    }

                    let mut line_row = div()
                        .id(("line", real_line as u32))
                        .relative()
                        .w_full()
                        .whitespace_nowrap()
                        .text_size(px(self.font_size))
                        .line_height(px(self.cell_height))
                        .font_family(cx.theme().mono_font_family.clone());

                    if let Some((left, width)) = line_highlight {
                        line_row = line_row.child(
                            div()
                                .absolute()
                                .left(px(left))
                                .top(px(0.0))
                                .h(px(self.cell_height))
                                .w(px(width))
                                .bg(cx.theme().selection),
                        );
                    }

                    if let Some(pos) = cursor_position.filter(|pos| pos.line == real_line) {
                        if self.cursor_visible {
                            match cursor_shape {
                                ansi::CursorShape::Beam => {
                                    let width = (self.cell_width * 0.15).max(2.0);
                                    let left = pos.column as f32 * self.cell_width;
                                    line_row = line_row.child(
                                        div()
                                            .absolute()
                                            .left(px(left))
                                            .top(px(0.0))
                                            .w(px(width))
                                            .h(px(self.cell_height))
                                            .bg(palette.cursor_bg()),
                                    );
                                }
                                ansi::CursorShape::Underline => {
                                    let height = (self.cell_height * 0.15).clamp(1.0, 3.0);
                                    let left = pos.column as f32 * self.cell_width;
                                    let top = self.cell_height - height;
                                    line_row = line_row.child(
                                        div()
                                            .absolute()
                                            .left(px(left))
                                            .top(px(top))
                                            .w(px(self.cell_width))
                                            .h(px(height))
                                            .bg(palette.cursor_bg()),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }

                    lines_view = lines_view.child(line_row.child(line_content));
                }
                if bottom_padding > 0.0 {
                    lines_view = lines_view.child(div().w_full().h(px(bottom_padding)));
                }

                scroll_area.child(lines_view)
            });

        if let Some(scrollbar) = scrollbar {
            container = container.child(scrollbar);
        }

        container
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SimpleDimensions, TerminalEventProxy};
    use super::*;
    use alacritty_terminal::{term::Config, vte::ansi};
    use futures::channel::mpsc;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn profile_buffer_segments_large_history() {
        let lines = 10_000;
        let cols = 120;

        let mut config = Config::default();
        config.scrolling_history = lines;

        let (tx, _rx) = mpsc::unbounded();
        let proxy = TerminalEventProxy { tx };
        let mut term = Term::new(config, &SimpleDimensions::new(cols, DEFAULT_ROWS), proxy);

        let mut parser: ansi::Processor = ansi::Processor::new();
        let line_content = format!("{}\n", "a".repeat(cols.saturating_sub(1)));

        let feed_started = Instant::now();
        for _ in 0..lines {
            parser.advance(&mut term, line_content.as_bytes());
        }
        let feed_elapsed = feed_started.elapsed();

        let snapshot_started = Instant::now();
        let snapshot =
            TerminalView::buffer_segments_for_term(&term, ThemeChoice::HamiMelonDark, 0, lines);
        let snapshot_elapsed = snapshot_started.elapsed();

        let window_lines = 200;
        let range_start = lines.saturating_sub(window_lines);
        let range_snapshot_started = Instant::now();
        let range_snapshot = TerminalView::buffer_segments_for_term(
            &term,
            ThemeChoice::HamiMelonDark,
            range_start,
            lines,
        );
        let range_snapshot_elapsed = range_snapshot_started.elapsed();

        let segment_count: usize = snapshot.segments_by_line.iter().map(|row| row.len()).sum();
        let range_segment_count: usize = range_snapshot
            .segments_by_line
            .iter()
            .map(|row| row.len())
            .sum();

        println!(
            "loaded {} lines x {} cols -> feed {:?}, snapshot {:?}, segments {} | window snapshot {:?}, segments {} (start {})",
            lines,
            cols,
            feed_elapsed,
            snapshot_elapsed,
            segment_count,
            range_snapshot_elapsed,
            range_segment_count,
            range_start
        );
    }
}
