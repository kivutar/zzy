use super::{
    input::selection_columns, viewport_start, BufferSnapshot, TerminalEventProxy, TerminalView,
};
use crate::colors::{palette_for, Palette};
use crate::settings::ScrollbarVisibility;
use crate::theme::ThemeChoice;
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::cell::Flags,
    term::Term,
    vte::ansi,
};
use gpui_kit::component::{
    h_flex,
    kbd::Kbd,
    menu::{ContextMenuExt, PopupMenuItem},
    v_flex, ActiveTheme,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    canvas, div, px, Context, FontWeight, Hsla, InteractiveElement, IntoElement, Keystroke,
    MouseButton, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
};

pub const DEFAULT_COLS: usize = 80;
pub const DEFAULT_ROWS: usize = 24;
pub const DEFAULT_CELL_WIDTH: f32 = 8.0;
pub const DEFAULT_CELL_HEIGHT: f32 = 24.0;
pub const DEFAULT_FONT_SIZE: f32 = 15.0;

impl TerminalView {
    pub(super) fn buffer_segments_for_term(
        term: &Term<TerminalEventProxy>,
        theme: ThemeChoice,
        bold_is_bright: bool,
        min_contrast: f32,
        start_line: usize,
        end_line: usize,
    ) -> BufferSnapshot {
        let content = term.renderable_content();
        let palette =
            Palette::new(content.colors, palette_for(theme)).with_min_contrast(min_contrast);
        let cursor = content.cursor;
        let grid_ref = term.grid();
        let total_lines = grid_ref.total_lines();
        let clamped_end = end_line.min(total_lines);
        let clamped_start = start_line.min(clamped_end);
        let screen_lines = grid_ref.screen_lines();
        let history_size = total_lines.saturating_sub(screen_lines);
        let cols = grid_ref.columns();
        let mut cursor_position = None;

        let mut segments = Vec::with_capacity(clamped_end - clamped_start);
        for line_idx in clamped_start..clamped_end {
            let grid_line = Line(line_idx as i32 - history_size as i32);
            let mut runs: Vec<super::CellRender> = Vec::new();
            for col_idx in 0..cols {
                let cell = &grid_ref[grid_line][Column(col_idx)];
                // The right half of a wide character: covered by that character's run.
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let wide = cell.flags.contains(Flags::WIDE_CHAR);
                let (fg, bg) = palette.fg_bg(cell, bold_is_bright);
                let is_cursor = cursor.point.line == grid_line
                    && cursor.point.column == Column(col_idx)
                    && cursor.shape != ansi::CursorShape::Hidden;
                let is_bold = cell.flags.contains(Flags::BOLD);
                if is_cursor {
                    cursor_position = Some(super::CellPosition {
                        line: line_idx,
                        column: col_idx,
                    });
                }

                // Extend the current run when the style matches; wide characters
                // always get a run of their own, exactly two cells wide.
                let extends = runs.last().is_some_and(|run| {
                    !wide
                        && run.cells == run.text.chars().count()
                        && run.fg == fg
                        && run.bg == bg
                        && run.is_cursor == is_cursor
                        && run.is_bold == is_bold
                });
                if !extends {
                    runs.push(super::CellRender {
                        text: String::new(),
                        column: col_idx,
                        cells: 0,
                        fg,
                        bg,
                        is_cursor,
                        is_bold,
                    });
                }
                let run = runs.last_mut().expect("a run was just ensured");
                run.text.push(if cell.c == '\t' { ' ' } else { cell.c });
                if let Some(extra) = cell.zerowidth() {
                    run.text.extend(extra.iter());
                }
                run.cells += if wide { 2 } else { 1 };
            }
            segments.push(runs);
        }

        BufferSnapshot {
            segments_by_line: segments,
            palette,
            columns: cols,
            start_line: clamped_start,
            cursor_shape: cursor.shape,
            cursor_position,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_cell_metrics(window, cx);

        // One consistent snapshot under a single lock: the PTY thread may be
        // parsing more output concurrently.
        let (buffer, selection, history) = {
            let term = self.term.lock();
            // The visible screen, starting at alacritty's display offset.
            let start = viewport_start(&term);
            let buffer = Self::buffer_segments_for_term(
                &term,
                self.theme,
                self.settings.bold_text.bright_colors(),
                self.settings.minimum_contrast,
                start,
                start + term.grid().screen_lines(),
            );
            let selection = term.selection.as_ref().and_then(|s| s.to_range(&term));
            let grid = term.grid();
            (
                buffer,
                selection,
                grid.total_lines().saturating_sub(grid.screen_lines()),
            )
        };
        let palette = buffer.palette;
        let cursor_shape = buffer.cursor_shape;
        let block_cursor = matches!(
            cursor_shape,
            ansi::CursorShape::Block | ansi::CursorShape::HollowBlock
        );
        let cursor_position = buffer.cursor_position.filter(|_| self.cursor_visible);
        let cell_height = self.cell_height;
        let view = cx.entity();

        let mut lines_view = v_flex()
            .id("terminal-lines")
            .w_full()
            .when_some(self.error.clone(), |this, err| this.child(div().child(err)));

        for (idx, segments) in buffer.segments_by_line.into_iter().enumerate() {
            let line = buffer.start_line + idx;
            // Runs are absolutely positioned, so the row gets its height explicitly.
            let mut line_row = div()
                .id(("line", line as u32))
                .relative()
                .w_full()
                .h(px(cell_height));

            // Painted first so it sits behind the text. Cells using the default
            // background don't paint one (the container already does), so the
            // highlight shows through them.
            // Selections are in alacritty grid lines (negative = scrollback).
            let grid_line = Line(line as i32 - history as i32);
            if let Some((left, width)) = selection.as_ref().and_then(|range| {
                selection_columns(range, grid_line, buffer.columns).map(|(start, end)| {
                    (
                        start as f32 * self.cell_width,
                        (end - start + 1) as f32 * self.cell_width,
                    )
                })
            }) {
                line_row = line_row.child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top_0()
                        .h(px(cell_height))
                        .w(px(width))
                        .bg(cx.theme().selection),
                );
            }

            // Each run is placed at its column with its exact width, so the text
            // stays on the cell grid that selection, cursor and mouse use.
            line_row = line_row.children(segments.into_iter().map(|seg| {
                let (fg, bg) = if seg.is_cursor && block_cursor && self.cursor_visible {
                    (palette.cursor_color(), palette.cursor_bg())
                } else {
                    (seg.fg, seg.bg)
                };
                div()
                    .absolute()
                    .top_0()
                    .left(px(seg.column as f32 * self.cell_width))
                    .w(px(seg.cells as f32 * self.cell_width))
                    .h(px(cell_height))
                    .when(seg.is_bold && self.settings.bold_text.bold_font(), |this| {
                        this.font_weight(FontWeight::BOLD)
                    })
                    .text_color(fg)
                    .when(bg != palette.default_bg, |this| this.bg(bg))
                    .child(seg.text)
            }));

            // Thin cursors are drawn on top of the glyphs.
            if let Some(pos) = cursor_position.filter(|pos| pos.line == line) {
                let left = pos.column as f32 * self.cell_width;
                let bar = match cursor_shape {
                    ansi::CursorShape::Beam => {
                        Some((left, 0.0, (self.cell_width * 0.15).max(2.0), cell_height))
                    }
                    ansi::CursorShape::Underline => {
                        let height = (cell_height * 0.15).clamp(1.0, 3.0);
                        Some((left, cell_height - height, self.cell_width, height))
                    }
                    _ => None,
                };
                if let Some((left, top, w, h)) = bar {
                    line_row = line_row.child(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(px(w))
                            .h(px(h))
                            .bg(palette.cursor_bg()),
                    );
                }
            }

            lines_view = lines_view.child(line_row);
        }
        let content = div()
            .id("terminal-content")
            .size_full()
            .overflow_hidden()
            .font_family(self.settings.font_family.clone())
            .text_size(px(self.font_size))
            .line_height(px(cell_height))
            .whitespace_nowrap()
            .cursor_text()
            // Wheel: mouse reporting, alternate scroll, zoom, or scrollback.
            .on_scroll(cx.listener(Self::handle_scroll))
            // Inside the container so mouse reporting runs before selection
            // and the context menu, which it suppresses while active.
            .on_any_mouse_down(cx.listener(Self::report_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::report_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::report_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::report_mouse_up))
            .on_mouse_move(cx.listener(Self::report_mouse_move))
            .child(
                canvas(
                    {
                        let view = view.clone();
                        move |bounds, _, cx| {
                            view.update(cx, |view, cx| view.record_bounds(bounds, cx));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(lines_view);

        div()
            .id("terminal-view")
            .flex_1()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .focusable()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .child(content)
            .children(self.render_scrollbar(cx))
            .when(self.bell_flash_visible(), |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(Hsla::from(palette.default_fg).opacity(0.15)),
                )
            })
            .context_menu(move |menu, _window, cx| {
                let (focus_handle, has_selection) = {
                    let view = view.read(cx);
                    (view.focus_handle.clone(), view.has_selection())
                };
                let can_paste = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .is_some();

                type MenuAction = fn(&mut TerminalView, &mut Context<TerminalView>);
                let items: [(&'static str, &'static str, bool, MenuAction); 4] = [
                    ("menu.copy", "ctrl-shift-c", has_selection, |view, cx| {
                        view.copy_selection_to_clipboard(cx);
                    }),
                    ("menu.paste", "ctrl-shift-v", can_paste, |view, cx| {
                        view.paste_from_clipboard(cx);
                    }),
                    ("menu.select_all", "ctrl-shift-a", true, |view, cx| {
                        if view.select_all() {
                            cx.notify();
                        }
                    }),
                    (
                        "menu.select_none",
                        "ctrl-shift-d",
                        has_selection,
                        |view, cx| {
                            if view.clear_selection() {
                                cx.notify();
                            }
                        },
                    ),
                ];

                items.into_iter().fold(
                    menu.action_context(focus_handle),
                    |menu, (label, keys, enabled, action)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::element(move |_, _| {
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .child(crate::i18n::tr(label))
                                    .children(Keystroke::parse(keys).ok().map(Kbd::new))
                            })
                            .on_click(move |_, _, cx| view.update(cx, action))
                            .disabled(!enabled),
                        )
                    },
                )
            })
    }
}

impl TerminalView {
    /// A thumb showing where the viewport is in the scrollback (display only).
    fn render_scrollbar(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let viewport_height: f32 = self.last_bounds?.size.height.into();
        let term = self.term.lock();
        let grid = term.grid();
        let screen = grid.screen_lines() as f32;
        let history = grid.total_lines().saturating_sub(grid.screen_lines()) as f32;
        match self.settings.scrollbar {
            ScrollbarVisibility::Never => return None,
            ScrollbarVisibility::Auto if history == 0.0 || viewport_height <= 0.5 => return None,
            _ => {}
        }

        let thumb_height =
            (screen / (screen + history) * viewport_height).clamp(20.0, viewport_height);
        let track_height = (viewport_height - thumb_height).max(0.0);
        // 1.0 at the bottom (display offset 0), 0.0 at the top of the history.
        let progress = if history == 0.0 {
            1.0
        } else {
            1.0 - grid.display_offset() as f32 / history
        };

        Some(
            div()
                .absolute()
                .right(px(-self.settings.padding.min(4.0)))
                .top(px(progress * track_height))
                .w(px(8.0))
                .h(px(thumb_height))
                .rounded(px(4.0))
                .bg(cx.theme().border.alpha(0.65)),
        )
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
    fn renders_tabs_as_spaces() {
        let config = Config {
            scrolling_history: 10,
            ..Config::default()
        };

        let (tx, _rx) = mpsc::unbounded();
        let proxy = TerminalEventProxy { tx };
        let mut term = Term::new(config, &SimpleDimensions::new(16, DEFAULT_ROWS), proxy);
        let mut parser: ansi::Processor = ansi::Processor::new();
        parser.advance(&mut term, b"\t\x1b[31mred\x1b[0m\n");

        let snapshot = TerminalView::buffer_segments_for_term(
            &term,
            ThemeChoice::HamiMelonDark,
            false,
            0.0,
            0,
            1,
        );
        let line = snapshot
            .segments_by_line
            .first()
            .expect("line 0")
            .iter()
            .map(|seg| seg.text.as_str())
            .collect::<String>();

        let prefix: String = line.chars().take_while(|c| *c != 'r').collect();
        assert!(!prefix.contains('\t'));
        assert!(prefix.chars().all(|c| c == ' '));
        assert_eq!(prefix.len(), 8);
    }

    #[test]
    fn wide_characters_keep_following_text_on_the_grid() {
        let (tx, _rx) = mpsc::unbounded();
        let mut term = Term::new(
            Config::default(),
            &SimpleDimensions::new(16, DEFAULT_ROWS),
            TerminalEventProxy { tx },
        );
        let mut parser: ansi::Processor = ansi::Processor::new();
        parser.advance(&mut term, "月 bar".as_bytes());

        let snapshot = TerminalView::buffer_segments_for_term(
            &term,
            ThemeChoice::HamiMelonDark,
            false,
            0.0,
            0,
            1,
        );
        let runs: Vec<(usize, usize, &str)> = snapshot.segments_by_line[0]
            .iter()
            .map(|run| (run.column, run.cells, run.text.as_str()))
            .take(2)
            .collect();
        // The wide character covers columns 0-1 on its own; no stray space for
        // its spacer cell, so "bar" starts at column 3 like in the grid.
        assert_eq!(runs[0], (0, 2, "月"));
        assert_eq!(runs[1].0, 2);
        assert!(runs[1].2.starts_with(" bar"), "{:?}", runs[1].2);
    }

    #[test]
    #[ignore]
    fn profile_buffer_segments_large_history() {
        let lines = 10_000;
        let cols = 120;

        let config = Config {
            scrolling_history: lines,
            ..Config::default()
        };

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
        let snapshot = TerminalView::buffer_segments_for_term(
            &term,
            ThemeChoice::HamiMelonDark,
            false,
            0.0,
            0,
            lines,
        );
        let snapshot_elapsed = snapshot_started.elapsed();

        let window_lines = 200;
        let range_start = lines.saturating_sub(window_lines);
        let range_snapshot_started = Instant::now();
        let range_snapshot = TerminalView::buffer_segments_for_term(
            &term,
            ThemeChoice::HamiMelonDark,
            false,
            0.0,
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
