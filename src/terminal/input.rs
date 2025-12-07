use super::{CellPosition, SelectionRange, TerminalView};
use alacritty_terminal::{grid::Dimensions, index::Column, index::Line, term::cell::Flags};
use gpui::{
    ClipboardItem, Context, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Window,
};

impl TerminalView {
    pub(crate) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.eq_ignore_ascii_case("v")
            && event.keystroke.modifiers.control
            && event.keystroke.modifiers.shift
        {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    self.cursor_visible = true;
                    self.write_bytes(text.as_bytes());
                }
            }
            cx.stop_propagation();
            return;
        }

        if event.keystroke.key.eq_ignore_ascii_case("c")
            && event.keystroke.modifiers.control
            && event.keystroke.modifiers.shift
        {
            if let Some(text) = self.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            cx.stop_propagation();
            return;
        }

        // Prevent focus traversal (Root binds Tab/Shift-Tab) so Tab can flow to the PTY.
        if event.keystroke.key == "tab" {
            cx.stop_propagation();
        }

        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            self.cursor_visible = true;
            self.write_bytes(&bytes);
            cx.notify();
        }
    }

    pub(crate) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        window.focus(&self.focus_handle);
        if let Some(pos) = self.cell_position_from_point(event.position) {
            if event.click_count >= 2 {
                if let Some(range) = self.word_bounds_at(pos) {
                    self.selection_start = Some(range.start);
                    self.selection_end = Some(range.end);
                } else {
                    self.selection_start = Some(pos);
                    self.selection_end = Some(pos);
                }
                self.is_selecting = true;
                cx.notify();
            } else {
                if event.modifiers.shift {
                    if self.selection_start.is_none() {
                        self.selection_start = Some(pos);
                    }
                    self.selection_end = Some(pos);
                } else {
                    self.selection_start = Some(pos);
                    self.selection_end = Some(pos);
                }
                self.is_selecting = true;
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    pub(crate) fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }

        if let Some(pos) = self.cell_position_from_point(event.position) {
            self.selection_end = Some(pos);
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(crate) fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        self.is_selecting = false;
        cx.stop_propagation();
    }

    pub(crate) fn cell_position_from_point(
        &self,
        position: gpui::Point<gpui::Pixels>,
    ) -> Option<CellPosition> {
        let bounds = self.last_bounds?;
        let grid = self.term.grid();
        let total_lines = grid.total_lines();
        let columns = grid.columns();
        if total_lines == 0 || columns == 0 || self.cell_width <= 0.0 || self.cell_height <= 0.0 {
            return None;
        }

        let origin_x = bounds.origin.x;
        let origin_y = bounds.origin.y;
        let rel_x: f32 = (position.x - origin_x).into();
        let rel_y: f32 = (position.y - origin_y).into();
        let content_x = rel_x;
        let content_y = rel_y;

        let max_col = (columns.saturating_sub(1)) as isize;
        let max_line = (total_lines.saturating_sub(1)) as isize;

        let column = ((content_x / self.cell_width).floor() as isize).clamp(0, max_col) as usize;
        let line = ((content_y / self.cell_height).floor() as isize).clamp(0, max_line) as usize;

        Some(CellPosition { line, column })
    }

    pub(crate) fn selection_range(&self) -> Option<SelectionRange> {
        let start = self.selection_start?;
        let end = self.selection_end.unwrap_or(start);
        let grid = self.term.grid();
        let total_lines = grid.total_lines();
        let columns = grid.columns();
        if total_lines == 0 || columns == 0 {
            return None;
        }

        let clamp = |pos: CellPosition| CellPosition {
            line: pos.line.min(total_lines.saturating_sub(1)),
            column: pos.column.min(columns.saturating_sub(1)),
        };

        let mut a = clamp(start);
        let mut b = clamp(end);
        if (b.line < a.line) || (b.line == a.line && b.column < a.column) {
            std::mem::swap(&mut a, &mut b);
        }

        Some(SelectionRange { start: a, end: b })
    }

    pub(crate) fn selection_span_for_line(
        &self,
        range: &SelectionRange,
        line_idx: usize,
        columns: usize,
    ) -> Option<(f32, f32)> {
        if columns == 0 || line_idx < range.start.line || line_idx > range.end.line {
            return None;
        }

        let start_col = if line_idx == range.start.line {
            range.start.column
        } else {
            0
        };
        let end_col = if line_idx == range.end.line {
            range.end.column
        } else {
            columns.saturating_sub(1)
        };

        if end_col < start_col {
            return None;
        }

        let left = start_col as f32 * self.cell_width;
        let width = (end_col - start_col + 1) as f32 * self.cell_width;
        Some((left, width))
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        let selection = self.selection_range()?;
        let grid_ref = self.term.grid();
        let columns = grid_ref.columns();
        let total_lines = grid_ref.total_lines();
        if columns == 0 || total_lines == 0 {
            return None;
        }
        let screen_lines = grid_ref.screen_lines();
        let history_size = total_lines.saturating_sub(screen_lines);

        let mut lines = Vec::new();
        for line_idx in selection.start.line..=selection.end.line {
            let grid_line = Line(line_idx as i32 - history_size as i32);
            let line_start_col = if line_idx == selection.start.line {
                selection.start.column
            } else {
                0
            };
            let line_end_col = if line_idx == selection.end.line {
                selection.end.column
            } else {
                columns.saturating_sub(1)
            };

            if line_end_col < line_start_col {
                continue;
            }

            let mut line = String::new();
            for col_idx in line_start_col..=line_end_col {
                let cell = &grid_ref[grid_line][Column(col_idx)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    line.push(' ');
                    continue;
                }

                line.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    for c in extra {
                        line.push(*c);
                    }
                }
            }

            lines.push(line.trim_end().to_string());
        }

        Some(lines.join("\n"))
    }

    pub(crate) fn word_bounds_at(&self, pos: CellPosition) -> Option<SelectionRange> {
        let grid_ref = self.term.grid();
        let columns = grid_ref.columns();
        if columns == 0 || grid_ref.total_lines() == 0 {
            return None;
        }

        let line = pos.line.min(grid_ref.total_lines().saturating_sub(1));
        let mut col = pos.column.min(columns.saturating_sub(1));
        let ch = self.char_for_column(line, col).unwrap_or(' ');
        let is_word = !ch.is_whitespace();

        // If we clicked on a spacer, shift left until we hit a non-spacer so we include the wide char.
        while col > 0 {
            let grid_line =
                Line(line as i32 - (grid_ref.total_lines() - grid_ref.screen_lines()) as i32);
            let cell = &grid_ref[grid_line][Column(col)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                col -= 1;
                continue;
            }
            break;
        }

        let mut start = col;
        while start > 0 {
            if let Some(c) = self.char_for_column(line, start.saturating_sub(1)) {
                if (!c.is_whitespace()) == is_word {
                    start -= 1;
                    continue;
                }
            }
            break;
        }

        let mut end = col;
        while end + 1 < columns {
            if let Some(c) = self.char_for_column(line, end + 1) {
                if (!c.is_whitespace()) == is_word {
                    end += 1;
                    continue;
                }
            }
            break;
        }

        Some(SelectionRange {
            start: CellPosition {
                line,
                column: start,
            },
            end: CellPosition { line, column: end },
        })
    }

    fn char_for_column(&self, line_idx: usize, column: usize) -> Option<char> {
        let grid_ref = self.term.grid();
        let columns = grid_ref.columns();
        if line_idx >= grid_ref.total_lines() || column >= columns {
            return None;
        }

        let screen_lines = grid_ref.screen_lines();
        let history_size = grid_ref.total_lines().saturating_sub(screen_lines);
        let grid_line = Line(line_idx as i32 - history_size as i32);
        let cell = &grid_ref[grid_line][Column(column)];

        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            if column == 0 {
                return Some(' ');
            }
            let prev_cell = &grid_ref[grid_line][Column(column.saturating_sub(1))];
            return Some(prev_cell.c);
        }

        Some(cell.c)
    }
}

fn keystroke_to_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    if let Some(text) = &keystroke.key_char {
        return Some(text.clone().into_bytes());
    }

    if keystroke.modifiers.control && keystroke.key.len() == 1 {
        let ch = keystroke.key.chars().next().unwrap().to_ascii_lowercase();
        if ('a'..='z').contains(&ch) {
            return Some(vec![(ch as u8 - b'a' + 1)]);
        }
    }

    match keystroke.key.as_str() {
        "enter" => Some(b"\r".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "space" => Some(b" ".to_vec()),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}
