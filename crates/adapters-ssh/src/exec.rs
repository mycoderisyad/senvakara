//! Streaming exec for workflow scripts (WF-SPIKE-02, ADR-030): stdin then
//! EOF, stdout and stderr kept apart, a deadline and a cancel switch.
//!
//! What the spike proved and this code keeps to:
//! - Stdin always ends with `CHANNEL_EOF` (`eof()`), never `close()`: a
//!   closed channel loses the exit status and its reader hangs.
//! - Stopping a remote program is the caller's `stop` (a kill of its process
//!   group through a second exec). Closing the channel or an SSH `signal`
//!   leaves orphans that keep the channel open, so they are not used.
//! - After a stop the channel is drained for a bounded time only; the end is
//!   then `TimedOut` or `Cancelled`, whatever exit code arrives, because the
//!   code differs between sudo and no sudo.

use crate::{shell_quote, SshConnection};
use anyhow::Result;
use russh::ChannelMsg;
use senvakara_core::{CoreError, ErrorCode};
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

/// A chunk of output, in the order the server sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

/// How a streamed program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecEnd {
    Exited(u32),
    /// Ended by a signal the server reported (`TERM`, `KILL`, ...).
    Signalled(String),
    /// The deadline passed; `stop` was called.
    TimedOut,
    /// The cancel switch was set; `stop` was called.
    Cancelled,
    /// The channel or connection went away without an exit status: the
    /// remote outcome is unknown and must not be retried blindly.
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecReport {
    pub end: ExecEnd,
    pub stdout_total: u64,
    pub stderr_total: u64,
}

/// Limits of one streamed exec.
#[derive(Debug, Clone, Copy)]
pub struct ExecLimits {
    /// From start to the stop.
    pub timeout: Duration,
    /// How long output and the exit status are still collected after a stop.
    pub drain: Duration,
}

impl SshConnection {
    /// Runs `argv` (each argument POSIX-quoted), writes `stdin` and then EOF
    /// (EOF also when there is no stdin), and hands every output chunk to
    /// `on_event` as it arrives. At the deadline or when `cancel` turns true
    /// it awaits `stop` (which must end the remote program), then drains for
    /// at most `limits.drain`.
    ///
    /// `stdin` may hold a sudo password: it is never logged here, but russh
    /// keeps its own copy of sent bytes that is not wiped (ADR-030). Output
    /// size limits are the caller's: `on_event` sees everything.
    ///
    /// Fails when the channel cannot be opened or the command not sent, with
    /// a `Timeout` error when that takes longer than `limits.timeout` (a
    /// stalled connection); a connection lost later is `ExecEnd::Lost`.
    pub async fn exec_stream<S, F>(
        &self,
        argv: &[&str],
        stdin: Option<&[u8]>,
        limits: ExecLimits,
        mut cancel: watch::Receiver<bool>,
        mut on_event: impl FnMut(ExecEvent),
        stop: S,
    ) -> Result<ExecReport>
    where
        S: FnOnce() -> F,
        F: Future<Output = ()>,
    {
        let command = argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let start = async {
            let channel = self.handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            let (reader, writer) = channel.split();
            if let Some(bytes) = stdin {
                writer.data_bytes(bytes.to_vec()).await?;
            }
            writer.eof().await?;
            anyhow::Ok(reader)
        };
        let mut reader = tokio::time::timeout(limits.timeout, start)
            .await
            .map_err(|_| {
                CoreError::new(
                    ErrorCode::Timeout,
                    "the host did not start the command in time",
                )
            })??;

        let mut report = ExecReport {
            end: ExecEnd::Lost,
            stdout_total: 0,
            stderr_total: 0,
        };
        let mut exit: Option<ExecEnd> = None;
        let mut stopped: Option<ExecEnd> = None;
        let mut stop = Some(stop);
        let deadline = tokio::time::sleep(limits.timeout);
        tokio::pin!(deadline);
        let mut drain_until: Option<tokio::time::Instant> = None;
        let mut can_cancel = true;
        // Already cancelled before the start: stop at the first turn.
        if *cancel.borrow() {
            deadline.as_mut().reset(tokio::time::Instant::now());
            stopped = Some(ExecEnd::Cancelled);
        }

        loop {
            let msg = match drain_until {
                Some(until) => match tokio::time::timeout_at(until, reader.wait()).await {
                    Ok(m) => m,
                    Err(_) => break,
                },
                None => {
                    let why = tokio::select! {
                        m = reader.wait() => {
                            match m {
                                Some(m) => {
                                    take(m, &mut report, &mut exit, &mut on_event);
                                    continue;
                                }
                                None => break,
                            }
                        }
                        _ = &mut deadline => ExecEnd::TimedOut,
                        changed = cancel.changed(), if can_cancel => {
                            match changed {
                                Ok(()) if *cancel.borrow() => ExecEnd::Cancelled,
                                Ok(()) => continue,
                                // Sender gone: nobody can cancel any more.
                                Err(_) => {
                                    can_cancel = false;
                                    continue;
                                }
                            }
                        }
                    };
                    stopped = Some(why);
                    if let Some(stop) = stop.take() {
                        stop().await;
                    }
                    drain_until = Some(tokio::time::Instant::now() + limits.drain);
                    continue;
                }
            };
            match msg {
                Some(m) => take(m, &mut report, &mut exit, &mut on_event),
                None => break,
            }
        }
        report.end = stopped.or(exit).unwrap_or(ExecEnd::Lost);
        Ok(report)
    }
}

fn take(
    msg: ChannelMsg,
    report: &mut ExecReport,
    exit: &mut Option<ExecEnd>,
    on_event: &mut impl FnMut(ExecEvent),
) {
    match msg {
        ChannelMsg::Data { data } => {
            report.stdout_total += data.len() as u64;
            on_event(ExecEvent::Stdout(data.to_vec()));
        }
        // Only ext 1 (stderr) exists in practice; anything else is treated as
        // stderr too rather than dropped.
        ChannelMsg::ExtendedData { data, .. } => {
            report.stderr_total += data.len() as u64;
            on_event(ExecEvent::Stderr(data.to_vec()));
        }
        ChannelMsg::ExitStatus { exit_status } => *exit = Some(ExecEnd::Exited(exit_status)),
        ChannelMsg::ExitSignal { signal_name, .. } => {
            *exit = Some(ExecEnd::Signalled(format!("{signal_name:?}")))
        }
        _ => {}
    }
}
