use super::{CellPosition, TerminalView};
use alacritty_terminal::{
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as AlacPoint, Side},
    selection::{Selection, SelectionRange, SelectionType},
    term::TermMode,
};
use gpui_kit::{
    ClipboardItem, Context, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Window,
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
            && !self.term.lock().mode().contains(TermMode::ALT_SCREEN)
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

        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = keystroke_to_bytes(keystroke, app_cursor) {
            self.cursor_visible = true;
            self.scroll_to_bottom_for_input(cx);
            self.write_bytes(&bytes);
        }
    }

    /// Left button: start a selection (click: characters, double click: word,
    /// triple click: line; Shift+click extends the current one).
    pub(crate) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        window.focus(&self.focus_handle, cx);
        if let Some((point, side)) = self.grid_point(event.position) {
            let mut term = self.term.lock();
            match term.selection.as_mut() {
                Some(selection) if event.modifiers.shift => selection.update(point, side),
                _ => {
                    let ty = match event.click_count {
                        0 | 1 => SelectionType::Simple,
                        2 => SelectionType::Semantic,
                        _ => SelectionType::Lines,
                    };
                    term.selection = Some(Selection::new(ty, point, side));
                }
            }
            drop(term);
            self.is_selecting = true;
            cx.notify();
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
        // The button was released outside the terminal (over the tab bar or
        // outside the window), where its mouse up never reached us.
        if event.pressed_button != Some(MouseButton::Left) {
            self.is_selecting = false;
            return;
        }
        if let Some((point, side)) = self.grid_point(event.position) {
            if let Some(selection) = self.term.lock().selection.as_mut() {
                selection.update(point, side);
                cx.notify();
            }
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
        // A plain click (no drag) leaves an empty selection: drop it.
        let mut term = self.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
            cx.notify();
        }
        drop(term);
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
                let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
                self.write_bytes(&paste_bytes(&text, bracketed));
                return true;
            }
        }

        false
    }

    /// The visible cell under `position` (row 0 = top of the screen), and which
    /// half of the cell it is in.
    pub(crate) fn cell_at(&self, position: Point<Pixels>) -> Option<(CellPosition, Side)> {
        let bounds = self.last_bounds?;
        let term = self.term.lock();
        let grid = term.grid();
        let columns = grid.columns();
        if columns == 0 || self.cell_width <= 0.0 || self.cell_height <= 0.0 {
            return None;
        }
        let rel_x: f32 = (position.x - bounds.origin.x).into();
        let rel_y: f32 = (position.y - bounds.origin.y).into();
        let max_col = columns.saturating_sub(1) as isize;
        let max_row = grid.screen_lines().saturating_sub(1) as isize;
        let column = ((rel_x / self.cell_width).floor() as isize).clamp(0, max_col) as usize;
        let line = ((rel_y / self.cell_height).floor() as isize).clamp(0, max_row) as usize;
        let side = if rel_x - column as f32 * self.cell_width > self.cell_width / 2.0 {
            Side::Right
        } else {
            Side::Left
        };
        Some((CellPosition { line, column }, side))
    }

    /// The alacritty grid point under `position` (lines above the screen are
    /// negative), accounting for the scrollback display offset.
    fn grid_point(&self, position: Point<Pixels>) -> Option<(AlacPoint, Side)> {
        let (cell, side) = self.cell_at(position)?;
        let line = Line(cell.line as i32 - self.term.lock().grid().display_offset() as i32);
        Some((AlacPoint::new(line, Column(cell.column)), side))
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub(crate) fn clear_selection(&mut self) -> bool {
        self.is_selecting = false;
        self.term.lock().selection.take().is_some()
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    pub(crate) fn select_all(&mut self) -> bool {
        let mut term = self.term.lock();
        let start = AlacPoint::new(term.grid().topmost_line(), Column(0));
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.include_all();
        term.selection = Some(selection);
        true
    }
}

/// Columns of grid line `line` covered by `range`, inclusive.
pub(crate) fn selection_columns(
    range: &SelectionRange,
    line: Line,
    columns: usize,
) -> Option<(usize, usize)> {
    if columns == 0 || line < range.start.line || line > range.end.line {
        return None;
    }
    let last = columns - 1;
    let (start, end) = if range.is_block {
        (range.start.column.0, range.end.column.0)
    } else {
        (
            if line == range.start.line {
                range.start.column.0
            } else {
                0
            },
            if line == range.end.line {
                range.end.column.0
            } else {
                last
            },
        )
    };
    (start <= end).then_some((start.min(last), end.min(last)))
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
            // xterm's control codes for the keys around the letters.
            b'@' | b'2' => vec![0],
            b'[' | b'3' => vec![0x1b],
            b'\\' | b'4' => vec![0x1c],
            b']' | b'5' => vec![0x1d],
            b'^' | b'6' => vec![0x1e],
            b'_' | b'/' | b'7' => vec![0x1f],
            b'8' => vec![0x7f],
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
    use super::super::{SimpleDimensions, TerminalEventProxy};
    use super::{keystroke_to_bytes, paste_bytes, selection_columns};
    use alacritty_terminal::{
        index::{Column, Line, Point, Side},
        selection::{Selection, SelectionRange, SelectionType},
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

    /// Selects from (line, column) to (line, column), like a mouse drag.
    fn select(
        term: &mut Term<TerminalEventProxy>,
        ty: SelectionType,
        from: (i32, usize),
        to: (i32, usize),
    ) {
        let mut selection =
            Selection::new(ty, Point::new(Line(from.0), Column(from.1)), Side::Left);
        selection.update(Point::new(Line(to.0), Column(to.1)), Side::Right);
        term.selection = Some(selection);
    }

    #[test]
    fn copies_soft_wrapped_lines_as_one() {
        let mut term = term_with("hello world foo");
        select(&mut term, SelectionType::Simple, (0, 0), (1, 9));
        assert_eq!(
            term.selection_to_string().unwrap().trim_end(),
            "hello world foo"
        );
    }

    #[test]
    fn copies_hard_line_breaks() {
        let mut term = term_with("ab\r\ncd");
        select(&mut term, SelectionType::Simple, (0, 0), (1, 1));
        assert_eq!(term.selection_to_string().unwrap(), "ab\ncd");
    }

    #[test]
    fn double_click_selects_a_word() {
        let mut term = term_with("foo bar.baz");
        select(&mut term, SelectionType::Semantic, (0, 5), (0, 5));
        assert_eq!(term.selection_to_string().unwrap(), "bar.baz");
    }

    #[test]
    fn selection_follows_text_as_output_scrolls_a_full_history() {
        let (tx, _rx) = mpsc::unbounded();
        let config = Config {
            scrolling_history: 2,
            ..Config::default()
        };
        let mut term = Term::new(
            config,
            &SimpleDimensions::new(10, 4),
            TerminalEventProxy { tx },
        );
        let mut parser = ansi::Processor::<ansi::StdSyncHandler>::new();
        // Screen: 4, 5, 6, (prompt); the history (2 lines) is already full.
        parser.advance(&mut term, b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n");
        select(&mut term, SelectionType::Lines, (1, 0), (1, 0));
        assert_eq!(term.selection_to_string().unwrap().trim(), "5");

        // Two more lines push "5" up into the history; the selection follows it.
        parser.advance(&mut term, b"7\r\n8\r\n");
        assert_eq!(term.selection_to_string().unwrap().trim(), "5");
    }

    #[test]
    fn selection_columns_per_line() {
        let range = SelectionRange::new(
            Point::new(Line(0), Column(3)),
            Point::new(Line(2), Column(4)),
            false,
        );
        assert_eq!(selection_columns(&range, Line(0), 10), Some((3, 9)));
        assert_eq!(selection_columns(&range, Line(1), 10), Some((0, 9)));
        assert_eq!(selection_columns(&range, Line(2), 10), Some((0, 4)));
        assert_eq!(selection_columns(&range, Line(3), 10), None);
        let block = SelectionRange::new(
            Point::new(Line(0), Column(3)),
            Point::new(Line(2), Column(4)),
            true,
        );
        assert_eq!(selection_columns(&block, Line(1), 10), Some((3, 4)));
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
        // Ctrl+/ is undo in readline and Emacs.
        assert_eq!(bytes("ctrl-/", false), Some(vec![0x1f]));
        assert_eq!(bytes("ctrl-2", false), Some(vec![0]));
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
