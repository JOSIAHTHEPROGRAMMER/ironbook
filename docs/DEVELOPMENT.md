# Development

This document covers local setup, the verification workflow every change goes through, the git and PR process, and every gotcha that has actually been hit while building this project. If something here seems overly specific, it's because it was learned the hard way once and is recorded so it doesn't have to be relearned.

## Prerequisites

- Rust 1.97.1, pinned in `rust-toolchain.toml`. If you have `rustup` installed, running any `cargo` command in the repo root will pick up the pinned version automatically.
- No other tooling is required to build or test the project. Criterion benchmarks and `cargo doc` work with the same toolchain.

## Local setup

```bash
git clone https://github.com/JOSIAHTHEPROGRAMMER/ironbook.git
cd ironbook
cargo build
cargo test
```

## Verification workflow

Every change, before it's considered done, passes all four of these:

```bash
cargo fmt --all --check
cargo build
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
```

`clippy::pedantic` is enabled workspace wide (`Cargo.toml`'s `[workspace.lints.clippy]`), along with `unsafe_code = "deny"` and `missing_docs = "warn"`. A change that introduces a new `#[allow(...)]` should have a comment explaining why the lint doesn't apply, not just silence it.

### Running the benchmarks

```bash
cargo bench
```

This runs the full Criterion statistical suite across all five workload shapes at 100, 1,000, 10,000, and 100,000 orders, twenty cases total. At the 100,000 order scale this can take several minutes; Criterion's default is around 100 samples per case. For a quick correctness check without the full statistical run:

```bash
cargo bench -- --test
```

This runs every case once, confirming nothing panics, without collecting timing statistics. See [BENCHMARKING.md](BENCHMARKING.md) for how to interpret the actual numbers.

### Running the CLI locally

```bash
cargo run --bin ironbook
```

Type `help` inside the session for the full command list.

### Building the docs

```bash
cargo doc --workspace --no-deps --open
```

The published version at GitHub Pages is rebuilt automatically on every push to `main` by `.github/workflows/docs.yml`.

## Git and PR workflow

Branch protection on `main` requires a PR, requires status checks to pass, and blocks force pushes and branch deletion. Every phase or logical change is one feature branch, one commit, one PR:

```bash
git checkout -b feat/whatever
git add .
git commit -m "feat: conventional commit message"
git push -u origin feat/whatever
gh pr create --title "feat: conventional commit message" --body-file test.md
gh pr merge --auto --squash --delete-branch
```

`test.md` is a scratch PR description file, gitignored, regenerated fresh for every PR, matching the structure of `.github/PULL_REQUEST_TEMPLATE.md` with checklist boxes reflecting what was actually verified. `gh pr create --fill` does not work for this, it pulls only from the commit message and ignores the template entirely.

## Known gotchas

These have each caused a real problem once. Recorded here so they don't cause a second one.

**`dtolnay/rust-toolchain` does not read `rust-toolchain.toml` automatically.** Every CI workflow that needs a specific Rust version hardcodes it in an `env:` block (`RUST_TOOLCHAIN: 1.97.1`) rather than assuming the action picks up the pinned version from the repo.

**GitHub branch protection rulesets with matrix jobs need the resolved check names added individually.** A rule requiring `Test (${{ matrix.os }})` as a literal string will never match; the ruleset needs the actual resolved names (`Test (ubuntu-latest)`, `Test (macos-latest)`, `Test (windows-latest)`), and those only become selectable in the ruleset UI after the workflow has run at least once.

**`gh pr merge --auto` needs "Allow auto-merge" enabled separately.** It's a repo setting under Settings → General, not something the `gh` CLI or a workflow can turn on from outside.

**A hand written `help` subcommand in `clap` collides with clap's own auto-registered one.** `disable_help_subcommand = true` on the top level `#[command(...)]` attribute is required, or the REPL panics at runtime the first time `help` is typed, this was caught by an actual piped session, not by unit tests, since the panic only happens when the two `help` registrations are both present at once.

**`codecov.yml` belongs at the repository root, not inside `.github/workflows/`.** A file with that name placed in the workflows directory gets validated by GitHub as an Actions workflow (missing `on:`/`jobs:` keys, since it isn't one) instead of being read as Codecov's coverage configuration. The actual coverage generating workflow (`cargo llvm-cov` plus `codecov/codecov-action`) is a separate file, `coverage.yml`.

**A function already returning something `#[must_use]` internally does not need a redundant `#[must_use]` of its own.** `HashMap::values()` is already `#[must_use]`; wrapping it in a function and adding `#[must_use]` again is unnecessary and clippy will not ask for it, don't add it defensively.

**GitHub Pages' `environment: name: github-pages` may show as invalid in an editor's Actions extension before the environment exists.** This is the editor validating against a locally cached list of known environments, not a real error in the workflow. GitHub creates the `github-pages` environment automatically on first successful deploy, or immediately once Pages' source is manually switched to "GitHub Actions" in repo Settings → Pages, which is also the one manual step no workflow can perform from outside.

**Files delivered outside the actual git history (copy pasted, downloaded, regenerated from a different source of truth) can pick up formatting drift**, extra blank lines, a missing trailing newline, that `cargo fmt --check` will flag even though the content is otherwise correct. Running plain `cargo fmt` (not `--check`) resolves this in place; it's not a sign the underlying change is wrong.

**Never regenerate a `Cargo.toml` from a scratch or sandbox copy of the crate.** A `Cargo.toml` reconstructed from an isolated verification environment can silently carry over that environment's simplified fields, wrong path dependencies, or a different pinned Rust version, none of which show up as a compile error until the file is actually swapped into the real workspace. When only a dependency needs to change, edit the real file's diff, don't regenerate the whole thing from elsewhere.

## Coverage

`codecov.yml` (repository root) excludes `crates/orderbook-cli/src/main.rs` from the coverage percentage, it's a one line binary entry point that no test suite can call directly, that's not a gap in testing, it's structurally how Rust binaries work. `repl.rs`'s `run()` function is similarly excluded in spirit, though not mechanically: it blocks on real stdin, so `parse()` and `handle()` were deliberately split out of it specifically so the actual logic is unit testable even though the thin I/O loop around them isn't. This is an accepted, intentional structural gap, not something to force coverage onto.
