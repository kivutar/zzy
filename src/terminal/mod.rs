mod input;
mod mouse;
mod pty;
mod render;

use crate::colors::Palette;
use crate::settings::{
    AppSettings, BellMode, BoldText, CursorShapeSetting, ScrollbarVisibility, FONT_SIZE_RANGE,
};
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
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct TerminalView {
    focus_handle: FocusHandle,
    parser: ansi::Processor,
    term: Term<TerminalEventProxy>,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    _child: Option<Box<dyn Child + Send + Sync>>,
    rows: usize,
    cols: usize,
    /// Effective font size: the setting plus this tab's zoom.
    font_size: f32,
    /// Font size from the settings, before zoom.
    base_font_size: f32,
    /// Per-tab zoom offset in points (Ctrl+=/-/0, Ctrl+scroll); not saved.
    zoom: f32,
    font_family: SharedString,
    line_height: f32,
    cell_width: f32,
    cell_height: f32,
    scroll_handle: ScrollHandle,
    program_name: String,
    /// Program and arguments to run in the PTY.
    argv: Vec<String>,
    /// Starting directory; `None` means zedterm's own working directory.
    cwd: Option<PathBuf>,
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
    /// Button (xterm code) reported as pressed to the program, if any.
    mouse_button_down: Option<u8>,
    last_mouse_cell: Option<CellPosition>,
    /// Fractional wheel lines carried over between (trackpad) scroll events.
    scroll_lines_remainder: f32,
    cursor_blink_enabled: bool,
    bold_text: BoldText,
    /// Padding around the view; the scrollbar thumb sits in this gutter.
    padding: f32,
    bell: BellMode,
    ignore_program_title: bool,
    last_bell: Option<Instant>,
    /// The visual bell is shown until this instant.
    flash_until: Option<Instant>,
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

/// Emitted when the bell should sound; the window plays it.
pub struct TerminalBell;

impl EventEmitter<TerminalBell> for TerminalView {}

/// Bells closer together than this are dropped (e.g. `cat` on a binary file).
const BELL_MIN_INTERVAL: Duration = Duration::from_millis(100);
const BELL_FLASH_DURATION: Duration = Duration::from_millis(150);

pub(crate) struct BufferSnapshot {
    segments_by_line: Vec<Vec<CellRender>>,
    palette: Palette,
    columns: usize,
    total_lines: usize,
    start_line: usize,
    cursor_shape: ansi::CursorShape,
    cursor_position: Option<CellPosition>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    pub fn new(cx: &mut Context<Self>, settings: &AppSettings, cwd: Option<PathBuf>) -> Self {
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
        let (argv, error) = match pty::command_argv(settings) {
            Ok(argv) => (argv, None),
            Err(err) => (
                pty::command_argv(&AppSettings::default()).unwrap_or_default(),
                Some(err),
            ),
        };
        let program_name = argv
            .first()
            .and_then(|program| Path::new(program).file_name())
            .and_then(|name| name.to_str())
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
            base_font_size: font_size,
            zoom: 0.0,
            font_family,
            line_height: settings.line_height,
            cell_width: render::DEFAULT_CELL_WIDTH,
            cell_height: render::DEFAULT_CELL_HEIGHT,
            scroll_handle,
            program_name,
            argv,
            cwd,
            last_bounds: None,
            title: None,
            error,
            keystroke_sub: None,
            cursor_visible: true,
            cell_metrics_measured: false,
            selection_start: None,
            selection_end: None,
            is_selecting: false,
            selection_started_with_single_click: false,
            selection_dragged: false,
            mouse_button_down: None,
            last_mouse_cell: None,
            scroll_lines_remainder: 0.0,
            cursor_blink_enabled: settings.cursor_blink,
            bold_text: settings.bold_text,
            padding: settings.padding,
            bell: settings.bell,
            ignore_program_title: settings.ignore_program_title,
            last_bell: None,
            flash_until: None,
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
            Event::Bell => self.ring_bell(cx),
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

    fn ring_bell(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.bell == BellMode::Off
            || self
                .last_bell
                .is_some_and(|last| now.duration_since(last) < BELL_MIN_INTERVAL)
        {
            return;
        }
        self.last_bell = Some(now);

        if self.bell.sound() {
            cx.emit(TerminalBell);
        }
        if self.bell.flash() {
            self.flash_until = Some(now + BELL_FLASH_DURATION);
            cx.notify();
            cx.spawn(async move |view, cx| {
                cx.background_executor().timer(BELL_FLASH_DURATION).await;
                view.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
    }

    pub(crate) fn bell_flash_visible(&self) -> bool {
        self.flash_until.is_some_and(|until| Instant::now() < until)
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
                    self.cell_height = measured_height * self.line_height;
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

    /// The shell's current directory (Linux: `/proc/<pid>/cwd`), used to
    /// open new tabs in the same place.
    pub fn current_dir(&self) -> Option<PathBuf> {
        let pid = self._child.as_ref()?.process_id()?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    pub fn tab_title(&self) -> String {
        let program = self
            .title
            .as_deref()
            .filter(|s| !s.is_empty() && !self.ignore_program_title)
            .unwrap_or(&self.program_name);
        program.to_string()
    }

    /// Zoom this tab by `steps` points; 0 resets to the configured size.
    pub(crate) fn zoom(&mut self, steps: f32, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = (FONT_SIZE_RANGE.0 as f32, FONT_SIZE_RANGE.1 as f32);
        self.zoom = if steps == 0.0 {
            0.0
        } else {
            // Keep the offset within what the font size can reach, so zooming
            // back doesn't first have to "unwind" clamped steps.
            (self.zoom + steps).clamp(min - self.base_font_size, max - self.base_font_size)
        };
        self.refresh_font(window, cx);
    }

    /// Re-measure cells after a font change and resize the PTY to match.
    fn refresh_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (min, max) = (FONT_SIZE_RANGE.0 as f32, FONT_SIZE_RANGE.1 as f32);
        self.font_size = (self.base_font_size + self.zoom).clamp(min, max);
        self.cell_metrics_measured = false;
        self.update_cell_metrics(window, cx);
        if let Some(bounds) = self.last_bounds {
            self.update_dimensions(bounds, cx);
        }
        cx.notify();
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: &AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.base_font_size != settings.font_size
            || self.font_family != settings.font_family
            || self.line_height != settings.line_height
        {
            self.base_font_size = settings.font_size;
            self.font_family = settings.font_family.clone();
            self.line_height = settings.line_height;
            self.refresh_font(window, cx);
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

        self.bold_text = settings.bold_text;
        self.padding = settings.padding;
        self.bell = settings.bell;
        self.ignore_program_title = settings.ignore_program_title;
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
    fn builds_command() {
        let with = |command: &str, login_shell| AppSettings {
            command: command.to_string().into(),
            login_shell,
            ..Default::default()
        };
        assert_eq!(
            pty::command_argv(&with("tmux new -s 'my session'", false)).unwrap(),
            vec!["tmux", "new", "-s", "my session"]
        );
        // Login applies to the default shell only.
        assert_eq!(
            pty::command_argv(&with("fish", true)).unwrap(),
            vec!["fish"]
        );
        let shell = pty::command_argv(&with("  ", true)).unwrap();
        assert_eq!(shell.len(), 2);
        assert_eq!(shell[1], "-l");
        assert!(pty::command_argv(&with("echo 'unterminated", false)).is_err());
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
