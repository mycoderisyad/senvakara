//! Remote interactive shell over an SSH PTY channel (TERM-02, task CORE-05).
//! Split into a writer (input, resize, close) and a reader (output, exit)
//! so callers can drive them from different tasks.

use anyhow::Result;
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};

pub struct SshPtyWriter(ChannelWriteHalf<Msg>);
pub struct SshPtyReader(ChannelReadHalf);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshPtyEvent {
    Data(Vec<u8>),
    /// Remote process exited (exit status if the server sent one).
    Exit(Option<u32>),
}

impl SshPtyWriter {
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.0.data_bytes(bytes.to_vec()).await?;
        Ok(())
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.0.window_change(cols, rows, 0, 0).await?;
        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        self.0.close().await?;
        Ok(())
    }
}

impl SshPtyReader {
    /// Next output chunk or exit. `None` once the channel is fully closed.
    pub async fn next(&mut self) -> Option<SshPtyEvent> {
        loop {
            match self.0.wait().await? {
                ChannelMsg::Data { data } => return Some(SshPtyEvent::Data(data.to_vec())),
                ChannelMsg::ExtendedData { data, .. } => {
                    return Some(SshPtyEvent::Data(data.to_vec()))
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    return Some(SshPtyEvent::Exit(Some(exit_status)))
                }
                ChannelMsg::Eof | ChannelMsg::Close => return Some(SshPtyEvent::Exit(None)),
                _ => continue,
            }
        }
    }
}

pub(crate) fn split(ch: russh::Channel<Msg>) -> (SshPtyWriter, SshPtyReader) {
    let (r, w) = ch.split();
    (SshPtyWriter(w), SshPtyReader(r))
}
