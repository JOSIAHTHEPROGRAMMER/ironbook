//! Command line entry point for ironbook.
//!
//! Launches the interactive session, reading commands from standard
//! input until the user exits.

mod commands;
mod formatting;
mod repl;
mod session;

fn main() {
    repl::run();
}
