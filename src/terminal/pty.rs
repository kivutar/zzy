//! The shell runs in a PTY driven by alacritty's `EventLoop` on its own
//! thread: it reads the output, parses it straight into the shared `Term`
//! (behind a `FairMutex`) and writes our input. The UI thread locks the `Term`
//! to render and handle input, and receives events (wakeup, title, bell,
//! exit...) through `TerminalEventProxy`, so heavy output never blocks the UI.

use super::TerminalEventProxy;
use crate::settings::AppSettings;
use alacritty_terminal::{
    event::WindowSize,
    event_loop::{EventLoop, EventLoopSender, Msg},
    sync::FairMutex,
    term::Term,
    tty,
};
use anyhow::Context as _;
use std::{borrow::Cow, collections::HashMap, path::PathBuf, sync::Arc};

pub(crate) struct Pty {
    sender: EventLoopSender,
    /// The shell's process id (for its working directory).
    #[cfg(unix)]
    shell_pid: u32,
    /// Our own handle on the PTY master, to ask which process group is in the
    /// foreground. A duplicate, so it stays valid however long we keep it.
    #[cfg(unix)]
    master: std::fs::File,
}

impl Pty {
    pub(crate) fn spawn(
        term: Arc<FairMutex<Term<TerminalEventProxy>>>,
        proxy: TerminalEventProxy,
        argv: &[String],
        cwd: Option<PathBuf>,
        size: WindowSize,
    ) -> anyhow::Result<Self> {
        let (program, args) = argv.split_first().context("empty command")?;
        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), args.to_vec())),
            working_directory: cwd
                .filter(|dir| dir.is_dir())
                .or_else(|| std::env::current_dir().ok()),
            drain_on_exit: true,
            // alacritty emulates an xterm-compatible terminal; don't pass on
            // the TERM zzy itself was started with (e.g. xterm-kitty).
            env: HashMap::from([
                ("TERM".to_string(), "xterm-256color".to_string()),
                ("COLORTERM".to_string(), "truecolor".to_string()),
            ]),
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let pty = tty::new(&options, size, 0).context("failed to open a PTY")?;
        #[cfg(unix)]
        let shell_pid = pty.child().id();
        #[cfg(unix)]
        let master = pty
            .file()
            .try_clone()
            .context("failed to clone the PTY master")?;

        let event_loop = EventLoop::new(term, proxy, pty, options.drain_on_exit, false)
            .context("failed to start the PTY event loop")?;
        let sender = event_loop.channel();
        event_loop.spawn();

        Ok(Self {
            sender,
            #[cfg(unix)]
            shell_pid,
            #[cfg(unix)]
            master,
        })
    }

    pub(crate) fn write(&self, bytes: &[u8]) {
        let _ = self.sender.send(Msg::Input(Cow::Owned(bytes.to_vec())));
    }

    pub(crate) fn resize(&self, size: WindowSize) {
        let _ = self.sender.send(Msg::Resize(size));
    }

    #[cfg(unix)]
    pub(crate) fn shell_pid(&self) -> u32 {
        self.shell_pid
    }

    /// The PTY's foreground process group (its leader's pid), e.g. `sudo` or
    /// `ssh` while they run, the shell otherwise.
    #[cfg(unix)]
    pub(crate) fn foreground_process_group(&self) -> Option<i32> {
        use std::os::fd::AsRawFd;
        // SAFETY: `master` is an open descriptor we own for as long as `self`.
        let pgid = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        (pgid > 0).then_some(pgid)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
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
