mod input;
mod pty;
mod render;

use crate::colors::Palette;
use crate::settings::{AppSettings, CursorShapeSetting, ScrollbarVisibility};
use crate::theme::ThemeChoice;
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    term::{Config, Term},
    vte::ansi,
};
use futures::channel::mpsc;
use gpui_kit::{
    font, px, Bounds, Context, EventEmitter, FocusHandle, Focusable, Hsla, Rgba, ScrollHandle,
    SharedString, Subscription, TextRun, Window,
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
    last_bounds: Option<Bounds<gpui_kit::Pixels>>,
    title: Option<String>,
    error: Option<String>,
    keystroke_sub: Option<Subscription>,
    cursor_visible: bool,
    cell_metrics_measured: bool,
    selection_start: Option<CellPosition>,
    selection_end: Option<CellPosition>,
    is_selecting: bool,
    selection_started_with_single_click: bool,
    selection_dragged: bool,
    cursor_blink_enabled: bool,
    ignore_bold: bool,
    config: Config,
    scrollbar: ScrollbarVisibility,
    theme: ThemeChoice,
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
    /// The PTY reader hit EOF/error: the shell is gone.
    Exited,
}

/// Emitted when the shell process exits, so the owner can close the tab.
pub struct TerminalExited;

impl EventEmitter<TerminalExited> for TerminalView {}

pub(crate) struct BufferSnapshot {
    segments_by_line: Vec<Vec<CellRender>>,
    palette: Palette,
    columns: usize,
    total_lines: usize,
    start_line: usize,
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
        let config = config_from_settings(settings);
        let term = Term::new(
            config.clone(),
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
            selection_started_with_single_click: false,
            selection_dragged: false,
            cursor_blink_enabled: settings.cursor_blink,
            ignore_bold: settings.ignore_bold,
            config,
            scrollbar: settings.scrollbar,
            theme: settings.theme,
        };

        view.install_tab_passthrough(cx);
        view.spawn_message_loop(rx, cx);
        view.start_pty(tx, cx);
        view.start_cursor_blink(cx);
        view.start_scroll_invalidation(cx);
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
                    let _ = entity.update(cx, |view, cx| {
                        view.cursor_visible = true;
                        view.scroll_to_bottom_for_input(cx);
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
            TerminalMsg::Exited => cx.emit(TerminalExited),
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
            _ => {}
        }
        cx.notify();
    }

    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(530))
                .await;
            let alive = view.update(cx, |view, cx| {
                if view.cursor_blink_enabled {
                    view.cursor_visible = !view.cursor_visible;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// We virtualize the terminal buffer and `ScrollHandle::scroll_to_bottom`
    /// only lands on the next layout, so poll the offset and re-render when it
    /// moves to keep the visible slice in sync.
    fn start_scroll_invalidation(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            let mut last_offset = None;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let alive = view.update(cx, |view, cx| {
                    let offset = view.scroll_handle.offset();
                    if last_offset != Some(offset) {
                        last_offset = Some(offset);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
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

    fn record_bounds(&mut self, bounds: Bounds<gpui_kit::Pixels>, cx: &mut Context<Self>) {
        self.last_bounds = Some(bounds);
        self.update_dimensions(bounds, cx);
    }

    fn update_dimensions(&mut self, bounds: Bounds<gpui_kit::Pixels>, cx: &mut Context<Self>) {
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
        let max_y: f32 = (-max_offset.y).into();

        (offset_y - max_y).abs() < 1.0 || offset_y < max_y
    }

    fn scroll_to_bottom_for_input(&mut self, cx: &mut Context<Self>) {
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
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
        if self.font_size != settings.font_size || self.font_family != settings.font_family {
            self.font_size = settings.font_size;
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

        // The configured cursor is the terminal's *default* style: programs can
        // still override it (DECSCUSR) and resetting goes back to this one.
        let config = config_from_settings(settings);
        if config.scrolling_history != self.config.scrolling_history
            || config.default_cursor_style != self.config.default_cursor_style
        {
            self.term.set_options(config.clone());
            self.config = config;
        }

        self.ignore_bold = settings.ignore_bold;
        self.scrollbar = settings.scrollbar;
        self.theme = settings.theme;
        cx.notify();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &gpui_kit::App) -> FocusHandle {
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

fn config_from_settings(settings: &AppSettings) -> Config {
    Config {
        scrolling_history: settings.scrollback,
        default_cursor_style: ansi::CursorStyle {
            shape: cursor_shape_from_settings(settings),
            blinking: settings.cursor_blink,
        },
        ..Config::default()
    }
}

fn cursor_shape_from_settings(settings: &AppSettings) -> ansi::CursorShape {
    match settings.cursor_shape {
        CursorShapeSetting::Beam => ansi::CursorShape::Beam,
        CursorShapeSetting::Underline => ansi::CursorShape::Underline,
        _ => ansi::CursorShape::Block,
    }
}
