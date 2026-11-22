//! What the foreground program of a tab is doing, so the header can be tinted
//! like Ptyxis and Black Box: red while running as root, green during SSH.
//!
//! Linux only: the PTY's foreground process group is inspected via `/proc`.
//! Elsewhere every tab is reported as `Normal`.

use super::TerminalView;
use crate::colors::rgba_hex;
use gpui_kit::{Context, Hsla};
use std::time::Duration;

/// How often the foreground process is checked.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessContext {
    #[default]
    Normal,
    /// The foreground program runs as root (sudo, su, ...) while we don't.
    Root,
    /// The foreground program is a remote shell client.
    Remote,
}

impl ProcessContext {
    /// Tint color for this context (GNOME palette, as Ptyxis uses).
    pub fn color(self) -> Option<Hsla> {
        let hex = match self {
            Self::Normal => return None,
            Self::Root => 0xe01b24ff,   // red 3
            Self::Remote => 0x26a269ff, // green 4
        };
        Some(rgba_hex(hex).into())
    }
}

const REMOTE_CLIENTS: &[&str] = &["ssh", "mosh-client", "et"];

impl TerminalView {
    pub fn process_context(&self) -> ProcessContext {
        self.process_context
    }

    pub(crate) fn start_context_polling(&mut self, cx: &mut Context<Self>) {
        let our_uid = effective_uid("self");
        cx.spawn(async move |view, cx| loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let alive = view.update(cx, |view, cx| {
                let context = view
                    .master
                    .as_ref()
                    .and_then(|master| master.process_group_leader())
                    .map(|pid| detect(&pid.to_string(), our_uid))
                    .unwrap_or_default();
                if context != view.process_context {
                    view.process_context = context;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }
}

fn detect(pid: &str, our_uid: Option<u32>) -> ProcessContext {
    if our_uid != Some(0) && effective_uid(pid) == Some(0) {
        return ProcessContext::Root;
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    if REMOTE_CLIENTS.contains(&comm.trim()) {
        ProcessContext::Remote
    } else {
        ProcessContext::Normal
    }
}

/// Effective UID from `/proc/<pid>/status` (`Uid: real effective saved fs`).
fn effective_uid(pid: &str) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_effective_uid(&status)
}

fn parse_effective_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_effective_uid() {
        let status = "Name:\tsudo\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_effective_uid(status), Some(0));
        assert_eq!(parse_effective_uid("Name:\tbash\n"), None);
    }

    #[test]
    fn detects_our_own_shell_as_normal() {
        let our_uid = effective_uid("self");
        assert!(our_uid.is_some());
        assert_eq!(detect("self", our_uid), ProcessContext::Normal);
    }
}
