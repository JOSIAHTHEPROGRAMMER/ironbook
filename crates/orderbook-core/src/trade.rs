//! The `Trade` domain type produced when the matching engine fills orders.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::types::{OrderId, Price, Quantity, Side, Symbol, TradeId};

/// A completed match between two orders.
///
/// Built only by the matching engine from values already validated when
/// the underlying orders were constructed, so `Trade::new` takes no
/// fallible path, there is nothing left to check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    id: TradeId,
    symbol: Symbol,
    price: Price,
    quantity: Quantity,
    maker_order_id: OrderId,
    taker_order_id: OrderId,
    taker_side: Side,
    executed_at: SystemTime,
}

impl Trade {
    /// Builds a trade record for a fill between a resting maker order
    /// and an incoming taker order.
    ///
    /// The maker order is the one that was already resting on the book
    /// and provided liquidity, the taker order is the incoming one that
    /// matched against it and removed that liquidity. `taker_side` is
    /// the side of the incoming order, the maker's side is always its
    /// opposite.
    #[must_use]
    pub fn new(
        id: TradeId,
        symbol: Symbol,
        price: Price,
        quantity: Quantity,
        maker_order_id: OrderId,
        taker_order_id: OrderId,
        taker_side: Side,
    ) -> Self {
        Self {
            id,
            symbol,
            price,
            quantity,
            maker_order_id,
            taker_order_id,
            taker_side,
            executed_at: SystemTime::now(),
        }
    }

    /// Returns the engine assigned identifier for this trade.
    #[must_use]
    pub const fn id(&self) -> TradeId {
        self.id
    }

    /// Returns the symbol this trade occurred on.
    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// Returns the price the trade executed at.
    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }

    /// Returns the quantity that changed hands.
    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    /// Returns the identifier of the resting order that provided liquidity.
    #[must_use]
    pub const fn maker_order_id(&self) -> OrderId {
        self.maker_order_id
    }

    /// Returns the identifier of the incoming order that removed liquidity.
    #[must_use]
    pub const fn taker_order_id(&self) -> OrderId {
        self.taker_order_id
    }

    /// Returns the side of the incoming order that triggered this trade.
    #[must_use]
    pub const fn taker_side(&self) -> Side {
        self.taker_side
    }

    /// Returns the side of the resting order, always opposite the taker.
    #[must_use]
    pub const fn maker_side(&self) -> Side {
        self.taker_side.opposite()
    }

    /// Returns when this trade executed.
    #[must_use]
    pub const fn executed_at(&self) -> SystemTime {
        self.executed_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trade() -> Trade {
        Trade::new(
            TradeId::from_sequence(1),
            Symbol::new("AAPL"),
            Price::from_ticks(100),
            Quantity::from_units(10),
            OrderId::from_sequence(1),
            OrderId::from_sequence(2),
            Side::Buy,
        )
    }

    #[test]
    fn maker_side_is_opposite_of_taker_side() {
        let trade = sample_trade();
        assert_eq!(trade.taker_side(), Side::Buy);
        assert_eq!(trade.maker_side(), Side::Sell);
    }

    #[test]
    fn fields_round_trip_through_accessors() {
        let trade = sample_trade();
        assert_eq!(trade.id(), TradeId::from_sequence(1));
        assert_eq!(trade.price(), Price::from_ticks(100));
        assert_eq!(trade.quantity(), Quantity::from_units(10));
        assert_eq!(trade.maker_order_id(), OrderId::from_sequence(1));
        assert_eq!(trade.taker_order_id(), OrderId::from_sequence(2));
    }
}
