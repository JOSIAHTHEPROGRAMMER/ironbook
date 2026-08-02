# Contributing to ironbook

## Setup

1. Install Rust via [rustup](https://rustup.rs).
2. Clone the repo, `rustup` will pick up the pinned toolchain from `rust-toolchain.toml` automatically.
3. Run `cargo build` and `cargo test` to confirm everything works.

## Before opening a pull request

Run these locally, CI runs the same checks and will block the merge if any fail.

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

## Commit messages

This project follows [Conventional Commits](https://www.conventionalcommits.org). Examples:

```
feat: add price time priority matching
fix: correct partial fill quantity on market orders
test: add randomized matching stress test
docs: document order book time complexity
```

## Code style

- Idiomatic Rust, small focused functions, single responsibility.
- Comments explain why, not what. No decorative banners, no filler comments.
- Public items need doc comments, `cargo doc` runs with warnings denied in CI.
- Avoid unnecessary allocations and cloning, this is a matching engine, performance is a feature.

## Project phases

Development follows the phases described in the README roadmap. Each phase is scoped to compile, pass tests, and be independently usable before the next one starts.
