//! Fundamental value types shared across the order book and matching engine.
//!
//! Every numeric value that has a specific meaning, a price, a quantity,
//! an identifier, gets its own type here instead of passing raw `i64` or
//! `u64` around. The compiler then rejects code that mixes them up, a
//! quantity can never be passed where a price is expected, at zero
//! runtime cost since these are all `#[repr(transparent)]` in practice.

use serde::{Deserialize, Serialize};

/// A price expressed as an integer number of ticks.
///
/// Exchanges do not use floating point for prices, rounding error in a
/// matching engine is unacceptable. A tick is the smallest price
/// increment a symbol trades in, defined by configuration, so a price of
/// 10050 with a tick size of 0.01 represents 100.50 in the traded
/// currency. Conversion to and from a human readable decimal happens at
/// the display and parsing boundary, not inside the matching engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Price(i64);

impl Price {
    /// Creates a price from a raw tick count.
    #[must_use]
    pub const fn from_ticks(ticks: i64) -> Self {
        Self(ticks)
    }

    /// Returns the raw tick count.
    #[must_use]
    pub const fn ticks(self) -> i64 {
        self.0
    }

    /// Returns true if the price is a valid limit price, strictly positive.
    ///
    /// Zero and negative prices are rejected at order construction time,
    /// this helper exists so that validation logic in [`crate::orders`]
    /// reads as a plain boolean check instead of an inline comparison.
    #[must_use]
    pub const fn is_valid_limit_price(self) -> bool {
        self.0 > 0
    }
}

/// An order or trade quantity, always non negative.
///
/// Stored as `u64` in the smallest tradable unit of the symbol, shares,
/// contracts, satoshis, whatever the unit is. Negative quantity has no
/// meaning, using an unsigned type makes that invariant structural
/// instead of something every call site has to check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Quantity(u64);

impl Quantity {
    /// Creates a quantity from a raw unit count.
    #[must_use]
    pub const fn from_units(units: u64) -> Self {
        Self(units)
    }

    /// Returns the raw unit count.
    #[must_use]
    pub const fn units(self) -> u64 {
        self.0
    }

    /// Returns true if the quantity is greater than zero.
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Subtracts `other` from this quantity, returning `None` on underflow.
    ///
    /// Used when applying a fill to an order's remaining quantity. A
    /// checked operation is required here rather than plain subtraction,
    /// a matching bug that tries to fill more than an order has
    /// remaining must be caught, not wrap silently into a huge quantity.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(result) => Some(Self(result)),
            None => None,
        }
    }
}

/// The engine assigned identifier for an order.
///
/// Values are handed out from a single monotonically increasing counter
/// owned by the matching engine, this is what makes `OrderId` double as
/// the time priority key: comparing two `OrderId` values with `<` is
/// equivalent to asking which order arrived first. No separate sequence
/// counter is needed for FIFO ordering within a price level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OrderId(u64);

impl OrderId {
    /// Wraps a raw sequence value into an `OrderId`.
    ///
    /// Only the matching engine's ID generator should call this directly,
    /// it is not meant for constructing arbitrary IDs elsewhere.
    #[must_use]
    pub const fn from_sequence(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Returns the raw sequence value.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.0
    }
}

/// The engine assigned identifier for a trade.
///
/// Assigned from its own monotonically increasing counter, separate
/// from the one that produces [`OrderId`]. A trade is not an order,
/// using the same counter for both would tie their numbering together
/// for no reason and make either sequence harder to reason about on
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TradeId(u64);

impl TradeId {
    /// Wraps a raw sequence value into a `TradeId`.
    #[must_use]
    pub const fn from_sequence(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Returns the raw sequence value.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.0
    }
}

/// A caller supplied identifier used for duplicate detection and
/// client side order tracking.
///
/// This is separate from [`OrderId`] on purpose. [`OrderId`] is
/// assigned by the engine and used internally for time priority,
/// `ClientOrderId` is supplied by whoever submits the order and is
/// what duplicate submission checks compare against. Represented as
/// `u64` for now rather than a string, matching the CLI's expected
/// input format, revisit if a future interface needs opaque string
/// client IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientOrderId(u64);

impl ClientOrderId {
    /// Wraps a raw value supplied by the caller into a `ClientOrderId`.
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A tradable symbol, for example a ticker.
///
/// Backed by `String` rather than a fixed size buffer, simple and
/// correct for now. If profiling in the optimization phase shows symbol
/// allocation as a hot path, this is the type to revisit for a small
/// string optimization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Symbol(String);

impl Symbol {
    /// Creates a symbol from any string like input.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the symbol as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The side of the book an order or trade belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    /// A buy order, rests on the bid book.
    Buy,
    /// A sell order, rests on the ask book.
    Sell,
}

impl Side {
    /// Returns the opposite side.
    ///
    /// Used by [`crate::matching::MatchingEngine`] to find the resting
    /// orders a new order should match against, a buy order matches
    /// against the ask book, so the incoming side's opposite gives the
    /// book to search.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_valid_limit_price_rejects_non_positive() {
        assert!(Price::from_ticks(1).is_valid_limit_price());
        assert!(!Price::from_ticks(0).is_valid_limit_price());
        assert!(!Price::from_ticks(-1).is_valid_limit_price());
    }

    #[test]
    fn price_ordering_matches_tick_ordering() {
        assert!(Price::from_ticks(100) < Price::from_ticks(200));
    }

    #[test]
    fn quantity_checked_sub_rejects_underflow() {
        let five = Quantity::from_units(5);
        let three = Quantity::from_units(3);
        assert_eq!(five.checked_sub(three), Some(Quantity::from_units(2)));
        assert_eq!(three.checked_sub(five), None);
    }

    #[test]
    fn quantity_is_positive() {
        assert!(Quantity::from_units(1).is_positive());
        assert!(!Quantity::from_units(0).is_positive());
    }

    #[test]
    fn order_id_ordering_reflects_sequence_order() {
        let first = OrderId::from_sequence(1);
        let second = OrderId::from_sequence(2);
        assert!(first < second);
    }

    #[test]
    fn trade_id_ordering_reflects_sequence_order() {
        let first = TradeId::from_sequence(1);
        let second = TradeId::from_sequence(2);
        assert!(first < second);
    }

    #[test]
    fn side_opposite_is_involutive() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn symbol_as_str_roundtrips() {
        let symbol = Symbol::new("AAPL");
        assert_eq!(symbol.as_str(), "AAPL");
    }
}
