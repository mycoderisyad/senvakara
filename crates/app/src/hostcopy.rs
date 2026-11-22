//! Host to host (FILE-H2H-01): copy a file or a folder from one Linux host
//! to another over SFTP, streamed through this PC without a temporary copy
//! on it. Both hosts are reached over their own pinned connection; the copy
//! logic and its rules are in `senvakara_adapters_ssh::sftp`.
//!
//! The facade methods of this module and the folder command of `files` are
//! in the `impl App` block below, so `lib.rs` does not grow.

use crate::android_tools::{finish, new_transfer};
use crate::files::{self, TransferEvent, TransferState};
use crate::logging;
use crate::sink::Sink;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_ssh::sftp;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// File or folder `remote_path` of one host into the folder `remote_dir`
/// of another. The same host on both sides is allowed for a different
/// folder only. Returns the transfer id; an existing name ends the transfer
/// with `Conflict` unless `overwrite` is set.
pub(crate) async fn xfer_host_to_host(
    app: &Arc<App>,
    from_host: String,
    remote_path: String,
    to_host: String,
    remote_dir: String,
    overwrite: bool,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    let state = &app.state;
    // Connect up front so connection errors surface on the request itself.
    let from = state
        .connection(&from_host)
        .await?
        .sftp()
        .await
        .map_err(|e| to_core("sftp", e))?;
    let to = state
        .connection(&to_host)
        .await?
        .sftp()
        .await
        .map_err(|e| to_core("sftp", e))?;
    let same_host = from_host == to_host;
    let (id, cancel) = new_transfer(&app.transfers);
    logging::info(&format!(
        "host-to-host transfer {id}: host {from_host} → host {to_host}"
    ));
    let task_app = app.clone();
    app.rt.spawn(async move {
        let mut last = Instant::now() - Duration::from_secs(1);
        let events = on_event.clone();
        let progress = move |done: u64, total: u64| {
            // About ten events a second; the end is reported separately.
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                events(TransferEvent {
                    id,
                    state: TransferState::Running,
                    done,
                    total,
                    result_path: None,
                    error: None,
                });
            }
        };
        // A link to a folder is not a folder here: links are not followed.
        let is_dir = from
            .symlink_metadata(remote_path.clone())
            .await
            .is_ok_and(|meta| meta.file_type().is_dir());
        let result = if is_dir {
            sftp::copy_dir_between(
                &from,
                &remote_path,
                &to,
                &remote_dir,
                overwrite,
                same_host,
                &cancel,
                progress,
            )
            .await
            .map(|copied| copied.describe())
        } else {
            sftp::copy_between(
                &from,
                &remote_path,
                &to,
                &remote_dir,
                overwrite,
                same_host,
                &cancel,
                progress,
            )
            .await
        };
        finish(
            &task_app,
            id,
            &on_event,
            result.map_err(|e| to_core("host-to-host transfer", e)),
        );
    });
    Ok(id)
}

impl App {
    /// File or folder of one host → folder of another host, without a local
    /// copy. Progress and the result go to `sink`.
    pub async fn xfer_host_to_host(
        self: &Arc<Self>,
        from_host: String,
        remote_path: String,
        to_host: String,
        remote_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        let app = self.clone();
        self.run(async move {
            xfer_host_to_host(
                &app,
                from_host,
                remote_path,
                to_host,
                remote_dir,
                overwrite,
                sink,
            )
            .await
        })
        .await
    }

    /// Create the folder `name` in `dir` on a host. Returns its path;
    /// `Conflict` when the name is taken.
    pub async fn sftp_mkdir(
        self: &Arc<Self>,
        host_id: String,
        dir: String,
        name: String,
    ) -> CmdResult<String> {
        let app = self.clone();
        self.run(async move { files::sftp_mkdir(&app, host_id, dir, name).await })
            .await
    }
}
