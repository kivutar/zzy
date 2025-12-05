use std::{
    io::{Read, Write},
    sync::Arc,
    thread,
};

use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    grid::Scroll,
    term::{cell::Flags, Config, Term},
    vte::ansi,
};
use anyhow::Context as _;
use futures::{channel::mpsc, StreamExt};
use gpui::{
    canvas, div, px, rgb, AsyncApp, Bounds, Context, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, ParentElement, Render,
    ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement, Styled, WeakEntity, Window,
};
use gpui_component::v_flex;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;
const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_CELL_WIDTH: f32 = 8.0;
const DEFAULT_CELL_HEIGHT: f32 = 18.0;

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
    last_bounds: Option<Bounds<gpui::Pixels>>,
    title: Option<String>,
    error: Option<String>,
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
            Config::default(),
            &SimpleDimensions::new(DEFAULT_COLS, DEFAULT_ROWS),
            proxy,
        );

        let mut view = Self {
            focus_handle,
            parser: ansi::Processor::new(),
            term,
            master: None,
            writer: None,
            _child: None,
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            font_size: DEFAULT_FONT_SIZE,
            cell_width: DEFAULT_CELL_WIDTH,
            cell_height: DEFAULT_CELL_HEIGHT,
            last_bounds: None,
            title: None,
            error: None,
        };

        view.spawn_message_loop(rx, cx);
        view.start_pty(tx, cx);
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

    fn handle_msg(&mut self, msg: TerminalMsg, cx: &mut Context<Self>) {
        match msg {
            TerminalMsg::Output(bytes) => {
                self.parser.advance(&mut self.term, &bytes);
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

    fn visible_text(&self) -> Vec<String> {
        let mut grid = vec![vec![String::from(" "); self.cols]; self.rows];
        let content = self.term.renderable_content();

        for cell in content.display_iter {
            let line = cell.point.line.0;
            if line < 0 {
                continue;
            }
            let line_idx = line as usize;
            if line_idx >= self.rows {
                continue;
            }
            let col = cell.point.column.0 as usize;
            if col >= self.cols {
                continue;
            }

            if cell.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }

            let mut ch = String::new();
            ch.push(cell.cell.c);
            if let Some(extra) = cell.cell.zerowidth() {
                for c in extra {
                    ch.push(*c);
                }
            }
            grid[line_idx][col] = ch;
        }

        grid.into_iter()
            .map(|row| row.into_iter().collect::<String>())
            .collect()
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            self.write_bytes(&bytes);
            cx.notify();
        }
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y as isize,
            ScrollDelta::Pixels(delta) => {
                let dy: f32 = delta.y.into();
                (dy / self.cell_height).round() as isize
            }
        };

        if lines != 0 {
            let clamped = lines.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
            self.term.scroll_display(Scroll::Delta(clamped));
            cx.notify();
        }
    }

    fn focus_on_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        cx.notify();
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.visible_text();
        let error = self.error.clone();

        let view = cx.entity();
        div()
            .id("terminal-view")
            .flex_1()
            .w_full()
            .h_full()
            .text_color(rgb(0xffffff))
            .font_family(".ZedMono")
            .text_size(px(self.font_size))
            .line_height(px(self.cell_height))
            .whitespace_nowrap()
            .cursor_text()
            .overflow_scroll()
            .track_focus(&self.focus_handle)
            .focusable()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_scroll_wheel(cx.listener(Self::handle_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::focus_on_click))
            .child(
                canvas(
                    {
                        let view = view.clone();
                        move |bounds, _, cx| {
                            let _ = view.update(cx, |view, cx| view.record_bounds(bounds, cx));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child({
                let mut lines_view = v_flex().id("terminal-lines").w_full();
                if let Some(err) = error {
                    lines_view = lines_view.child(
                        div()
                            .w_full()
                            .whitespace_nowrap()
                            .text_size(px(self.font_size))
                            .line_height(px(self.cell_height))
                            .font_family(".ZedMono")
                            .child(err),
                    );
                }
                for line in lines.into_iter() {
                    lines_view = lines_view.child(
                        div()
                            .w_full()
                            .whitespace_nowrap()
                            .text_size(px(self.font_size))
                            .line_height(px(self.cell_height))
                            .font_family(".ZedMono")
                            .child(line),
                    );
                }
                lines_view
            })
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
