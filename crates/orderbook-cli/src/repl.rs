//! The interactive command loop: read a line, parse it, run it, print
//! the result, repeat.

use std::io::{self, Write};

use clap::Parser;

use crate::commands::{Command, Outcome, execute};
use crate::session::Session;

/// A single line of input parsed into a command.
///
/// Wraps `Command` in a struct so clap has a top level parser to derive,
/// `Command` alone is a `Subcommand`, not a `Parser`, it needs a
/// container to be the thing `try_parse_from` is called on.
#[derive(Parser, Debug)]
#[command(
    name = "ironbook",
    no_binary_name = true,
    disable_help_subcommand = true
)]
struct ReplLine {
    #[command(subcommand)]
    command: Command,
}

/// Runs the interactive loop until the user exits or input ends.
pub fn run() {
    let mut session = Session::new();
    println!("ironbook interactive session, type help for a list of commands");

    loop {
        print!("> ");
        if io::stdout().flush().is_err() {
            break;
        }

        let mut line = String::new();
        let bytes_read = io::stdin().read_line(&mut line);
        match bytes_read {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                println!("error reading input: {error}");
                continue;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match parse(trimmed) {
            Ok(command) => {
                if !handle(command, &mut session) {
                    break;
                }
            }
            Err(error) => println!("{error}"),
        }
    }
}

fn parse(line: &str) -> Result<Command, clap::Error> {
    let tokens = line.split_whitespace();
    ReplLine::try_parse_from(tokens).map(|repl_line| repl_line.command)
}

/// Runs one already parsed command, returning false if the session
/// should end.
fn handle(command: Command, session: &mut Session) -> bool {
    match execute(command, session) {
        Outcome::Lines(lines) => {
            for line in lines {
                println!("{line}");
            }
            true
        }
        Outcome::Clear => {
            print!("\x1B[2J\x1B[1;1H");
            true
        }
        Outcome::Exit => {
            println!("goodbye");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_a_valid_command() {
        let result = parse("buy --symbol AAPL --price 100.00 --quantity 10 --client-order-id 1");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_rejects_an_unknown_command() {
        let result = parse("frobnicate");
        assert!(result.is_err());
    }

    #[test]
    fn parse_rejects_a_missing_required_flag() {
        let result = parse("buy --symbol AAPL --price 100.00 --quantity 10");
        assert!(result.is_err());
    }

    #[test]
    fn parse_reads_a_bare_command_with_no_arguments() {
        assert!(parse("help").is_ok());
        assert!(parse("exit").is_ok());
        assert!(parse("clear").is_ok());
        assert!(parse("reset").is_ok());
    }

    #[test]
    fn handle_exit_signals_the_loop_to_stop() {
        let mut session = Session::new();
        let should_continue = handle(Command::Exit, &mut session);
        assert!(!should_continue);
    }

    #[test]
    fn handle_help_signals_the_loop_to_continue() {
        let mut session = Session::new();
        let should_continue = handle(Command::Help, &mut session);
        assert!(should_continue);
    }
}
