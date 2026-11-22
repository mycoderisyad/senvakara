//! Local interactive shell sessions backed by a pseudo-terminal
//! (ConPTY on Windows). Requirement TERM-01 / task POC-03, CORE-04.
//!
//! Commands are built as executable + argument list; no shell string
//! interpolation (docs/SECURITY.md §6).

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;

/// Which local shell to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellKind {
    /// Windows PowerShell (`powershell.exe`).
    PowerShell,
    /// A WSL distribution; `None` uses the user's default distro.
    Wsl { distro: Option<String> },
    /// Command Prompt (`cmd.exe`).
    Cmd,
    /// PowerShell 7 or later: the full path of its `pwsh.exe`.
    Pwsh { path: PathBuf },
    /// Git Bash: the full path of Git for Windows' `bash.exe`, started as an
    /// interactive login shell.
    GitBash { path: PathBuf },
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
            ShellKind::Cmd => CommandBuilder::new("cmd.exe"),
            ShellKind::Pwsh { path } => {
                let mut c = CommandBuilder::new(path);
                c.arg("-NoLogo");
                c
            }
            ShellKind::GitBash { path } => {
                let mut c = CommandBuilder::new(path);
                c.args(["--login", "-i"]);
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
    writer: Option<Box<dyn Write + Send>>,
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
            writer: Some(writer),
        })
    }

    /// Independent reader for the output stream (run it on its own thread).
    pub fn reader(&self) -> Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader().context("clone pty reader")
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<()> {
        let writer = self
            .writer
            .as_mut()
            .context("pty writer was taken by take_writer")?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Hand the input stream to a dedicated writer (thread), so a write that
    /// blocks on a full pipe cannot hold up `resize` or `kill`. Afterwards
    /// `write_input` fails; returns `None` if already taken.
    pub fn take_writer(&mut self) -> Option<Box<dyn Write + Send>> {
        self.writer.take()
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

    fn argv(kind: ShellKind, extra: &[&str]) -> Vec<String> {
        kind.command(extra)
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn wsl_command_uses_discrete_args() {
        let kind = ShellKind::Wsl {
            distro: Some("Ubuntu-24.04".into()),
        };
        assert_eq!(
            argv(kind, &["--", "uname"]),
            ["wsl.exe", "-d", "Ubuntu-24.04", "--", "uname"]
        );
    }

    #[test]
    fn git_bash_starts_as_an_interactive_login_shell() {
        let path = r"C:\Program Files\Git\bin\bash.exe";
        let kind = ShellKind::GitBash { path: path.into() };
        // A path with a space stays one argument: nothing is joined into a shell string.
        assert_eq!(argv(kind, &[]), [path, "--login", "-i"]);
    }

    #[test]
    fn pwsh_and_cmd_run_their_own_executable() {
        let path = r"C:\Program Files\PowerShell\7\pwsh.exe";
        let kind = ShellKind::Pwsh { path: path.into() };
        assert_eq!(argv(kind, &[]), [path, "-NoLogo"]);
        assert_eq!(argv(ShellKind::Cmd, &[]), ["cmd.exe"]);
    }
}
