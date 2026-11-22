use super::{TerminalMsg, TerminalView};
use crate::settings::AppSettings;
use anyhow::Context as _;
use futures::{channel::mpsc, StreamExt};
use gpui_kit::Context;
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

        let (program, args) = self.argv.split_first().context("empty command")?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        // When the app is launched from a desktop launcher, TERM is usually missing.
        // Provide a sensible default so tools like git/less do not warn about a
        // non-functional terminal.
        cmd.env("TERM", normalize_term_env(std::env::var_os("TERM")));
        if let Some(cwd) = self
            .cwd
            .clone()
            .filter(|dir| dir.is_dir())
            .or_else(|| std::env::current_dir().ok())
        {
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
            // Linux reports the slave side closing (shell exited) as EIO, so
            // both EOF and errors mean the session is over.
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if tx
                    .unbounded_send(TerminalMsg::Output(buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.unbounded_send(TerminalMsg::Exited);
        });
    }

    pub(crate) fn spawn_message_loop(
        &self,
        mut rx: mpsc::UnboundedReceiver<TerminalMsg>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view, cx| {
            while let Some(msg) = rx.next().await {
                if view
                    .update(cx, |view, cx| view.handle_msg(msg, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}

/// The command to run: the custom command from the settings, split like a
/// shell would, or the user's `$SHELL` (with `-l` when a login shell is wanted).
pub(crate) fn command_argv(settings: &AppSettings) -> Result<Vec<String>, String> {
    let custom = settings.command.trim();
    if !custom.is_empty() {
        let argv = shell_words::split(custom).map_err(|err| format!("Invalid command: {err}"))?;
        if !argv.is_empty() {
            return Ok(argv);
        }
    }
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    let mut argv = vec![shell];
    if settings.login_shell {
        argv.push("-l".to_string());
    }
    Ok(argv)
}

pub(crate) fn normalize_term_env(term: Option<OsString>) -> OsString {
    match term {
        Some(value) if !value.is_empty() => value,
        _ => OsString::from("xterm-256color"),
    }
}
