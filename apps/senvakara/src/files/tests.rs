//! Tests of the rules that span the children: which copies exist and why one is refused, how a
//! list is ordered and filtered, the path bar, drag tokens and the list of places.

use super::copy::{copy_problem, job_spec, Batch, Item, Route};
use super::dragdrop::{explorer_side, parse_token, token};
use super::entries::{sort_entries, view, Column, Entry, Sort};
use super::folders::new_folder_problem;
use super::paths::{crumbs, folder_name_problem};
use super::places::{filter_places, Place};
use super::*;
use crate::jobs::JobSpec;

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
        (Host, Host, true),
        (Device, Device, false),
        (Local, Local, false),
        (Local, None, false),
        (None, Host, false),
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
        permissions: String::new(),
    }
}

fn folder(name: &str) -> Entry {
    Entry {
        is_dir: true,
        size: 0,
        ..file(name)
    }
}

fn route(from: Kind, to: Kind) -> Route<'static> {
    Route {
        from,
        to,
        to_loaded: true,
        to_path: "/tmp",
        phone_ready: true,
        same_folder: false,
    }
}

#[test]
fn copy_reasons() {
    let (a, dir) = (file("a.txt"), folder("docs"));
    assert_eq!(copy_problem(route(Kind::Local, Kind::Host), &[&a]), "");
    assert!(copy_problem(route(Kind::Device, Kind::Device), &[&a]).starts_with("Not supported yet"));
    assert!(copy_problem(route(Kind::Local, Kind::Local), &[&a]).starts_with("Not supported yet"));
    assert_eq!(
        copy_problem(route(Kind::Local, Kind::Host), &[]),
        "Select one or more files or folders first."
    );
    assert!(copy_problem(
        Route {
            to_path: "",
            ..route(Kind::Host, Kind::Local)
        },
        &[&a]
    )
    .starts_with("Open a folder"));
    assert!(copy_problem(
        Route {
            to_loaded: false,
            ..route(Kind::Host, Kind::Local)
        },
        &[&a]
    )
    .starts_with("The other side"));
    assert!(copy_problem(
        Route {
            phone_ready: false,
            ..route(Kind::Local, Kind::Device)
        },
        &[&a]
    )
    .starts_with("The phone must"));
    assert!(
        copy_problem(route(Kind::Local, Kind::None), &[&a]).starts_with("Select a host"),
        "a side with no host yet takes nothing"
    );
    let _ = dir;
}

#[test]
fn folders_are_copied_between_this_pc_and_hosts_but_not_with_a_phone() {
    let (a, dir) = (file("a.txt"), folder("docs"));
    for (from, to) in [
        (Kind::Local, Kind::Host),
        (Kind::Host, Kind::Local),
        (Kind::Host, Kind::Host),
    ] {
        assert_eq!(copy_problem(route(from, to), &[&a, &dir]), "", "{from:?}");
    }
    for (from, to) in [
        (Kind::Local, Kind::Device),
        (Kind::Device, Kind::Local),
        (Kind::Host, Kind::Device),
        (Kind::Device, Kind::Host),
    ] {
        assert!(
            copy_problem(route(from, to), &[&a, &dir]).starts_with("Folders cannot"),
            "{from:?} to {to:?}"
        );
        assert_eq!(copy_problem(route(from, to), &[&a]), "");
    }
    let drive = Entry {
        is_drive: true,
        ..folder("C:\\")
    };
    assert!(copy_problem(route(Kind::Local, Kind::Host), &[&drive]).starts_with("A drive"));
}

#[test]
fn a_host_copies_to_another_folder_of_itself_but_not_onto_the_same_folder() {
    let a = file("a.txt");
    assert_eq!(copy_problem(route(Kind::Host, Kind::Host), &[&a]), "");
    assert!(copy_problem(
        Route {
            same_folder: true,
            ..route(Kind::Host, Kind::Host)
        },
        &[&a]
    )
    .starts_with("Both sides show the same folder"));
}

#[test]
fn every_pair_of_places_gets_the_job_that_copies_between_them() {
    let item = Item {
        name: "site".into(),
        path: "/srv/site".into(),
    };
    let batch = |from: Source, to: Source| Batch {
        from,
        from_dir: "/srv".into(),
        to,
        to_dir: "/tmp".into(),
        items: vec![item.clone()],
    };
    let across = batch(Source::Host("a".into()), Source::Host("b".into()));
    assert_eq!(
        job_spec(&across, &item, ""),
        Some(JobSpec::HostToHost {
            from_host: "a".into(),
            remote_path: "/srv/site".into(),
            to_host: "b".into(),
            remote_dir: "/tmp".into(),
            overwrite: false,
        })
    );
    let down = batch(Source::Host("a".into()), Source::Local);
    assert!(matches!(
        job_spec(&down, &item, ""),
        Some(JobSpec::SftpDownload { .. })
    ));
    let push = batch(Source::Local, Source::Device("r".into()));
    assert!(matches!(
        job_spec(&push, &item, "SERIAL"),
        Some(JobSpec::DevicePush { serial, .. }) if serial == "SERIAL"
    ));
    assert_eq!(
        job_spec(&batch(Source::Local, Source::Local), &item, ""),
        None
    );
    assert_eq!(
        job_spec(&batch(Source::Local, Source::None), &item, ""),
        None
    );
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

fn sample() -> Vec<Entry> {
    vec![
        Entry {
            size: 300,
            modified: Some(30),
            ..file("b.txt")
        },
        Entry {
            modified: Some(20),
            ..folder("zeta")
        },
        Entry {
            size: 100,
            modified: Some(10),
            ..file("A.PDF")
        },
        Entry {
            modified: Some(40),
            ..folder("Alpha")
        },
        Entry {
            size: 200,
            modified: None,
            ..file("notes")
        },
    ]
}

#[test]
fn folders_come_first() {
    let mut entries = sample();
    sort_entries(&mut entries, Sort::default());
    assert_eq!(
        names(&entries),
        ["Alpha", "zeta", "A.PDF", "b.txt", "notes"]
    );
}

#[test]
fn a_list_is_ordered_by_any_column_both_ways_with_folders_always_first() {
    let sorted = |column, descending| {
        let mut entries = sample();
        sort_entries(&mut entries, Sort { column, descending });
        names(&entries).join(" ")
    };
    assert_eq!(sorted(Column::Name, true), "zeta Alpha notes b.txt A.PDF");
    assert_eq!(sorted(Column::Size, false), "Alpha zeta A.PDF notes b.txt");
    assert_eq!(sorted(Column::Size, true), "zeta Alpha b.txt notes A.PDF");
    // No date sorts before any date.
    assert_eq!(
        sorted(Column::Modified, false),
        "zeta Alpha notes A.PDF b.txt"
    );
    assert_eq!(
        sorted(Column::Modified, true),
        "Alpha zeta b.txt A.PDF notes"
    );
    // Kind: "file" (no extension), "pdf", "txt".
    assert_eq!(sorted(Column::Kind, false), "Alpha zeta notes A.PDF b.txt");
}

#[test]
fn a_header_click_starts_ascending_and_a_second_click_turns_around() {
    let start = Sort::default();
    assert_eq!((start.column, start.descending), (Column::Name, false));
    let turned = start.clicked(Column::Name);
    assert!(turned.descending);
    assert!(!turned.clicked(Column::Name).descending);
    let other = turned.clicked(Column::Size);
    assert_eq!((other.column, other.descending), (Column::Size, false));
    for column in [Column::Name, Column::Modified, Column::Size, Column::Kind] {
        assert_eq!(Column::from_id(column.id()), Some(column));
    }
    assert_eq!(Column::from_id("owner"), None);
}

#[test]
fn the_filter_keeps_names_that_contain_the_text_whatever_the_case() {
    let all = sample();
    assert_eq!(view(&all, "", Sort::default()).len(), 5);
    assert_eq!(names(&view(&all, " PDF ", Sort::default())), ["A.PDF"]);
    assert_eq!(
        names(&view(&all, "a", Sort::default())),
        ["Alpha", "zeta", "A.PDF"]
    );
    assert!(view(&all, "nothing", Sort::default()).is_empty());
}

#[test]
fn the_kind_column_names_folders_links_and_extensions() {
    assert_eq!(folder("docs").kind_text(), "folder");
    assert_eq!(file("Report.PDF").kind_text(), "pdf");
    assert_eq!(file("archive.tar.gz").kind_text(), "gz");
    assert_eq!(file("Makefile").kind_text(), "file");
    assert_eq!(file(".bashrc").kind_text(), "file");
    let link = Entry {
        is_symlink: true,
        ..file("latest")
    };
    assert_eq!(link.kind_text(), "link");
}

#[test]
fn a_side_keeps_the_selection_through_sorting_and_remembers_where_it_was() {
    let mut side = Side::new(Source::Host("h".into()));
    side.show("/home/me".into(), sample());
    assert_eq!(
        names(&side.entries),
        ["Alpha", "zeta", "A.PDF", "b.txt", "notes"]
    );
    side.selected = BTreeSet::from([3]);
    side.cursor = Some(3);
    side.sort = side.sort.clicked(Column::Size).clicked(Column::Size);
    side.arrange(true);
    assert_eq!(
        names(&side.entries),
        ["zeta", "Alpha", "b.txt", "notes", "A.PDF"]
    );
    assert_eq!(side.selected, BTreeSet::from([2]));
    assert_eq!(side.cursor, Some(2));
    // A filter that hides the selected row drops it from the selection.
    side.filter = "alp".into();
    side.arrange(true);
    assert!(side.selected.is_empty());

    // Opening another folder empties the filter and goes on the history; a refresh does not.
    let rev = side.filter_rev;
    side.show("/home/me/Alpha".into(), Vec::new());
    assert_eq!(
        (side.back.as_slice(), side.filter.as_str()),
        (&["/home/me".to_string()][..], "")
    );
    assert_ne!(side.filter_rev, rev);
    side.show("/home/me/Alpha".into(), Vec::new());
    assert_eq!(side.back.len(), 1);
    // Going back must not add to the history.
    side.travelling = true;
    side.show("/home/me".into(), sample());
    assert_eq!(side.back.len(), 1);
    assert!(!side.travelling);
}

#[test]
fn the_path_bar_lists_every_folder_down_to_the_one_on_screen() {
    let labels = |kind, path: &str| -> Vec<(String, String)> { crumbs(kind, path) };
    let pair = |label: &str, path: &str| (label.to_string(), path.to_string());
    assert_eq!(labels(Kind::Local, ""), [pair("This PC", "")]);
    assert_eq!(
        labels(Kind::Local, "C:\\"),
        [pair("This PC", ""), pair("C:", "C:\\")]
    );
    assert_eq!(
        labels(Kind::Local, "C:\\Users\\me"),
        [
            pair("This PC", ""),
            pair("C:", "C:\\"),
            pair("Users", "C:\\Users"),
            pair("me", "C:\\Users\\me"),
        ]
    );
    assert_eq!(
        labels(Kind::Local, "\\\\nas\\share\\films"),
        [
            pair("This PC", ""),
            pair("\\\\nas\\share", "\\\\nas\\share"),
            pair("films", "\\\\nas\\share\\films"),
        ]
    );
    assert_eq!(labels(Kind::Host, "/"), [pair("/", "/")]);
    assert_eq!(
        labels(Kind::Device, "/sdcard/Download/"),
        [
            pair("/", "/"),
            pair("sdcard", "/sdcard"),
            pair("Download", "/sdcard/Download"),
        ]
    );
    assert!(labels(Kind::None, "").is_empty());
}

#[test]
fn a_new_folder_needs_a_place_that_takes_one_and_a_plain_name() {
    assert_eq!(new_folder_problem(Kind::Host, true, false), "");
    assert_eq!(new_folder_problem(Kind::Local, true, false), "");
    assert!(new_folder_problem(Kind::Local, true, true).starts_with("Open a drive"));
    assert!(new_folder_problem(Kind::Device, true, false).contains("phone"));
    assert!(new_folder_problem(Kind::Host, false, false).starts_with("No folder"));
    assert!(new_folder_problem(Kind::None, false, false).starts_with("Select a host"));

    assert_eq!(folder_name_problem(Kind::Host, " backups "), "");
    assert_eq!(folder_name_problem(Kind::Host, "a:b"), "");
    for bad in ["", "  ", ".", "..", "a/b", "../up"] {
        assert_ne!(folder_name_problem(Kind::Host, bad), "", "{bad:?}");
        assert_ne!(folder_name_problem(Kind::Local, bad), "", "{bad:?}");
    }
    for bad in ["a\\b", "C:x", "what?", "trail."] {
        assert_ne!(folder_name_problem(Kind::Local, bad), "", "{bad:?}");
    }
}

#[test]
fn a_drag_token_names_the_list_it_comes_from_and_nothing_else_is_taken() {
    assert_eq!(parse_token(&token(42, 1)), Some((42, 1)));
    assert_eq!(parse_token(&token(7, 0)), Some((7, 0)));
    for bad in [
        "",
        "hello",
        "senvakara-files:42",
        "senvakara-files:42:2",
        "senvakara-files:x:1",
        "senvakara-files:42:1:extra",
        "other:42:1",
        "C:\\Users\\me\\file.txt",
    ] {
        assert_eq!(parse_token(bad), None, "{bad:?}");
    }
}

#[test]
fn explorer_drops_go_to_the_hovered_side_unless_it_is_this_pc() {
    use Kind::*;
    assert_eq!(explorer_side([Local, Host], 1), Some(1));
    assert_eq!(explorer_side([Local, Host], 0), Some(1));
    assert_eq!(explorer_side([Host, Device], 0), Some(0));
    assert_eq!(explorer_side([Host, Device], 1), Some(1));
    assert_eq!(explorer_side([Device, Local], 1), Some(0));
    assert_eq!(explorer_side([Local, None], 1), Option::None);
    assert_eq!(explorer_side([Local, Local], 0), Option::None);
}

#[test]
fn the_list_of_places_is_searched_by_name_and_by_user() {
    let place = |title: &str, detail: &str, group: &'static str| Place {
        source: Source::Host(title.into()),
        title: title.into(),
        detail: detail.into(),
        group,
        hue: 0,
        connected: false,
    };
    let all = vec![
        place("192.168.1.10", "ssh, bob", "Hosts"),
        place("Hermes VPS", "ssh, root", "Hosts"),
        place("Pixel 8", "phone, Fedora > USB", "Phones"),
    ];
    let titles = |query: &str| -> Vec<String> {
        filter_places(all.clone(), query)
            .into_iter()
            .map(|place| place.title)
            .collect()
    };
    assert_eq!(titles("").len(), 3);
    assert_eq!(titles("ROOT"), ["Hermes VPS"]);
    assert_eq!(titles("ssh 192"), ["192.168.1.10"]);
    assert_eq!(titles("pixel usb"), ["Pixel 8"]);
    assert!(titles("windows").is_empty());
}
