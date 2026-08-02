//! Command line entry point for ironbook.
//!
//! Phase one only wires up argument parsing and a version command so the
//! CI pipeline has a real binary to build and test. The actual trading
//! commands, buy, sell, cancel, and so on, are added in phase six once
//! the matching engine exists.

use clap::Parser;

/// ironbook command line trading engine.
#[derive(Parser, Debug)]
#[command(name = "ironbook", version, about, long_about = None)]
struct Cli {
    /// Print the running orderbook-core version and exit.
    #[arg(long)]
    core_version: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.core_version {
        println!("orderbook-core {}", orderbook_core::version());
        return;
    }

    println!("ironbook {}", env!("CARGO_PKG_VERSION"));
    println!("no commands implemented yet, this lands in phase six");
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        // this catches clap configuration mistakes, like conflicting
        // flags or bad defaults, without needing to spawn the binary
        Cli::command().debug_assert();
    }
}
