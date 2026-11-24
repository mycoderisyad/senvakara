//! Remote SFTP item management (rename, recursive delete, chmod) against the
//! in-memory fake SFTP server.

use crate::code;
use crate::fake::{connect, content, disk, file, paths, Node, HOME};
use senvakara_adapters_ssh::sftp;
use senvakara_core::ErrorCode;

#[tokio::test]
async fn renaming_moves_a_file_or_folder_and_refuses_existing_targets() {
    let host = disk(&[
        ("/home/me/a.txt", file(b"hello")),
        ("/home/me/taken.txt", file(b"keep")),
        ("/home/me/broken", Node::Link),
        ("/home/me/docs", Node::Dir),
        ("/home/me/docs/note.md", file(b"note")),
    ]);
    let session = connect(&host).await;

    // Renaming to the same name is a no-op that returns the target path.
    assert_eq!(
        sftp::rename(&session, HOME, "a.txt", "a.txt")
            .await
            .unwrap(),
        "/home/me/a.txt"
    );
    assert_eq!(content(&host, "/home/me/a.txt"), Some(b"hello".to_vec()));

    // Existing file and existing symlink targets both yield Conflict.
    for target in ["taken.txt", "broken"] {
        let clash = sftp::rename(&session, HOME, "a.txt", target)
            .await
            .unwrap_err();
        assert_eq!(code(&clash), Some(ErrorCode::Conflict), "{target}");
    }
    assert_eq!(content(&host, "/home/me/a.txt"), Some(b"hello".to_vec()));
    assert_eq!(content(&host, "/home/me/taken.txt"), Some(b"keep".to_vec()));

    assert_eq!(
        sftp::rename(&session, HOME, "a.txt", "b.txt")
            .await
            .unwrap(),
        "/home/me/b.txt"
    );
    assert_eq!(content(&host, "/home/me/a.txt"), None);
    assert_eq!(content(&host, "/home/me/b.txt"), Some(b"hello".to_vec()));

    // Renaming a folder keeps its nested entries.
    assert_eq!(
        sftp::rename(&session, HOME, "docs", "archive")
            .await
            .unwrap(),
        "/home/me/archive"
    );
    assert_eq!(
        content(&host, "/home/me/archive/note.md"),
        Some(b"note".to_vec())
    );
    assert!(paths(&host, "/home/me/docs").is_empty());

    // Missing source fails.
    assert!(sftp::rename(&session, HOME, "missing.txt", "other.txt")
        .await
        .is_err());
}

#[tokio::test]
async fn removing_a_file_or_symlink_never_follows_links() {
    let host = disk(&[
        ("/home/me/a.txt", file(b"hello")),
        ("/home/me/link", Node::Link),
        ("/home/me/keep.txt", file(b"safe")),
    ]);
    let session = connect(&host).await;

    sftp::remove(&session, "/home/me/a.txt").await.unwrap();
    // Trailing slash on a symlink still removes only the link.
    sftp::remove(&session, "/home/me/link/").await.unwrap();
    assert_eq!(paths(&host, HOME), ["/home/me/keep.txt"]);

    assert!(sftp::remove(&session, "/home/me/missing").await.is_err());
}

#[tokio::test]
async fn removing_a_folder_deletes_the_tree_deepest_first_and_keeps_siblings() {
    let host = disk(&[
        ("/home/me/site", Node::Dir),
        ("/home/me/site/index.html", file(b"<html>")),
        ("/home/me/site/link", Node::Link),
        ("/home/me/site/css", Node::Dir),
        ("/home/me/site/css/main.css", file(b"body {}")),
        ("/home/me/site/css/empty", Node::Dir),
        ("/home/me/keep", Node::Dir),
        ("/home/me/keep/ok.txt", file(b"stay")),
    ]);
    let session = connect(&host).await;

    sftp::remove(&session, "/home/me/site/").await.unwrap();
    assert_eq!(
        paths(&host, HOME),
        ["/home/me/keep", "/home/me/keep/ok.txt"]
    );
}

#[tokio::test]
async fn removing_a_folder_with_a_rogue_entry_refuses_to_touch_outside_paths() {
    let host = disk(&[
        ("/home/me/dirty", Node::Dir),
        ("/home/me/escape", file(b"untouched")),
    ]);
    host.lock()
        .unwrap()
        .rogue
        .push(("/home/me/dirty".into(), "../escape".into()));
    let session = connect(&host).await;

    let refused = sftp::remove(&session, "/home/me/dirty").await.unwrap_err();
    assert_eq!(code(&refused), Some(ErrorCode::InvalidInput));
    assert_eq!(
        content(&host, "/home/me/escape"),
        Some(b"untouched".to_vec())
    );
}

#[tokio::test]
async fn chmod_updates_permissions_and_rejects_bad_modes_or_missing_paths() {
    let host = disk(&[
        ("/home/me/run.sh", file(b"#!/bin/sh")),
        ("/home/me/secret", Node::Dir),
    ]);
    let session = connect(&host).await;

    sftp::chmod(&session, "/home/me/run.sh", 0o755)
        .await
        .unwrap();
    sftp::chmod(&session, "/home/me/secret", 0o700)
        .await
        .unwrap();

    let listing = sftp::list(&session, HOME).await.unwrap();
    let seen: Vec<(&str, Option<&str>)> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.permissions.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            ("secret", Some("drwx------")),
            ("run.sh", Some("-rwxr-xr-x")),
        ]
    );
    assert_eq!(
        content(&host, "/home/me/run.sh"),
        Some(b"#!/bin/sh".to_vec())
    );

    let bad = sftp::chmod(&session, "/home/me/run.sh", 0o100755)
        .await
        .unwrap_err();
    assert_eq!(code(&bad), Some(ErrorCode::InvalidInput));
    assert!(sftp::chmod(&session, "/home/me/missing", 0o644)
        .await
        .is_err());
}
