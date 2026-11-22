# Official sources: what they say, and where this repo differs

Read this when you need more of an official page than `SKILL.md` carries, or when house style and
an official page seem to disagree. Every page below was fetched on 2026-11-22; the summaries
cover only what a formatter cannot decide for you. Pages change: when a detail matters, fetch the
page again instead of trusting this file.

`cargo fmt` implements the Style Guide's layout rules (indentation, line breaks, trailing commas,
spacing, the 100-column width). They are not repeated here. Run the tool.

## The Rust Style Guide

Introduction, comments and attributes: https://doc.rust-lang.org/stable/style-guide/index.html

- The guide defines the default style and recommends it; `rustfmt` uses it as its reference. If
  the two differ, that is a bug in one of them.
- Sorting is "version sorting": `x8` before `x16`, `_` treated as a word separator, upper-case
  before lower-case.
- Comments (recommendations; a formatter may skip them): prefer `//` to `/* */`; one space after
  the sigil; prefer a comment on its own line; comments should usually be complete sentences
  starting with a capital and ending with a period; a comment-only line is limited to 80
  characters excluding indentation, or the maximum line width, whichever is smaller.
- Doc comments: prefer `///` to `/** */`; use `//!` only for module-level or crate-level
  documentation; put doc comments before attributes.
- Attributes: each on its own line; prefer outer attributes; only one `derive` attribute per
  item.

Items: https://doc.rust-lang.org/stable/style-guide/items.html

- `use` statements and `mod foo;` declarations come before other items. Put imports before
  module declarations. Version-sort each, with `self` and `super` first.
- A group of imports is a run of imports with no blank line or other item between them. Sort
  within a group; never merge or reorder groups. Tools do not merge or un-merge import lists by
  default.
- Function definitions must stay searchable as `fn name`; avoid comments inside a signature.
- Prefer a unit struct (`struct Foo;`) to an empty struct. For a tuple struct with more than a
  few fields, prefer named fields.
- If an enum has struct variants that need several lines each, consider factoring the fields into
  their own struct.
- Prefer single-letter names for generic parameters. Prefer a `where` clause when a generics
  clause would need line breaks; prefer an inline bound when the `where` clause would be very
  short.
- Always write the ABI on extern items (`extern "C" fn`).

Statements: https://doc.rust-lang.org/stable/style-guide/statements.html

- Entirely layout (let, let-else, macro and expression statements). One habit to keep by hand:
  end an expression statement of unit type with a semicolon even where it could be the tail
  expression.

Expressions: https://doc.rust-lang.org/stable/style-guide/expressions.html

- Mostly layout. Points a tool leaves to you: prefer dereferencing to taking references in
  comparisons (`*t op u`, not `t op &u`); use parentheses where they make precedence clear, and
  put them around compound range bounds (`0..(x - 10)`); qualify enum variants with the enum name
  unless the enum is in the prelude; do not mix upper and lower case inside one hex literal, and
  use one case across a project (this repo: predominantly lower case, as in `0xffe1`; write new
  literals that way).

Types: https://doc.rust-lang.org/stable/style-guide/types.html

- Entirely layout (spacing and line breaks in types and bounds).

Other style advice: https://doc.rust-lang.org/stable/style-guide/advice.html

- Prefer expression-oriented code: `let x = if y { 1 } else { 0 };` rather than declaring `x` and
  assigning in each branch.
- Names: types and enum variants `UpperCamelCase`; struct fields, functions, methods, local
  variables and macros `snake_case`; constants and immutable statics `SCREAMING_SNAKE_CASE`.
- For a name that is a reserved word, use a raw identifier (`r#crate`) or a trailing underscore
  (`crate_`); do not misspell it (`krate`).
- Avoid `#[path]` annotations where possible.

Cargo.toml conventions: https://doc.rust-lang.org/stable/style-guide/cargo.html

- Same line width and indentation as Rust code. A blank line before each section header; none
  between a header and its keys or between keys.
- `[package]` at the top, with `name` and `version` first and `description` last. Version-sort
  key names in every other section.
- Bare keys for standard key names; a single space on both sides of `=`.
- An array or inline table goes on one line when it fits; otherwise block-indent an array with a
  trailing comma on every item, and turn a long table into its own section.
- `license` must be a valid SPDX expression. In `description`, wrap at 80 columns and do not
  start with the crate name.

Guiding principles: https://doc.rust-lang.org/stable/style-guide/principles.html

- Priority order: readability (including scan-ability and reading in plain-text contexts such as
  diffs and grep), aesthetics, specifics (small diffs, no rightward drift, little vertical space),
  application (easy to apply by hand and by tools).

Style editions: https://doc.rust-lang.org/stable/style-guide/editions.html

- Changes to the default style only appear in style editions. Code in a given Rust edition uses
  the matching style edition by default. Rust 2015, 2018 and 2021 share one style edition; the
  current guide describes the Rust 2024 style edition.
- Notable 2024 changes listed on the page: version-sort (`x8`, `x16`, `x32`, `x64`, `x128` in
  that order) and "non-lowercase before lowercase" in place of the older "ASCIIbetical" sort.
- Consequence here: the workspace is edition 2021 (root `Cargo.toml:7`), so `cargo fmt` applies
  the older style edition. Where the guide's sorting rules and the tool's output differ, for
  example `use serde_json::{json, Value};` in `apps/senvakara/src/settings.rs`, the tool is
  right for this repo. The same caution applies to every "version-sort" mentioned above for Rust
  code; the Cargo.toml conventions are not applied by rustfmt and are unaffected.

Nightly-only syntax: https://doc.rust-lang.org/stable/style-guide/nightly.html

- Covers only nightly features (at fetch time: frontmatter). Not relevant: this project builds
  on stable 1.94.1.

## The Rust Reference, modules

https://doc.rust-lang.org/reference/items/modules.html

- A `mod name;` without a body is loaded from a file; the path mirrors the module path, with
  ancestor modules as directories.
- A module's file is `name.rs`, or alternatively `name/mod.rs`; having both is not allowed. The
  page's note says `mod.rs` was required before rustc 1.30 and that the newer convention is
  encouraged because it is more consistent and avoids many files named `mod.rs`.
- `#[path = "..."]` can override the file location, with different base directories for
  `mod.rs`-style and other files.

## Rust API Guidelines

The guidelines target published libraries. This workspace publishes nothing
(`publish = false`, root `Cargo.toml:9`), so they apply where they make code easier to read.
Identifiers are from https://rust-lang.github.io/api-guidelines/checklist.html

Naming: https://rust-lang.github.io/api-guidelines/naming.html

- C-CASE: modules, functions, methods, locals `snake_case`; types, traits, enum variants
  `UpperCamelCase`; statics and constants `SCREAMING_SNAKE_CASE`; type parameters a concise
  `UpperCamelCase`, usually one letter; lifetimes short lower-case. Acronyms count as one word
  (`Uuid`, `Stdin`).
- C-CONV: `as_` for free borrowed-to-borrowed views, `to_` for expensive conversions, `into_`
  for owned-to-owned conversions that consume the value.
- C-GETTER: no `get_` prefix on getters; `get` alone only where there is a single obvious thing
  to get.
- C-ITER: `iter`, `iter_mut`, `into_iter` for methods on collections.
- C-FEATURE: a Cargo feature name has no placeholder words such as `use-` or `with-`.
- C-WORD-ORDER: use one word order across related names (the standard library uses
  verb-object-error, as in `ParseIntError`).

Documentation: https://rust-lang.github.io/api-guidelines/documentation.html

- C-FAILURE: document error conditions in an "Errors" section, panic conditions in a "Panics"
  section, and the caller's obligations for an unsafe function in a "Safety" section.
- C-LINK: link to related items from prose.
- C-EXAMPLE and C-QUESTION-MARK: every public item has a rustdoc example, written with `?`
  rather than `unwrap`. Applied "within reason" by the page's own words.
- C-HIDDEN: keep implementation details out of rustdoc; `pub(crate)` is one tool for that.

## Clippy

Usage: https://doc.rust-lang.org/clippy/usage.html

- `cargo clippy` runs the default set, the `clippy::all` group. `-D warnings` fails the build on
  any warning, including rustc's own such as `dead_code`.
- The page says that since Cargo 1.97 the recommended way to deny warnings in CI is the
  `build.warnings` config option (`CARGO_BUILD_WARNINGS=deny`), and that older Cargo uses
  `cargo clippy -- -Dwarnings`. Local Cargo here is 1.94.1 and CI uses the `-- -D warnings` form
  (`.github/workflows/ci.yml:24`); do not change CI as a side effect of other work.
- `cargo clippy -p <crate>` lints one workspace member and its workspace dependencies; adding
  `-- --no-deps` restricts it to that crate.
- `cargo clippy --fix` applies suggestions and implies `--all-targets`. Review what it changed
  like any other diff.
- `pedantic` and `restriction` are allow-by-default groups. The page says not to enable the whole
  `restriction` group. This repo enables neither.

Lint groups: https://doc.rust-lang.org/clippy/lints.html

- `correctness` is deny-by-default: the code is outright wrong or useless; allowing these lints
  is not recommended.
- `suspicious`: fix it; if the code is intentional, allow the lint specifically and locally and
  give a reason.
- `complexity`: usually simplify; allow case by case when the longer form is clearer.
- `perf`: usually easy to apply, and recommended.
- `style`: the most opinionated warn-by-default group; disagreeing is permitted.

## rustc dev guide: writing code with LLMs

https://rustc-dev-guide.rust-lang.org/llm-guidance/writing.html

The page is the Rust compiler project's policy for its own contributors. Carried over into
`SKILL.md` because it fits any codebase:

- Make the change small and targeted; make the smallest change that fixes the problem and do not
  combine it with unrelated refactors or cleanups.
- When fixing a bug, verify that the test fails before the change and succeeds after it. Tests
  are required.
- For mass renames or rewrites, strongly prefer a proper syntax rewrite tool.
- Think before adding dependencies.
- Verify your understanding against the existing code, documentation and tests; do not rely on
  the LLM as a source of truth.
- Treat generated code as a draft. Avoid unnecessary abstractions and compatibility layers.
- Remove outdated or prototyping code and debugging. Re-read the whole diff, not the
  conversation.
- Run tests. Report manual testing or benchmarking you did.
- Be able to explain the change: the bug, why this fix, edge cases, invariants preserved, what
  is unchanged and which test establishes it, what you are still not certain about. It is fine
  to be uncertain and to ask.
- Disclose the extent and purpose of LLM use, not the model name. A bare tool trailer is the
  page's example of a bad disclosure.

Not carried over, because it is specific to contributing to rust-lang/rust: finding a reviewer on
Zulip, the `E-easy` and `E-mentor` labels, rustc's test, performance and diagnostic-snapshot
procedures, and the bans on LLM-written PR descriptions, GitHub comments and commit messages. This
repo is set up for agents to write code, summaries and commit messages (`docs/AGENTS.md`; the
co-author trailers in `git log`); the substance that the page protects (a human can understand
and check the change) is kept through small changes, honest "Not verified" reporting and owner
review.

## Where this repo differs from the official pages

| Topic | Official page | House style | What to do |
|---|---|---|---|
| Order of `mod` and `use` | Imports before module declarations (Style Guide, items) | `mod` first, then `pub use`, then `use` in most roots (`apps/senvakara/src/main.rs`, `jobs.rs`, `files.rs`, `inventory.rs`, `term.rs`; `crates/app/src/lib.rs:20-69` and the other `lib.rs` files). Three app roots have imports first: `hostform.rs`, `device.rs`, `desktop.rs` | Use the house order in a new root; do not reorder existing ones in passing; `cargo fmt` accepts both |
| Comment width | 80 characters (Style Guide, comments) | 80 in `crates/*`; 100 in `apps/senvakara` | Match the crate |
| Key order in `Cargo.toml` | Version-sorted keys outside `[package]` (Style Guide, Cargo.toml) | Dependencies grouped by purpose, path crates first, with comments | Keep the grouping; do not re-sort |
| `[package]` key order | `description` last | `description` last in `crates/*`; `build` follows it in `apps/senvakara/Cargo.toml:7-8` | Leave as is; put `description` last in a new manifest |
| `license` value | Valid SPDX expression | `license = "UNLICENSED"` (root `Cargo.toml:8`) | Leave it: the licence is an open owner decision (`docs/DECISIONS.md:57`) |
| Rustdoc examples | Every public item (C-EXAMPLE) | None; unit tests instead | Do not add doctests unless asked |
| Failure docs | `# Errors`, `# Panics` sections (C-FAILURE) | A sentence in the doc comment; `// SAFETY:` on unsafe blocks | Either form; never silence |
| Package metadata | authors, repository, keywords, categories (C-METADATA) | Only name, version, edition, license, publish, description | Nothing is published; add none |
| Reporting test runs | "Do NOT report which UI tests you ran" in a PR (rustc LLM guidance, about rustc's CI) | "Report actual command/test output" (`docs/AGENTS.md`, rule 2) | Follow `docs/AGENTS.md` |
| `description` wording | Do not start with the crate name (Style Guide, Cargo.toml) | Three manifests start with "Senvakara" | Leave them; describe the crate directly in a new manifest |
| LLM-written commit and PR text | Banned for rust-lang/rust | Existing commits are agent-written (co-author trailers in `git log`) | Write them when asked, with a "Not verified:" paragraph |
| Clippy invocation | `CARGO_BUILD_WARNINGS=deny` since Cargo 1.97 | `cargo clippy ... -- -D warnings` (`ci.yml:24`) | Use the CI form |
