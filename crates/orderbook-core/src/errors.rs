//! Error types produced while constructing and validating orders.

use thiserror::Error;

use crate::types::Price;

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
    /// Builds a `NonPositiveLimitPrice` error from a rejected price.
    #[must_use]
    pub const fn non_positive_limit_price(price: Price) -> Self {
        Self::NonPositiveLimitPrice {
            price: price.ticks(),
        }
    }
}

/// Convenience alias for results produced while validating orders.
pub type OrderResult<T> = Result<T, OrderError>;

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
