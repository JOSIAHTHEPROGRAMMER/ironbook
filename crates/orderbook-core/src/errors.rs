//! Error types produced while constructing and validating orders.

use thiserror::Error;

use crate::types::{ClientOrderId, OrderId, Price};

/// Reasons an order fails validation at construction time.
///
/// Marked non exhaustive because future order types, stop orders and
/// iceberg orders, will add their own rejection cases. Matching on this
/// enum from outside the crate should always include a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OrderError {
    /// A limit order was constructed with a price that is zero or negative.
    #[error("limit price must be positive, got {price} ticks")]
    NonPositiveLimitPrice {
        /// The rejected price, included so the caller can log or display it.
        price: i64,
    },

    /// An order was constructed with zero quantity.
    #[error("order quantity must be greater than zero")]
    ZeroQuantity,

    /// A market order was constructed with a price attached.
    ///
    /// Market orders take whatever price is available on the book, a
    /// price on a market order is not a mistake worth silently ignoring,
    /// it usually means the caller mixed up limit and market order
    /// construction.
    #[error("market orders cannot specify a price")]
    PriceOnMarketOrder,
}

impl OrderError {
    /// Builds a [`OrderError::NonPositiveLimitPrice`] error from a
    /// rejected price.
    #[must_use]
    pub const fn non_positive_limit_price(price: Price) -> Self {
        Self::NonPositiveLimitPrice {
            price: price.ticks(),
        }
    }
}

/// Convenience alias for results produced while validating orders.
pub type OrderResult<T> = Result<T, OrderError>;

/// Reasons an operation on the order book fails.
///
/// Marked non exhaustive for the same reason as [`OrderError`], future
/// order types will likely add rejection cases specific to resting
/// orders, an iceberg order revealing its next slice, for example.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OrderBookError {
    /// The client order id is already resting on the book.
    #[error("client order id {0:?} is already resting on the book")]
    DuplicateClientOrderId(ClientOrderId),

    /// The order id does not exist on the book.
    #[error("no resting order found with id {0:?}")]
    UnknownOrder(OrderId),

    /// A market order was passed to an operation that only accepts
    /// resting orders.
    ///
    /// Market orders match immediately against the book and never rest
    /// on it, inserting one here would leave it stuck as a phantom
    /// limit order with no price, which [`crate::matching::MatchingEngine`]
    /// could never clean up correctly.
    #[error("market orders cannot be inserted into the book")]
    MarketOrderCannotRest,
}

/// Convenience alias for results produced by order book operations.
pub type OrderBookResult<T> = Result<T, OrderBookError>;

/// Reasons the matching engine rejects an incoming order.
///
/// Wraps [`OrderError`] and [`OrderBookError`] rather than
/// redeclaring their variants, an order can be rejected either
/// because the order itself is invalid or because its client order id
/// collides with a resting order, both are already fully described by
/// the layers below, this type just lets
/// [`crate::matching::MatchingEngine`] return one error type to its
/// caller.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MatchingError {
    /// The order failed validation at construction time.
    #[error(transparent)]
    InvalidOrder(#[from] OrderError),

    /// The order could not be placed on the book.
    #[error(transparent)]
    OrderBook(#[from] OrderBookError),
}

/// Convenience alias for results produced by the matching engine.
pub type MatchingResult<T> = Result<T, MatchingError>;

#[cfg(test)]
mod matching_error_tests {
    use super::*;

    #[test]
    fn invalid_order_wraps_the_underlying_message() {
        let error: MatchingError = OrderError::ZeroQuantity.into();
        assert_eq!(
            error.to_string(),
            "order quantity must be greater than zero"
        );
    }

    #[test]
    fn order_book_wraps_the_underlying_message() {
        let error: MatchingError = OrderBookError::MarketOrderCannotRest.into();
        assert_eq!(
            error.to_string(),
            "market orders cannot be inserted into the book"
        );
    }
}

#[cfg(test)]
mod order_book_error_tests {
    use super::*;

    #[test]
    fn duplicate_client_order_id_message_includes_the_id() {
        let error = OrderBookError::DuplicateClientOrderId(ClientOrderId::from_raw(7));
        assert!(error.to_string().contains('7'));
    }

    #[test]
    fn unknown_order_message_includes_the_id() {
        let error = OrderBookError::UnknownOrder(OrderId::from_sequence(42));
        assert!(error.to_string().contains("42"));
    }

    #[test]
    fn market_order_cannot_rest_has_a_fixed_message() {
        let error = OrderBookError::MarketOrderCannotRest;
        assert_eq!(
            error.to_string(),
            "market orders cannot be inserted into the book"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_positive_limit_price_message_includes_the_rejected_value() {
        let error = OrderError::non_positive_limit_price(Price::from_ticks(-5));
        assert_eq!(
            error.to_string(),
            "limit price must be positive, got -5 ticks"
        );
    }

    #[test]
    fn zero_quantity_has_a_fixed_message() {
        let error = OrderError::ZeroQuantity;
        assert_eq!(
            error.to_string(),
            "order quantity must be greater than zero"
        );
    }

    #[test]
    fn price_on_market_order_has_a_fixed_message() {
        let error = OrderError::PriceOnMarketOrder;
        assert_eq!(error.to_string(), "market orders cannot specify a price");
    }
}
