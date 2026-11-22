//! Tests of the rules that span the children: which copies exist and why one is refused.

use super::copy::copy_problem;
use super::entries::{sort_entries, Entry};
use super::*;

#[test]
fn which_copies_exist() {
    use Kind::*;
    for (from, to, ok) in [
        (Local, Host, true),
        (Host, Local, true),
        (Local, Device, true),
        (Device, Local, true),
        (Host, Device, true),
        (Device, Host, true),
        (Host, Host, false),
        (Device, Device, false),
        (Local, Local, false),
    ] {
        assert_eq!(pair_supported(from, to), ok, "{from:?} to {to:?}");
    }
}

fn file(name: &str) -> Entry {
    Entry {
        name: name.into(),
        is_dir: false,
        is_symlink: false,
        is_drive: false,
        size: 10,
        modified: None,
    }
}

#[test]
fn copy_reasons() {
    let (a, dir) = (
        file("a.txt"),
        Entry {
            is_dir: true,
            ..file("docs")
        },
    );
    assert_eq!(
        copy_problem(Kind::Local, Kind::Host, &[&a], true, "/tmp", true),
        ""
    );
    assert!(
        copy_problem(Kind::Host, Kind::Host, &[&a], true, "/tmp", true)
            .starts_with("Not supported yet")
    );
    assert!(
        copy_problem(Kind::Device, Kind::Device, &[&a], true, "/sdcard", true)
            .starts_with("Not supported yet")
    );
    assert_eq!(
        copy_problem(Kind::Local, Kind::Host, &[], true, "/tmp", true),
        "Select one or more files first."
    );
    assert!(
        copy_problem(Kind::Local, Kind::Host, &[&a, &dir], true, "/tmp", true)
            .starts_with("Folders cannot")
    );
    assert!(
        copy_problem(Kind::Host, Kind::Local, &[&a], true, "", true).starts_with("Open a folder")
    );
    assert!(
        copy_problem(Kind::Host, Kind::Local, &[&a], false, "", true).starts_with("The other side")
    );
    assert!(
        copy_problem(Kind::Local, Kind::Device, &[&a], true, "/sdcard", false)
            .starts_with("The phone must")
    );
}

#[test]
fn folders_come_first() {
    let mut entries = vec![
        file("b.txt"),
        Entry {
            is_dir: true,
            ..file("zeta")
        },
        file("A.txt"),
        Entry {
            is_dir: true,
            ..file("Alpha")
        },
    ];
    sort_entries(&mut entries);
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt"]);
}
