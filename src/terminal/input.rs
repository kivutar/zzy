use super::{CellPosition, SelectionRange, TerminalView};
use alacritty_terminal::{
    grid::{Dimensions, Scroll},
    index::{Column, Line},
    term::{cell::Flags, Term, TermMode},
};
use gpui_kit::{
    ClipboardItem, Context, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Window,
};

impl TerminalView {
    pub(crate) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        // Zoom: Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0 (Ptyxis, GNOME Console, WT).
        if keystroke.modifiers.control && !keystroke.modifiers.alt {
            let steps = match keystroke.key.as_str() {
                "=" | "+" => Some(1.0),
                "-" => Some(-1.0),
                "0" => Some(0.0),
                _ => None,
            };
            if let Some(steps) = steps {
                self.zoom(steps, window, cx);
                cx.stop_propagation();
                return;
            }
        }

        // Shift+PageUp/PageDown/Home/End scroll the scrollback (Konsole, xterm),
        // except in the alternate screen where programs get the keys.
        let mods = &keystroke.modifiers;
        if mods.shift
            && !mods.control
            && !mods.alt
            && !self.term.mode().contains(TermMode::ALT_SCREEN)
        {
            let scroll = match keystroke.key.as_str() {
                "pageup" => Some(Scroll::PageUp),
                "pagedown" => Some(Scroll::PageDown),
                "home" => Some(Scroll::Top),
                "end" => Some(Scroll::Bottom),
                _ => None,
            };
            if let Some(scroll) = scroll {
                self.scroll(scroll, cx);
                cx.stop_propagation();
                return;
            }
        }

        if keystroke.modifiers.control && keystroke.modifiers.shift {
            let handled = match keystroke.key.to_ascii_lowercase().as_str() {
                "c" => {
                    self.copy_selection_to_clipboard(cx);
                    true
                }
                "v" => {
                    self.paste_from_clipboard(cx);
                    true
                }
                "a" => {
                    if self.select_all() {
                        cx.notify();
                    }
                    true
                }
                "d" => {
                    if self.clear_selection() {
                        cx.notify();
                    }
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
                return;
            }
        }

        // Prevent focus traversal (Root binds Tab/Shift-Tab) so Tab can flow to the PTY.
        if event.keystroke.key == "tab" {
            cx.stop_propagation();
        }

        let app_cursor = self.term.mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = keystroke_to_bytes(keystroke, app_cursor) {
            self.cursor_visible = true;
            self.scroll_to_bottom_for_input(cx);
            self.write_bytes(&bytes);
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

        self.selection_started_with_single_click = false;
        self.selection_dragged = false;
        window.focus(&self.focus_handle, cx);
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
                    let had_selection = self.clear_selection();
                    self.selection_start = Some(pos);
                    self.selection_end = Some(pos);
                    self.selection_started_with_single_click = true;
                    if had_selection {
                        cx.notify();
                    }
                }
                self.is_selecting = true;
                if !self.selection_started_with_single_click {
                    cx.notify();
                }
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
            let _ = self.selection_start.get_or_insert(pos);
            self.selection_dragged = true;
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
        if self.selection_started_with_single_click
            && !self.selection_dragged
            && self.clear_selection()
        {
            cx.notify();
        }
        self.selection_started_with_single_click = false;
        self.selection_dragged = false;
        cx.stop_propagation();
    }

    pub(crate) fn copy_selection_to_clipboard(&self, cx: &mut Context<Self>) -> bool {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return true;
        }

        false
    }

    pub(crate) fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                self.cursor_visible = true;
                self.scroll_to_bottom_for_input(cx);
                let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
                self.write_bytes(&paste_bytes(&text, bracketed));
                return true;
            }
        }

        false
    }

    pub(crate) fn cell_position_from_point(
        &self,
        position: gpui_kit::Point<gpui_kit::Pixels>,
    ) -> Option<CellPosition> {
        let bounds = self.last_bounds?;
        let grid = self.term.grid();
        let total_lines = grid.total_lines();
        let columns = grid.columns();
        if total_lines == 0 || columns == 0 || self.cell_width <= 0.0 || self.cell_height <= 0.0 {
            return None;
        }

        let rel_x: f32 = (position.x - bounds.origin.x).into();
        let rel_y: f32 = (position.y - bounds.origin.y).into();
        let max_col = columns.saturating_sub(1) as isize;
        let max_row = grid.screen_lines().saturating_sub(1) as isize;
        let column = ((rel_x / self.cell_width).floor() as isize).clamp(0, max_col) as usize;
        let row = ((rel_y / self.cell_height).floor() as isize).clamp(0, max_row) as usize;

        // Selections are kept in scrollback coordinates (line 0 = oldest).
        let line = (self.viewport_start() + row).min(total_lines - 1);
        Some(CellPosition { line, column })
    }

    pub(crate) fn selection_range(&self) -> Option<SelectionRange> {
        if self.selection_started_with_single_click && !self.selection_dragged && self.is_selecting
        {
            return None;
        }

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
        text_in_range(&self.term, selection)
    }

    pub(crate) fn clear_selection(&mut self) -> bool {
        let changed = self.selection_start.take().is_some() || self.selection_end.take().is_some();
        self.is_selecting = false;
        changed
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub(crate) fn select_all(&mut self) -> bool {
        let grid_ref = self.term.grid();
        let columns = grid_ref.columns();
        let total_lines = grid_ref.total_lines();
        if columns == 0 || total_lines == 0 {
            let changed =
                self.selection_start.take().is_some() || self.selection_end.take().is_some();
            return changed;
        }

        let start = Some(CellPosition { line: 0, column: 0 });
        let end = Some(CellPosition {
            line: total_lines.saturating_sub(1),
            column: columns.saturating_sub(1),
        });
        let changed = self.selection_start != start || self.selection_end != end;
        self.selection_start = start;
        self.selection_end = end;
        changed
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

/// The text in `selection`: trailing blanks trimmed per line, soft-wrapped
/// lines joined back into one, tabs as spaces.
fn text_in_range<T>(term: &Term<T>, selection: SelectionRange) -> Option<String> {
    let grid_ref = term.grid();
    let columns = grid_ref.columns();
    let total_lines = grid_ref.total_lines();
    if columns == 0 || total_lines == 0 {
        return None;
    }
    let screen_lines = grid_ref.screen_lines();
    let history_size = total_lines.saturating_sub(screen_lines);

    let mut text = String::new();
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

            let mut ch = cell.c;
            if ch == '\t' {
                ch = ' ';
            }
            line.push(ch);
            if let Some(extra) = cell.zerowidth() {
                for c in extra {
                    line.push(*c);
                }
            }
        }

        // Soft-wrapped lines are one logical line: join them unchanged.
        let wrapped = line_idx < selection.end.line
            && grid_ref[grid_line][Column(columns - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        if wrapped {
            text.push_str(&line);
        } else {
            text.push_str(line.trim_end());
            if line_idx < selection.end.line {
                text.push('\n');
            }
        }
    }

    Some(text)
}

/// What to send to the PTY for a paste. Shells execute on CR, not LF, so line
/// breaks become CR; with bracketed paste (enabled by the shell) the text is
/// wrapped so a multi-line paste isn't executed line by line. An embedded end
/// marker is stripped so the pasted text can't end the bracket early.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let text = text.replace("\x1b[201~", "");
        [b"\x1b[200~", text.as_bytes(), b"\x1b[201~"].concat()
    } else {
        text.into_bytes()
    }
}

fn keystroke_to_bytes(keystroke: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let mods = &keystroke.modifiers;
    if let Some(bytes) = special_key_bytes(&keystroke.key, modifier_param(mods), app_cursor) {
        return Some(bytes);
    }

    let mut bytes = if let Some(text) = keystroke.key_char.as_ref().filter(|_| !mods.control) {
        text.clone().into_bytes()
    } else if mods.control && keystroke.key.len() == 1 {
        match keystroke.key.as_bytes()[0].to_ascii_lowercase() {
            ch @ b'a'..=b'z' => vec![ch - b'a' + 1],
            b'[' => vec![0x1b],
            b'\\' => vec![0x1c],
            b']' => vec![0x1d],
            _ => return None,
        }
    } else if mods.alt && keystroke.key.chars().count() == 1 {
        keystroke.key.clone().into_bytes()
    } else {
        match keystroke.key.as_str() {
            "enter" => b"\r".to_vec(),
            "tab" => b"\t".to_vec(),
            "space" if mods.control => vec![0],
            "space" => b" ".to_vec(),
            // ^H: delete the previous word in many shells and editors.
            "backspace" if mods.control => vec![0x08],
            "backspace" => vec![0x7f],
            "escape" => vec![0x1b],
            _ => return None,
        }
    };
    // Alt sends ESC-prefixed keys (meta), e.g. Alt+B / Alt+F word motions.
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// xterm modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
fn modifier_param(mods: &Modifiers) -> u8 {
    1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.control as u8
}

/// Cursor, editing and function keys, in xterm's encoding. Unmodified cursor
/// keys honor application cursor mode (SS3); modified keys carry the modifier
/// parameter (`CSI 1;<mod>A`, `CSI 15;<mod>~`).
fn special_key_bytes(key: &str, modifier: u8, app_cursor: bool) -> Option<Vec<u8>> {
    let letter = |c: char, ss3: bool| match modifier {
        1 if ss3 => format!("\x1bO{c}"),
        1 => format!("\x1b[{c}"),
        _ => format!("\x1b[1;{modifier}{c}"),
    };
    let tilde = |code: u8| match modifier {
        1 => format!("\x1b[{code}~"),
        _ => format!("\x1b[{code};{modifier}~"),
    };
    let sequence = match key {
        "up" => letter('A', app_cursor),
        "down" => letter('B', app_cursor),
        "right" => letter('C', app_cursor),
        "left" => letter('D', app_cursor),
        "home" => letter('H', app_cursor),
        "end" => letter('F', app_cursor),
        "f1" => letter('P', true),
        "f2" => letter('Q', true),
        "f3" => letter('R', true),
        "f4" => letter('S', true),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        _ => return None,
    };
    Some(sequence.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::super::{CellPosition, SelectionRange, SimpleDimensions, TerminalEventProxy};
    use super::{keystroke_to_bytes, paste_bytes, text_in_range};
    use alacritty_terminal::{
        term::{Config, Term},
        vte::ansi,
    };
    use futures::channel::mpsc;
    use gpui_kit::Keystroke;

    /// A 10-column, 4-line terminal fed with `input`.
    fn term_with(input: &str) -> Term<TerminalEventProxy> {
        let (tx, _rx) = mpsc::unbounded();
        let mut term = Term::new(
            Config::default(),
            &SimpleDimensions::new(10, 4),
            TerminalEventProxy { tx },
        );
        ansi::Processor::<ansi::StdSyncHandler>::new().advance(&mut term, input.as_bytes());
        term
    }

    fn lines(start: usize, end: usize) -> SelectionRange {
        SelectionRange {
            start: CellPosition {
                line: start,
                column: 0,
            },
            end: CellPosition {
                line: end,
                column: 9,
            },
        }
    }

    #[test]
    fn copies_soft_wrapped_lines_as_one() {
        let term = term_with("hello world foo");
        assert_eq!(
            text_in_range(&term, lines(0, 1)).unwrap(),
            "hello world foo"
        );
    }

    #[test]
    fn copies_hard_line_breaks_and_trims_trailing_blanks() {
        let term = term_with("ab  \r\ncd");
        assert_eq!(text_in_range(&term, lines(0, 1)).unwrap(), "ab\ncd");
    }

    #[test]
    fn copies_partial_line() {
        let term = term_with("hello");
        let range = SelectionRange {
            start: CellPosition { line: 0, column: 1 },
            end: CellPosition { line: 0, column: 3 },
        };
        assert_eq!(text_in_range(&term, range).unwrap(), "ell");
    }

    #[test]
    fn paste_converts_newlines_and_brackets() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("ls\n", true), b"\x1b[200~ls\r\x1b[201~");
        // An embedded end marker can't close the bracket early.
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf ~\n", true),
            b"\x1b[200~xrm -rf ~\r\x1b[201~"
        );
    }

    fn bytes(keys: &'static str, app_cursor: bool) -> Option<Vec<u8>> {
        keystroke_to_bytes(&Keystroke::parse(keys).unwrap(), app_cursor)
    }

    #[test]
    fn encodes_keys() {
        assert_eq!(bytes("ctrl-c", false), Some(vec![3]));
        assert_eq!(bytes("alt-b", false), Some(b"\x1bb".to_vec()));
        assert_eq!(bytes("up", false), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes("up", true), Some(b"\x1bOA".to_vec()));
        assert_eq!(bytes("enter", false), Some(b"\r".to_vec()));
        assert_eq!(bytes("ctrl-backspace", false), Some(vec![0x08]));
    }

    #[test]
    fn encodes_function_and_modified_keys() {
        assert_eq!(bytes("f1", false), Some(b"\x1bOP".to_vec()));
        assert_eq!(bytes("f5", false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(bytes("f12", false), Some(b"\x1b[24~".to_vec()));
        assert_eq!(bytes("shift-f5", false), Some(b"\x1b[15;2~".to_vec()));
        assert_eq!(bytes("ctrl-f1", false), Some(b"\x1b[1;5P".to_vec()));
        assert_eq!(bytes("insert", false), Some(b"\x1b[2~".to_vec()));
        // Modified arrows always use CSI, even in application cursor mode.
        assert_eq!(bytes("ctrl-left", true), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(bytes("alt-up", false), Some(b"\x1b[1;3A".to_vec()));
        assert_eq!(
            bytes("ctrl-shift-right", false),
            Some(b"\x1b[1;6C".to_vec())
        );
        assert_eq!(bytes("shift-pageup", false), Some(b"\x1b[5;2~".to_vec()));
    }
}
