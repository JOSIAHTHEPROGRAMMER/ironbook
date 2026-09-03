//! The [`Order`] domain type and its validating constructors.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::errors::{OrderError, OrderResult};
use crate::types::{ClientOrderId, OrderId, Price, Quantity, Side, Symbol};

/// Whether an order trades at a specified price or at whatever price is
/// currently available on the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Trades only at the specified price or better.
    Limit,
    /// Trades immediately at the best available price on the book.
    Market,
}

/// A buy or sell order, either resting on the book or in the process of
/// being matched.
///
/// Constructed only through [`Order::new_limit`] or
/// [`Order::new_market`], both validate their inputs and return
/// [`OrderError`] on invalid data, so a live `Order` can never be in
/// an invalid state, there is no path that produces a limit order
/// with a bad price or any order with zero quantity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    id: OrderId,
    client_order_id: ClientOrderId,
    symbol: Symbol,
    side: Side,
    kind: OrderType,
    price: Option<Price>,
    original_quantity: Quantity,
    remaining_quantity: Quantity,
    submitted_at: SystemTime,
}

impl Order {
    /// Checks that a price and quantity are valid for a limit order.
    ///
    /// Pulled out of [`Order::new_limit`] so
    /// [`crate::matching::MatchingEngine::modify_order`] can run the
    /// same check before touching the book, a rejected modify must
    /// never cancel the existing order first and discover the
    /// replacement is invalid only afterward, by then there would be
    /// nothing left to roll back to.
    pub(crate) fn validate_limit_inputs(price: Price, quantity: Quantity) -> OrderResult<()> {
        if !quantity.is_positive() {
            return Err(OrderError::ZeroQuantity);
        }
        if !price.is_valid_limit_price() {
            return Err(OrderError::non_positive_limit_price(price));
        }
        Ok(())
    }

    /// Builds a limit order, rejecting a non positive price or zero quantity.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::ZeroQuantity`] if `quantity` is zero, or
    /// [`OrderError::NonPositiveLimitPrice`] if `price` is zero or
    /// negative.
    pub fn new_limit(
        id: OrderId,
        client_order_id: ClientOrderId,
        symbol: Symbol,
        side: Side,
        price: Price,
        quantity: Quantity,
    ) -> OrderResult<Self> {
        Self::validate_limit_inputs(price, quantity)?;

        Ok(Self {
            id,
            client_order_id,
            symbol,
            side,
            kind: OrderType::Limit,
            price: Some(price),
            original_quantity: quantity,
            remaining_quantity: quantity,
            submitted_at: SystemTime::now(),
        })
    }

    /// Builds a market order, rejecting zero quantity.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::ZeroQuantity`] if `quantity` is zero.
    pub fn new_market(
        id: OrderId,
        client_order_id: ClientOrderId,
        symbol: Symbol,
        side: Side,
        quantity: Quantity,
    ) -> OrderResult<Self> {
        if !quantity.is_positive() {
            return Err(OrderError::ZeroQuantity);
        }

        Ok(Self {
            id,
            client_order_id,
            symbol,
            side,
            kind: OrderType::Market,
            price: None,
            original_quantity: quantity,
            remaining_quantity: quantity,
            submitted_at: SystemTime::now(),
        })
    }

    /// Returns the engine assigned identifier and time priority key.
    #[must_use]
    pub const fn id(&self) -> OrderId {
        self.id
    }

    /// Returns the caller supplied identifier used for duplicate detection.
    #[must_use]
    pub const fn client_order_id(&self) -> ClientOrderId {
        self.client_order_id
    }

    /// Returns the symbol this order trades.
    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// Returns which side of the book this order belongs to.
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    /// Returns whether this order is a limit or market order.
    #[must_use]
    pub const fn order_type(&self) -> OrderType {
        self.kind
    }

    /// Returns the limit price, `None` for market orders.
    #[must_use]
    pub const fn price(&self) -> Option<Price> {
        self.price
    }

    /// Returns the quantity the order was submitted with.
    #[must_use]
    pub const fn original_quantity(&self) -> Quantity {
        self.original_quantity
    }

    /// Returns the quantity still unfilled.
    #[must_use]
    pub const fn remaining_quantity(&self) -> Quantity {
        self.remaining_quantity
    }

    /// Returns when the order was submitted, for display and audit only.
    ///
    /// Not used for time priority, [`OrderId`] ordering serves that role.
    #[must_use]
    pub const fn submitted_at(&self) -> SystemTime {
        self.submitted_at
    }

    /// Returns true once the order has no remaining quantity.
    #[must_use]
    pub const fn is_fully_filled(&self) -> bool {
        !self.remaining_quantity.is_positive()
    }

    /// Reduces the remaining quantity by `fill_quantity`.
    ///
    /// Returns false if `fill_quantity` exceeds what remains. That case
    /// should never occur if [`crate::matching::MatchingEngine`] is
    /// correct, an overfill is an internal consistency bug, not user
    /// input to validate, so it is reported as a boolean rather than an
    /// [`OrderError`] and left to the caller to treat as seriously as
    /// it deserves.
    pub fn fill(&mut self, fill_quantity: Quantity) -> bool {
        match self.remaining_quantity.checked_sub(fill_quantity) {
            Some(new_remaining) => {
                self.remaining_quantity = new_remaining;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ids() -> (OrderId, ClientOrderId) {
        (OrderId::from_sequence(1), ClientOrderId::from_raw(100))
    }

    #[test]
    fn validate_limit_inputs_rejects_zero_quantity() {
        let result = Order::validate_limit_inputs(Price::from_ticks(100), Quantity::from_units(0));
        assert_eq!(result.unwrap_err(), OrderError::ZeroQuantity);
    }

    #[test]
    fn validate_limit_inputs_rejects_non_positive_price() {
        let result = Order::validate_limit_inputs(Price::from_ticks(0), Quantity::from_units(10));
        assert_eq!(
            result.unwrap_err(),
            OrderError::non_positive_limit_price(Price::from_ticks(0))
        );
    }

    #[test]
    fn validate_limit_inputs_accepts_valid_input() {
        let result = Order::validate_limit_inputs(Price::from_ticks(100), Quantity::from_units(10));
        assert!(result.is_ok());
    }

    #[test]
    fn new_limit_rejects_zero_quantity() {
        let (id, client_id) = sample_ids();
        let result = Order::new_limit(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Buy,
            Price::from_ticks(100),
            Quantity::from_units(0),
        );
        assert_eq!(result.unwrap_err(), OrderError::ZeroQuantity);
    }

    #[test]
    fn new_limit_rejects_non_positive_price() {
        let (id, client_id) = sample_ids();
        let result = Order::new_limit(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Buy,
            Price::from_ticks(0),
            Quantity::from_units(10),
        );
        assert_eq!(
            result.unwrap_err(),
            OrderError::non_positive_limit_price(Price::from_ticks(0))
        );
    }

    #[test]
    fn new_limit_succeeds_with_valid_input() {
        let (id, client_id) = sample_ids();
        let order = Order::new_limit(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Buy,
            Price::from_ticks(100),
            Quantity::from_units(10),
        )
        .unwrap();

        assert_eq!(order.price(), Some(Price::from_ticks(100)));
        assert_eq!(order.remaining_quantity(), order.original_quantity());
        assert_eq!(order.order_type(), OrderType::Limit);
    }

    #[test]
    fn new_market_rejects_zero_quantity() {
        let (id, client_id) = sample_ids();
        let result = Order::new_market(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Sell,
            Quantity::from_units(0),
        );
        assert_eq!(result.unwrap_err(), OrderError::ZeroQuantity);
    }

    #[test]
    fn new_market_has_no_price() {
        let (id, client_id) = sample_ids();
        let order = Order::new_market(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Sell,
            Quantity::from_units(10),
        )
        .unwrap();

        assert_eq!(order.price(), None);
    }

    #[test]
    fn fill_reduces_remaining_quantity() {
        let (id, client_id) = sample_ids();
        let mut order = Order::new_limit(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Buy,
            Price::from_ticks(100),
            Quantity::from_units(10),
        )
        .unwrap();

        assert!(order.fill(Quantity::from_units(4)));
        assert_eq!(order.remaining_quantity(), Quantity::from_units(6));
        assert!(!order.is_fully_filled());

        assert!(order.fill(Quantity::from_units(6)));
        assert_eq!(order.remaining_quantity(), Quantity::from_units(0));
        assert!(order.is_fully_filled());
    }

    #[test]
    fn fill_rejects_overfill_and_leaves_quantity_unchanged() {
        let (id, client_id) = sample_ids();
        let mut order = Order::new_limit(
            id,
            client_id,
            Symbol::new("AAPL"),
            Side::Buy,
            Price::from_ticks(100),
            Quantity::from_units(5),
        )
        .unwrap();

        assert!(!order.fill(Quantity::from_units(6)));
        assert_eq!(order.remaining_quantity(), Quantity::from_units(5));
    }
}
