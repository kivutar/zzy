use super::{TerminalMsg, TerminalView};
use anyhow::Context as _;
use futures::{channel::mpsc, StreamExt};
use gpui::{AsyncApp, Context, WeakEntity};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    ffi::OsString,
    io::{Read, Write},
    sync::Arc,
    thread,
};

impl TerminalView {
    pub(crate) fn start_pty(
        &mut self,
        tx: mpsc::UnboundedSender<TerminalMsg>,
        cx: &mut Context<Self>,
    ) {
        match self.open_pty() {
            Ok((master, writer, reader, child)) => {
                self.master = Some(master);
                self.writer = Some(Arc::new(parking_lot::Mutex::new(writer)));
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

    pub(crate) fn spawn_message_loop(
        &self,
        rx: mpsc::UnboundedReceiver<TerminalMsg>,
        cx: &mut Context<Self>,
    ) {
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
}

pub(crate) fn normalize_term_env(term: Option<OsString>) -> OsString {
    match term {
        Some(value) if !value.is_empty() => value,
        _ => OsString::from("xterm-256color"),
    }
}
