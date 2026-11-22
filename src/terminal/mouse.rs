//! Mouse reporting to terminal programs (vim, htop, tmux, less...).
//!
//! When a program enables a mouse mode, clicks, drags, motion and the wheel are
//! encoded as escape sequences and written to the PTY instead of driving
//! selection, the context menu and scrolling. Holding Shift bypasses reporting
//! so text can still be selected, as in most terminals.

use super::{CellPosition, TerminalView};
use alacritty_terminal::{grid::Scroll, term::TermMode};
use gpui_kit::{
    px, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, ScrollEvent, Window,
};

/// Button codes from the xterm mouse protocol.
const LEFT: u8 = 0;
const MIDDLE: u8 = 1;
const RIGHT: u8 = 2;
const NO_BUTTON: u8 = 3;
const MOTION: u8 = 32;
const WHEEL_UP: u8 = 64;
const WHEEL_DOWN: u8 = 65;

impl TerminalView {
    /// Whether mouse events should go to the program rather than the UI.
    fn reports_mouse(&self, modifiers: &Modifiers) -> bool {
        !modifiers.shift && self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    pub(crate) fn report_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = button_code(event.button) else {
            return;
        };
        if !self.reports_mouse(&event.modifiers) {
            return;
        }
        if let Some(cell) = self.viewport_cell(event.position) {
            self.mouse_button_down = Some(code);
            self.last_mouse_cell = Some(cell);
            self.write_mouse_report(code, true, cell, &event.modifiers);
        }
        // Keep the press from starting a selection or opening the context menu.
        cx.stop_propagation();
    }

    pub(crate) fn report_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = button_code(event.button) else {
            return;
        };
        // Release whatever we reported as pressed, even if Shift is now held
        // or the mode changed meanwhile, so the program never sees a stuck button.
        if self.mouse_button_down != Some(code) {
            return;
        }
        self.mouse_button_down = None;
        let cell = self
            .viewport_cell(event.position)
            .or(self.last_mouse_cell)
            .unwrap_or_default();
        self.write_mouse_report(code, false, cell, &event.modifiers);
        cx.stop_propagation();
    }

    pub(crate) fn report_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.reports_mouse(&event.modifiers) {
            return;
        }
        let mode = *self.term.mode();
        let code = match self.mouse_button_down {
            Some(button) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => {
                button + MOTION
            }
            None if mode.contains(TermMode::MOUSE_MOTION) => NO_BUTTON + MOTION,
            _ => return,
        };
        let Some(cell) = self.viewport_cell(event.position) else {
            return;
        };
        // Programs only care about cell changes; skip sub-cell jitter.
        if self.last_mouse_cell != Some(cell) {
            self.last_mouse_cell = Some(cell);
            self.write_mouse_report(code, true, cell, &event.modifiers);
        }
        cx.stop_propagation();
    }

    /// Wheel handling. Returns without stopping propagation when the wheel
    /// should scroll the scrollback natively.
    pub(crate) fn handle_scroll(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Ctrl+wheel zooms, even when a program captures the mouse.
        if event.modifiers.control {
            cx.stop_propagation();
            let steps = self.wheel_steps(event);
            if steps != 0 {
                self.zoom(steps as f32, window, cx);
            }
            return;
        }

        let mode = *self.term.mode();
        let report = self.reports_mouse(&event.modifiers);
        // In the alternate screen (less, man...) there is no scrollback; turn
        // the wheel into arrow keys like other terminals do.
        let arrows = !report
            && !event.modifiers.shift
            && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL);
        cx.stop_propagation();
        let steps = self.wheel_steps(event);
        let count = steps.unsigned_abs() as usize;
        if count == 0 {
            return;
        }
        let up = steps > 0;

        if !report && !arrows {
            // Scroll the scrollback (alacritty's display offset; up = positive).
            self.scroll(Scroll::Delta(steps), cx);
            return;
        }

        if report {
            let Some(cell) = self.viewport_cell(event.position) else {
                return;
            };
            let code = if up { WHEEL_UP } else { WHEEL_DOWN };
            for _ in 0..count {
                self.write_mouse_report(code, true, cell, &event.modifiers);
            }
        } else {
            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            let seq: &[u8] = match (up, app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write_bytes(&seq.repeat(count));
        }
    }

    /// Whole wheel notches (lines) in this event, positive = up. Fractional
    /// trackpad deltas are carried over to the next event.
    fn wheel_steps(&mut self, event: &ScrollEvent) -> i32 {
        let delta: f32 = event.delta.pixel_delta(px(self.cell_height)).y.into();
        let lines = self.scroll_lines_remainder + delta / self.cell_height.max(1.0);
        let whole = lines.trunc();
        self.scroll_lines_remainder = lines - whole;
        whole as i32
    }

    /// The cell under `position`, relative to the visible screen (what the
    /// program sees), not to the top of the scrollback.
    fn viewport_cell(&self, position: Point<Pixels>) -> Option<CellPosition> {
        self.cell_at(position).map(|(cell, _)| cell)
    }

    fn write_mouse_report(
        &self,
        button: u8,
        pressed: bool,
        cell: CellPosition,
        modifiers: &Modifiers,
    ) {
        if let Some(bytes) =
            encode_mouse_report(*self.term.mode(), button, pressed, cell, modifiers)
        {
            self.write_bytes(&bytes);
        }
    }
}

fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(LEFT),
        MouseButton::Middle => Some(MIDDLE),
        MouseButton::Right => Some(RIGHT),
        _ => None,
    }
}

/// Encodes one event in the protocol the program asked for: SGR
/// (`CSI < b ; x ; y M/m`), UTF-8 extended, or the legacy X10 format.
fn encode_mouse_report(
    mode: TermMode,
    button: u8,
    pressed: bool,
    cell: CellPosition,
    modifiers: &Modifiers,
) -> Option<Vec<u8>> {
    let mut code = button;
    if modifiers.shift {
        code += 4;
    }
    if modifiers.alt {
        code += 8;
    }
    if modifiers.control {
        code += 16;
    }
    let (x, y) = (cell.column + 1, cell.line + 1);

    if mode.contains(TermMode::SGR_MOUSE) {
        let action = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{x};{y}{action}").into_bytes());
    }

    // Legacy encodings can't say which button was released.
    if !pressed {
        code = (code & !0b11) | NO_BUTTON;
    }
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code);
    for value in [x, y] {
        let value = 32 + value;
        if mode.contains(TermMode::UTF8_MOUSE) {
            let ch = char::from_u32(value as u32)?;
            bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
        } else {
            // X10 coordinates are a single byte; beyond that we can't report.
            bytes.push(u8::try_from(value).ok()?);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(column: usize, line: usize) -> CellPosition {
        CellPosition { line, column }
    }

    #[test]
    fn encodes_sgr() {
        let mode = TermMode::SGR_MOUSE;
        let none = Modifiers::default();
        assert_eq!(
            encode_mouse_report(mode, LEFT, true, cell(4, 9), &none),
            Some(b"\x1b[<0;5;10M".to_vec())
        );
        assert_eq!(
            encode_mouse_report(mode, RIGHT, false, cell(0, 0), &none),
            Some(b"\x1b[<2;1;1m".to_vec())
        );
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(
            encode_mouse_report(mode, WHEEL_UP, true, cell(0, 0), &ctrl),
            Some(b"\x1b[<80;1;1M".to_vec())
        );
    }

    #[test]
    fn encodes_legacy() {
        let none = Modifiers::default();
        assert_eq!(
            encode_mouse_report(TermMode::empty(), LEFT, true, cell(0, 0), &none),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        // Release is reported as button 3.
        assert_eq!(
            encode_mouse_report(TermMode::empty(), MIDDLE, false, cell(1, 2), &none),
            Some(vec![0x1b, b'[', b'M', 35, 34, 35])
        );
        // Out of range for X10, fine with UTF-8.
        assert_eq!(
            encode_mouse_report(TermMode::empty(), LEFT, true, cell(300, 0), &none),
            None
        );
        assert!(
            encode_mouse_report(TermMode::UTF8_MOUSE, LEFT, true, cell(300, 0), &none).is_some()
        );
    }
}
