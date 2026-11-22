//! Folder copies and host-to-host copies (FILE-DIR-01, FILE-H2H-01) against
//! an in-process SFTP server that keeps its files in memory (`fake`). The
//! client talks to it over an in-memory pipe: no network, no SSH.

mod fake;
mod folders;

use fake::{connect, content, disk, file, paths, Node, HOME};
use senvakara_adapters_ssh::sftp;
use senvakara_core::{CoreError, ErrorCode};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

fn code(error: &anyhow::Error) -> Option<ErrorCode> {
    error.downcast_ref::<CoreError>().map(|error| error.code)
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// Bytes that differ from place to place, longer than two 256 KiB chunks.
fn big() -> Vec<u8> {
    (0..700_000u32).map(|i| (i % 251) as u8).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("senvakara-sftp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_listing_reports_permissions_the_way_ls_writes_them() {
    let host = disk(&[
        ("/home/me/docs", Node::Dir),
        ("/home/me/a.txt", file(b"hello")),
        ("/home/me/latest", Node::Link),
    ]);
    let listing = sftp::list(&connect(&host).await, ".").await.unwrap();
    assert_eq!(listing.path, HOME);
    let seen: Vec<(&str, Option<&str>)> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.permissions.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            ("docs", Some("drwxr-xr-x")),
            ("a.txt", Some("-rw-r-----")),
            ("latest", Some("lrwxrwxrwx")),
        ]
    );
}

#[tokio::test]
async fn a_file_is_streamed_from_one_host_to_another() {
    let (from, to) = (disk(&[("/home/me/big.bin", file(&big()))]), disk(&[]));
    let mut seen = Vec::new();
    let target = sftp::copy_between(
        &connect(&from).await,
        "/home/me/big.bin",
        &connect(&to).await,
        "/tmp",
        false,
        false,
        &never(),
        |done, total| seen.push((done, total)),
    )
    .await
    .unwrap();
    assert_eq!(target, "/tmp/big.bin");
    assert_eq!(content(&to, "/tmp/big.bin"), Some(big()));
    // Nothing is left under the temporary name.
    assert_eq!(paths(&to, "/tmp"), ["/tmp/big.bin"]);
    assert_eq!(seen.first(), Some(&(0, 700_000)));
    assert_eq!(seen.last(), Some(&(700_000, 700_000)));
    // 256 KiB at a time: more than one step, never more than a chunk each.
    assert!(seen
        .windows(2)
        .all(|pair| pair[1].0 - pair[0].0 <= 256 * 1024));
    assert!(seen.len() >= 4);
}

#[tokio::test]
async fn an_existing_file_is_a_conflict_until_replacing_is_allowed() {
    let from = disk(&[("/home/me/a.txt", file(b"new text"))]);
    let to = disk(&[("/tmp/a.txt", file(b"old"))]);
    let (src, dst) = (connect(&from).await, connect(&to).await);
    let refused = sftp::copy_between(
        &src,
        "/home/me/a.txt",
        &dst,
        "/tmp",
        false,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    assert_eq!(code(&refused), Some(ErrorCode::Conflict));
    assert_eq!(content(&to, "/tmp/a.txt"), Some(b"old".to_vec()));

    sftp::copy_between(
        &src,
        "/home/me/a.txt",
        &dst,
        "/tmp",
        true,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(content(&to, "/tmp/a.txt"), Some(b"new text".to_vec()));
    assert_eq!(paths(&to, "/tmp"), ["/tmp/a.txt"]);
}

#[tokio::test]
async fn cancelling_a_host_to_host_copy_removes_the_partial_file() {
    let (from, to) = (disk(&[("/home/me/big.bin", file(&big()))]), disk(&[]));
    let cancel = never();
    let stopped = sftp::copy_between(
        &connect(&from).await,
        "/home/me/big.bin",
        &connect(&to).await,
        "/tmp",
        false,
        false,
        &cancel,
        |done, _| {
            if done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap_err();
    assert_eq!(code(&stopped), Some(ErrorCode::Cancelled));
    assert!(paths(&to, "/tmp").is_empty());
}

#[tokio::test]
async fn a_failed_write_removes_the_partial_file_and_keeps_the_old_one() {
    let from = disk(&[("/home/me/big.bin", file(&big()))]);
    let to = disk(&[("/tmp/big.bin", file(b"old"))]);
    to.lock().unwrap().full_at = Some(300_000);
    let failed = sftp::copy_between(
        &connect(&from).await,
        "/home/me/big.bin",
        &connect(&to).await,
        "/tmp",
        true,
        false,
        &never(),
        |_, _| {},
    )
    .await;
    assert!(failed.is_err());
    assert_eq!(paths(&to, "/tmp"), ["/tmp/big.bin"]);
    assert_eq!(content(&to, "/tmp/big.bin"), Some(b"old".to_vec()));
}

#[tokio::test]
async fn on_one_host_a_file_is_copied_to_another_folder_but_never_onto_itself() {
    let host = disk(&[("/home/me/a.txt", file(b"same host"))]);
    let (src, dst) = (connect(&host).await, connect(&host).await);
    // "." is the folder the file is in already, however it is spelled.
    for folder in [".", "/home/me", "/home/me/", "/tmp/../home/me"] {
        let refused =
            sftp::copy_between(&src, "a.txt", &dst, folder, true, true, &never(), |_, _| {})
                .await
                .unwrap_err();
        assert_eq!(code(&refused), Some(ErrorCode::InvalidInput), "{folder}");
    }
    assert_eq!(
        content(&host, "/home/me/a.txt"),
        Some(b"same host".to_vec())
    );

    sftp::copy_between(
        &src,
        "a.txt",
        &dst,
        "/tmp",
        false,
        true,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(content(&host, "/tmp/a.txt"), Some(b"same host".to_vec()));
}

#[tokio::test]
async fn a_new_folder_needs_a_free_name_that_is_one_path_component() {
    let host = disk(&[("/home/me/docs", Node::Dir)]);
    let session = connect(&host).await;
    assert_eq!(
        sftp::mkdir(&session, HOME, "new").await.unwrap(),
        "/home/me/new"
    );
    assert_eq!(
        host.lock().unwrap().nodes.get("/home/me/new"),
        Some(&Node::Dir)
    );
    let taken = sftp::mkdir(&session, HOME, "docs").await.unwrap_err();
    assert_eq!(code(&taken), Some(ErrorCode::Conflict));
    for bad in ["", ".", "..", "a/b", "../up"] {
        let refused = sftp::mkdir(&session, HOME, bad).await.unwrap_err();
        assert_eq!(code(&refused), Some(ErrorCode::InvalidInput), "{bad:?}");
    }
}
