//! Local interactive shell sessions backed by a pseudo-terminal
//! (ConPTY on Windows). Requirement TERM-01 / task POC-03, CORE-04.
//!
//! Commands are built as executable + argument list; no shell string
//! interpolation (docs/SECURITY.md §6).

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};

/// Which local shell to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellKind {
    /// Windows PowerShell (`powershell.exe`).
    PowerShell,
    /// A WSL distribution; `None` uses the user's default distro.
    Wsl { distro: Option<String> },
}

impl ShellKind {
    /// Build the command. `extra_args` are passed as discrete arguments.
    pub fn command(&self, extra_args: &[&str]) -> CommandBuilder {
        let mut cmd = match self {
            ShellKind::PowerShell => {
                let mut c = CommandBuilder::new("powershell.exe");
                c.args(["-NoLogo", "-NoProfile"]);
                c
            }
            ShellKind::Wsl { distro } => {
                let mut c = CommandBuilder::new("wsl.exe");
                if let Some(d) = distro {
                    c.args(["-d", d.as_str()]);
                }
                c
            }
        };
        cmd.args(extra_args);
        cmd
    }
}

/// A running PTY session. Dropping it kills the child process so no
/// orphaned shells are left behind (NFR-02).
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

impl PtySession {
    pub fn spawn(kind: &ShellKind, extra_args: &[&str], cols: u16, rows: u16) -> Result<Self> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;
        let child = pair
            .slave
            .spawn_command(kind.command(extra_args))
            .with_context(|| format!("spawn {kind:?}"))?;
        // The slave handle must be released after spawn so EOF propagates.
        drop(pair.slave);
        let writer = pair.master.take_writer().context("take pty writer")?;
        Ok(Self {
            master: pair.master,
            child,
            writer,
        })
    }

    /// Independent reader for the output stream (run it on its own thread).
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader().context("clone pty reader")
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resize pty")
    }

    /// Returns `Some(exit_code)` once the child has exited.
    pub fn try_exit_code(&mut self) -> Result<Option<u32>> {
        Ok(self.child.try_wait()?.map(|s| s.exit_code()))
    }

    pub fn kill(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill().context("kill pty child")?;
        }
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_command_uses_discrete_args() {
        let cmd = ShellKind::Wsl {
            distro: Some("Ubuntu-24.04".into()),
        }
        .command(&["--", "uname"]);
        let argv: Vec<String> = cmd
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(argv, ["wsl.exe", "-d", "Ubuntu-24.04", "--", "uname"]);
    }
}
