//! Folders: copied between hosts, downloaded and uploaded with everything in
//! them.

use crate::fake::{connect, content, disk, file, paths, Node};
use crate::{big, code, never, temp_dir};
use senvakara_adapters_ssh::sftp;
use senvakara_core::ErrorCode;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn site() -> Vec<(&'static str, Node)> {
    vec![
        ("/home/me/site", Node::Dir),
        ("/home/me/site/index.html", file(b"<html>")),
        ("/home/me/site/css", Node::Dir),
        ("/home/me/site/css/main.css", file(b"body {}")),
        ("/home/me/site/css/empty", Node::Dir),
        ("/home/me/site/current", Node::Link),
    ]
}

#[tokio::test]
async fn a_failure_in_a_folder_copy_names_the_file_and_leaves_no_partial_one() {
    let (from, to) = (disk(&site()), disk(&[]));
    to.lock().unwrap().full_at = Some(6);
    let failed = sftp::copy_dir_between(
        &connect(&from).await,
        "/home/me/site",
        &connect(&to).await,
        "/tmp",
        false,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    // index.html (6 bytes) fits, css/main.css (7 bytes) does not.
    assert!(format!("{failed:#}").contains("css/main.css"), "{failed:#}");
    assert_eq!(
        paths(&to, "/tmp"),
        [
            "/tmp/site",
            "/tmp/site/css",
            "/tmp/site/css/empty",
            "/tmp/site/index.html",
        ]
    );
}

#[tokio::test]
async fn a_folder_is_copied_between_hosts_with_its_subfolders_and_links_are_skipped() {
    let (from, to) = (disk(&site()), disk(&[]));
    let mut last = (0, 0);
    let copied = sftp::copy_dir_between(
        &connect(&from).await,
        "/home/me/site/",
        &connect(&to).await,
        "/tmp",
        false,
        false,
        &never(),
        |done, total| last = (done, total),
    )
    .await
    .unwrap();
    assert_eq!((copied.path.as_str(), copied.skipped), ("/tmp/site", 1));
    assert!(copied.describe().contains("1 link"));
    assert_eq!(
        paths(&to, "/tmp"),
        [
            "/tmp/site",
            "/tmp/site/css",
            "/tmp/site/css/empty",
            "/tmp/site/css/main.css",
            "/tmp/site/index.html",
        ]
    );
    assert_eq!(
        content(&to, "/tmp/site/css/main.css"),
        Some(b"body {}".to_vec())
    );
    // Progress counts the whole tree: 6 + 7 bytes.
    assert_eq!(last, (13, 13));
}

#[tokio::test]
async fn an_existing_folder_is_a_conflict_and_replacing_merges_into_it() {
    let from = disk(&site());
    let to = disk(&[
        ("/tmp/site", Node::Dir),
        ("/tmp/site/index.html", file(b"old page")),
        ("/tmp/site/notes.txt", file(b"keep me")),
    ]);
    let (src, dst) = (connect(&from).await, connect(&to).await);
    let refused = sftp::copy_dir_between(
        &src,
        "/home/me/site",
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
    assert_eq!(
        content(&to, "/tmp/site/index.html"),
        Some(b"old page".to_vec())
    );

    sftp::copy_dir_between(
        &src,
        "/home/me/site",
        &dst,
        "/tmp",
        true,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(
        content(&to, "/tmp/site/index.html"),
        Some(b"<html>".to_vec())
    );
    assert_eq!(
        content(&to, "/tmp/site/notes.txt"),
        Some(b"keep me".to_vec())
    );
    assert_eq!(
        content(&to, "/tmp/site/css/main.css"),
        Some(b"body {}".to_vec())
    );
}

#[tokio::test]
async fn on_one_host_a_folder_is_never_copied_onto_or_into_itself() {
    let host = disk(&site());
    let (src, dst) = (connect(&host).await, connect(&host).await);
    let before = paths(&host, "/home/me");
    for folder in [
        "/home/me",
        "/home/me/site",
        "/home/me/site/css",
        "site/css/..",
    ] {
        let refused = sftp::copy_dir_between(
            &src,
            "/home/me/site",
            &dst,
            folder,
            true,
            true,
            &never(),
            |_, _| {},
        )
        .await
        .unwrap_err();
        assert_eq!(code(&refused), Some(ErrorCode::InvalidInput), "{folder}");
    }
    assert_eq!(paths(&host, "/home/me"), before);

    sftp::copy_dir_between(
        &src,
        "/home/me/site",
        &dst,
        "/tmp",
        false,
        true,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap();
    assert_eq!(
        content(&host, "/tmp/site/index.html"),
        Some(b"<html>".to_vec())
    );
}

#[tokio::test]
async fn a_folder_is_downloaded_and_names_that_could_leave_it_are_skipped() {
    let host = disk(&site());
    {
        let mut disk = host.lock().unwrap();
        for name in ["..\\evil.exe", "../../evil", "C:evil", "nul."] {
            disk.rogue.push(("/home/me/site".into(), name.into()));
        }
    }
    let local = temp_dir("download");
    let copied = sftp::download_dir(
        &connect(&host).await,
        "/home/me/site",
        &local,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap();
    // The link and the four rogue names.
    assert_eq!(copied.skipped, 5);
    let site = local.join("site");
    assert_eq!(PathBuf::from(&copied.path), site);
    assert_eq!(std::fs::read(site.join("index.html")).unwrap(), b"<html>");
    assert_eq!(
        std::fs::read(site.join("css").join("main.css")).unwrap(),
        b"body {}"
    );
    assert!(site.join("css").join("empty").is_dir());
    let mut top: Vec<String> = std::fs::read_dir(&site)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    top.sort();
    assert_eq!(top, ["css", "index.html"]);
    // Nothing was written next to the folder either.
    assert_eq!(std::fs::read_dir(&local).unwrap().count(), 1);

    let again = sftp::download_dir(
        &connect(&host).await,
        "/home/me/site",
        &local,
        false,
        &never(),
        |_, _| {},
    )
    .await
    .unwrap_err();
    assert_eq!(code(&again), Some(ErrorCode::Conflict));
    std::fs::remove_dir_all(&local).unwrap();
}

#[tokio::test]
async fn cancelling_a_folder_download_stops_the_walk_and_leaves_no_partial_file() {
    let host = disk(&[
        ("/home/me/data", Node::Dir),
        ("/home/me/data/a.bin", file(&big())),
        ("/home/me/data/b.bin", file(&big())),
    ]);
    let local = temp_dir("cancel");
    let cancel = never();
    let stopped = sftp::download_dir(
        &connect(&host).await,
        "/home/me/data",
        &local,
        false,
        &cancel,
        |done, total| {
            assert_eq!(total, 1_400_000);
            if done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap_err();
    assert_eq!(code(&stopped), Some(ErrorCode::Cancelled));
    let left: Vec<String> = std::fs::read_dir(local.join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
    std::fs::remove_dir_all(&local).unwrap();
}

#[tokio::test]
async fn a_folder_is_uploaded_with_its_subfolders() {
    let local = temp_dir("upload");
    let photos = local.join("photos");
    std::fs::create_dir_all(photos.join("2026").join("empty")).unwrap();
    std::fs::write(photos.join("readme.txt"), b"hello").unwrap();
    std::fs::write(photos.join("2026").join("a.jpg"), big()).unwrap();
    let host = disk(&[]);
    let session = connect(&host).await;
    let mut last = (0, 0);
    let copied = sftp::upload_dir(&session, &photos, "/tmp", false, &never(), |done, total| {
        last = (done, total)
    })
    .await
    .unwrap();
    assert_eq!((copied.path.as_str(), copied.skipped), ("/tmp/photos", 0));
    assert_eq!(
        paths(&host, "/tmp"),
        [
            "/tmp/photos",
            "/tmp/photos/2026",
            "/tmp/photos/2026/a.jpg",
            "/tmp/photos/2026/empty",
            "/tmp/photos/readme.txt",
        ]
    );
    assert_eq!(content(&host, "/tmp/photos/2026/a.jpg"), Some(big()));
    assert_eq!(last, (700_005, 700_005));

    let again = sftp::upload_dir(&session, &photos, "/tmp", false, &never(), |_, _| {})
        .await
        .unwrap_err();
    assert_eq!(code(&again), Some(ErrorCode::Conflict));
    std::fs::remove_dir_all(&local).unwrap();
}
