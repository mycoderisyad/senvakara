---
name: rust-guidelines
description: Rust coding guidelines for the Senvakara workspace (crates/* backend crates and the apps/senvakara native Slint desktop app). Use whenever writing, editing, refactoring or reviewing .rs files in this repo, adding or splitting a module, adding a crate, or changing any Cargo.toml (dependencies, version pins, features, profiles). Covers the house style and module layout, naming, imports, comments and doc comments, errors and user-facing text, keeping work off the UI thread, tests, Clippy and unsafe, Cargo.toml conventions, build constraints on this machine, and the checks to run before finishing. Not for .slint files.
---

# Rust guidelines for Senvakara

Rust 1.94.1, edition 2021 (`Cargo.toml:7`), one workspace: `crates/*` plus `apps/senvakara`
(`Cargo.toml:3`). Rules are of two kinds: **official** rules link to the page they come from;
**house** rules cite the repo file they were read from. App sources (`apps/senvakara/src`) are
cited by file and item name, bare names like `jobs/queue.rs` being relative to that folder;
`docs/AGENTS.md` is cited by rule or section; line numbers appear only for other docs, manifests
and `crates/*`.

`.slint` files are out of scope: the `slint:slint` plugin skill covers them.

Read when needed:

- `references/module-layout.md`: before adding, splitting or moving a module in the app crate.
- `references/house-patterns.md`: before writing a feature module, a backend call from the UI, an
  error message or a commit message; it shows the patterns with real code.
- `references/official-sources.md`: for what each official page says in more detail, and the list
  of points where this repo deliberately differs from it.

## Order of authority

1. `cargo fmt` decides layout. There is no `rustfmt.toml`, so it is the default Rust style for
   edition 2021 ([Style Guide][sg]); never hand-format against it, because CI rejects the diff.
2. CI decides what merges: fmt, Clippy with `-D warnings`, tests (`.github/workflows/ci.yml:21-26`).
3. This skill decides what a tool cannot: names, grouping, comments, module shape, manifests.
4. In an old file, match the file. Do not restyle code you are not changing, because a mixed diff
   hides the real change (`docs/AGENTS.md`, rule 11).

## Build constraints on this machine

- Use no language or library feature newer than Rust 1.94, because local builds run 1.94.1. CI
  installs current stable and nothing pins the toolchain (`ci.yml:17`; no `rust-toolchain` file),
  so a lint can fail in CI that local Clippy does not know.
- Slint stays at `=1.18.1` for `slint` and `slint-build` (`apps/senvakara/Cargo.toml:17,35`),
  because Slint 1.19 needs rustc 1.95 (`docs/DECISIONS.md:31`, ADR-023; `docs/TECH_STACK.md:9`).
  `sysinfo` stays at 0.38 for the same reason (`crates/app/Cargo.toml`; `docs/TESTING.md:119`).
- Drive D: is short on space; a debug cache once filled it (`docs/TESTING.md:119`). While
  iterating, check one crate, not the workspace:

  ```sh
  cargo check -p senvakara
  cargo clippy -p senvakara-app --all-targets -- -D warnings
  cargo test -p senvakara-storage
  ```

- Never change workspace-wide rustflags or the `[profile.*]` tables in the root `Cargo.toml`: the
  dev profile is tuned to keep `target/` small (`Cargo.toml:25-31`), and a flag change rebuilds
  every dependency.

## Layers

- `apps/senvakara` is the Slint UI: `.slint` files draw view models, Rust owns all state and logic.
  `crates/app` is the headless service layer; the other crates are adapters, storage, secrets,
  metrics and core types (`docs/ARCHITECTURE.md:8-11`, `crates/app/src/lib.rs:1-18`).
- Never add `slint` or window types under `crates/`, so the backend stays testable headless.
- UI code reaches the backend through `senvakara_app::App` (`self.backend`), not an adapter crate
  (`docs/AGENTS.md`, Coding conventions). The one exception is the local terminal, which uses
  `senvakara-adapters-pty` directly (`apps/senvakara/Cargo.toml:11`).

## Module layout in `apps/senvakara/src`

The tree follows these rules; keep it that way. `references/module-layout.md` walks through the
real `jobs` and `files` modules and what the split taught.

1. A module root is `src/<name>.rs` with its children in `src/<name>/` (`jobs.rs` with
   `jobs/queue.rs` and `jobs/spec.rs`). No `mod.rs`: the Reference encourages the `<name>.rs`
   form because it avoids many files named `mod.rs` ([Reference, modules][ref-mod]).
2. The root holds the `//!` module docs, `mod` declarations, re-exports and small core types.
   Children are split by responsibility (pure model, parse, format and validation helpers; the
   `State` struct; `impl App` handler groups by user activity), never by line count alone,
   because a file cut at an arbitrary line has no name that says what it holds. A type whose
   private fields many children touch stays in the root (`Job` and `State` in `jobs.rs`, `Side`
   and `FilesPane` in `files.rs`), because children can read an ancestor's private items.
3. Aim for 400 lines or fewer per file and leave none above 600 (the largest today is
   `logcat.rs` at 562). A file is split before it could reach 1,000.
4. Name a file after what it holds (`files/paths.rs`, `inventory/metrics.rs`). No `utils.rs`,
   `helpers.rs`, `misc.rs` or `common.rs`, because those names collect unrelated code.
5. Use the narrowest visibility that works: private, then `pub(super)`, then `pub(crate)`. Other
   modules reach a feature only through its root re-exports, so its inner files can change
   without touching callers. Re-export only what outside code names: an unused `pub use` fails
   the build under `-D warnings`.
6. Unit tests sit next to the code they exercise in `#[cfg(test)] mod tests`. A tests block over
   about 150 lines moves to its own `tests.rs` (`automation/draft/tests.rs`,
   `workspace/stored/tests.rs`), so the code file stays readable.
7. A refactor is pure code motion in its own change, never mixed with a behaviour change, so a
   reviewer can check it by comparing moved text.
8. `main.rs` stays a thin crate root: crate docs, the crate attribute, the `mod` list, the root
   re-exports, `slint::include_modules!()` and `fn main`. App-wide code lives in `state.rs`
   (`App`, `app()`, `notifier`), `pane.rs` (`Pane`, `Place`), `confirm.rs` (`Question`,
   `Pending`, `ask_with`), `tabs.rs`, `present.rs`, `input.rs` and `platform.rs`.

Do not use `#[path]` to place a file elsewhere ([Style Guide, other advice][sg-advice]).

## Shape of a feature module in the app

Code for each point is in `references/house-patterns.md`.

- Start with `//!` docs that say what the feature is for the user, plus any thread or storage fact
  a reader needs first (the first lines of `settings.rs` and `term.rs`).
- Keep rules and wording in pure functions and plain types with no Slint handle (`palette.rs`
  `filter`; `wireless.rs` `address_problem`; `settings.rs` `Values::parse`), because those can be
  unit-tested without a window.
- Hold mutable state in one `#[derive(Default)] struct State` with private fields: `Cell` for
  `Copy` values, `RefCell` for collections, `Rc<VecModel<T>>` for models (`palette.rs`). `App`
  owns one `State` per feature in a `pub(crate)` field (`state.rs`, `struct App`).
- Add behaviour as `impl App` blocks: one `init_<feature>(self: &Rc<Self>, ui:
  &crate::MainWindow)` that binds models and callbacks, plus handlers named `<feature>_<verb>`
  (`palette_toggle`, `wireless_scan`, `job_start`, `files_load`). The prefix matters because
  every feature shares the one `App` namespace. A callback closure only clones `self` and calls
  one handler. A handler another file of the feature calls is `pub(super)` (`files/navigate.rs`).
- Never hold a `RefCell` borrow across a call that may borrow the same cell; end it first with a
  scope or `drop(...)` (`jobs/queue.rs` `job_failed`), because a second borrow panics at run time.

## Naming

- Official casing: types, traits and enum variants `UpperCamelCase`; functions, methods, fields,
  locals, modules and macros `snake_case`; constants and statics `SCREAMING_SNAKE_CASE`
  ([Style Guide, other advice][sg-advice]; [API Guidelines, C-CASE][api-naming]). An acronym is
  one word: `Uuid`, not `UUID` ([C-CASE][api-naming]); the repo agrees (`SftpUpload`, `VncTarget`).
- No `get_` prefix on getters: `fn id(&self)` ([C-GETTER][api-naming]; `pane.rs` `impl Pane`). The
  `get_x`/`set_x` calls you see are generated by Slint.
- Conversions: `as_` is free and borrowed, `to_` does work, `into_` consumes
  ([C-CONV][api-naming]; `settings.rs` `Values::to_text`).
- House: a feature's state struct is always `State`, addressed as `palette::State`, so the module
  name carries the meaning. In the app, bind full words (`error`, `entry`, `service`), not `e` or
  `s`, except in a one-expression closure.

## Imports and item order

- `cargo fmt` sorts imports inside a group; it never merges or reorders groups, so the grouping
  is yours ([Style Guide, items][sg-items]). Do not hand-sort names to match the guide: it
  describes the 2024 style edition, and this edition-2021 workspace gets the older order, for
  example `{json, Value}` ([Style Guide, style editions][sg-editions]).
- App crate: groups separated by a blank line: `std`, then external crates (including
  `senvakara_*`), then `crate::`, then `super::` or the file's own children (`palette.rs`,
  `jobs/queue.rs`, `files/navigate.rs`). Backend crates: one sorted block
  (`crates/app/src/terminal.rs:12-25`). Match the crate you are in.
- In a module root or crate root, write `mod` declarations first, then `pub use` re-exports, then
  `use` imports (`main.rs`, `jobs.rs`, `files.rs`, `inventory.rs`, `term.rs`;
  `crates/app/src/lib.rs:20-69`). The Style Guide puts imports before module declarations
  ([items][sg-items]); the house order is preferred because a root then reads as a table of
  contents. `hostform.rs`, `device.rs` and `desktop.rs` have imports first: leave them as is.
- Tests come last in a file. `impl App` comes after the types and helpers it uses.

## Comments and doc comments

- Use `//` and `///`, not block comments; one space after the sigil; a comment on its own line;
  complete sentences with a capital and a period. `//!` is only for module-level or crate-level
  docs. Doc comments go before attributes ([Style Guide, comments][sg]).
- Width: the Style Guide limits comment lines to 80 characters ([Style Guide][sg]). Backend crates
  follow that; the app crate wraps at 100. Match the crate you are in.
- Say what the thing is or means to its caller or user, not how the code works (`palette.rs`, doc
  of `filter`). Comment a field only when its name is not enough (`jobs.rs` `struct Job`).
- An inline `//` comment explains why: a constraint, a trap, a decision (`wireless.rs`: "The code
  is used up; never keep it around."). Delete comments that restate the code.
- Put threading and blocking contracts in the doc comment, because the types do not show them:
  "Blocking: run it off the UI thread." (`settings.rs` `decoder_self_test`).
- Name the requirement or ADR a rule comes from (`TERM-01`, `ADR-023`) so a reader can find the
  decision (`crates/app/src/terminal.rs:1-2`), and link items with rustdoc links such as
  ``[`JobSpec`]`` ([C-LINK][api-docs]; module docs of `jobs.rs`).
- For a `pub` function in `crates/*`, say when it fails or panics. The official form is an
  `# Errors` or `# Panics` section ([C-FAILURE][api-docs]); existing code uses a sentence.

## Errors and user-facing text

- Backend functions return `CmdResult<T>`, that is `Result<T, CoreError>` with an `ErrorCode` and
  a message (`crates/app/src/state.rs:65`, `crates/core/src/error.rs`). Convert internal errors
  with `to_core("what was being done", e)`, which redacts and logs (same `state.rs`, lines 67-78).
  The message must already be redacted: no secrets, key paths or credentials
  (`crates/core/src/error.rs:34-35`).
- Use `anyhow` with `.with_context(...)` inside adapters and at startup, and `thiserror` for error
  types callers match on (`crates/app/src/lib.rs:97-102`, `crates/core/src/error.rs:36`).
- Reject bad input at the boundary with `ErrorCode::InvalidInput`, before any side effect
  (`crates/app/src/terminal.rs:88-94`).
- In the UI, turn a `CoreError` into words with `inventory::friendly` or `friendly_with`
  (`inventory/text.rs`): a plain sentence that says what to check, with the backend's wording in
  parentheses. Never show a bare error code or `{error:?}` to the user.
- Write user text as full sentences that name the next step ("Add the port after the address, for
  example 192.0.2.20:37123."). Validation helpers return the problem as text, or nothing when the
  input is fine (`wireless.rs` `address_problem`), so the view can say why a button is disabled.
- Do not `unwrap()` or `expect()` on anything that depends on user input, files, the network or a
  device; handle it and show or log the error. `lock().unwrap()` on a `Mutex` is the house idiom.
- Log through `senvakara_app::logging`, which redacts every line (`crates/app/src/logging.rs`).
  Never log terminal output, file contents or logcat data (`docs/AGENTS.md`, rule 4).

## Threads and async

- The UI thread never waits (`docs/AGENTS.md`, Coding conventions). Start backend work with
  `self.run(async move { .. }, move |app, result| { .. })` (`inventory.rs` `App::run`): the future
  runs on the backend runtime and the closure runs back on the UI thread.
- Run blocking or CPU-heavy work on `std::thread::spawn` and return through
  `slint::invoke_from_event_loop` plus `crate::app()` (`settings.rs` `decoder_test`). `Rc<App>` is
  not `Send`, so a worker never captures it.
- A callback handed to a session thread may only queue work for the UI thread (`state.rs`
  `notifier`; `crates/app/src/lib.rs:16-18`). Coalesce bursts with a dirty flag (`jobs.rs`
  `Inbox`, used by `job_start` in `jobs/queue.rs`) so a fast stream does not flood the event loop.
- When a result arrives, check that it is still wanted: the dialog may have closed or moved on
  (`wireless.rs` `wireless_scan`, the `asked` check).
- Start no polling loop, stream or child process without a user action behind it, and tear it
  down on close; long operations report progress and can be cancelled (`docs/AGENTS.md`, rule 8).

## Tests

- Name a test as a sentence about behaviour: `an_empty_query_lists_everything_in_order`,
  `values_that_are_not_on_offer_fall_back_one_by_one` (`palette.rs`, `settings.rs`), so a failure
  reads as the broken promise.
- Cover adverse paths: bad input, missing file, cancelled work, stale route (`docs/AGENTS.md`,
  rule 12; `settings.rs` `a_missing_or_broken_file_means_the_defaults`).
- When fixing a bug, see the new test fail before the fix and pass after it
  ([rustc dev guide, LLM guidance][llm]).
- Integration tests live in `crates/<crate>/tests/` and use in-process fakes, not the real network
  (`crates/adapters-ssh/tests/fallback.rs`). No hardware means "not tested", never "passed"
  (`docs/TESTING.md:88`).

## Lints and unsafe

- `-D warnings` turns every Clippy and rustc warning into an error, `dead_code` included
  ([Clippy, usage][clippy-usage]).
- Fix a lint instead of allowing it. When the code is right as written, allow that one lint on
  that one item with a comment that says why ([Clippy, lints][clippy-lints]; `pane.rs` on `Pane`,
  `crates/app/src/crosshost.rs:33`). Do not allow a `clippy::correctness` lint: Clippy reserves
  that group for code that is outright wrong or useless ([Clippy, lints][clippy-lints]).
- Add no crate-wide `#![allow(...)]`, no `clippy.toml` and no extra lint group without the owner's
  decision; the repo has none today.
- `unsafe` exists only for the Win32 calls in `crates/app/src/instance.rs`, each with a
  `// SAFETY:` comment written from the API contract. Keep `unsafe` out of `apps/senvakara`.
- Leave no `dbg!`, `todo!`, `unimplemented!` or stray `println!` in shipped code
  ([rustc dev guide, LLM guidance][llm]: remove prototyping code and debugging).

## Cargo.toml

- Official form ([Style Guide, Cargo.toml conventions][sg-cargo]): `[package]` first, with `name`
  then `version` at the top and `description` last; a blank line between sections and none after
  a section header; bare keys; one space around `=`; a list or inline table on one line when it
  fits.
- Member crates inherit `version`, `edition`, `license` and `publish` with `<key>.workspace =
  true` and set their own `description` (every `crates/*/Cargo.toml`). Shared dependencies
  (`anyhow`, `serde`, `thiserror`) come from `[workspace.dependencies]` (`Cargo.toml:11-14`). A
  new backend crate goes in `crates/<name>` as package `senvakara-<name>`; the `crates/*` glob
  picks it up (`Cargo.toml:3`).
- Pin with `=` any dependency whose 0.x API moves or whose version is coupled to something else
  (ADR-016, `docs/DECISIONS.md:24`), as `russh`, `slint`, `alacritty_terminal`, `openh264` and
  `vnc-rs` are. Put a comment above every pin or non-obvious dependency saying why
  (`apps/senvakara/Cargo.toml:14-32`).
- Dependencies are grouped by purpose, not version-sorted as the Style Guide suggests
  ([Cargo.toml conventions][sg-cargo]). Add next to related entries; never re-sort in passing.
- Before adding a dependency ([rustc dev guide][llm]: think before adding dependencies), check
  that no existing crate already does the job, that it builds on rustc 1.94.1, that its licence
  is permissive (`docs/TECH_STACK.md:33`), and turn off default features you do not need (as
  `arboard`, `rfd`, `image` and `chrono` do). Name the dependency and its licence in your summary;
  `tools/licenses/gen-notices.mjs` builds the notices that ship.

## Security rules that reach Rust code

Read the twelve rules in `docs/AGENTS.md` and `docs/SECURITY.md` before touching connections,
commands or storage. In code: run programs as executable plus argument list, never a shell string
built from input (`docs/SECURITY.md:54`); bind every Android command to a concrete host and
transport id (rule 6); listen on loopback only (`docs/SECURITY.md:31`); never weaken host-key
checking (rule 3); keep secrets out of SQLite, logs and errors (`docs/SECURITY.md:33`).

## How to work

Carried over from [the rustc dev guide's LLM guidance][llm] where it fits this repo, together
with `docs/AGENTS.md`:

- Make the smallest change that solves the task; attach no unrelated refactors or cleanups.
- Verify against the code, docs and tests in front of you. Do not rely on memory for an API, a
  flag or a version: read the source or fetch the documentation.
- Avoid abstractions and compatibility layers nobody needs. Nothing here is published, so internal
  APIs may change freely; stored formats (`settings.json`, workspace JSON, SQLite migrations) are
  the contracts to keep.
- Be able to say what the change does, which behaviour is unchanged and which test shows it, and
  what you are still unsure about. Asking beats guessing.
- Report what you ran and what you did not (`docs/AGENTS.md`, rule 2). Commits here end their
  body with a "Not verified:" paragraph (`git log`, for example `68f6278`); a bare tool trailer
  says nothing about what was done.
- List new `// SAFETY:` comments and security-relevant code in your summary so the owner reviews
  them; the rustc guide reserves those for human authors.

## Before you finish

1. Run the three CI checks exactly as CI does (`.github/workflows/ci.yml:21-26`):

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   If a workspace run is not possible (disk space, another build holding `target/`), run the same
   checks with `-p <crate>` for every crate you touched and say so.
2. Treat what you generated as a draft and re-read the whole diff, not the conversation
   ([rustc dev guide][llm]): no debugging leftovers, no unrelated edits, no file over the size
   limits, no new `mod.rs`, a sentence-named test for new logic.
3. Update docs the change makes untrue: `docs/CHANGELOG.md` for user-visible changes,
   `docs/DECISIONS.md` for a new architectural choice or pin, the `docs/TASKS.md` checkbox only
   when the work is verifiably done (`docs/AGENTS.md`, workflow step 5).
4. In your summary, give the commands run with their real results, what was not run or not
   tested, and anything you could not verify. Never report a check as passed unless it ran
   (`docs/AGENTS.md`, Suggested checks).

[sg]: https://doc.rust-lang.org/stable/style-guide/index.html
[sg-items]: https://doc.rust-lang.org/stable/style-guide/items.html
[sg-editions]: https://doc.rust-lang.org/stable/style-guide/editions.html
[sg-advice]: https://doc.rust-lang.org/stable/style-guide/advice.html
[sg-cargo]: https://doc.rust-lang.org/stable/style-guide/cargo.html
[ref-mod]: https://doc.rust-lang.org/reference/items/modules.html
[api-naming]: https://rust-lang.github.io/api-guidelines/naming.html
[api-docs]: https://rust-lang.github.io/api-guidelines/documentation.html
[clippy-usage]: https://doc.rust-lang.org/clippy/usage.html
[clippy-lints]: https://doc.rust-lang.org/clippy/lints.html
[llm]: https://rustc-dev-guide.rust-lang.org/llm-guidance/writing.html
