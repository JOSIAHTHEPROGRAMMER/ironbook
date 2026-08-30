//! Entry point for the interactive ironbook session.

mod benchmark;
mod commands;
mod formatting;
mod repl;
mod session;

fn main() {
    repl::run();
}
