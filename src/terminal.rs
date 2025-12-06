use crate::colors::Palette;
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::Path,
    sync::Arc,
    thread,
    time::Duration,
};

use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    index::{Column, Line},
    term::{cell::Flags, Config, Term},
    vte::ansi,
};
use anyhow::Context as _;
use futures::{channel::mpsc, StreamExt};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    canvas, div, font, px, AsyncApp, Bounds, Context, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    ParentElement, Render, Rgba, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, TextRun, WeakEntity, Window,
};
use gpui_component::{h_flex, v_flex, ActiveTheme};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;
const DEFAULT_CELL_WIDTH: f32 = 8.0;
const DEFAULT_CELL_HEIGHT: f32 = 24.0;
const DEFAULT_FONT_SIZE: f32 = 15.0;

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

enum TerminalMsg {
    Output(Vec<u8>),
    Event(Event),
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

impl alacritty_terminal::grid::Dimensions for SimpleDimensions {
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
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let (tx, rx) = mpsc::unbounded();
        let proxy = TerminalEventProxy { tx: tx.clone() };
        let term = Term::new(
            Config {
                scrolling_history: 1000,
                ..Config::default()
            },
            &SimpleDimensions::new(DEFAULT_COLS, DEFAULT_ROWS),
            proxy,
        );
        let font_size: f32 = DEFAULT_FONT_SIZE;
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
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            font_size,
            cell_width: DEFAULT_CELL_WIDTH,
            cell_height: DEFAULT_CELL_HEIGHT,
            scroll_handle,
            program_name,
            last_bounds: None,
            title: None,
            error: None,
            keystroke_sub: None,
            cursor_visible: true,
            cell_metrics_measured: false,
        };

        let focus = view.focus_handle.clone();
        let weak = cx.entity().downgrade();
        view.keystroke_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.key == "tab"
                && window
                    .focused(cx)
                    .is_some_and(|focused| focused.contains(&focus, window))
            {
                if let Some(entity) = weak.upgrade() {
                    let _ = entity.update(cx, |view, _cx| {
                        // Send Tab (or back-tab) directly to the PTY when the terminal is focused.
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

        view.spawn_message_loop(rx, cx);
        view.start_pty(tx, cx);
        view.start_cursor_blink(cx);
        view
    }

    fn start_pty(&mut self, tx: mpsc::UnboundedSender<TerminalMsg>, cx: &mut Context<Self>) {
        match self.open_pty() {
            Ok((master, writer, reader, child)) => {
                self.master = Some(master);
                self.writer = Some(Arc::new(Mutex::new(writer)));
                self._child = Some(child);
                self.spawn_reader_thread(reader, tx);
                cx.notify();
            }
            Err(err) => {
                self.error = Some(format!("Failed to start terminal: {err:#}"));
                cx.notify();
            }
        }
    }

    fn open_pty(
        &self,
    ) -> anyhow::Result<(
        Box<dyn MasterPty + Send>,
        Box<dyn Write + Send>,
        Box<dyn Read + Send>,
        Box<dyn Child + Send + Sync>,
    )> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows: self.rows as u16,
            cols: self.cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system.openpty(size)?;
        let writer = pair
            .master
            .take_writer()
            .context("failed to take pty writer")?;
        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone pty reader")?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(shell);
        // When the app is launched from a desktop launcher, TERM is usually missing.
        // Provide a sensible default so tools like git/less do not warn about a
        // non-functional terminal.
        cmd.env("TERM", normalize_term_env(std::env::var_os("TERM")));
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        let child = pair.slave.spawn_command(cmd)?;

        Ok((pair.master, writer, reader, child))
    }

    fn spawn_reader_thread(
        &self,
        mut reader: Box<dyn Read + Send>,
        tx: mpsc::UnboundedSender<TerminalMsg>,
    ) {
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx
                            .unbounded_send(TerminalMsg::Output(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        eprintln!("pty read error: {err:?}");
                        break;
                    }
                }
            }
        });
    }

    fn spawn_message_loop(&self, rx: mpsc::UnboundedReceiver<TerminalMsg>, cx: &mut Context<Self>) {
        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut rx = rx;
            let mut app = cx.clone();
            async move {
                while let Some(msg) = rx.next().await {
                    let _ = view.update(&mut app, |view, cx| view.handle_msg(msg, cx));
                }
            }
        })
        .detach();
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
                            view.cursor_visible = !view.cursor_visible;
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

    fn update_cell_metrics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cell_metrics_measured {
            return;
        }

        let text: SharedString = "W".into();
        let runs = vec![TextRun {
            len: text.len(),
            font: font(cx.theme().mono_font_family.clone()),
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
            let _ = master.resize(PtySize {
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

    pub fn tab_title(&self) -> String {
        let program = self
            .title
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.program_name);
        program.to_string()
    }

    fn is_scrolled_to_bottom(&self) -> bool {
        let offset = self.scroll_handle.offset();
        let offset_y: f32 = offset.y.into();
        let max_offset = self.scroll_handle.max_offset();
        let max_y: f32 = (-max_offset.height).into();

        (offset_y - max_y).abs() < 1.0 || offset_y < max_y
    }

    fn buffer_segments(&self) -> (Vec<Vec<CellRender>>, Palette) {
        let content = self.term.renderable_content();
        let palette = Palette::new(content.colors);
        let cursor = content.cursor;
        let grid_ref = self.term.grid();
        let total_lines = grid_ref.total_lines();
        let screen_lines = grid_ref.screen_lines();
        let history_size = total_lines.saturating_sub(screen_lines);
        let cols = grid_ref.columns();

        let mut grid = vec![
            vec![
                CellRender {
                    text: " ".into(),
                    fg: palette.default_fg,
                    bg: palette.default_bg,
                    is_cursor: false,
                    is_bold: false,
                };
                cols
            ];
            total_lines
        ];

        for line_idx in 0..total_lines {
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
                grid[line_idx][col_idx] = CellRender {
                    text: ch,
                    fg,
                    bg,
                    is_cursor,
                    is_bold,
                };
            }
        }

        let segments = grid
            .into_iter()
            .map(|row| {
                let mut segments = Vec::new();
                let mut current: Option<CellRender> = None;

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

        (segments, palette)
    }

    fn handle_key_down(
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

    fn focus_on_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        cx.notify();
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_cell_metrics(window, cx);

        let error = self.error.clone();
        let (segments_by_line, palette) = self.buffer_segments();
        let scrollbar = {
            let max_offset = self.scroll_handle.max_offset();
            self.last_bounds.and_then(|bounds| {
                let viewport_height: f32 = bounds.size.height.into();
                let scroll_max: f32 = max_offset.height.into();
                if scroll_max <= 0.5 || viewport_height <= 0.5 {
                    return None;
                }

                let content_height = viewport_height + scroll_max;
                let thumb_height = (viewport_height / content_height * viewport_height)
                    .clamp(20.0, viewport_height);
                let track_height = (viewport_height - thumb_height).max(0.0);
                let offset_y: f32 = (-self.scroll_handle.offset().y).into();
                let progress = (offset_y / scroll_max).clamp(0.0, 1.0);
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
            .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_on_click))
            .child({
                let scroll_area = div()
                    .id("terminal-scroll")
                    .w_full()
                    .h_full()
                    .font_family(cx.theme().mono_font_family.clone())
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
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(err),
                    );
                }
                for (idx, segments) in segments_by_line.into_iter().enumerate() {
                    let mut line_row = h_flex()
                        .id(("line", idx as u32))
                        .w_full()
                        .whitespace_nowrap()
                        .text_size(px(self.font_size))
                        .line_height(px(self.cell_height))
                        .font_family(cx.theme().mono_font_family.clone());

                    for seg in segments {
                        let show_cursor = seg.is_cursor && self.cursor_visible;
                        line_row = line_row.child(
                            div()
                                .whitespace_nowrap()
                                .when(seg.is_bold, |this| this.font_weight(FontWeight::BOLD))
                                .text_color(if show_cursor {
                                    palette.cursor_color()
                                } else {
                                    seg.fg
                                })
                                .bg(if show_cursor {
                                    palette.cursor_bg()
                                } else {
                                    seg.bg
                                })
                                .child(seg.text),
                        );
                    }

                    lines_view = lines_view.child(line_row);
                }

                scroll_area.child(lines_view)
            });

        if let Some(scrollbar) = scrollbar {
            container = container.child(scrollbar);
        }

        container
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

fn normalize_term_env(term: Option<OsString>) -> OsString {
    match term {
        Some(value) if !value.is_empty() => value,
        _ => OsString::from("xterm-256color"),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
