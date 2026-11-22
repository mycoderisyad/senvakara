# House patterns, with code

Read this before writing a feature module in `apps/senvakara`, a backend call from the UI, an
error message, or a commit message. Every snippet is copied from the repo, shortened where marked
with `..`. App files are named relative to `apps/senvakara/src` and cited by item name, not line
number.

Contents: 1 feature module, 2 backend work off the UI thread, 3 events from worker threads,
4 `RefCell` borrows, 5 errors, 6 backend API shape, 7 tests, 8 allowing a lint, 9 `unsafe`,
10 commit messages.

## 1. A feature module in the app

Condensed from `palette.rs`, a feature small enough to be one file. Order: module docs, imports
in three groups, plain types and pure functions, `State`, `impl App`, tests. A feature with
children keeps the same parts and spreads them over files; `module-layout.md` shows how with
`jobs` and `files`.

```rust
//! The command palette (Ctrl+Shift+K): every "open ..." action the inventory offers right
//! now, plus saved workspaces and the Home sections, filtered by what the user types.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use crate::{App, Nav, Palette, PaletteItem};

/// The entries that match `query`, best first. Every word of the query must appear in the
/// title or the detail, ignoring case; ..
pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> { .. }

#[derive(Default)]
pub struct State {
    /// Everything that can be run, as it was when the palette opened.
    all: RefCell<Vec<Entry>>,
    shown: RefCell<Vec<Entry>>,
    current: Cell<usize>,
    model: Rc<VecModel<PaletteItem>>,
}

impl App {
    pub fn init_palette(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Palette>();
        view.set_items(self.palette.model.clone().into());
        view.on_edited({
            let app = self.clone();
            move |text| app.palette_filter(&text)
        });
        ..
    }

    fn palette_filter(&self, query: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let shown = filter(&self.palette.all.borrow(), query);
        ..
    }
}
```

What to copy:

- `filter` knows nothing about Slint. The handler `palette_filter` only moves data between the
  pure function and the view. That is why `filter` has four unit tests and the handler has none.
- The doc comment on `filter` states the rule the user experiences (every word must match, titles
  first), not the implementation.
- `State` derives `Default` so `main.rs` can build it with `palette::State::default()`; `App`
  holds it in a `pub(crate)` field (`state.rs`).
- Each callback closure clones `self` and calls one named handler.
- New code uses the narrowest visibility that works (`pub(super)`, `pub(crate)`) instead of the
  plain `pub` shown here; see `module-layout.md`.

In a feature with children, a handler file imports what it needs from the root and its siblings
through `super::`, in a fourth import group, and marks what a sibling calls `pub(super)`. From
`files/navigate.rs`:

```rust
//! Listing a folder into a side and moving from one folder to another.

use std::rc::Rc;

use crate::inventory::friendly;
use crate::App;

use super::entries::{list_local, sort_entries, Entry};
use super::paths::{join, parent, tidy_local, tidy_posix};
use super::{Kind, Source};

impl App {
    /// Say what the folder field of a side should show.
    fn set_input(&self, id: i32, side: usize, text: &str) {
        self.with_files(id, |pane| pane.sides[side].borrow_mut().set_input(text));
        self.files_publish(id);
    }

    /// List `path` of `source` into a side. The rows on screen stay until the answer is in.
    pub(super) fn files_load(self: &Rc<Self>, id: i32, side: usize, source: Source, path: String) {
        ..
    }
}
```

`pane.sides` is a private field of `FilesPane`, defined in the root `files.rs`; the child may use
it because it is a descendant of that module.

## 2. Backend work off the UI thread

The helper, `App::run` in the root `inventory.rs`:

```rust
/// Run a backend call and handle its result back on the UI thread.
pub fn run<T: Send + 'static>(
    self: &Rc<Self>,
    work: impl std::future::Future<Output = T> + Send + 'static,
    done: impl FnOnce(&Rc<App>, T) + Send + 'static,
) {
    self.backend.rt.spawn(async move {
        let output = work.await;
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = crate::app() {
                done(&app, output);
            }
        });
    });
}
```

A caller, `wireless_connect` in `wireless.rs`:

```rust
wireless.set_busy("connect".into());
let backend = self.backend.clone();
let shown = address.clone();
self.run(
    async move { backend.wireless_connect(host, address).await },
    move |app, result| {
        let Some(ui) = app.ui.upgrade() else { return };
        ui.global::<Wireless>().set_busy(SharedString::new());
        match result {
            Ok(_) => {
                app.wireless_note(
                    true,
                    format!("Connected to {shown}. The phone is now listed under Devices."),
                );
                app.reload();
            }
            Err(error) => app.wireless_note(
                false,
                said(
                    "Could not connect. Check the address and that the phone is paired with this host.",
                    &error,
                ),
            ),
        }
    },
);
```

What to copy:

- Read everything you need from the view before the call; move owned values into the future.
- Mark the view busy first and clear it in the `done` closure on both outcomes.
- The `done` closure receives `&Rc<App>`; it never captures `self`, because `Rc<App>` is not
  `Send`.
- Check that the result is still wanted. `wireless_scan` keeps the host it asked about in `asked`
  and compares it when the answer arrives, because the picker may have moved on. The Files pane
  does the same with a counter: `Side::request` in `files.rs` is "Bumped by every request: an
  answer to an older one is dropped."
- Both outcomes tell the user something. No result is dropped silently.

For blocking or CPU-heavy work there is no future to await: spawn a thread, catch a panic if the
code is a codec or parser, and return through the event loop (`settings.rs`, `decoder_test`):

```rust
std::thread::spawn(|| {
    // A panic inside the codec must not take the app down or leave the test "running".
    let result = std::panic::catch_unwind(|| decoder_self_test(360, 640, 30))
        .unwrap_or_else(|_| Err("The decoder stopped unexpectedly.".into()));
    let _ = slint::invoke_from_event_loop(move || {
        let Some(app) = crate::app() else { return };
        ..
    });
});
```

## 3. Events from worker threads

A sink called from a transfer thread only stores the newest event and queues one drain on the UI
thread (`job_start` in `jobs/queue.rs`; the `Inbox` struct is in `jobs.rs`):

```rust
inbox.events.lock().unwrap().insert(id, event);
if !inbox.dirty.swap(true, Ordering::AcqRel) {
    let _ = slint::invoke_from_event_loop(|| {
        if let Some(app) = crate::app() {
            app.jobs_drain();
        }
    });
}
```

The dirty flag makes a burst of progress events cost one UI wake-up. `jobs_publish` then updates
rows in place when the row count is unchanged, so a tick does not rebuild the list. Idle and
redraw cost is measured in this project (`docs/TESTING.md`, section 9), so keep both habits.

## 4. `RefCell` borrows

End a borrow before calling anything that may borrow the same cell (`job_failed` in
`jobs/queue.rs`):

```rust
let job = jobs.remove(index);
drop(jobs);
self.jobs_publish();
```

`jobs_publish` borrows `self.jobs.jobs` again; without the `drop` the app would panic at run time.
When a helper takes a closure while holding a borrow, say so in its docs, as `with_place` in
`pane.rs` does:
"Run `f` on the pane at `place`. `f` must not open, close or move sessions."

## 5. Errors

Backend side (`crates/app/src/terminal.rs`): validate first, return a coded error, give context
when converting.

```rust
if let Some(name) = &tmux {
    if !valid_tmux_name(name) {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "invalid tmux session name",
        ));
    }
}
let conn = app.state.connection(&host_id).await?;
let (writer, mut reader) = conn
    .open_pty_or_tmux(cols.into(), rows.into(), tmux.as_deref())
    .await
    .map_err(|e| to_core("open ssh shell", e))?;
```

`to_core` (`crates/app/src/state.rs`) keeps an existing `CoreError`, maps storage errors, redacts
everything else, and logs one line. Backend messages are short lower-case fragments; they are not
shown alone.

UI side (`friendly_with` in `inventory/text.rs`, re-exported as `inventory::friendly_with`): the
code picks the sentence, the backend's wording follows in parentheses.

```rust
let lead = match error.code {
    ErrorCode::HostOffline => "The host did not answer. Check that it is on and reachable on this network.",
    ErrorCode::AdbUnauthorized => "The phone has not allowed USB debugging from this computer. Unlock it and accept the prompt.",
    ..
    _ => "Something went wrong.",
};
if error.message.is_empty() {
    lead.to_string()
} else {
    format!("{lead} ({})", error.message)
}
```

A new `ErrorCode` variant needs a sentence here. When a dialog knows more than the code does,
lead with its own sentence, as `said(lead, &error)` in `wireless.rs` does.

Validation that drives a disabled button returns the reason as text (`wireless.rs`):

```rust
/// Why `address` cannot be used as "IP address and port", or None when it can.
pub fn address_problem(address: &str) -> Option<&'static str> { .. }
```

## 6. Backend API shape (`crates/app`)

`crates/app/src/lib.rs` is a list of thin methods on `App`, grouped by a banner comment per area;
the work is a module function that takes `&Arc<App>` first.

```rust
/// Read the host key the server presents, without authenticating.
pub async fn host_check_key(self: &Arc<Self>, id: String) -> CmdResult<HostKeyCheck> {
    on_rt!(self, hosts::host_check_key(id))
}
```

`on_rt!` runs the function on `App::rt`, so the method can be awaited from any executor and never
needs `block_on` (`crates/app/src/lib.rs:5-18` states the boundary rules). Add a new operation the
same way: a module function returning `CmdResult<T>`, plus a one-line method. Take owned
arguments (`String`, not `&str`) on async methods, because the work moves to another task.

## 7. Tests

Names are sentences; fixtures are small local functions (`palette.rs`):

```rust
#[test]
fn the_detail_matches_too_but_ranks_after_titles() {
    let all = vec![
        entry("Open screen of Redmi", "Bench host  >  USB"),
        entry("Open workspace Bench", "Saved workspace"),
        entry("Bench notes", "x"),
    ];
    assert_eq!(
        titles(&filter(&all, "bench")),
        [
            "Bench notes",
            "Open workspace Bench",
            "Open screen of Redmi"
        ]
    );
    assert_eq!(titles(&filter(&all, "usb")), ["Open screen of Redmi"]);
}
```

More names to imitate: `a_missing_or_broken_file_means_the_defaults`,
`what_is_written_reads_back` (`settings.rs`); `only_file_copies_can_be_retried_with_replace`
(`jobs/spec.rs`); `second_holder_is_refused_until_the_first_lets_go` (`crates/app/src/instance.rs`);
`only_transport_errors_are_retryable` (`crates/core/src/error.rs`). Older tests with short names
(`file_names`, `progress_words`) exist; do not copy that style and do not rename them in passing.

A comment inside a test explains the case, not the assertion: "Already replacing: a second clash
is a real failure." (`jobs/spec.rs`).

## 8. Allowing a lint

On one item, with the reason next to it (`Pane` in `pane.rs`; `crates/app/src/crosshost.rs`):

```rust
// A handful of long-lived panes: boxing the large ones would only add a hop to every access.
#[allow(clippy::large_enum_variant)]
pub enum Pane { .. }

#[allow(clippy::too_many_arguments)] // one transfer, fully described
```

## 9. `unsafe`

Only in `crates/app/src/instance.rs`, for Win32 calls. Each block has a `// SAFETY:` comment that
states the fact that makes the call sound:

```rust
// SAFETY: `wide` is a valid NUL-terminated UTF-16 string that outlives the
// call; a null security descriptor means the default one.
let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
```

## 10. Commit messages

Commit only when asked. The shape used in this repo (`git log`, for example `68f6278`):

```text
Idle CPU fix, software renderer by default, dark title bar

- Home: a health sample redraws only that host's card instead of
  rebuilding every list; ..
- Docs: CHANGELOG, TASKS (UI-MIG-05/11), TESTING section 9.

Not verified: dark title bar with the light Windows theme, mirror CPU
with the software renderer.
```

A subject that names the change, a bullet per area saying what changed and why with measured
numbers where they exist, then a "Not verified:" paragraph. The rustc dev guide's LLM guidance
asks contributors to disclose the extent and purpose of LLM use and shows a bare tool trailer as
a bad disclosure, because it says nothing about what was done
(https://rustc-dev-guide.rust-lang.org/llm-guidance/writing.html). In this repo the body and the
"Not verified:" paragraph carry that information; whatever attribution trailer the owner's
tooling adds comes after them.
