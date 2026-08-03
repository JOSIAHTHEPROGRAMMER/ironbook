//! Core domain logic for ironbook.
//!
//! This crate holds everything that does not depend on a user interface.
//! It owns the order book, the matching engine, and the domain types that
//! represent orders and trades. The CLI crate is a thin client on top of
//! this library, and any future interface, a REST API, a WebSocket feed,
//! can be built the same way without touching this crate's public API.

#![doc(html_root_url = "https://docs.rs/orderbook-core")]

pub mod errors;
pub mod orders;
pub mod trade;
pub mod types;

/// Returns the crate version as declared in Cargo.toml.
///
/// This exists mostly as a canary for the CI pipeline in phase one,
/// giving the pipeline one real function with a real test to check
/// before any domain logic exists.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_toml() {
        assert_eq!(version(), "0.1.0");
    }
}
