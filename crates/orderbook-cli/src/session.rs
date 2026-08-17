//! Holds one matching engine per symbol for an interactive CLI session.

use std::collections::HashMap;

use orderbook_core::matching::MatchingEngine;
use orderbook_core::types::Symbol;

/// A CLI session's state: one independent matching engine per symbol.
///
/// `orderbook-core`'s `MatchingEngine` is deliberately single symbol,
/// this type is what turns that into a multi symbol session at the CLI
/// layer, without `orderbook-core` needing to know symbols exist as a
/// concept beyond a label on an order.
#[derive(Debug, Default)]
pub struct Session {
    engines: HashMap<Symbol, MatchingEngine>,
}

impl Session {
    /// Creates an empty session with no symbols traded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the engine for a symbol, creating an empty one on first use.
    pub fn engine_mut(&mut self, symbol: &Symbol) -> &mut MatchingEngine {
        self.engines
            .entry(symbol.clone())
            .or_insert_with(|| MatchingEngine::new(symbol.clone()))
    }

    /// Returns the engine for a symbol if it has been created, without
    /// creating one.
    ///
    /// Used by read only commands, looking at the book for a symbol
    /// that has never traded should report an empty book, not silently
    /// create an engine for it.
    #[must_use]
    pub fn engine(&self, symbol: &Symbol) -> Option<&MatchingEngine> {
        self.engines.get(symbol)
    }

    /// Returns a mutable reference to a symbol's engine if it exists,
    /// without creating one.
    ///
    /// Used by cancel and modify, an order can only exist if its symbol
    /// already has an engine, using `engine_mut` here would leave an
    /// orphan empty engine behind every time someone tried to cancel an
    /// order on a symbol that was never traded.
    pub fn engine_mut_if_exists(&mut self, symbol: &Symbol) -> Option<&mut MatchingEngine> {
        self.engines.get_mut(symbol)
    }

    /// Removes every symbol's engine, returning the session to empty.
    pub fn reset(&mut self) {
        self.engines.clear();
    }

    /// Returns the number of symbols with an engine in this session.
    #[must_use]
    pub fn symbol_count(&self) -> usize {
        self.engines.len()
    }
}

#[cfg(test)]
mod tests {
    use orderbook_core::types::{ClientOrderId, Price, Quantity, Side};

    use super::*;

    #[test]
    fn engine_mut_creates_an_engine_on_first_use() {
        let mut session = Session::new();
        assert_eq!(session.symbol_count(), 0);

        session.engine_mut(&Symbol::new("AAPL"));
        assert_eq!(session.symbol_count(), 1);
    }

    #[test]
    fn engine_mut_reuses_the_existing_engine_for_the_same_symbol() {
        let mut session = Session::new();
        let symbol = Symbol::new("AAPL");

        session
            .engine_mut(&symbol)
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        assert_eq!(session.engine_mut(&symbol).book().order_count(), 1);
        assert_eq!(session.symbol_count(), 1);
    }

    #[test]
    fn engine_returns_none_for_a_symbol_never_traded() {
        let session = Session::new();
        assert!(session.engine(&Symbol::new("AAPL")).is_none());
    }

    #[test]
    fn engine_mut_if_exists_returns_none_without_creating_an_engine() {
        let mut session = Session::new();
        assert!(session.engine_mut_if_exists(&Symbol::new("AAPL")).is_none());
        assert_eq!(session.symbol_count(), 0);
    }

    #[test]
    fn engine_mut_if_exists_returns_the_engine_once_it_has_traded() {
        let mut session = Session::new();
        let symbol = Symbol::new("AAPL");
        session.engine_mut(&symbol);

        assert!(session.engine_mut_if_exists(&symbol).is_some());
    }

    #[test]
    fn different_symbols_get_independent_engines() {
        let mut session = Session::new();
        session
            .engine_mut(&Symbol::new("AAPL"))
            .submit_limit_order(
                ClientOrderId::from_raw(1),
                Side::Buy,
                Price::from_ticks(100),
                Quantity::from_units(10),
            )
            .unwrap();

        assert_eq!(session.symbol_count(), 1);
        assert!(session.engine(&Symbol::new("MSFT")).is_none());
    }

    #[test]
    fn reset_clears_all_engines() {
        let mut session = Session::new();
        session.engine_mut(&Symbol::new("AAPL"));
        session.engine_mut(&Symbol::new("MSFT"));
        assert_eq!(session.symbol_count(), 2);

        session.reset();
        assert_eq!(session.symbol_count(), 0);
    }
}
