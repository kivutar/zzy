mod input;
mod pty;
mod render;

use crate::colors::Palette;
use crate::settings::{AppSettings, CursorShapeSetting};
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    term::{Config, Term},
    vte::ansi,
};
use futures::channel::mpsc;
use gpui::{
    font, px, AsyncApp, Bounds, Context, FocusHandle, Focusable, Hsla, Rgba, ScrollHandle,
    SharedString, Subscription, TextRun, WeakEntity, Window,
};
use parking_lot::Mutex;
use portable_pty::{Child, MasterPty};
pub use render::DEFAULT_FONT_SIZE;
use std::{io::Write, path::Path, sync::Arc, time::Duration};

pub struct TerminalView {
    focus_handle: FocusHandle,
    parser: ansi::Processor,
    term: Term<TerminalEventProxy>,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    _child: Option<Box<dyn Child + Send + Sync>>,
    rows: usize,
    cols: usize,
    font_size: f32,
    font_family: SharedString,
    cell_width: f32,
    cell_height: f32,
    scroll_handle: ScrollHandle,
    program_name: String,
    last_bounds: Option<Bounds<gpui::Pixels>>,
    title: Option<String>,
    error: Option<String>,
    keystroke_sub: Option<Subscription>,
    cursor_visible: bool,
    cell_metrics_measured: bool,
    selection_start: Option<CellPosition>,
    selection_end: Option<CellPosition>,
    is_selecting: bool,
    cursor_blink_enabled: bool,
    cursor_shape: ansi::CursorShape,
}

#[derive(Clone)]
struct CellRender {
    text: String,
    fg: Rgba,
    bg: Rgba,
    is_cursor: bool,
    is_bold: bool,
}

#[derive(Clone)]
struct TerminalEventProxy {
    tx: mpsc::UnboundedSender<TerminalMsg>,
}

pub(crate) enum TerminalMsg {
    Output(Vec<u8>),
    Event(Event),
}

pub(crate) struct BufferSnapshot {
    segments_by_line: Vec<Vec<CellRender>>,
    palette: Palette,
    columns: usize,
    cursor_shape: ansi::CursorShape,
    cursor_position: Option<CellPosition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellPosition {
    line: usize,
    column: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectionRange {
    start: CellPosition,
    end: CellPosition,
}

#[derive(Clone, Copy)]
struct SimpleDimensions {
    columns: usize,
    lines: usize,
}

impl SimpleDimensions {
    fn new(columns: usize, lines: usize) -> Self {
        Self { columns, lines }
    }
}

impl Dimensions for SimpleDimensions {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>, settings: &AppSettings) -> Self {
        let focus_handle = cx.focus_handle();
        let (tx, rx) = mpsc::unbounded();
        let proxy = TerminalEventProxy { tx: tx.clone() };
        let cursor_shape = cursor_shape_from_settings(settings);
        let term = Term::new(
            Config {
                scrolling_history: 1000,
                default_cursor_style: ansi::CursorStyle {
                    shape: cursor_shape,
                    blinking: true,
                },
                ..Config::default()
            },
            &SimpleDimensions::new(render::DEFAULT_COLS, render::DEFAULT_ROWS),
            proxy,
        );
        let font_size: f32 = settings.font_size;
        let font_family = settings.font_family.clone();
        let scroll_handle = ScrollHandle::new();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let program_name = Path::new(&shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        let mut view = Self {
            focus_handle,
            parser: ansi::Processor::new(),
            term,
            master: None,
            writer: None,
            _child: None,
            rows: render::DEFAULT_ROWS,
            cols: render::DEFAULT_COLS,
            font_size,
            font_family,
            cell_width: render::DEFAULT_CELL_WIDTH,
            cell_height: render::DEFAULT_CELL_HEIGHT,
            scroll_handle,
            program_name,
            last_bounds: None,
            title: None,
            error: None,
            keystroke_sub: None,
            cursor_visible: true,
            cell_metrics_measured: false,
            selection_start: None,
            selection_end: None,
            is_selecting: false,
            cursor_blink_enabled: settings.cursor_blink,
            cursor_shape,
        };

        view.install_tab_passthrough(cx);
        view.spawn_message_loop(rx, cx);
        view.start_pty(tx, cx);
        view.apply_cursor_shape(cursor_shape);
        view.start_cursor_blink(cx);
        view
    }

    fn install_tab_passthrough(&mut self, cx: &mut Context<Self>) {
        let focus = self.focus_handle.clone();
        let weak = cx.entity().downgrade();
        self.keystroke_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.key == "tab"
                && window
                    .focused(cx)
                    .is_some_and(|focused| focused.contains(&focus, window))
            {
                if let Some(entity) = weak.upgrade() {
                    let _ = entity.update(cx, |view, _cx| {
                        view.cursor_visible = true;
                        if event.keystroke.modifiers.shift {
                            view.write_bytes(b"\x1b[Z");
                        } else {
                            view.write_bytes(b"\t");
                        }
                    });
                }
                cx.stop_propagation();
            }
        }));
    }

    fn handle_msg(&mut self, msg: TerminalMsg, cx: &mut Context<Self>) {
        match msg {
            TerminalMsg::Output(bytes) => {
                let stick_to_bottom = self.is_scrolled_to_bottom();
                self.parser.advance(&mut self.term, &bytes);
                self.cursor_visible = true;
                if stick_to_bottom {
                    self.scroll_handle.scroll_to_bottom();
                }
                cx.notify();
            }
            TerminalMsg::Event(event) => self.handle_event(event, cx),
        }
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::PtyWrite(text) => {
                self.write_bytes(text.as_bytes());
            }
            Event::TextAreaSizeRequest(formatter) => {
                if let Some(size) = self.current_window_size() {
                    let seq = formatter(size);
                    self.write_bytes(seq.as_bytes());
                }
            }
            Event::Title(title) => {
                self.title = Some(title);
            }
            Event::ResetTitle => {
                self.title = None;
            }
            Event::ChildExit(code) => {
                self.error
                    .get_or_insert_with(|| format!("Process exited with code {code}"));
            }
            Event::Wakeup => cx.notify(),
            _ => {}
        }
        cx.notify();
    }

    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        cx.spawn(move |_view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let view = view.clone();
            let mut app = cx.clone();
            async move {
                loop {
                    app.background_executor()
                        .timer(Duration::from_millis(530))
                        .await;
                    if view
                        .update(&mut app, |view, cx| {
                            if view.cursor_blink_enabled {
                                view.cursor_visible = !view.cursor_visible;
                            } else {
                                view.cursor_visible = true;
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn update_cell_metrics(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.cell_metrics_measured {
            return;
        }

        let text: SharedString = "W".into();
        let runs = vec![TextRun {
            len: text.len(),
            font: font(self.font_family.clone()),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];

        if let Ok(lines) =
            window
                .text_system()
                .shape_text(text, px(self.font_size), &runs, None, None)
        {
            if let Some(line) = lines.first() {
                let measured_width: f32 = line.width().into();
                let ascent: f32 = line.ascent().into();
                let descent: f32 = line.descent().into();
                let measured_height = ascent + descent;

                if measured_width > 0.0 {
                    self.cell_width = measured_width;
                }
                if measured_height > 0.0 {
                    self.cell_height = measured_height;
                }

                self.cell_metrics_measured = true;
            }
        }
    }

    fn write_bytes(&self, bytes: &[u8]) {
        if let Some(writer) = &self.writer {
            let _ = writer.lock().write_all(bytes);
        }
    }

    fn record_bounds(&mut self, bounds: Bounds<gpui::Pixels>, cx: &mut Context<Self>) {
        self.last_bounds = Some(bounds);
        self.update_dimensions(bounds, cx);
    }

    fn update_dimensions(&mut self, bounds: Bounds<gpui::Pixels>, cx: &mut Context<Self>) {
        let width: f32 = bounds.size.width.into();
        let height: f32 = bounds.size.height.into();

        let cols = (width / self.cell_width).floor().max(1.0) as usize;
        let rows = (height / self.cell_height).floor().max(1.0) as usize;

        if cols == self.cols && rows == self.rows {
            return;
        }

        self.cols = cols;
        self.rows = rows;
        self.term.resize(SimpleDimensions::new(cols, rows));

        if let Some(master) = &self.master {
            let _ = master.resize(portable_pty::PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: width as u16,
                pixel_height: height as u16,
            });
        }

        cx.notify();
    }

    fn current_window_size(&self) -> Option<WindowSize> {
        if self.rows == 0 || self.cols == 0 {
            return None;
        }

        Some(WindowSize {
            num_lines: self.rows as u16,
            num_cols: self.cols as u16,
            cell_width: self.cell_width as u16,
            cell_height: self.cell_height as u16,
        })
    }

    fn is_scrolled_to_bottom(&self) -> bool {
        let offset = self.scroll_handle.offset();
        let offset_y: f32 = offset.y.into();
        let max_offset = self.scroll_handle.max_offset();
        let max_y: f32 = (-max_offset.height).into();

        (offset_y - max_y).abs() < 1.0 || offset_y < max_y
    }

    pub fn tab_title(&self) -> String {
        let program = self
            .title
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.program_name);
        program.to_string()
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: &AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (self.font_size - settings.font_size).abs() > f32::EPSILON {
            self.font_size = settings.font_size;
            self.cell_metrics_measured = false;
            self.update_cell_metrics(window, cx);
            if let Some(bounds) = self.last_bounds {
                self.update_dimensions(bounds, cx);
            }
        }

        if self.font_family != settings.font_family {
            self.font_family = settings.font_family.clone();
            self.cell_metrics_measured = false;
            self.update_cell_metrics(window, cx);
            if let Some(bounds) = self.last_bounds {
                self.update_dimensions(bounds, cx);
            }
        }

        self.cursor_blink_enabled = settings.cursor_blink;
        if !self.cursor_blink_enabled {
            self.cursor_visible = true;
        }

        let shape = cursor_shape_from_settings(settings);
        if self.cursor_shape != shape {
            self.apply_cursor_shape(shape);
            cx.notify();
        }
    }

    fn apply_cursor_shape(&mut self, shape: ansi::CursorShape) {
        self.cursor_shape = shape;
        let code = match shape {
            ansi::CursorShape::Underline => 4,
            ansi::CursorShape::Beam => 6,
            _ => 2,
        };
        let seq = format!("\u{1b}[{code} q");
        self.parser.advance(&mut self.term, seq.as_bytes());
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.unbounded_send(TerminalMsg::Event(event));
    }
}

#[cfg(test)]
mod tests {
    use super::pty::normalize_term_env;
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn normalizes_missing_term() {
        assert_eq!(normalize_term_env(None), OsString::from("xterm-256color"));
        assert_eq!(
            normalize_term_env(Some(OsString::from(""))),
            OsString::from("xterm-256color")
        );
    }

    #[test]
    fn preserves_existing_term() {
        assert_eq!(
            normalize_term_env(Some(OsString::from("alacritty"))),
            OsString::from("alacritty")
        );
    }

    #[test]
    fn maps_cursor_shape() {
        assert_eq!(
            cursor_shape_from_settings(&AppSettings {
                cursor_shape: CursorShapeSetting::Block,
                ..Default::default()
            }),
            ansi::CursorShape::Block
        );
        assert_eq!(
            cursor_shape_from_settings(&AppSettings {
                cursor_shape: CursorShapeSetting::Beam,
                ..Default::default()
            }),
            ansi::CursorShape::Beam
        );
        assert_eq!(
            cursor_shape_from_settings(&AppSettings {
                cursor_shape: CursorShapeSetting::Underline,
                ..Default::default()
            }),
            ansi::CursorShape::Underline
        );
    }
}

fn cursor_shape_from_settings(settings: &AppSettings) -> ansi::CursorShape {
    match settings.cursor_shape {
        CursorShapeSetting::Beam => ansi::CursorShape::Beam,
        CursorShapeSetting::Underline => ansi::CursorShape::Underline,
        _ => ansi::CursorShape::Block,
    }
}
