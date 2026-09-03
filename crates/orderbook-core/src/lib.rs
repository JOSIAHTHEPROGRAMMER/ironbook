//! Core domain logic for ironbook.
//!
//! This crate holds everything that does not depend on a user interface.
//! It owns the [`orderbook::OrderBook`], the [`matching::MatchingEngine`],
//! and the domain types that represent orders and trades. The CLI crate
//! is a thin client on top of this library, and any future interface, a
//! REST API, a WebSocket feed, can be built the same way without
//! touching this crate's public API.
//!
//! # Example
//!
//! ```
//! use orderbook_core::matching::MatchingEngine;
//! use orderbook_core::types::{ClientOrderId, Price, Quantity, Side, Symbol};
//!
//! let mut engine = MatchingEngine::new(Symbol::new("AAPL"));
//!
//! // A resting sell order provides liquidity at 100.00.
//! engine
//!     .submit_limit_order(
//!         ClientOrderId::from_raw(1),
//!         Side::Sell,
//!         Price::from_ticks(10_000),
//!         Quantity::from_units(10),
//!     )
//!     .unwrap();
//!
//! // A crossing buy order matches against it, at the maker's price.
//! let report = engine
//!     .submit_limit_order(
//!         ClientOrderId::from_raw(2),
//!         Side::Buy,
//!         Price::from_ticks(10_050),
//!         Quantity::from_units(4),
//!     )
//!     .unwrap();
//!
//! assert_eq!(report.trades().len(), 1);
//! assert_eq!(report.trades()[0].price(), Price::from_ticks(10_000));
//! ```

pub mod errors;
pub mod matching;
pub mod metrics;
pub mod orderbook;
pub mod orders;
pub mod trade;
pub mod types;
/// Returns the crate version as declared in `Cargo.toml`.
///
/// Originally added in Phase 1 as a canary for the CI pipeline before
/// any domain logic existed, kept since as a simple, stable smoke test
/// that the crate built and versioned correctly still has a real
/// caller.
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
