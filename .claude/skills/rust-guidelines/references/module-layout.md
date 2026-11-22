# Module layout for `apps/senvakara/src`

Read this before adding, splitting or moving a module in the app crate. It expands the eight
layout rules in `SKILL.md` with the real tree. Every path below is relative to
`apps/senvakara/src` and exists; item names are given instead of line numbers so the references
survive edits.

## How Rust finds the files

From the Rust Reference, https://doc.rust-lang.org/reference/items/modules.html :

- `mod jobs;` in `main.rs` loads `jobs.rs`. `mod spec;` inside `jobs.rs` loads `jobs/spec.rs`:
  ancestor module path components are directories.
- `jobs/mod.rs` is the older way to write the same module. Having both `jobs.rs` and
  `jobs/mod.rs` is an error. The Reference encourages the `jobs.rs` form because it avoids many
  files named `mod.rs` in one project. That is rule 1; the crate has no `mod.rs`.
- A `#[path]` attribute can override the location. Do not use it: the Style Guide says to avoid
  `#[path]` where possible (https://doc.rust-lang.org/stable/style-guide/advice.html), and a path
  that mirrors the module tree is what lets a reader find a file from its name.

## The crate root and the app-wide files (rule 8)

| File | Holds |
|---|---|
| `main.rs` | crate docs, `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, the `mod` list, root re-exports, `slint::include_modules!()`, `fn main` |
| `state.rs` | `App`, the thread-local `APP`, `app()`, `notifier()`, `take_id` |
| `pane.rs` | `Pane`, `Place`, and how `App` finds a pane (`locate`, `with_pane`, `each_pane`) |
| `confirm.rs` | `Question`, `Pending`, `ask`, `ask_with`, `answer` |
| `tabs.rs` | opening, selecting and closing session tabs, Home sections, window shortcuts |
| `present.rs` | drawing sessions into their views, `refresh` |
| `input.rs` | callbacks of the session views: `wire_terminal`, `wire_mirror`, `wire_desktop`, `wire_logcat` |
| `platform.rs` | `wsl_distros`, `select_backend` |

Facts to keep in mind:

- `slint::include_modules!()` stays in `main.rs`: the whole crate names the generated types as
  `crate::MainWindow`, `crate::Palette` and so on.
- `App` is defined in `state.rs`, not in the crate root, so its fields are `pub(crate)`; that is
  how every feature reaches its own `State` (`self.jobs`, `self.inv`).
- `main.rs` re-exports `Pane`, `Place`, `Pending` and `Question` with `pub use`, so the crate
  writes `crate::Pane`. Keep it `pub use`: during the split a private `use` there made rustc
  report dead fields in the desktop and mirror panes.
- `main.rs` builds `App` from each feature's `State::default()` and calls each `init_<feature>`.
  A new feature adds a `State` field in `state.rs` and, in `main.rs`, its `mod` line, one line
  in the `App { .. }` literal and one `init_` call. Nothing else belongs there.

## A small feature: `jobs`

```text
jobs.rs          root: docs, `mod queue; mod spec;`, re-export, Phase, Job, progress, Inbox, State
jobs/spec.rs     what a job does: `JobSpec` and its methods, `file_name`, their tests
jobs/queue.rs    the job list at work: `impl App` to start, cancel, apply events, publish rows
```

The top of `jobs.rs`:

```rust
//! Transfers and installs in progress: one compact list on Home, with progress, Cancel and the
//! result. A job is described by a [`JobSpec`], so it can be started again with "replace the
//! existing file" after the user agrees.

mod queue;
mod spec;

pub use spec::{file_name, JobSpec};

use std::cell::{Cell, RefCell};
..
```

And the top of `jobs/queue.rs`:

```rust
//! The job list at work: `impl App` to start, cancel, apply transfer events and publish rows.

use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use senvakara_app::{CmdResult, CoreError, ErrorCode, Sink, TransferEvent, TransferState};
use slint::{ComponentHandle, Model};

use crate::inventory::{fact, friendly_with};
use crate::{App, JobItem, Jobs, Pending, Question};

use super::{Job, JobSpec, Phase};
```

What it shows:

- The root is a table of contents: docs, `mod` lines, the re-export, then the small types.
- `Job`, `Phase`, `Inbox` and `State` stay in the root with private fields. `jobs/queue.rs` reads
  and writes those fields through `super::`, because a child module can use the private items of
  its ancestors. Nothing had to be widened.
- `JobSpec` moved out because it is a self-contained subject with its own tests. Two of its
  methods that only `jobs/queue.rs` calls are `pub(super)` (`target`, `conflict_wording`).
- Other features write `crate::jobs::JobSpec`, never `crate::jobs::spec::JobSpec`.
- `progress` and its test stayed in the root next to `Phase`: the test moves with the code.

## A large feature: `files`

```text
files.rs             root: docs, mod lines, Source, Kind, pair_supported, Side, FilesPane
files/entries.rs     the entries of a listing: listing this PC, sorting, the row for one entry
files/paths.rs       path arithmetic for Windows paths and for POSIX paths
files/selection.rs   impl App: selecting rows with the mouse, and the keys of a file list
files/navigate.rs    impl App: listing a folder into a side, moving between folders
files/copy.rs        impl App: copying the selection to the other side through the job list
files/open.rs        impl App: opening a Files tab, wiring callbacks, choosing each side's place
files/present.rs     impl App: presenting a pane to the view
files/tests.rs       tests of the rules that span the children
```

What it shows:

- Children are named after a subject (`paths`, `entries`) or a user activity (`navigate`, `copy`,
  `open`). None is named for its size or as a catch-all (rule 4).
- `Side` and `FilesPane` stay in `files.rs`. Almost every child touches their private fields, so
  keeping them in the root avoided marking every field `pub(super)`.
- Handlers one child calls in another are `pub(super)`: `files_load`, `files_go`, `files_up` in
  `files/navigate.rs`; `copy_problem` and `files_copy` in `files/copy.rs`. `init_files` in
  `files/open.rs` is callable from `main.rs`.
- `files.rs` declares `#[cfg(test)] mod tests;`, so the tests that exercise several children
  together live in `files/tests.rs`. Tests of one child stay inline in that child
  (`files/paths.rs`, `files/entries.rs`, `files/open.rs`).

`inventory` has the same shape with eight children; its `inventory/text.rs` holds the wording
shared across the app (`friendly`, `friendly_with`, `fact`, `transport_label`), and `App::run`
lives in the root `inventory.rs`.

## What goes where (rule 2)

| Kind of code | Goes in | Real example |
|---|---|---|
| `//!` docs for the feature, `mod` lines, re-exports | the root | `jobs.rs`, `inventory.rs` |
| Small types every child needs | the root | `Source`, `Kind` in `files.rs`; `Layout` in `workspace.rs` |
| A type whose private fields many children touch | the root | `Job`, `State` in `jobs.rs`; `Side`, `FilesPane` in `files.rs` |
| Parsing, formatting, validation: no Slint handle | a child named after the subject | `files/paths.rs`, `hostform/validation.rs`, `inventory/metrics.rs`, `workspace/naming.rs` |
| `State`, when few children need its fields | the root, or the child that owns it with `pub(super)` fields | `inventory.rs`; `workspace/saved.rs`, re-exported as `workspace::State` |
| `impl App` blocks | one child per user activity | `files/navigate.rs`, `inventory/host_actions.rs`, `device/shell.rs` |
| Unit tests | `#[cfg(test)] mod tests` at the bottom of the file they test | `jobs/spec.rs` |

Split by responsibility, never by line count alone. The test for a good split: each child has a
name that says what it holds, and its `//!` line describes it in one sentence, as every child
here does. If the only honest name is `utils`, `helpers`, `misc` or `common`, the split is wrong:
find the subject the functions share, or leave them with their single caller.

## Size limits (rule 3)

- Aim for 400 lines or fewer per file. No file is above 600; the largest are `logcat.rs` (562)
  and `settings.rs` (520), both single-file features.
- A file is split before it could reach 1,000 lines. When a file approaches 600, look for a
  second responsibility in it and give that a child module; do not cut at an arbitrary line.

Count with `wc -l` before you finish.

## Visibility (rule 5)

Use the narrowest level that compiles, in this order:

1. Private (no keyword): visible in its own module and that module's children. Right for
   helpers used by one file, and for root items that only the feature's own children use
   (`Job`, `Phase` and `progress` in `jobs.rs`).
2. `pub(super)`: visible to the parent module, so to every file of the same feature. Right for
   handlers one child calls in another (`files_load`) and for fields of a `State` that lives in
   a child (`records` and `items` in `workspace/saved.rs`).
3. `pub(crate)`: visible to the whole app. Right for what another feature or `main.rs` calls.
   The app-wide files use it (`state.rs`, `pane.rs`, `input.rs`).

Much code that was moved verbatim still says plain `pub`. In this binary crate that reaches no
further than `pub(crate)`, and the move left it as it was. Do not copy it: write the narrowest
level in new code, and narrow an old `pub` only in a change of its own.

What the split taught:

- A type whose private fields many sibling files touch stays in the module root. Children can
  read an ancestor's private items, so nothing needs widening (`files.rs`, `jobs.rs`).
- A method in an `impl App` block has the visibility you write on it, relative to the module the
  block is in. A private handler in `files/navigate.rs` cannot be called from `files/copy.rs`;
  it needs `pub(super)`.
- Adding `pub(super)` can push a signature past 100 columns. `cargo fmt` then re-wraps it; that
  diff is expected in a move and is not a behaviour change.
- Re-export only what code outside the module names. A `pub use` of an item nothing outside
  uses is reported as an unused import, which fails under `-D warnings`. `inventory.rs`
  re-exports exactly `desktop_problem`, `fact`, `friendly`, `friendly_with` and
  `transport_label`; the other helpers of `inventory/text.rs` are reached inside the feature
  through `super::text::`.
- Keep child modules private (`mod spec;`) and re-export from the root, so a later re-split does
  not touch callers.

`pub(crate)` is also what the Rust API Guidelines suggest for keeping implementation details out
of a public surface (C-HIDDEN, https://rust-lang.github.io/api-guidelines/documentation.html).

## Tests (rule 6)

- Keep `#[cfg(test)] mod tests { use super::*; .. }` at the bottom of the file whose code it
  exercises. When a function moves to a child, its tests move with it in the same change, as the
  `JobSpec` tests moved to `jobs/spec.rs`.
- When a tests block grows past about 150 lines, replace it with `#[cfg(test)] mod tests;` and
  move the body to a file. Without `#[path]`, Rust loads that file from the declaring module's
  directory (Reference, as above): `automation/draft.rs` declares it and the tests are in
  `automation/draft/tests.rs`; `workspace/stored.rs` and `workspace/stored/tests.rs` likewise;
  a root such as `files.rs` gets `files/tests.rs`.
- A tests file starts with a `//!` line and `use super::*;`, and still sees private items,
  because it is a child module of the code it tests.
- Test names stay sentences (`only_file_copies_can_be_retried_with_replace` in `jobs/spec.rs`).

## How to do a split (rule 7)

A refactor is pure code motion, in its own change, with no behaviour change mixed in. A reviewer
should be able to confirm it by checking that text only moved.

1. Read the whole file and write down its responsibilities. Decide the children and their names
   before moving anything.
2. Move text verbatim: cut and paste, or use a syntax-aware tool. Do not retype or "improve"
   code on the way. The rustc dev guide's LLM guidance says mass rewrites should strongly prefer
   a proper syntax rewrite tool and that an LLM performing the rewrite directly needs great
   caution (https://rustc-dev-guide.rust-lang.org/llm-guidance/writing.html).
3. The only edits allowed in a motion change are the ones the move forces: `mod` lines, `use`
   lines, visibility keywords, `super::`/`crate::` paths, a new `//!` line for each new file,
   and whatever `cargo fmt` re-wraps as a result.
4. Keep doc comments and inline comments attached to the code they describe. Update comments
   and manifests that name a file that moved (the `unstable-winit-030` comment in
   `apps/senvakara/Cargo.toml` now points at `platform.rs`).
5. Do not rename, reorder arguments, change wording of user-facing text, or fix a bug you
   notice. Note it and do it in a separate change.
6. Check the crate you touched: `cargo fmt --all --check`, then
   `cargo clippy -p senvakara --all-targets -- -D warnings` and `cargo test -p senvakara`. The
   test count should be the same before and after; say so in your summary, or say that you did
   not compare.
7. Confirm with `wc -l` that every resulting file is within the limits, and that no `mod.rs`
   and no `utils.rs`-style file was created.

## Checklist for a new or split module

- [ ] Root is `<name>.rs`; children are in `<name>/`; no `mod.rs`; no `#[path]`.
- [ ] Root has `//!` docs, `mod` lines, re-exports and the types its children share; every child
      starts with a one-sentence `//!` line.
- [ ] Every file is named after what it holds and is at most 600 lines, ideally 400 or fewer.
- [ ] Each new item has the narrowest visibility that compiles; the root re-exports only what
      outside code names.
- [ ] Tests sit with the code; a tests block over about 150 lines is in its own `tests.rs`.
- [ ] The change is motion only, or behaviour only, not both.
